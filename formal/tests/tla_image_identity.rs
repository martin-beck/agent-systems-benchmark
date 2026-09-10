// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Portable OCI repository-digest and platform identity boundary.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

const BUILD: &str = include_str!("../tla-provenance/build.sh");
const POSITIVE: &str = include_str!("../tla-provenance/fixtures/image-identity-positive.json");
const MUTATIONS: &str = include_str!("../tla-provenance/fixtures/image-identity-mutations.json");
const IMAGE: &str =
    "eclipse-temurin@sha256:c0d1549d1e0f5fa5b83622ec0033b00456107e0b1d0cfcce4c1d831532ce621e";
const CONFIG: &str = "sha256:c0d1549d1e0f5fa5b83622ec0033b00456107e0b1d0cfcce4c1d831532ce621e";

fn verifier() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tla-provenance/verify_image_identity.sh")
}

fn evidence(config_id: &str) -> Value {
    json!({
        "config_id": config_id,
        "repo_digests": [IMAGE],
        "os": "linux",
        "architecture": "amd64"
    })
}

fn verify(input: &[u8]) -> std::process::Output {
    let mut child = Command::new(verifier())
        .args([IMAGE, "linux", "amd64"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start identity verifier");
    child
        .stdin
        .take()
        .expect("open verifier stdin")
        .write_all(input)
        .expect("write evidence");
    child.wait_with_output().expect("wait for verifier")
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "asb-tla-image-identity-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).expect("create test scratch");
        Self(path)
    }

    fn write_executable(&self, name: &str, contents: &str) {
        let path = self.0.join("bin").join(name);
        fs::create_dir_all(path.parent().expect("tool parent")).expect("create tool directory");
        fs::write(&path, contents).expect("write fake tool");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("make tool executable");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_build(
    identity: &[u8],
    inspect_exit: i32,
    inspect_stderr: &str,
) -> (std::process::Output, Scratch) {
    let scratch = Scratch::new();
    let cache = scratch.0.join("cache");
    fs::create_dir(&cache).expect("create cache");
    fs::write(cache.join("tlaplus-b123b226.tar.gz"), []).expect("source placeholder");
    fs::write(cache.join("apache-ant-1.10.15-bin.tar.gz"), []).expect("Ant placeholder");
    scratch.write_executable(
        "python3",
        "#!/bin/sh\nif [ \"$1\" = - ]; then cat >/dev/null; exit 0; fi\nexec /usr/bin/python3 \"$@\"\n",
    );
    scratch.write_executable("sha256sum", "#!/bin/sh\ncat >/dev/null\nexit 0\n");
    scratch.write_executable(
        "stat",
        "#!/bin/sh\ncase \"$4\" in *tlaplus*) echo 82989507;; *) echo 6925830;; esac\n",
    );
    scratch.write_executable("cp", "#!/bin/sh\nexit 0\n");
    scratch.write_executable(
        "docker",
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >>\"$FAKE_DOCKER_ARGV\"\ncase \"$1 $2\" in\n  'info ') exit 0;;\n  'image inspect') printf '%s' \"$FAKE_DOCKER_EVIDENCE\"; printf '%s' \"$FAKE_DOCKER_STDERR\" >&2; exit \"$FAKE_DOCKER_INSPECT_EXIT\";;\n  'run --rm') : >\"$FAKE_DOCKER_EFFECT\"; exit 91;;\nesac\nexit 92\n",
    );
    let output = Command::new(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tla-provenance/build.sh"),
    )
    .args([&cache, &scratch.0.join("output.jar")])
    .env_clear()
    .env(
        "PATH",
        format!("{}:/usr/bin:/bin", scratch.0.join("bin").display()),
    )
    .env("TMPDIR", &scratch.0)
    .env(
        "FAKE_DOCKER_EVIDENCE",
        String::from_utf8_lossy(identity).as_ref(),
    )
    .env("FAKE_DOCKER_STDERR", inspect_stderr)
    .env("FAKE_DOCKER_INSPECT_EXIT", inspect_exit.to_string())
    .env("FAKE_DOCKER_ARGV", scratch.0.join("docker.argv"))
    .env("FAKE_DOCKER_EFFECT", scratch.0.join("docker.effect"))
    .output()
    .expect("run build with fake Docker");
    (output, scratch)
}

fn mutation(name: &str) -> Vec<u8> {
    let mut value = evidence(CONFIG);
    match name {
        "empty" => return Vec::new(),
        "malformed" => return b"{".to_vec(),
        "oversized" => return vec![b' '; 16_385],
        "trailing-json" => return b"{} {}".to_vec(),
        "duplicate-field" => {
            return format!(
                "{{\"config_id\":\"{CONFIG}\",\"config_id\":\"{CONFIG}\",\"repo_digests\":[\"{IMAGE}\"],\"os\":\"linux\",\"architecture\":\"amd64\"}}"
            )
            .into_bytes();
        }
        "unknown-field" => value["extra"] = json!(true),
        "missing-field" => {
            value.as_object_mut().expect("object").remove("os");
        }
        "config-id-malformed" => value["config_id"] = json!("sha256:not-a-digest"),
        "repo-digests-null" => value["repo_digests"] = Value::Null,
        "repo-digests-empty" => value["repo_digests"] = json!([]),
        "repo-digests-extra" => {
            value["repo_digests"] = json!([
                IMAGE,
                "other/image@sha256:2222222222222222222222222222222222222222222222222222222222222222"
            ]);
        }
        "repo-digest-tag" => value["repo_digests"] = json!(["eclipse-temurin:17"]),
        "repo-digest-wrong-name" => {
            value["repo_digests"] = json!([
                "other/image@sha256:c0d1549d1e0f5fa5b83622ec0033b00456107e0b1d0cfcce4c1d831532ce621e"
            ]);
        }
        "repo-digest-wrong-value" => {
            value["repo_digests"] = json!([
                "eclipse-temurin@sha256:2222222222222222222222222222222222222222222222222222222222222222"
            ]);
        }
        "repo-digest-prefix" => {
            value["repo_digests"] = json!(["eclipse-temurin@sha256:c0d1549d"]);
        }
        "repo-digest-suffix" => value["repo_digests"] = json!([format!("{IMAGE}0")]),
        "local-id-only" => value["repo_digests"] = json!([CONFIG]),
        "wrong-os" => value["os"] = json!("windows"),
        "wrong-architecture" => value["architecture"] = json!("arm64"),
        "non-utf8" => return vec![0xff, 0xfe],
        _ => panic!("unhandled mutation {name}"),
    }
    serde_json::to_vec(&value).expect("encode mutation")
}

#[test]
fn matching_and_distinct_configuration_ids_accept_the_same_repository_digest() {
    let fixtures: Value = serde_json::from_str(POSITIVE).expect("positive fixture JSON");
    for fixture in fixtures.as_array().expect("positive array") {
        let output = verify(
            &serde_json::to_vec(&evidence(fixture["config_id"].as_str().expect("config ID")))
                .expect("encode evidence"),
        );
        assert!(
            output.status.success(),
            "fixture failed: {}",
            fixture["name"]
        );
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn every_declared_identity_mutation_fails_with_one_stable_diagnostic() {
    let names: Value = serde_json::from_str(MUTATIONS).expect("mutation fixture JSON");
    let names = names.as_array().expect("mutation array");
    let unique = names
        .iter()
        .map(|name| name.as_str().expect("mutation name"))
        .collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), names.len(), "duplicate mutation identity");
    for name in unique {
        let output = verify(&mutation(name));
        assert_eq!(output.status.code(), Some(65), "mutation accepted: {name}");
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr, b"OCI image identity rejected\n",
            "unstable diagnostic: {name}"
        );
    }
}

#[test]
fn build_verifies_closed_identity_before_any_container_effect() {
    let inspect = BUILD
        .find("image inspect --format")
        .expect("machine-readable inspect");
    let verify = BUILD
        .find("verify_image_identity.sh")
        .expect("identity verifier");
    let run = BUILD
        .find("run --rm -i --pull never --network none")
        .expect("offline build run");
    assert!(inspect < verify && verify < run);
    assert!(!BUILD.contains("{{.Id}} {{.Os}}/{{.Architecture}}"));
    assert!(BUILD.contains(IMAGE));
    assert!(!BUILD.contains("image_evidence=$("));
}

#[test]
fn invalid_or_private_inspection_never_reaches_a_container_effect() {
    let cases = [
        ("invalid", b"{}".to_vec(), 0, ""),
        ("oversized", vec![b'x'; 16_385], 0, ""),
        (
            "inspect-error",
            Vec::new(),
            17,
            "PRIVATE-DAEMON-SENTINEL",
        ),
    ];
    for (name, identity, inspect_exit, private_stderr) in cases {
        let (output, scratch) = run_build(&identity, inspect_exit, private_stderr);
        assert_eq!(output.status.code(), Some(65), "unexpected status for {name}");
        assert!(
            !scratch.0.join("docker.effect").exists(),
            "container effect reached for {name}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("PRIVATE-DAEMON-SENTINEL"),
            "private inspect stderr leaked"
        );
        assert!(stderr.len() <= 128, "unbounded diagnostic for {name}");
        assert!(stderr.contains("OCI image inspection or verification failed"));
    }
}

#[test]
fn valid_projection_reaches_one_exact_offline_license_check() {
    let bytes = serde_json::to_vec(&evidence(CONFIG)).expect("encode evidence");
    let (output, scratch) = run_build(&bytes, 0, "");
    assert_eq!(output.status.code(), Some(91));
    assert!(scratch.0.join("docker.effect").exists());
    let argv = fs::read_to_string(scratch.0.join("docker.argv")).expect("read Docker argv");
    let expected = format!(
        "info\nimage\ninspect\n--format\n{{\"config_id\":{{{{json .Id}}}},\"repo_digests\":{{{{json .RepoDigests}}}},\"os\":{{{{json .Os}}}},\"architecture\":{{{{json .Architecture}}}}}}\n{IMAGE}\nrun\n--rm\n-i\n--pull\nnever\n--network\nnone\n{IMAGE}\nsha256sum\n-c\n-\n"
    );
    assert_eq!(argv, expected);
}
