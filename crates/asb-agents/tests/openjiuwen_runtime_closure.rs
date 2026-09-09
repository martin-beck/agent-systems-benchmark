// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
#![allow(missing_docs)]
#![cfg(target_os = "linux")]

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const PROVENANCE: &str = include_str!("fixtures/openjiuwen-provenance.json");
const LOCK: &str = include_str!("fixtures/openjiuwen-runtime.lock");
const WHEEL_NAME: &str = "openjiuwen-0.1.17.post1-py3-none-any.whl";
const WHEEL_SHA256: &str = "21e9479c6b858cda28c250d63066f862fc0915cf2039edb00f016cbec7f9abba";
const WHEEL_SIZE: u64 = 5_850_168;
const UV_SHA256: &str = "085e6be0fbb5f63c7ba39829703a7229cd62d2bd0b78ae145da9bf897e0fc007";
const INVENTORY_SHA256: &str = "05ac9dae97b398b18f2d466cda559ad6cde0485b7e449cf4772a545935debeab";
const PRIVATE_SENTINEL: &str = "asb-private-runtime-closure-sentinel";

#[derive(Clone, Debug, Eq, PartialEq)]
struct LockedPackage {
    name: String,
    version: String,
    hashes: Vec<String>,
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "asb-openjiuwen-closure-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("wall clock precedes epoch")
                .as_nanos()
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("create private scratch directory");
        Self(path)
    }

    fn child(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn parse_lock(lock: &str) -> Result<Vec<LockedPackage>, &'static str> {
    if !lock.ends_with('\n') {
        return Err("lock must end with newline");
    }
    let lines: Vec<_> = lock.lines().collect();
    let mut packages = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if line.is_empty() || line.starts_with('#') {
            index += 1;
            continue;
        }
        if line.starts_with(' ') || !line.ends_with(" \\") {
            return Err("unexpected lock line");
        }
        let requirement = &line[..line.len() - 2];
        let (name, version) = requirement
            .split_once("==")
            .ok_or("floating or malformed requirement")?;
        let valid_name = !name.is_empty()
            && name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_.".contains(&byte)
            });
        let valid_version = !version.is_empty()
            && !version.contains("==")
            && version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b".!+-_".contains(&byte));
        if !valid_name || !valid_version {
            return Err("invalid package identity");
        }
        if packages
            .last()
            .is_some_and(|prior: &LockedPackage| prior.name.as_str() >= name)
        {
            return Err("duplicate or unsorted package");
        }
        index += 1;
        let mut hashes = Vec::new();
        while index < lines.len() && lines[index].starts_with("    --hash=sha256:") {
            let raw = lines[index].trim();
            let continued = raw.ends_with(" \\");
            let without_prefix = raw.strip_prefix("--hash=sha256:").expect("prefix checked");
            let hash = without_prefix.strip_suffix(" \\").unwrap_or(without_prefix);
            if hash.len() != 64
                || !hash
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err("invalid artifact hash");
            }
            if hashes
                .last()
                .is_some_and(|prior: &String| prior.as_str() >= hash)
            {
                return Err("duplicate or unsorted artifact hash");
            }
            hashes.push(hash.to_owned());
            index += 1;
            let another_hash =
                index < lines.len() && lines[index].starts_with("    --hash=sha256:");
            if continued != another_hash {
                return Err("invalid hash continuation");
            }
        }
        if hashes.is_empty() {
            return Err("package has no artifact hash");
        }
        packages.push(LockedPackage {
            name: name.to_owned(),
            version: version.to_owned(),
            hashes,
        });
        while index < lines.len() && lines[index].starts_with("    #") {
            index += 1;
        }
    }
    Ok(packages)
}

fn inventory_bytes(packages: &[LockedPackage]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for package in packages {
        bytes.extend_from_slice(package.name.as_bytes());
        bytes.extend_from_slice(b"==");
        bytes.extend_from_slice(package.version.as_bytes());
        bytes.push(b'|');
        bytes.extend_from_slice(package.hashes.join(",").as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

fn validate(provenance: &str, lock: &str) -> Result<Vec<LockedPackage>, &'static str> {
    let value: Value = serde_json::from_str(provenance).map_err(|_| "invalid provenance JSON")?;
    let package = &value["package"];
    if package["sha256"] != WHEEL_SHA256 || package["size_bytes"] != WHEEL_SIZE {
        return Err("wheel identity drift");
    }
    if package["runtime_lock_sha256"] != digest(lock.as_bytes()) {
        return Err("lock digest drift");
    }
    let packages = parse_lock(lock)?;
    if packages.len() != 170 || digest(&inventory_bytes(&packages)) != INVENTORY_SHA256 {
        return Err("closed package oracle drift");
    }
    let resolution = &package["runtime_lock_resolution"];
    if resolution["resolver"] != "uv"
        || resolution["resolver_version"] != "0.9.28"
        || resolution["resolver_sha256"] != UV_SHA256
        || resolution["requirement"] != "openjiuwen[cli,observability]==0.1.17.post1"
        || resolution["python_version"] != "3.11"
        || resolution["python_platform"] != "x86_64-unknown-linux-gnu"
        || resolution["package_count"] != packages.len()
        || resolution["inventory_sha256"] != INVENTORY_SHA256
        || resolution["required_extras"] != serde_json::json!(["cli", "observability"])
        || resolution["prompt_toolkit_version"] != "3.0.53"
        || resolution["opentelemetry_sdk_version"] != "1.44.0"
    {
        return Err("resolution identity drift");
    }
    Ok(packages)
}

fn provenance_for_lock(lock: &str) -> String {
    let current = digest(LOCK.as_bytes());
    PROVENANCE.replacen(&current, &digest(lock.as_bytes()), 1)
}

fn mutate_package(lock: &str, package: &LockedPackage, field: &str) -> String {
    match field {
        "name" => lock.replacen(
            &format!("{}=={} \\", package.name, package.version),
            &format!("tampered-{}=={} \\", package.name, package.version),
            1,
        ),
        "version" => lock.replacen(
            &format!("{}=={} \\", package.name, package.version),
            &format!("{}=={}.tampered \\", package.name, package.version),
            1,
        ),
        "hash" => {
            let hash = &package.hashes[0];
            let replacement = format!(
                "{}{}",
                if hash.starts_with('0') { '1' } else { '0' },
                &hash[1..]
            );
            lock.replacen(hash, &replacement, 1)
        }
        _ => unreachable!(),
    }
}

fn assert_private_free(output: &Output) {
    assert!(output.stdout.len() + output.stderr.len() <= 64 * 1024);
    let combined = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(!combined.contains(PRIVATE_SENTINEL));
}

fn checked_artifact(path: &Path, name: &str, sha256: &str, size: Option<u64>) -> PathBuf {
    assert_eq!(path.file_name().and_then(|part| part.to_str()), Some(name));
    let metadata = fs::symlink_metadata(path).expect("artifact metadata");
    assert!(metadata.file_type().is_file() && !metadata.file_type().is_symlink());
    if let Some(expected) = size {
        assert_eq!(metadata.size(), expected);
    }
    let canonical = path.canonicalize().expect("canonical artifact path");
    assert_eq!(
        digest(&fs::read(&canonical).expect("read artifact")),
        sha256
    );
    canonical
}

fn uv_command(uv: &Path, scratch: &Path, args: &[&str], deny_network: bool) -> Output {
    let mut command = Command::new(uv);
    command
        .env_clear()
        .current_dir(scratch)
        .env("HOME", scratch)
        .env("PYTHONNOUSERSITE", "1")
        .env("OPENAI_API_KEY", PRIVATE_SENTINEL)
        .args(args);
    if deny_network {
        command
            .env("UV_OFFLINE", "1")
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .env("HTTP_PROXY", "http://127.0.0.1:9");
    }
    let output = command.output().expect("execute pinned uv");
    assert_private_free(&output);
    output
}

fn install(
    uv: &Path,
    root: &Path,
    cache: &Path,
    wheelhouse: &Path,
    wheel_requirement: &Path,
    lock: &Path,
    offline: bool,
) -> PathBuf {
    let env = root.join(if offline { "offline-env" } else { "online-env" });
    let created = uv_command(
        uv,
        root,
        &[
            "venv",
            "--python",
            "3.11",
            env.to_str().expect("UTF-8 path"),
        ],
        offline,
    );
    assert!(
        created.status.success(),
        "fresh environment creation failed"
    );
    let python = env.join("bin/python");
    let wheel_installed = uv_command(
        uv,
        root,
        &[
            "pip",
            "install",
            "--python",
            python.to_str().expect("UTF-8 path"),
            "--no-deps",
            "--no-index",
            "--find-links",
            wheelhouse.to_str().expect("UTF-8 path"),
            "--only-binary",
            ":all:",
            "--require-hashes",
            "--cache-dir",
            cache.to_str().expect("UTF-8 path"),
            "-r",
            wheel_requirement.to_str().expect("UTF-8 path"),
        ],
        true,
    );
    assert!(
        wheel_installed.status.success(),
        "exact wheel installation failed"
    );
    let mut args = vec![
        "pip",
        "sync",
        "--python",
        python.to_str().expect("UTF-8 path"),
        "--require-hashes",
        "--only-binary",
        ":all:",
        "--cache-dir",
        cache.to_str().expect("UTF-8 path"),
    ];
    if offline {
        args.push("--offline");
    }
    args.push(lock.to_str().expect("UTF-8 path"));
    let synchronized = uv_command(uv, root, &args, offline);
    assert!(
        synchronized.status.success(),
        "hash-required synchronization failed"
    );
    env
}

fn runtime_inventory(python: &Path, env: &Path) -> Vec<String> {
    let script = r#"
import importlib.metadata as metadata
import json, pathlib, site, sys
prefix = pathlib.Path(sys.prefix).resolve()
assert prefix == pathlib.Path(sys.argv[1]).resolve()
assert pathlib.Path(sys.executable).absolute().is_relative_to(prefix)
assert all(pathlib.Path(p).resolve().is_relative_to(prefix) for p in site.getsitepackages())
assert site.ENABLE_USER_SITE is False
dist = metadata.distribution("openjiuwen")
entry = [e for e in dist.entry_points if e.group == "console_scripts" and e.name == "openjiuwen"]
assert len(entry) == 1 and entry[0].value == "openjiuwen.harness.cli.cli:cli"
import prompt_toolkit, opentelemetry.sdk
import openjiuwen.harness.cli.ui.runner, openjiuwen.harness.cli.cli
normalize = lambda name: name.lower().replace("_", "-").replace(".", "-")
print(json.dumps(sorted(normalize(d.metadata["Name"]) + "==" + d.version for d in metadata.distributions())))
"#;
    let output = Command::new(python)
        .env_clear()
        .current_dir(env)
        .env("HOME", env)
        .env("PYTHONNOUSERSITE", "1")
        .env("OPENAI_API_KEY", PRIVATE_SENTINEL)
        .args(["-I", "-c", script, env.to_str().expect("UTF-8 path")])
        .output()
        .expect("inspect installed runtime");
    assert_private_free(&output);
    assert!(
        output.status.success(),
        "runtime metadata/import validation failed"
    );
    let inventory = output
        .stdout
        .split(|byte| *byte == b'\n')
        .rev()
        .find(|line| !line.is_empty())
        .expect("inventory output");
    serde_json::from_slice(inventory).expect("bounded inventory JSON")
}

#[test]
fn provenance_binds_closed_cli_observability_oracle() {
    let packages = validate(PROVENANCE, LOCK).expect("canonical closure");
    assert_eq!(packages.len(), 170);
    assert_eq!(digest(&inventory_bytes(&packages)), INVENTORY_SHA256);
}

#[test]
fn every_package_identity_and_hash_mutation_fails_closed() {
    let packages = validate(PROVENANCE, LOCK).expect("canonical closure");
    for package in &packages {
        for field in ["name", "version", "hash"] {
            let mutated = mutate_package(LOCK, package, field);
            let expected = match parse_lock(&mutated) {
                Ok(_) => "closed package oracle drift",
                Err(error) => error,
            };
            assert_eq!(
                validate(&provenance_for_lock(&mutated), &mutated),
                Err(expected)
            );
        }
    }
}

#[test]
fn structural_resolution_mutations_fail_closed() {
    let packages = parse_lock(LOCK).expect("canonical lock");
    let first = &packages[0];
    let first_header = format!("{}=={} \\", first.name, first.version);
    let mutations = [
        LOCK.replacen(
            &first_header,
            &format!("{}>={} \\", first.name, first.version),
            1,
        ),
        LOCK.replacen(&first_header, "", 1),
        format!(
            "{LOCK}{first_header}\n    --hash=sha256:{}\n",
            first.hashes[0]
        ),
        LOCK.replacen(
            &format!("    --hash=sha256:{} \\\n", first.hashes[0]),
            "",
            1,
        ),
        LOCK.replacen(
            WHEEL_SHA256,
            "c0e4ca7723e2abab3e6cc79eee7c10b26d6f725f8bbee083d9a573d6f92271d0",
            1,
        ),
    ];
    for lock in mutations {
        assert!(validate(&provenance_for_lock(&lock), &lock).is_err());
    }
    for (from, to) in [
        (
            "openjiuwen[cli,observability]==0.1.17.post1",
            "openjiuwen==0.1.17.post1",
        ),
        (
            INVENTORY_SHA256,
            "15ac9dae97b398b18f2d466cda559ad6cde0485b7e449cf4772a545935debeab",
        ),
        (
            UV_SHA256,
            "185e6be0fbb5f63c7ba39829703a7229cd62d2bd0b78ae145da9bf897e0fc007",
        ),
    ] {
        assert!(validate(&PROVENANCE.replacen(from, to, 1), LOCK).is_err());
    }
}

#[test]
fn arbitrary_artifacts_are_rejected() {
    let result = std::panic::catch_unwind(|| {
        checked_artifact(
            Path::new("/bin/true"),
            WHEEL_NAME,
            WHEEL_SHA256,
            Some(WHEEL_SIZE),
        )
    });
    assert!(result.is_err());
}

#[test]
#[ignore = "performs fresh exact online and hash-required offline OpenJiuwen installations"]
fn fresh_online_and_offline_runtime_closure() {
    let uv = checked_artifact(
        &PathBuf::from(std::env::var_os("ASB_OPENJIUWEN_UV").expect("pinned uv path")),
        "uv",
        UV_SHA256,
        None,
    );
    let wheel = checked_artifact(
        &PathBuf::from(std::env::var_os("ASB_OPENJIUWEN_WHEEL").expect("exact wheel path")),
        WHEEL_NAME,
        WHEEL_SHA256,
        Some(WHEEL_SIZE),
    );
    let scratch = Scratch::new();
    let lock = scratch.child("openjiuwen-runtime.lock");
    fs::write(&lock, LOCK).expect("write exact lock");
    let cache = scratch.child("cache");
    fs::create_dir(&cache).expect("create fresh cache");
    let wheelhouse = scratch.child("wheelhouse");
    fs::create_dir(&wheelhouse).expect("create wheelhouse");
    fs::copy(&wheel, wheelhouse.join(WHEEL_NAME)).expect("stage exact wheel");
    let wheel_requirement = scratch.child("openjiuwen-wheel-requirement.txt");
    fs::write(
        &wheel_requirement,
        format!("openjiuwen==0.1.17.post1 --hash=sha256:{WHEEL_SHA256}\n"),
    )
    .expect("write wheel-only requirement");

    let online = install(
        &uv,
        &scratch.0,
        &cache,
        &wheelhouse,
        &wheel_requirement,
        &lock,
        false,
    );
    let offline = install(
        &uv,
        &scratch.0,
        &cache,
        &wheelhouse,
        &wheel_requirement,
        &lock,
        true,
    );
    let mut expected: Vec<_> = parse_lock(LOCK)
        .expect("canonical closure")
        .into_iter()
        .map(|package| format!("{}=={}", package.name, package.version))
        .collect();
    expected.sort();
    let online_inventory = runtime_inventory(&online.join("bin/python"), &online);
    let offline_inventory = runtime_inventory(&offline.join("bin/python"), &offline);
    assert_eq!(online_inventory, expected);
    assert_eq!(offline_inventory, expected);

    let wheel_check = Command::new(offline.join("bin/python"))
        .env_clear()
        .current_dir(&scratch.0)
        .args([
            "-I",
            "-c",
            r#"import base64,csv,hashlib,io,pathlib,sys,zipfile
p=pathlib.Path(sys.argv[1])
with zipfile.ZipFile(p) as z:
 names=z.namelist()
 meta=[n for n in names if n.endswith('.dist-info/METADATA')]
 entry=[n for n in names if n.endswith('.dist-info/entry_points.txt')]
 assert len(meta)==1 and len(entry)==1
 m=z.read(meta[0]).decode(); e=z.read(entry[0]).decode()
 assert '\nName: openjiuwen\n' in '\n'+m and '\nVersion: 0.1.17.post1\n' in '\n'+m
 assert 'openjiuwen = openjiuwen.harness.cli.cli:cli' in e
 rows=list(csv.reader(io.StringIO(z.read([n for n in names if n.endswith('.dist-info/RECORD')][0]).decode())))
 import importlib.metadata as metadata
 dist=metadata.distribution('openjiuwen')
 for name, encoded, size in rows:
  if not encoded:
   continue
  installed=pathlib.Path(dist.locate_file(name))
  assert installed.is_file() and installed.stat().st_size == int(size)
  algorithm, expected=encoded.split('=', 1)
  assert algorithm == 'sha256'
  actual=base64.urlsafe_b64encode(hashlib.sha256(installed.read_bytes()).digest()).rstrip(b'=').decode()
  assert actual == expected
"#,
            wheel.to_str().expect("UTF-8 wheel path"),
        ])
        .output()
        .expect("inspect wheel metadata");
    assert_private_free(&wheel_check);
    assert!(wheel_check.status.success(), "wheel metadata mismatch");

    let executable = offline.join("bin/openjiuwen");
    assert!(executable.is_file());
    for args in [&["--version"][..], &["--help"][..], &["run", "--help"][..]] {
        let output = Command::new(&executable)
            .env_clear()
            .current_dir(&scratch.0)
            .env("HOME", &scratch.0)
            .env("PYTHONNOUSERSITE", "1")
            .env("OPENAI_API_KEY", PRIVATE_SENTINEL)
            .args(args)
            .output()
            .expect("execute installed console script");
        assert_private_free(&output);
        assert!(output.status.success(), "entrypoint command failed");
    }

    let corrupt_wheelhouse = scratch.child("corrupt-wheelhouse");
    fs::create_dir(&corrupt_wheelhouse).expect("create corrupt wheelhouse");
    let mut corrupt_wheel = fs::read(&wheel).expect("read consumed wheel");
    let corrupt_offset = corrupt_wheel.len() / 2;
    corrupt_wheel[corrupt_offset] ^= 1;
    fs::write(corrupt_wheelhouse.join(WHEEL_NAME), corrupt_wheel)
        .expect("corrupt consumed wheel byte");
    let partial_cache = scratch.child("partial-cache");
    fs::create_dir(&partial_cache).expect("create partial cache");
    let partial_env = scratch.child("corrupt-env");
    let partial_created = uv_command(
        &uv,
        &scratch.0,
        &[
            "venv",
            "--python",
            "3.11",
            partial_env.to_str().expect("UTF-8 path"),
        ],
        true,
    );
    assert!(
        partial_created.status.success(),
        "offline interpreter cache missing"
    );
    let failed = uv_command(
        &uv,
        &scratch.0,
        &[
            "pip",
            "install",
            "--python",
            partial_env.join("bin/python").to_str().expect("UTF-8 path"),
            "--no-deps",
            "--no-index",
            "--find-links",
            corrupt_wheelhouse.to_str().expect("UTF-8 path"),
            "--only-binary",
            ":all:",
            "--require-hashes",
            "--offline",
            "--cache-dir",
            partial_cache.to_str().expect("UTF-8 path"),
            "-r",
            wheel_requirement.to_str().expect("UTF-8 path"),
        ],
        true,
    );
    assert!(
        !failed.status.success(),
        "corrupt consumed wheel must fail without network fallback"
    );
    assert!(
        !partial_env
            .join("lib/python3.11/site-packages/openjiuwen")
            .exists(),
        "failed wheel must not leave an installed package"
    );
}
