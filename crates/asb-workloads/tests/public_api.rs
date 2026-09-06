// SPDX-License-Identifier: MIT
#![allow(missing_docs)]

use asb_workloads::{FIXTURE_IDS, OriginalWorkloads};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn public_lifecycle_is_offline_content_free_and_clean() {
    for id in FIXTURE_IDS {
        let root = std::env::temp_dir().join(format!(
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
    let root = std::env::temp_dir().join(format!(
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
