// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Fail-closed cache boundary for the deterministic TLA+ artifact.

use std::collections::BTreeSet;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

const RUNNER: &str = include_str!("../run_temporal_models.sh");
const BUILD: &str = include_str!("../tla-provenance/build.sh");
const PINS: &str = include_str!("../toolchains.toml");
const POSITIVE: &str = include_str!("../fixtures/tla-artifact-positive.json");
const MUTATIONS: &str = include_str!("../fixtures/tla-artifact-mutations.json");
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

const OFFLINE_MUTATIONS: &[&str] = &[
    "missing",
    "digest",
    "size",
    "symlink",
    "hardlink",
    "world-writable",
];
const ONLINE_MUTATIONS: &[&str] = &[
    "partial",
    "http-404",
    "http-403",
    "http-429",
    "http-500",
    "timeout",
    "stalled",
    "disconnect",
    "redirect-loop",
    "redirect-host",
    "transfer-ceiling",
    "download-digest",
    "download-size",
    "destination-race",
    "cache-directory-mode",
    "archive-hardlink",
    "output-race",
];
const NETWORK_MUTATIONS: &[&str] = &["offline-network"];

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "asb-tla-acquisition-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create test scratch");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("make scratch private");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture_runner(scratch: &Scratch) -> (PathBuf, Vec<u8>) {
    let bytes = b"bounded deterministic TLA fixture".to_vec();
    let digest = Command::new("sha256sum")
        .arg("--version")
        .output()
        .expect("sha256sum exists");
    assert!(digest.status.success());
    let source = RUNNER
        .replace(
            "TLA_SHA256=8c200a88d151c6c183c8dbc57a6b633d135e7a2b18242a3afbf243a9e4b68d3e",
            "TLA_SHA256=7c0747909d5aac3aa902436bd2384593a556dce1ee11d2fdb10861ab9e279436",
        )
        .replace("TLA_BYTES=4512486", &format!("TLA_BYTES={}", bytes.len()))
        .replace(
            "TLA_SOURCE_SHA256=1f96ee7ef950e456794d13b7e4d8123c345a91528257a3a41b7cc1b506d1b58f",
            "TLA_SOURCE_SHA256=37d8e016150de41a171a80bbc6aafc84db614449c49f0817e36b43d57f12e9a4",
        )
        .replace("TLA_SOURCE_BYTES=82989507", "TLA_SOURCE_BYTES=22")
        .replace(
            "ANT_SHA256=71334d7e5d98cfe53d6c429a648a5021137a967378667306c5f613dff5180506",
            "ANT_SHA256=8500e9a0a7536863aa265642b1344b611665553106f578f4f8e0132d89bc1c19",
        )
        .replace("ANT_BYTES=6925830", "ANT_BYTES=19")
        .replace(
            "\"$(dirname \"$0\")/tla-provenance/build.sh\" \"$build_cache\" \"$built\"",
            "mock-build \"$build_cache\" \"$built\"",
        );
    let runner = scratch.0.join("runner.sh");
    fs::write(&runner, source).expect("write fixture runner");
    fs::set_permissions(&runner, fs::Permissions::from_mode(0o700))
        .expect("make runner executable");
    (runner, bytes)
}

fn mock_tools(scratch: &Scratch) -> PathBuf {
    let bin = scratch.0.join("bin");
    fs::create_dir(&bin).expect("create mock bin");
    fs::write(
        bin.join("docker"),
        r#"#!/bin/sh
case "$1 $2" in
  "info ") exit 0 ;;
  "image inspect")
    case " $* " in
      *" --format "*) printf '%s\n' 'sha256:c0d1549d1e0f5fa5b83622ec0033b00456107e0b1d0cfcce4c1d831532ce621e linux/amd64' ;;
    esac
    exit 0 ;;
esac
exit 98
"#,
    )
    .expect("write docker mock");
    fs::write(
        bin.join("mock-build"),
        r#"#!/bin/sh
sleep "${MOCK_BUILD_DELAY:-0}"
if [ "${MOCK_SWAP_ORIGINAL:-0}" = 1 ]; then
  rm -f -- "$MOCK_ORIGINAL_CACHE/tlaplus-b123b226.tar.gz"
  printf %s tampered >"$MOCK_ORIGINAL_CACHE/tlaplus-b123b226.tar.gz"
fi
[ "$(cat "$1/tlaplus-b123b226.tar.gz")" = "bounded source fixture" ] || exit 96
[ "$(cat "$1/apache-ant-1.10.15-bin.tar.gz")" = "bounded ant fixture" ] || exit 96
if [ "${MOCK_CURL_MODE:-ok}" = output-race ]; then
  printf %s intruder >"$MOCK_FINAL_OUTPUT"
fi
printf %s 'bounded deterministic TLA fixture' >"$2"
"#,
    )
    .expect("write build mock");
    fs::write(
        bin.join("curl"),
        r#"#!/bin/sh
output=
url=
max_filesize=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) output=$2; shift 2 ;;
    --max-filesize) max_filesize=$2; shift 2 ;;
    --write-out|--max-redirs|--proto|--proto-redir|--max-time|--retry|--retry-delay|--retry-max-time) shift 2 ;;
    --fail|--silent|--show-error|--location|--tlsv1.2) shift ;;
    *) url=$1; shift ;;
  esac
done
case "${MOCK_CURL_MODE:-ok}" in
  http-*) exit 22 ;;
  timeout|stalled) exit 28 ;;
  disconnect) printf partial >"$output"; exit 18 ;;
  redirect-loop) exit 47 ;;
  transfer-ceiling) printf %s PRIVATE-CURL-SENTINEL >&2; exit 63 ;;
  sentinel) exit 99 ;;
esac
case "$url" in
  https://codeload.github.com/*)
    [ "$max_filesize" -gt 22 ] || exit 63
    printf %s 'bounded source fixture' >"$output"
    ;;
  https://archive.apache.org/*)
    [ "$max_filesize" -gt 19 ] || exit 63
    printf %s 'bounded ant fixture' >"$output"
    ;;
  *) exit 97 ;;
esac
case "${MOCK_CURL_MODE:-ok}" in
  redirect-host) printf %s 'https://unapproved.invalid/artifact' ;;
  destination-race) cp -- "$output" "${output%.partial}"; printf %s "$url" ;;
  download-digest) printf X | dd of="$output" bs=1 seek=0 conv=notrunc status=none; printf %s "$url" ;;
  download-size) printf X >>"$output"; printf %s "$url" ;;
  *) printf %s "$url" ;;
esac
"#,
    )
    .expect("write curl mock");
    for program in ["docker", "mock-build", "curl"] {
        fs::set_permissions(bin.join(program), fs::Permissions::from_mode(0o700))
            .expect("make mock executable");
    }
    bin
}

fn run_offline(runner: &Path, root: &Path, scratch: &Path) -> std::process::Output {
    Command::new(runner)
        .args([root, scratch])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("ASB_FORMAL_OFFLINE", "1")
        .env("ASB_FORMAL_ACQUIRE_ONLY", "1")
        .output()
        .expect("run offline acquisition")
}

fn cache_path(root: &Path) -> PathBuf {
    root.join("tla2tools-v1.8.0.jar")
}

fn mutation_names() -> Vec<String> {
    serde_json::from_str::<Value>(MUTATIONS)
        .expect("mutation fixture JSON")
        .as_array()
        .expect("mutation array")
        .iter()
        .map(|value| value.as_str().expect("mutation name").to_owned())
        .collect()
}

#[test]
fn every_declared_mutation_has_one_executed_test_partition() {
    let declared = mutation_names().into_iter().collect::<BTreeSet<_>>();
    let executed = OFFLINE_MUTATIONS
        .iter()
        .chain(ONLINE_MUTATIONS)
        .chain(NETWORK_MUTATIONS)
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        declared.len(),
        OFFLINE_MUTATIONS.len() + ONLINE_MUTATIONS.len() + 1
    );
    assert_eq!(declared, executed);
}

#[test]
fn pins_consume_the_qualified_source_build_not_the_mutable_release() {
    let positive: Value = serde_json::from_str(POSITIVE).expect("positive fixture JSON");
    for value in positive.as_object().expect("positive object").values() {
        let rendered = match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            _ => panic!("unsupported fixture value"),
        };
        assert!(PINS.contains(&rendered) || RUNNER.contains(&rendered));
    }
    assert!(!RUNNER.contains("releases/download/v1.8.0/tla2tools.jar"));
    assert!(BUILD.contains("--network none"));
    assert!(BUILD.contains("--pull never"));
}

#[test]
fn verified_offline_cache_succeeds_without_network() {
    let scratch = Scratch::new();
    let (runner, bytes) = fixture_runner(&scratch);
    let cache = scratch.0.join("cache");
    fs::create_dir(&cache).expect("create cache");
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).expect("make cache private");
    let artifact = cache_path(&cache);
    fs::write(&artifact, bytes).expect("write artifact");
    fs::set_permissions(&artifact, fs::Permissions::from_mode(0o400))
        .expect("make artifact immutable to group and others");
    let output = run_offline(&runner, &cache, &scratch.0.join("run"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "TLA artifact acquired\n"
    );
}

#[test]
fn hostile_offline_cache_entries_fail_closed() {
    let mutations: Value = serde_json::from_str(MUTATIONS).expect("mutation fixture JSON");
    for name in mutations.as_array().expect("mutation array") {
        let name = name.as_str().expect("mutation name");
        if !OFFLINE_MUTATIONS.contains(&name) {
            continue;
        }
        let scratch = Scratch::new();
        let (runner, bytes) = fixture_runner(&scratch);
        let cache = scratch.0.join("cache");
        fs::create_dir(&cache).expect("create cache");
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).expect("make cache private");
        let artifact = cache_path(&cache);
        match name {
            "missing" => {}
            "digest" => {
                let mut corrupted = bytes.clone();
                corrupted[0] ^= 1;
                fs::write(&artifact, corrupted).expect("write digest mutation");
            }
            "size" => fs::write(&artifact, b"short").expect("write size mutation"),
            "symlink" => symlink("missing-target", &artifact).expect("create symlink"),
            "hardlink" => {
                let other = cache.join("other");
                fs::write(&other, &bytes).expect("write hardlink origin");
                fs::hard_link(other, &artifact).expect("create hardlink");
            }
            "world-writable" => {
                fs::write(&artifact, &bytes).expect("write permissive artifact");
                fs::set_permissions(&artifact, fs::Permissions::from_mode(0o666))
                    .expect("make artifact permissive");
            }
            _ => panic!("unhandled mutation {name}"),
        }
        let output = run_offline(&runner, &cache, &scratch.0.join("run"));
        assert!(!output.status.success(), "mutation accepted: {name}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains(&scratch.0.to_string_lossy().into_owned()));
    }
}

#[test]
fn bounded_online_build_and_verified_cache_reuse_succeed() {
    let scratch = Scratch::new();
    let (runner, bytes) = fixture_runner(&scratch);
    let bin = mock_tools(&scratch);
    let cache = scratch.0.join("cache");
    fs::create_dir(&cache).expect("create cache");
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).expect("make cache private");
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let first = Command::new(&runner)
        .args([&cache, &scratch.0.join("first")])
        .env_clear()
        .env("PATH", &path)
        .env("ASB_FORMAL_ACQUIRE_ONLY", "1")
        .output()
        .expect("run online build");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(
        fs::read(cache_path(&cache)).expect("read built cache"),
        bytes
    );
    let second = Command::new(&runner)
        .args([&cache, &scratch.0.join("second")])
        .env_clear()
        .env("PATH", &path)
        .env("MOCK_CURL_MODE", "sentinel")
        .env("ASB_FORMAL_ACQUIRE_ONLY", "1")
        .output()
        .expect("reuse online cache");
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
}

#[test]
fn bounded_online_acquisition_faults_do_not_promote() {
    let mutations: Value = serde_json::from_str(MUTATIONS).expect("mutation fixture JSON");
    for name in mutations.as_array().expect("mutation array") {
        let name = name.as_str().expect("mutation name");
        if !ONLINE_MUTATIONS.contains(&name) {
            continue;
        }
        let scratch = Scratch::new();
        let (runner, _) = fixture_runner(&scratch);
        let bin = mock_tools(&scratch);
        let cache = scratch.0.join("cache");
        fs::create_dir(&cache).expect("create cache");
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).expect("make cache private");
        let source_partial = cache.join("tla-source-cache/tlaplus-b123b226.tar.gz.partial");
        if name == "partial" {
            fs::create_dir(cache.join("tla-source-cache")).expect("create source cache");
            fs::write(&source_partial, b"owned partial").expect("write pre-existing partial");
        } else if name == "cache-directory-mode" {
            fs::create_dir(cache.join("tla-source-cache")).expect("create source cache");
            fs::set_permissions(
                cache.join("tla-source-cache"),
                fs::Permissions::from_mode(0o777),
            )
            .expect("make source cache unsafe");
        } else if name == "archive-hardlink" {
            let source_cache = cache.join("tla-source-cache");
            fs::create_dir(&source_cache).expect("create source cache");
            fs::set_permissions(&source_cache, fs::Permissions::from_mode(0o700))
                .expect("make source cache private");
            let other = source_cache.join("other");
            fs::write(&other, b"bounded source fixture").expect("write archive origin");
            fs::hard_link(&other, source_cache.join("tlaplus-b123b226.tar.gz"))
                .expect("create archive hardlink");
        }
        let output = Command::new(runner)
            .args([&cache, &scratch.0.join("run")])
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("MOCK_CURL_MODE", name)
            .env("MOCK_FINAL_OUTPUT", cache_path(&cache))
            .env("ASB_FORMAL_ACQUIRE_ONLY", "1")
            .output()
            .expect("run online fault");
        assert!(!output.status.success(), "online fault accepted: {name}");
        if name == "transfer-ceiling" {
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!stderr.contains("PRIVATE-CURL-SENTINEL"));
            assert!(stderr.contains("build input acquisition failed"));
            assert!(stderr.len() <= 128, "unbounded acquisition error: {stderr}");
        }
        if name == "output-race" {
            assert_ne!(
                fs::read(cache_path(&cache)).expect("read raced output"),
                bytes_fixture()
            );
        } else {
            assert!(
                !cache_path(&cache).exists(),
                "fault promoted output: {name}"
            );
        }
        if name == "partial" {
            assert_eq!(
                fs::read(source_partial).expect("read owned partial"),
                b"owned partial"
            );
        } else {
            assert!(!source_partial.exists(), "fault retained partial: {name}");
        }
        assert!(
            fs::read_dir(&cache)
                .expect("read tool cache")
                .all(|entry| !entry
                    .expect("read tool-cache entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".tla-build-inputs.")),
            "fault retained private build snapshot: {name}"
        );
    }
}

fn bytes_fixture() -> Vec<u8> {
    b"bounded deterministic TLA fixture".to_vec()
}

#[test]
fn private_build_snapshot_resists_original_archive_replacement() {
    let scratch = Scratch::new();
    let (runner, bytes) = fixture_runner(&scratch);
    let bin = mock_tools(&scratch);
    let cache = scratch.0.join("cache");
    fs::create_dir(&cache).expect("create cache");
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).expect("make cache private");
    let output = Command::new(runner)
        .args([&cache, &scratch.0.join("run")])
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("MOCK_SWAP_ORIGINAL", "1")
        .env("MOCK_ORIGINAL_CACHE", cache.join("tla-source-cache"))
        .env("MOCK_FINAL_OUTPUT", cache_path(&cache))
        .env("ASB_FORMAL_ACQUIRE_ONLY", "1")
        .output()
        .expect("run replacement race");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(cache_path(&cache)).expect("read output"), bytes);
}

#[test]
fn concurrent_acquisition_converges_on_one_verified_output() {
    let scratch = Scratch::new();
    let (runner, bytes) = fixture_runner(&scratch);
    let bin = mock_tools(&scratch);
    let cache = scratch.0.join("cache");
    fs::create_dir(&cache).expect("create cache");
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).expect("make cache private");
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let first = Command::new(&runner)
        .args([&cache, &scratch.0.join("first")])
        .env_clear()
        .env("PATH", &path)
        .env("MOCK_BUILD_DELAY", "0.2")
        .env("ASB_FORMAL_ACQUIRE_ONLY", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start first acquisition");
    let second = Command::new(&runner)
        .args([&cache, &scratch.0.join("second")])
        .env_clear()
        .env("PATH", &path)
        .env("MOCK_BUILD_DELAY", "0.2")
        .env("ASB_FORMAL_ACQUIRE_ONLY", "1")
        .output()
        .expect("run second acquisition");
    let first = first.wait_with_output().expect("wait first acquisition");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(
        fs::read(cache_path(&cache)).expect("read converged cache"),
        bytes
    );
}

#[test]
fn offline_missing_and_partial_inputs_never_invoke_network() {
    assert_eq!(NETWORK_MUTATIONS, &["offline-network"]);
    let scratch = Scratch::new();
    let (runner, _) = fixture_runner(&scratch);
    let cache = scratch.0.join("cache");
    fs::create_dir(&cache).expect("create cache");
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o700)).expect("make cache private");
    fs::write(cache_path(&cache).with_extension("jar.partial"), b"partial").expect("write partial");
    let bin = scratch.0.join("bin");
    fs::create_dir(&bin).expect("create bin");
    let marker = scratch.0.join("curl-invoked");
    fs::write(
        bin.join("curl"),
        "#!/bin/sh\nprintf invoked >\"$MOCK_CURL_MARKER\"\nexit 99\n",
    )
    .expect("write curl sentinel");
    fs::set_permissions(bin.join("curl"), fs::Permissions::from_mode(0o700))
        .expect("make sentinel executable");
    let output = Command::new(runner)
        .args([&cache, &scratch.0.join("run")])
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("MOCK_CURL_MARKER", &marker)
        .env("ASB_FORMAL_OFFLINE", "1")
        .env("ASB_FORMAL_ACQUIRE_ONLY", "1")
        .output()
        .expect("run offline sentinel");
    assert_eq!(output.status.code(), Some(2));
    assert!(!marker.exists());
}
