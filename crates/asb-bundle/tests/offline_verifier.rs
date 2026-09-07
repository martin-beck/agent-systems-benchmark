// SPDX-License-Identifier: MIT
//! Real SSHSIG and filesystem-boundary tests for the offline verifier.

use asb_bundle::{
    BundleArtifact, ExpectedTarget, RuntimeBundleManifest, RuntimeTarget, SIGNATURE_NAMESPACE,
    SbomDocument, VerifierConfig, VerifyError, content_digest, verify_bundle,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const PRINCIPAL: &str = "asb-test-release";

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    key: PathBuf,
    allowed: PathBuf,
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn target() -> ExpectedTarget<'static> {
    ExpectedTarget {
        operating_system: "linux",
        architecture: "x86_64",
        libc: "glibc",
        libc_version: "2.39",
    }
}

fn config(fixture: &Fixture) -> VerifierConfig {
    VerifierConfig {
        ssh_keygen: PathBuf::from("/usr/bin/ssh-keygen"),
        ssh_keygen_sha256: sha256(&fs::read("/usr/bin/ssh-keygen").expect("read ssh-keygen")),
        allowed_signers: fixture.allowed.clone(),
        principal: PRINCIPAL.into(),
    }
}

fn artifact(path: &str, bytes: &[u8], executable: bool) -> BundleArtifact {
    BundleArtifact {
        path: path.into(),
        size: bytes.len() as u64,
        sha256: sha256(bytes),
        mode: if executable { 0o755 } else { 0o644 },
        license_expression: "MIT".into(),
        license_evidence: vec!["LICENSE".into()],
    }
}

fn spdx(artifacts: &[BundleArtifact]) -> Value {
    json!({
        "spdxVersion": "SPDX-2.3",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": "fixture",
        "dataLicense": "CC0-1.0",
        "documentNamespace": "https://example.invalid/asb/fixture",
        "files": artifacts.iter().map(|artifact| json!({
            "fileName": artifact.path,
            "SPDXID": format!("SPDXRef-{}", artifact.path.replace('/', "-")),
            "checksums": [{
                "algorithm": "SHA256",
                "checksumValue": artifact.sha256
            }],
            "licenseConcluded": artifact.license_expression
        })).collect::<Vec<_>>()
    })
}

fn cyclonedx(artifacts: &[BundleArtifact]) -> Value {
    json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "serialNumber": "urn:uuid:00000000-0000-4000-8000-000000000001",
        "version": 1,
        "components": artifacts.iter().map(|artifact| json!({
            "type": "file",
            "bom-ref": artifact.path,
            "name": artifact.path,
            "hashes": [{
                "alg": "SHA-256",
                "content": artifact.sha256
            }],
            "licenses": [{"expression": artifact.license_expression}]
        })).collect::<Vec<_>>()
    })
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("serialize")).expect("write JSON");
}

fn sign(fixture: &Fixture) {
    sign_with_namespace(fixture, SIGNATURE_NAMESPACE);
}

fn sign_with_namespace(fixture: &Fixture, namespace: &str) {
    let signature = fixture.root.join("manifest.json.sig");
    let _ = fs::remove_file(signature);
    let status = Command::new("/usr/bin/ssh-keygen")
        .args([
            "-Y",
            "sign",
            "-q",
            "-f",
            fixture.key.to_str().expect("key path"),
            "-n",
            namespace,
            fixture
                .root
                .join("manifest.json")
                .to_str()
                .expect("manifest path"),
        ])
        .status()
        .expect("run ssh-keygen sign");
    assert!(status.success());
}

fn rewrite_manifest(fixture: &Fixture, transform: impl FnOnce(&mut RuntimeBundleManifest)) {
    let path = fixture.root.join("manifest.json");
    let mut manifest: RuntimeBundleManifest =
        serde_json::from_slice(&fs::read(&path).expect("read manifest")).expect("parse manifest");
    transform(&mut manifest);
    fs::write(
        path,
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write manifest");
    sign(fixture);
}

fn rewrite_sbom(fixture: &Fixture, name: &str, transform: impl FnOnce(&mut Value)) {
    let path = fixture.root.join(name);
    let mut value: Value =
        serde_json::from_slice(&fs::read(&path).expect("read SBOM")).expect("parse SBOM");
    transform(&mut value);
    write_json(&path, &value);
    rewrite_manifest(fixture, |manifest| {
        let hash = sha256(&fs::read(&path).expect("read updated SBOM"));
        if name == "spdx.json" {
            manifest.spdx.sha256 = hash;
        } else {
            manifest.cyclonedx.sha256 = hash;
        }
    });
}

fn assert_metadata(fixture: &Fixture) {
    assert!(matches!(
        verify_bundle(&fixture.root, &config(fixture), &target()),
        Err(VerifyError::Metadata(_))
    ));
}

fn create_fixture() -> Fixture {
    let temp = tempfile::tempdir().expect("temporary directory");
    let root = temp.path().join("bundle");
    fs::create_dir_all(root.join("bin")).expect("create payload");
    let executable = b"#!/bin/sh\nexit 0\n";
    let license = b"fixture MIT license\n";
    fs::write(root.join("bin/agent"), executable).expect("write executable");
    fs::set_permissions(root.join("bin/agent"), fs::Permissions::from_mode(0o755))
        .expect("chmod executable");
    fs::write(root.join("LICENSE"), license).expect("write license");
    fs::set_permissions(root.join("LICENSE"), fs::Permissions::from_mode(0o644))
        .expect("chmod license");
    let artifacts = vec![
        artifact("LICENSE", license, false),
        artifact("bin/agent", executable, true),
    ];
    let spdx_bytes = serde_json::to_vec_pretty(&spdx(&artifacts)).expect("SPDX");
    let cyclonedx_bytes = serde_json::to_vec_pretty(&cyclonedx(&artifacts)).expect("CycloneDX");
    fs::write(root.join("spdx.json"), &spdx_bytes).expect("write SPDX");
    fs::write(root.join("cyclonedx.json"), &cyclonedx_bytes).expect("write CycloneDX");
    let manifest = RuntimeBundleManifest {
        schema_version: 1,
        bundle_id: "fixture-agent".into(),
        bundle_version: "1.2.3".into(),
        target: RuntimeTarget {
            operating_system: "linux".into(),
            architecture: "x86_64".into(),
            libc: "glibc".into(),
            libc_version: "2.39".into(),
        },
        entrypoint: "bin/agent".into(),
        content_sha256: content_digest(&artifacts),
        artifacts,
        spdx: SbomDocument {
            path: "spdx.json".into(),
            sha256: sha256(&spdx_bytes),
        },
        cyclonedx: SbomDocument {
            path: "cyclonedx.json".into(),
            sha256: sha256(&cyclonedx_bytes),
        },
    };
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest"),
    )
    .expect("write manifest");

    let key = temp.path().join("release-key");
    assert!(
        Command::new("/usr/bin/ssh-keygen")
            .args([
                "-q",
                "-t",
                "ed25519",
                "-N",
                "",
                "-f",
                key.to_str().expect("key path")
            ])
            .status()
            .expect("generate key")
            .success()
    );
    let public_key = fs::read_to_string(key.with_extension("pub")).expect("read public key");
    let allowed = temp.path().join("allowed_signers");
    fs::write(&allowed, format!("{PRINCIPAL} {public_key}")).expect("write allowed signers");
    let fixture = Fixture {
        _temp: temp,
        root,
        key,
        allowed,
    };
    sign(&fixture);
    fixture
}

#[test]
fn verifies_signed_complete_offline_bundle() {
    let fixture = create_fixture();
    let verified = verify_bundle(&fixture.root, &config(&fixture), &target()).expect("verify");
    assert_eq!(verified.bundle_id, "fixture-agent");
    assert_eq!(verified.artifact_count, 2);
}

#[test]
fn rejects_payload_tampering_and_extra_files() {
    let fixture = create_fixture();
    fs::write(fixture.root.join("bin/agent"), b"changed").expect("tamper");
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Content(_))
    ));

    let fixture = create_fixture();
    fs::write(fixture.root.join("extra"), b"untracked").expect("extra");
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Content(_))
    ));
}

#[test]
fn rejects_symlink_and_wrong_target() {
    let fixture = create_fixture();
    fs::remove_file(fixture.root.join("bin/agent")).expect("remove");
    symlink("/bin/true", fixture.root.join("bin/agent")).expect("symlink");
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Topology(_))
    ));

    let fixture = create_fixture();
    let wrong = ExpectedTarget {
        architecture: "aarch64",
        ..target()
    };
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &wrong),
        Err(VerifyError::Target(_))
    ));
}

#[test]
fn rejects_invalid_signature_and_manifest_semantics() {
    let fixture = create_fixture();
    let path = fixture.root.join("manifest.json");
    let mut bytes = fs::read(&path).expect("read");
    bytes.push(b' ');
    fs::write(path, bytes).expect("tamper");
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Signature)
    ));

    let fixture = create_fixture();
    rewrite_manifest(&fixture, |manifest| {
        manifest.artifacts.swap(0, 1);
    });
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Metadata(_))
    ));

    let fixture = create_fixture();
    rewrite_manifest(&fixture, |manifest| {
        manifest.artifacts[1].license_evidence = vec!["missing-license".into()];
    });
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Metadata(_))
    ));
}

#[test]
fn rejects_sbom_omission_and_license_disagreement() {
    let fixture = create_fixture();
    let path = fixture.root.join("spdx.json");
    let mut value: Value =
        serde_json::from_slice(&fs::read(&path).expect("read SPDX")).expect("parse SPDX");
    value["files"].as_array_mut().expect("files").pop();
    write_json(&path, &value);
    rewrite_manifest(&fixture, |manifest| {
        manifest.spdx.sha256 = sha256(&fs::read(&path).expect("read SPDX"));
    });
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Metadata(_))
    ));

    let fixture = create_fixture();
    let path = fixture.root.join("cyclonedx.json");
    let mut value: Value =
        serde_json::from_slice(&fs::read(&path).expect("read CycloneDX")).expect("parse");
    value["components"][0]["licenses"][0]["expression"] = json!("Apache-2.0");
    write_json(&path, &value);
    rewrite_manifest(&fixture, |manifest| {
        manifest.cyclonedx.sha256 = sha256(&fs::read(&path).expect("read CycloneDX"));
    });
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Metadata(_))
    ));
}

#[test]
fn rejects_manifest_version_bounds_paths_and_reserved_names() {
    for transform in [
        (|manifest: &mut RuntimeBundleManifest| manifest.schema_version = 2)
            as fn(&mut RuntimeBundleManifest),
        |manifest| manifest.bundle_id.clear(),
        |manifest| manifest.content_sha256 = "A".repeat(64),
        |manifest| manifest.spdx.path = manifest.cyclonedx.path.clone(),
        |manifest| manifest.artifacts[0].path = "manifest.json".into(),
        |manifest| manifest.artifacts[0].license_evidence.clear(),
        |manifest| manifest.artifacts[1].mode = 0o644,
        |manifest| manifest.artifacts[0].mode = 0o666,
        |manifest| manifest.artifacts[0].mode = 0o4755,
        |manifest| manifest.artifacts[0].mode = 0o244,
    ] {
        let fixture = create_fixture();
        rewrite_manifest(&fixture, transform);
        assert_metadata(&fixture);
    }

    let fixture = create_fixture();
    rewrite_manifest(&fixture, |manifest| {
        manifest.entrypoint = "../bin/agent".into();
    });
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Topology(_))
    ));

    let fixture = create_fixture();
    rewrite_manifest(&fixture, |manifest| {
        manifest.artifacts.clear();
    });
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Limit(_))
    ));
}

#[test]
fn rejects_total_size_content_digest_mode_and_metadata_collision() {
    let fixture = create_fixture();
    rewrite_manifest(&fixture, |manifest| {
        manifest.artifacts[0].size = u64::MAX;
    });
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Limit(_))
    ));

    let fixture = create_fixture();
    rewrite_manifest(&fixture, |manifest| {
        manifest.content_sha256 = "0".repeat(64);
    });
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Content(_))
    ));

    let fixture = create_fixture();
    fs::set_permissions(
        fixture.root.join("bin/agent"),
        fs::Permissions::from_mode(0o644),
    )
    .expect("remove executable bit");
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Content(_))
    ));

    let fixture = create_fixture();
    rewrite_manifest(&fixture, |manifest| {
        manifest.spdx.path = "LICENSE".into();
    });
    assert_metadata(&fixture);
}

#[test]
fn rejects_oversized_or_unsafe_trust_metadata() {
    let fixture = create_fixture();
    fs::write(
        fixture.root.join("manifest.json.sig"),
        vec![b'x'; 1024 * 1024 + 1],
    )
    .expect("oversized signature");
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Limit(_))
    ));

    let fixture = create_fixture();
    fs::write(&fixture.allowed, vec![b'x'; 1024 * 1024 + 1]).expect("oversized signers");
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Limit(_))
    ));

    let fixture = create_fixture();
    let real = fixture.root.join("manifest.json.sig.real");
    fs::rename(fixture.root.join("manifest.json.sig"), &real).expect("rename signature");
    symlink(&real, fixture.root.join("manifest.json.sig")).expect("signature symlink");
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Io(_))
    ));
}

#[test]
fn rejects_invalid_sbom_documents_and_incomplete_identities() {
    let fixture = create_fixture();
    fs::write(fixture.root.join("spdx.json"), b"not JSON").expect("invalid SPDX");
    rewrite_manifest(&fixture, |manifest| {
        manifest.spdx.sha256 = sha256(b"not JSON");
    });
    assert_metadata(&fixture);

    for transform in [
        (|value: &mut Value| value["spdxVersion"] = json!("SPDX-2.2")) as fn(&mut Value),
        |value| {
            value.as_object_mut().expect("object").remove("files");
        },
        |value| value["files"][0]["fileName"] = json!("unknown"),
        |value| {
            let duplicate = value["files"][0].clone();
            value["files"]
                .as_array_mut()
                .expect("files")
                .push(duplicate);
        },
        |value| value["files"][0]["checksums"][0]["checksumValue"] = json!("0".repeat(64)),
    ] {
        let fixture = create_fixture();
        rewrite_sbom(&fixture, "spdx.json", transform);
        assert_metadata(&fixture);
    }

    for transform in [
        (|value: &mut Value| value["specVersion"] = json!("1.5")) as fn(&mut Value),
        |value| {
            value.as_object_mut().expect("object").remove("components");
        },
        |value| value["components"][0]["type"] = json!("application"),
        |value| value["components"][0]["bom-ref"] = json!("unknown"),
        |value| {
            let duplicate = value["components"][0].clone();
            value["components"]
                .as_array_mut()
                .expect("components")
                .push(duplicate);
        },
        |value| value["components"][0]["hashes"][0]["content"] = json!("0".repeat(64)),
    ] {
        let fixture = create_fixture();
        rewrite_sbom(&fixture, "cyclonedx.json", transform);
        assert_metadata(&fixture);
    }
}

#[test]
fn rejects_non_directory_root_and_unsafe_relative_artifact() {
    let fixture = create_fixture();
    assert!(matches!(
        verify_bundle(
            &fixture.root.join("manifest.json"),
            &config(&fixture),
            &target()
        ),
        Err(VerifyError::Topology(_))
    ));

    let fixture = create_fixture();
    rewrite_manifest(&fixture, |manifest| {
        manifest.artifacts[0].path = "/absolute".into();
    });
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Topology(_))
    ));
}

#[test]
fn command_reports_success_and_sanitized_failure() {
    let fixture = create_fixture();
    let output = Command::new(env!("CARGO_BIN_EXE_asb-bundle-verify"))
        .args([
            fixture.root.as_os_str(),
            fixture.allowed.as_os_str(),
            PRINCIPAL.as_ref(),
            OsStr::new("/usr/bin/ssh-keygen"),
            OsStr::new(&config(&fixture).ssh_keygen_sha256),
            OsStr::new("linux"),
            OsStr::new("x86_64"),
            OsStr::new("glibc"),
            OsStr::new("2.39"),
        ])
        .output()
        .expect("run verifier command");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("verified fixture-agent"));

    let output = Command::new(env!("CARGO_BIN_EXE_asb-bundle-verify"))
        .output()
        .expect("run usage failure");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage:"));
}

#[test]
fn rejects_every_target_dimension_and_invalid_signer_process() {
    for wrong in [
        ExpectedTarget {
            operating_system: "freebsd",
            ..target()
        },
        ExpectedTarget {
            libc: "musl",
            ..target()
        },
        ExpectedTarget {
            libc_version: "1.2.5",
            ..target()
        },
    ] {
        let fixture = create_fixture();
        assert!(matches!(
            verify_bundle(&fixture.root, &config(&fixture), &wrong),
            Err(VerifyError::Target(_))
        ));
    }

    let fixture = create_fixture();
    let mut bad = config(&fixture);
    bad.principal = "untrusted".into();
    assert!(matches!(
        verify_bundle(&fixture.root, &bad, &target()),
        Err(VerifyError::Signature)
    ));

    let fixture = create_fixture();
    let mut bad = config(&fixture);
    bad.ssh_keygen = fixture.root.join("bin/agent");
    assert!(matches!(
        verify_bundle(&fixture.root, &bad, &target()),
        Err(VerifyError::Signature)
    ));
}

#[test]
fn rejects_wrong_signature_namespace_and_bounds_the_verifier_process() {
    let fixture = create_fixture();
    sign_with_namespace(&fixture, "other-runtime-bundle");
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Signature)
    ));

    let fixture = create_fixture();
    let hanging_signer = fixture.root.join("hanging-signer");
    fs::write(&hanging_signer, "#!/bin/sh\nsleep 60\n").expect("write hanging signer");
    fs::set_permissions(&hanging_signer, fs::Permissions::from_mode(0o755))
        .expect("make hanging signer executable");
    let mut hanging = config(&fixture);
    hanging.ssh_keygen_sha256 = sha256(&fs::read(&hanging_signer).expect("read hanging signer"));
    hanging.ssh_keygen = hanging_signer;
    let started = Instant::now();
    assert!(matches!(
        verify_bundle(&fixture.root, &hanging, &target()),
        Err(VerifyError::Signature)
    ));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "signature subprocess must be terminated by its monotonic deadline"
    );
}

#[test]
fn rejects_bounded_string_and_hash_variants() {
    for transform in [
        (|manifest: &mut RuntimeBundleManifest| manifest.bundle_id = "x".repeat(4097))
            as fn(&mut RuntimeBundleManifest),
        |manifest| manifest.bundle_id = "bad\nidentity".into(),
        |manifest| manifest.content_sha256 = "0".repeat(63),
        |manifest| manifest.artifacts[0].sha256 = "g".repeat(64),
    ] {
        let fixture = create_fixture();
        rewrite_manifest(&fixture, transform);
        assert_metadata(&fixture);
    }
}

#[test]
fn rejects_oversized_manifest_directory_signature_and_fifo() {
    let fixture = create_fixture();
    fs::write(
        fixture.root.join("manifest.json"),
        vec![b'x'; 1024 * 1024 + 1],
    )
    .expect("oversized manifest");
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Limit(_))
    ));

    let fixture = create_fixture();
    fs::remove_file(fixture.root.join("manifest.json.sig")).expect("remove signature");
    fs::create_dir(fixture.root.join("manifest.json.sig")).expect("directory signature");
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Topology(_))
    ));

    let fixture = create_fixture();
    let fifo = fixture.root.join("fifo");
    assert!(
        Command::new("/usr/bin/mkfifo")
            .arg(&fifo)
            .status()
            .expect("create FIFO")
            .success()
    );
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Topology(_))
    ));
}

#[test]
fn rejects_equal_size_payload_tamper_and_unsigned_sbom_tamper() {
    let fixture = create_fixture();
    fs::write(fixture.root.join("bin/agent"), vec![b'x'; 17]).expect("equal-size tamper");
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Content(_))
    ));

    let fixture = create_fixture();
    fs::write(fixture.root.join("spdx.json"), b"{}").expect("tamper SPDX");
    assert!(matches!(
        verify_bundle(&fixture.root, &config(&fixture), &target()),
        Err(VerifyError::Content(_))
    ));
}

#[test]
fn rejects_missing_sbom_hashes_licenses_and_names() {
    for transform in [
        (|value: &mut Value| {
            value["files"][0]
                .as_object_mut()
                .expect("file")
                .remove("fileName");
        }) as fn(&mut Value),
        |value| {
            value["files"][0]
                .as_object_mut()
                .expect("file")
                .remove("checksums");
        },
    ] {
        let fixture = create_fixture();
        rewrite_sbom(&fixture, "spdx.json", transform);
        assert_metadata(&fixture);
    }
    for transform in [
        (|value: &mut Value| {
            value["components"][0]
                .as_object_mut()
                .expect("component")
                .remove("bom-ref");
        }) as fn(&mut Value),
        |value| {
            value["components"][0]
                .as_object_mut()
                .expect("component")
                .remove("licenses");
        },
    ] {
        let fixture = create_fixture();
        rewrite_sbom(&fixture, "cyclonedx.json", transform);
        assert_metadata(&fixture);
    }
}
