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
                let parent = current.try_clone()?;
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
                        fchmod(&next, Mode::from(mode))?;
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
    let next = match openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    ) {
        Ok(fd) => File::from(fd),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            mkdirat(parent, name, Mode::from(mode))?;
            File::from(openat(
                parent,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )?)
        }
        Err(error) => return Err(error.into()),
    };
    fchmod(&next, Mode::from(mode))?;
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
