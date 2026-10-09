// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Descriptor-relative preparation for command-owned workload fixtures.

use rustix::fs::{AtFlags, Mode, OFlags, fchmod, mkdirat, openat, unlinkat};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

#[derive(Debug)]
pub(crate) struct OwnedDirectory {
    pub(crate) file: File,
}

pub(crate) fn prepare_absolute(path: &Path, mode: u32) -> io::Result<OwnedDirectory> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe absolute path",
        ));
    }
    let mut current = File::open("/")?;
    let mut created = Vec::<(File, std::ffi::OsString)>::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let next = match openat(
            &current,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(fd) => File::from(fd),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let parent = match current.try_clone() {
                    Ok(parent) => parent,
                    Err(error) => {
                        rollback(&created);
                        return Err(error);
                    }
                };
                match mkdirat(&current, name, Mode::from(mode)) {
                    Ok(()) => created.push((parent, name.to_owned())),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => {
                        rollback(&created);
                        return Err(error.into());
                    }
                }
                match openat(
                    &current,
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                    Mode::empty(),
                ) {
                    Ok(fd) => {
                        let next = File::from(fd);
                        if let Err(error) = fchmod(&next, Mode::from(mode)) {
                            rollback(&created);
                            return Err(error.into());
                        }
                        next
                    }
                    Err(error) => {
                        rollback(&created);
                        return Err(error.into());
                    }
                }
            }
            Err(error) => {
                rollback(&created);
                return Err(error.into());
            }
        };
        current = next;
    }
    Ok(OwnedDirectory { file: current })
}

pub(crate) fn prepare_child(parent: &File, name: &OsStr, mode: u32) -> io::Result<OwnedDirectory> {
    if name.is_empty()
        || name
            .as_bytes()
            .iter()
            .any(|byte| *byte == b'/' || *byte == 0)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsafe directory name",
        ));
    }
    let (next, created) = match openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    ) {
        Ok(fd) => (File::from(fd), false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let created = match mkdirat(parent, name, Mode::from(mode)) {
                Ok(()) => true,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
                Err(error) => return Err(error.into()),
            };
            let next = match openat(
                parent,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            ) {
                Ok(fd) => File::from(fd),
                Err(error) => {
                    if created {
                        let _ = unlinkat(parent, name, AtFlags::REMOVEDIR);
                    }
                    return Err(error.into());
                }
            };
            (next, created)
        }
        Err(error) => return Err(error.into()),
    };
    if let Err(error) = fchmod(&next, Mode::from(mode)) {
        if created {
            let _ = unlinkat(parent, name, AtFlags::REMOVEDIR);
        }
        return Err(error.into());
    }
    Ok(OwnedDirectory { file: next })
}

pub(crate) fn write_new(parent: &File, name: &OsStr, bytes: &[u8]) -> io::Result<()> {
    let fd = openat(
        parent,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from(0o600),
    )?;
    let mut file = File::from(fd);
    if file.write_all(bytes).is_err() || file.sync_all().is_err() {
        let _ = unlinkat(parent, name, AtFlags::empty());
        return Err(io::Error::other("workload output cannot be written"));
    }
    parent.sync_all()
}

fn rollback(created: &[(File, std::ffi::OsString)]) {
    for (parent, name) in created.iter().rev() {
        let _ = unlinkat(parent, name, AtFlags::REMOVEDIR);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn concurrent_child_creation_reopens_the_winner_without_following_a_link() {
        let root = std::env::temp_dir().join(format!(
            "asb-safe-fs-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let parent = prepare_absolute(&root, 0o700).expect("root directory");
        let barrier = Arc::new(Barrier::new(8));
        let mut workers = Vec::new();
        for _ in 0..8 {
            let barrier = Arc::clone(&barrier);
            let descriptor = parent.file.try_clone().expect("parent descriptor");
            workers.push(thread::spawn(move || {
                barrier.wait();
                prepare_child(&descriptor, OsStr::new("child"), 0o700).is_ok()
            }));
        }
        assert!(workers.into_iter().all(|worker| worker.join().unwrap()));
        let metadata = fs::symlink_metadata(root.join("child")).expect("child");
        assert!(metadata.is_dir());
        assert!(!metadata.file_type().is_symlink());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn file_parent_fails_without_creating_children() {
        let root = std::env::temp_dir().join(format!(
            "asb-safe-fs-file-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::write(&root, b"not a directory").expect("file parent");
        let child = root.join("child");
        assert!(prepare_absolute(&child, 0o700).is_err());
        assert!(!child.exists());
        fs::remove_file(root).expect("cleanup");
    }
}
