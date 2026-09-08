// SPDX-License-Identifier: MIT
#![allow(missing_docs)]

use asb_workloads::{
    AdaptationKind, BenchmarkValidityRegistry, ExposureStatus, PlatformEvidence, PortabilityStatus,
    RegistryError,
};
use schemars::schema_for;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const REGISTRY: &str = include_str!("../registry/v1/original-workloads.json");
const SCHEMA: &str = include_str!("../registry/v1/benchmark-validity.schema.json");
const REFERENCES: &[(&str, &str)] = &[
    (
        "original.bug-fix",
        include_str!("../fixtures/v1/bug-fix/reference.patch"),
    ),
    (
        "original.build-repair",
        include_str!("../fixtures/v1/build-repair/reference.patch"),
    ),
    (
        "original.dependency-migration",
        include_str!("../fixtures/v1/dependency-migration/reference.patch"),
    ),
    (
        "original.feature-addition",
        include_str!("../fixtures/v1/feature-addition/reference.patch"),
    ),
    (
        "original.refactoring",
        include_str!("../fixtures/v1/refactoring/reference.patch"),
    ),
    (
        "original.repository-navigation",
        include_str!("../fixtures/v1/repository-navigation/reference.patch"),
    ),
    (
        "original.test-generation",
        include_str!("../fixtures/v1/test-generation/reference.patch"),
    ),
];

fn value() -> Value {
    serde_json::from_str(REGISTRY).unwrap()
}

fn runtime(input: &Value) -> Result<BenchmarkValidityRegistry, RegistryError> {
    BenchmarkValidityRegistry::from_json(&serde_json::to_vec(input).unwrap())
}

#[test]
fn checked_in_schema_is_generated_and_fixture_passes_both_boundaries() {
    let generated = schema_for!(BenchmarkValidityRegistry);
    let checked: Value = serde_json::from_str(SCHEMA).unwrap();
    assert_eq!(serde_json::to_value(generated).unwrap(), checked);
    assert!(
        jsonschema::validator_for(&checked)
            .unwrap()
            .is_valid(&value())
    );
    let registry = BenchmarkValidityRegistry::built_in().unwrap();
    assert_eq!(registry.workloads.len(), 7);
    assert!(registry.find("original.bug-fix", "1.0.0").is_some());
    assert!(registry.find("original.bug-fix", "2.0.0").is_none());
    for item in registry.workloads {
        assert_eq!(item.source.kind, asb_workloads::SourceKind::BuiltIn);
        assert_eq!(item.exposure.status, ExposureStatus::Public);
        assert!(item.performance_calibrations.is_empty());
        assert!(item.portability.iter().all(|cell| {
            cell.status == PortabilityStatus::Planned
                && cell.adaptation == AdaptationKind::None
                && cell.evidence.is_none()
        }));
        let reference = REFERENCES
            .iter()
            .find(|(id, _)| *id == item.workload_id)
            .unwrap()
            .1;
        assert_eq!(
            item.baseline.reference_artifact_sha256,
            format!("{:x}", Sha256::digest(reference.as_bytes()))
        );
    }
}

#[test]
fn closed_schema_and_runtime_reject_unknown_or_malformed_fields() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for bad in [
        {
            let mut bad = value();
            bad["secret"] = json!(true);
            bad
        },
        {
            let mut bad = value();
            bad["format_version"] = json!(2);
            bad
        },
        {
            let mut bad = value();
            bad["workloads"][0]["source"]["content_sha256"] = json!("A".repeat(64));
            bad
        },
        {
            let mut bad = value();
            bad["workloads"][0]["split"]["instance_count"] = json!(0);
            bad
        },
        {
            let mut bad = value();
            bad["workloads"][0]["portability"] = json!([]);
            bad
        },
    ] {
        assert!(!validator.is_valid(&bad));
        assert!(runtime(&bad).is_err());
    }
}

#[test]
fn semantic_cross_field_negatives_fail_closed() {
    let changes: &[fn(&mut Value)] = &[
        |bad| bad["workloads"].as_array_mut().unwrap().swap(0, 1),
        |bad| bad["workloads"][0]["baseline"]["reference_passed"] = json!(false),
        |bad| bad["workloads"][0]["baseline"]["counterexamples_rejected"] = json!(1),
        |bad| bad["workloads"][0]["exposure"]["holdout_set_sha256"] = json!("0".repeat(64)),
        |bad| bad["workloads"][0]["source"]["verified_on"] = json!("2026-02-30"),
        |bad| bad["workloads"][0]["limitations"][0] = json!("/private/location"),
        |bad| {
            bad["workloads"][0]["source"]["kind"] = json!("imported");
            bad["workloads"][0]["source"]["locator"] = json!("file:///local/source");
        },
        |bad| bad["workloads"][0]["portability"][0]["status"] = json!("native-tested"),
        |bad| bad["workloads"][0]["portability"][0]["adaptation"] = json!("translated"),
        |bad| {
            bad["workloads"][0]["performance_calibrations"] = json!([{
                "platform_id":"debian-13","architecture":"x86_64",
                "host_class_sha256":"0".repeat(64),"metric":"speedup",
                "threshold":1.1,"uncertainty":0.1,"sample_count":2,
                "artifact_sha256":"1".repeat(64)
            }])
        },
    ];
    for change in changes {
        let mut bad = value();
        change(&mut bad);
        assert!(runtime(&bad).is_err());
    }

    let mut duplicate = value();
    let first = duplicate["workloads"][0].clone();
    duplicate["workloads"]
        .as_array_mut()
        .unwrap()
        .insert(1, first);
    assert_eq!(
        runtime(&duplicate),
        Err(RegistryError::DuplicateOrUnordered)
    );
}

#[test]
fn explicit_holdout_native_adaptation_and_calibration_evidence_can_validate() {
    let mut positive = value();
    positive["workloads"][0]["exposure"] = json!({
        "status":"holdout","first_public_on":null,"holdout_set_sha256":"2".repeat(64)
    });
    positive["workloads"][0]["portability"][1] = json!({
        "platform_id":"ubuntu-24.04","architecture":"x86_64","status":"native-tested",
        "adaptation":"rebuilt","adaptation_sha256":"3".repeat(64),
        "semantic_parity_sha256":"4".repeat(64),
        "evidence":{"run_id":"public-run-1","artifact_sha256":"5".repeat(64),
                    "kernel_release":"6.8.0-generic","tested_on":"2026-09-07"},
        "limitation":"Rebuilt packaging; semantic parity is separately content-addressed."
    });
    positive["workloads"][0]["performance_calibrations"] = json!([{
        "platform_id":"ubuntu-24.04","architecture":"x86_64",
        "host_class_sha256":"6".repeat(64),"metric":"paired-speedup",
        "threshold":1.1,"uncertainty":0.05,"sample_count":8,
        "artifact_sha256":"7".repeat(64)
    }]);
    runtime(&positive).unwrap();

    let mut missing_kernel = positive.clone();
    missing_kernel["workloads"][0]["portability"][1]["evidence"]["kernel_release"] = Value::Null;
    assert!(runtime(&missing_kernel).is_err());

    let mut simulated_with_kernel = value();
    simulated_with_kernel["workloads"][0]["portability"][0]["status"] = json!("simulated");
    simulated_with_kernel["workloads"][0]["portability"][0]["evidence"] = json!({
        "run_id":"public-run-2","artifact_sha256":"8".repeat(64),
        "kernel_release":"host-kernel-must-not-qualify-guest","tested_on":"2026-09-07"
    });
    assert!(runtime(&simulated_with_kernel).is_err());

    let mut performance_without_native = value();
    performance_without_native["workloads"][0]["performance_calibrations"] = json!([{
        "platform_id":"ubuntu-24.04","architecture":"x86_64",
        "host_class_sha256":"6".repeat(64),"metric":"paired-speedup",
        "threshold":1.1,"uncertainty":0.05,"sample_count":8,
        "artifact_sha256":"7".repeat(64)
    }]);
    assert!(runtime(&performance_without_native).is_err());
}

#[test]
fn document_and_direct_numeric_bounds_are_enforced() {
    let oversized = vec![b' '; asb_workloads::MAX_REGISTRY_BYTES + 1];
    assert_eq!(
        BenchmarkValidityRegistry::from_json(&oversized),
        Err(RegistryError::DocumentTooLarge)
    );

    let mut registry = BenchmarkValidityRegistry::built_in().unwrap();
    registry.workloads[0]
        .performance_calibrations
        .push(asb_workloads::PerformanceCalibration {
            platform_id: "ubuntu-24.04".into(),
            architecture: "x86_64".into(),
            host_class_sha256: "0".repeat(64),
            metric: "paired-speedup".into(),
            threshold: f64::NAN,
            uncertainty: 0.1,
            sample_count: 2,
            artifact_sha256: "1".repeat(64),
        });
    assert_eq!(registry.validate(), Err(RegistryError::InvalidField));

    let evidence = PlatformEvidence {
        run_id: "run".into(),
        artifact_sha256: "0".repeat(64),
        kernel_release: Some("kernel".into()),
        tested_on: "2026-09-07".into(),
    };
    assert_eq!(evidence.kernel_release.as_deref(), Some("kernel"));
}
