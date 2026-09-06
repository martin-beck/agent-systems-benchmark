// SPDX-License-Identifier: MIT
#![allow(missing_docs)]

use asb_workloads::{FIXTURE_IDS, OriginalWorkloads};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn scratch_base() -> PathBuf {
    let configured = std::env::var_os("ASB_TEST_SCRATCH")
        .map(|value| ("ASB_TEST_SCRATCH", PathBuf::from(value), false))
        .or_else(|| {
            std::env::var_os("CARGO_TARGET_DIR")
                .map(|value| ("CARGO_TARGET_DIR", PathBuf::from(value), true))
        });
    let Some((name, path, is_target)) = configured else {
        return std::env::temp_dir();
    };
    assert!(path.is_absolute(), "{name} must be an absolute path");
    let base = if is_target {
        path.join("asb-test-scratch")
    } else {
        path
    };
    fs::create_dir_all(&base).unwrap();
    base
}

#[test]
fn public_lifecycle_is_offline_content_free_and_clean() {
    for id in FIXTURE_IDS {
        let root = scratch_base().join(format!(
            "asb-public-workload-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manifest = OriginalWorkloads::describe(id).unwrap();
        assert!(manifest.allowed_network_destinations.is_empty());
        let prepared = OriginalWorkloads::prepare(id, &root).unwrap();
        assert!(!prepared.prompt().is_empty());
        assert!(prepared.workspace().starts_with(&root));
        assert!(!prepared.workspace().join("reference.patch").exists());
        let initial = prepared.evaluate().unwrap();
        assert!(!initial.passed);
        assert!(initial.failed_checks.iter().all(|item| item.is_ascii()));
        prepared.reset().unwrap();
        prepared.cleanup().unwrap();
        assert!(!root.exists());
    }
}

#[test]
fn destination_material_is_never_overwritten() {
    let root = scratch_base().join(format!(
        "asb-public-existing-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("owned"), "caller").unwrap();
    assert!(OriginalWorkloads::prepare(FIXTURE_IDS[0], &root).is_err());
    assert_eq!(fs::read_to_string(root.join("owned")).unwrap(), "caller");
    fs::remove_dir_all(root).unwrap();
}
