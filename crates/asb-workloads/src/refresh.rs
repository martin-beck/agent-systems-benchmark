// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Content-addressed refresh manifests for evolving literature workloads.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

const HEX64: usize = 64;

/// A versioned, immutable identity for one literature evaluation window.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshManifestV1 {
    /// Schema version of this manifest.
    pub schema_version: u32,
    /// Evolving workload identity (`livecodebench` or `swe-rebench`).
    pub workload_id: String,
    /// Immutable source repository revision.
    pub source_revision: String,
    /// Immutable dataset or task-selection revision.
    pub dataset_revision: String,
    /// Human-readable but immutable window/split identity.
    pub evaluation_window: String,
    /// Contamination cutoff identity, never an unbounded date claim.
    pub contamination_cutoff: String,
    /// Digest of the exact selected task split.
    pub split_sha256: String,
    /// Immutable evaluator revision and image identity.
    pub evaluator_revision: String,
    /// Evaluator image digest, when execution is qualified.
    pub image_sha256: Option<String>,
    /// Evaluator SBOM digest, when execution is qualified.
    pub sbom_sha256: Option<String>,
    /// License evidence state.
    pub license_state: String,
    /// Qualification evidence state (`qualified`, `planned`, or `unavailable`).
    pub evidence_status: String,
    /// Content address over every field except this digest.
    pub identity_sha256: String,
}

/// Input fields used to construct a content-addressed refresh manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefreshManifestInput {
    /// Evolving workload identity.
    pub workload_id: String,
    /// Immutable source revision.
    pub source_revision: String,
    /// Immutable dataset revision.
    pub dataset_revision: String,
    /// Evaluation window identity.
    pub evaluation_window: String,
    /// Contamination cutoff identity.
    pub contamination_cutoff: String,
    /// Selected split digest.
    pub split_sha256: String,
    /// Evaluator revision.
    pub evaluator_revision: String,
    /// Evaluator image digest.
    pub image_sha256: Option<String>,
    /// Evaluator SBOM digest.
    pub sbom_sha256: Option<String>,
    /// License evidence state.
    pub license_state: String,
    /// Qualification evidence state.
    pub evidence_status: String,
}

/// Errors from refresh-manifest parsing, identity, or compatibility checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshManifestError {
    /// The schema version is not supported.
    UnsupportedSchema,
    /// A required textual field is empty or malformed.
    MissingField,
    /// A digest is not lowercase hexadecimal SHA-256.
    InvalidDigest,
    /// The workload is not an evolving workload supported by this contract.
    UnsupportedWorkload,
    /// A planned/unavailable record lacks the evidence required for selection.
    IncompleteEvidence,
    /// The content address does not match the manifest fields.
    IdentityMismatch,
    /// Two results belong to different immutable windows.
    WindowMismatch,
}

impl fmt::Display for RefreshManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedSchema => "unsupported refresh manifest schema",
            Self::MissingField => "refresh manifest has a missing field",
            Self::InvalidDigest => "refresh manifest has an invalid digest",
            Self::UnsupportedWorkload => "refresh manifest workload is not evolving",
            Self::IncompleteEvidence => "refresh manifest evidence is incomplete",
            Self::IdentityMismatch => "refresh manifest identity does not match its content",
            Self::WindowMismatch => "refresh manifests belong to different windows",
        })
    }
}

impl std::error::Error for RefreshManifestError {}

impl RefreshManifestV1 {
    /// Construct and content-address a refresh manifest without network access.
    pub fn new(input: RefreshManifestInput) -> Result<Self, RefreshManifestError> {
        let mut manifest = Self {
            schema_version: 1,
            workload_id: input.workload_id,
            source_revision: input.source_revision,
            dataset_revision: input.dataset_revision,
            evaluation_window: input.evaluation_window,
            contamination_cutoff: input.contamination_cutoff,
            split_sha256: input.split_sha256,
            evaluator_revision: input.evaluator_revision,
            image_sha256: input.image_sha256,
            sbom_sha256: input.sbom_sha256,
            license_state: input.license_state,
            evidence_status: input.evidence_status,
            identity_sha256: String::new(),
        };
        manifest.validate_fields()?;
        manifest.identity_sha256 = manifest.compute_identity();
        Ok(manifest)
    }

    /// Parse and validate a strict JSON manifest.
    pub fn from_json(bytes: &[u8]) -> Result<Self, RefreshManifestError> {
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|_| RefreshManifestError::MissingField)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate fields, content address, and evidence boundary.
    pub fn validate(&self) -> Result<(), RefreshManifestError> {
        self.validate_fields()?;
        if self.identity_sha256 != self.compute_identity() {
            return Err(RefreshManifestError::IdentityMismatch);
        }
        Ok(())
    }

    /// Whether this manifest may enter an executable selection.
    #[must_use]
    pub fn selectable(&self) -> bool {
        self.evidence_status == "qualified"
            && self.license_state == "verified"
            && self.image_sha256.is_some()
            && self.sbom_sha256.is_some()
    }

    /// Compare two results without permitting implicit cross-window mixing.
    pub fn ensure_compatible(left: &Self, right: &Self) -> Result<(), RefreshManifestError> {
        left.validate()?;
        right.validate()?;
        if left.identity_sha256 != right.identity_sha256 {
            return Err(RefreshManifestError::WindowMismatch);
        }
        Ok(())
    }

    fn validate_fields(&self) -> Result<(), RefreshManifestError> {
        if self.schema_version != 1 {
            return Err(RefreshManifestError::UnsupportedSchema);
        }
        if !matches!(self.workload_id.as_str(), "livecodebench" | "swe-rebench") {
            return Err(RefreshManifestError::UnsupportedWorkload);
        }
        for value in [
            &self.workload_id,
            &self.source_revision,
            &self.dataset_revision,
            &self.evaluation_window,
            &self.contamination_cutoff,
            &self.split_sha256,
            &self.evaluator_revision,
            &self.license_state,
            &self.evidence_status,
        ] {
            if value.trim().is_empty() {
                return Err(RefreshManifestError::MissingField);
            }
        }
        if !valid_digest(&self.split_sha256) {
            return Err(RefreshManifestError::InvalidDigest);
        }
        for value in [&self.image_sha256, &self.sbom_sha256]
            .into_iter()
            .flatten()
        {
            if !valid_digest(value) {
                return Err(RefreshManifestError::InvalidDigest);
            }
        }
        if !matches!(
            self.evidence_status.as_str(),
            "qualified" | "planned" | "unavailable"
        ) || (self.evidence_status == "qualified"
            && (self.image_sha256.is_none()
                || self.sbom_sha256.is_none()
                || self.license_state != "verified"))
        {
            return Err(RefreshManifestError::IncompleteEvidence);
        }
        if !valid_digest(&self.identity_sha256) && !self.identity_sha256.is_empty() {
            return Err(RefreshManifestError::InvalidDigest);
        }
        Ok(())
    }

    fn compute_identity(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"asb-literature-refresh-manifest-v1\0");
        for value in [
            self.workload_id.as_str(),
            self.source_revision.as_str(),
            self.dataset_revision.as_str(),
            self.evaluation_window.as_str(),
            self.contamination_cutoff.as_str(),
            self.split_sha256.as_str(),
            self.evaluator_revision.as_str(),
            self.image_sha256.as_deref().unwrap_or(""),
            self.sbom_sha256.as_deref().unwrap_or(""),
            self.license_state.as_str(),
            self.evidence_status.as_str(),
        ] {
            hasher.update(value.as_bytes());
            hasher.update([0]);
        }
        format!("{:x}", hasher.finalize())
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == HEX64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(window: &str, evaluator: &str) -> RefreshManifestV1 {
        RefreshManifestV1::new(RefreshManifestInput {
            workload_id: "livecodebench".into(),
            source_revision: "a".repeat(40),
            dataset_revision: "b".repeat(40),
            evaluation_window: window.into(),
            contamination_cutoff: "cutoff-20260927".into(),
            split_sha256: "c".repeat(64),
            evaluator_revision: evaluator.into(),
            image_sha256: Some("d".repeat(64)),
            sbom_sha256: Some("e".repeat(64)),
            license_state: "verified".into(),
            evidence_status: "qualified".into(),
        })
        .unwrap()
    }

    #[test]
    fn content_addressed_manifest_is_deterministic_and_selectable() {
        let first = manifest("release-v6", "evaluator-a");
        let second = manifest("release-v6", "evaluator-a");
        assert_eq!(first, second);
        assert!(first.selectable());
        assert_eq!(
            RefreshManifestV1::from_json(&serde_json::to_vec(&first).unwrap()),
            Ok(first)
        );
    }

    #[test]
    fn changed_window_or_evaluator_cannot_compare() {
        let first = manifest("release-v6", "evaluator-a");
        let changed_window = manifest("release-v7", "evaluator-a");
        let changed_evaluator = manifest("release-v6", "evaluator-b");
        assert_eq!(
            RefreshManifestV1::ensure_compatible(&first, &changed_window),
            Err(RefreshManifestError::WindowMismatch)
        );
        assert_eq!(
            RefreshManifestV1::ensure_compatible(&first, &changed_evaluator),
            Err(RefreshManifestError::WindowMismatch)
        );
    }

    #[test]
    fn missing_evidence_stays_inspection_only_and_tampering_fails_closed() {
        let mut planned = manifest("window-1", "evaluator-a");
        planned.evidence_status = "planned".into();
        planned.identity_sha256 = planned.compute_identity();
        assert!(!planned.selectable());
        assert!(RefreshManifestV1::from_json(&serde_json::to_vec(&planned).unwrap()).is_ok());

        let mut tampered = manifest("window-1", "evaluator-a");
        tampered.dataset_revision = "changed".into();
        assert_eq!(
            tampered.validate(),
            Err(RefreshManifestError::IdentityMismatch)
        );
        let mut unknown = serde_json::to_value(manifest("window-1", "evaluator-a")).unwrap();
        unknown["unexpected"] = serde_json::json!(true);
        assert!(RefreshManifestV1::from_json(&serde_json::to_vec(&unknown).unwrap()).is_err());
    }

    #[test]
    fn unsupported_or_invalid_manifest_is_rejected() {
        assert_eq!(
            RefreshManifestV1::new(RefreshManifestInput {
                workload_id: "swe-bench".into(),
                source_revision: "a".into(),
                dataset_revision: "b".into(),
                evaluation_window: "window".into(),
                contamination_cutoff: "cutoff".into(),
                split_sha256: "c".repeat(64),
                evaluator_revision: "evaluator".into(),
                image_sha256: None,
                sbom_sha256: None,
                license_state: "unknown".into(),
                evidence_status: "planned".into(),
            }),
            Err(RefreshManifestError::UnsupportedWorkload)
        );
        let mut value = manifest("window-1", "evaluator-a");
        value.split_sha256 = "bad".into();
        assert_eq!(value.validate(), Err(RefreshManifestError::InvalidDigest));
    }
}
