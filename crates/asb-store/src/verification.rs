// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Immutable content-addressed verifier observations.

use super::{
    ArtifactRef, AtomicStore, Id, StoreError, encode_bounded, hex_digest, private_dir,
    read_bounded, sync_dir, validate_artifact_name, validate_component, validate_digest,
    verify_artifact, write_new_synced,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Current immutable verifier-observation schema.
pub const VERIFICATION_SCHEMA_VERSION: u16 = 1;
/// Maximum serialized verifier observation.
pub const MAX_VERIFICATION_BYTES: u64 = 1024 * 1024;

const OBSERVATION_FILE: &str = "verification-observation.json";

/// Terminal execution result before a protected verifier assigns reward.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    /// Immutable task and submission artifacts are ready for protected scoring.
    Ready,
    /// Task acquisition or preparation was invalid.
    TaskFailed,
    /// The submitted patch could not be safely materialized.
    PatchFailed,
    /// Execution exceeded its declared deadline.
    TimedOut,
    /// The isolated execution environment failed.
    EnvironmentFailed,
}

/// Immutable facts shared by every independent score revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationObservation {
    schema_version: u16,
    attempt_id: Id,
    workload_id: Id,
    workload_sha256: String,
    environment: ArtifactRef,
    submission: Option<ArtifactRef>,
    outcome: VerificationOutcome,
}

impl VerificationObservation {
    /// Construct validated content-free observation evidence.
    pub fn new(
        attempt_id: Id,
        workload_id: Id,
        workload_sha256: String,
        environment: ArtifactRef,
        submission: Option<ArtifactRef>,
        outcome: VerificationOutcome,
    ) -> Result<Self, StoreError> {
        let value = Self {
            schema_version: VERIFICATION_SCHEMA_VERSION,
            attempt_id,
            workload_id,
            workload_sha256,
            environment,
            submission,
            outcome,
        };
        value.validate()?;
        Ok(value)
    }

    /// Attempt identity bound to the run manifest.
    pub fn attempt_id(&self) -> &Id {
        &self.attempt_id
    }

    /// Exact workload identity.
    pub fn workload_id(&self) -> &Id {
        &self.workload_id
    }

    /// Exact workload content digest.
    pub fn workload_sha256(&self) -> &str {
        &self.workload_sha256
    }

    /// Content address of the isolated execution environment.
    pub fn environment_sha256(&self) -> &str {
        &self.environment.sha256
    }

    /// Content-addressed isolated execution environment descriptor.
    pub fn environment(&self) -> &ArtifactRef {
        &self.environment
    }

    /// Submitted patch or workspace archive, when execution reached it.
    pub fn submission(&self) -> Option<&ArtifactRef> {
        self.submission.as_ref()
    }

    /// Distinct pre-verifier terminal classification.
    pub const fn outcome(&self) -> VerificationOutcome {
        self.outcome
    }

    fn validate(&self) -> Result<(), StoreError> {
        if self.schema_version != VERIFICATION_SCHEMA_VERSION {
            return Err(StoreError::InvalidVerification);
        }
        validate_public_id("attempt_id", &self.attempt_id.0)?;
        validate_public_id("workload_id", &self.workload_id.0)?;
        validate_digest(&self.workload_sha256)?;
        validate_artifact_name(&self.environment.name)?;
        validate_digest(&self.environment.sha256)?;
        if let Some(artifact) = &self.submission {
            validate_artifact_name(&artifact.name)?;
            validate_digest(&artifact.sha256)?;
        }
        match (self.outcome, self.submission.is_some()) {
            (VerificationOutcome::TaskFailed | VerificationOutcome::EnvironmentFailed, false)
            | (
                VerificationOutcome::Ready
                | VerificationOutcome::PatchFailed
                | VerificationOutcome::TimedOut,
                true,
            ) => Ok(()),
            _ => Err(StoreError::InvalidVerification),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredObservation {
    observation: VerificationObservation,
    sha256: String,
}

impl AtomicStore {
    /// Commit one immutable observation after re-hashing its submission artifact.
    pub fn commit_verification_observation(
        &self,
        run_id: &str,
        observation: &VerificationObservation,
    ) -> Result<String, StoreError> {
        observation.validate()?;
        let _lock = self.lock()?;
        let manifest = self.load_manifest_unlocked(run_id)?;
        if observation.attempt_id != manifest.attempt_id {
            return Err(StoreError::StaleAttempt);
        }
        if let Some(artifact) = &observation.submission {
            verify_artifact(
                &self
                    .run_dir(run_id)?
                    .join("artifacts")
                    .join(&artifact.sha256),
                artifact.size_bytes,
                &artifact.sha256,
            )?;
        }
        verify_artifact(
            &self
                .run_dir(run_id)?
                .join("artifacts")
                .join(&observation.environment.sha256),
            observation.environment.size_bytes,
            &observation.environment.sha256,
        )?;
        let directory = self.run_dir(run_id)?.join("verification");
        let destination = directory.join(OBSERVATION_FILE);
        if destination.exists() {
            return Err(StoreError::VerificationExists);
        }
        let observation_bytes = encode_bounded(
            observation,
            MAX_VERIFICATION_BYTES,
            "verification observation",
        )?;
        let digest = hex_digest(&Sha256::digest(&observation_bytes));
        let stored = StoredObservation {
            observation: observation.clone(),
            sha256: digest.clone(),
        };
        let bytes = encode_bounded(&stored, MAX_VERIFICATION_BYTES, "verification observation")?;
        private_dir(&directory)?;
        write_new_synced(&destination, &bytes)?;
        sync_dir(&directory)?;
        Ok(digest)
    }

    /// Load and verify the immutable observation and its self-address.
    pub fn load_verification_observation(
        &self,
        run_id: &str,
    ) -> Result<(VerificationObservation, String), StoreError> {
        let _lock = self.lock()?;
        self.load_manifest_unlocked(run_id)?;
        let bytes = read_bounded(
            &self
                .run_dir(run_id)?
                .join("verification")
                .join(OBSERVATION_FILE),
            MAX_VERIFICATION_BYTES,
            "verification observation",
        )?;
        let stored: StoredObservation = serde_json::from_slice(&bytes)?;
        stored.observation.validate()?;
        let encoded = encode_bounded(
            &stored.observation,
            MAX_VERIFICATION_BYTES,
            "verification observation",
        )?;
        let actual = hex_digest(&Sha256::digest(&encoded));
        if stored.sha256 != actual {
            return Err(StoreError::InvalidVerification);
        }
        if let Some(artifact) = &stored.observation.submission {
            verify_artifact(
                &self
                    .run_dir(run_id)?
                    .join("artifacts")
                    .join(&artifact.sha256),
                artifact.size_bytes,
                &artifact.sha256,
            )?;
        }
        verify_artifact(
            &self
                .run_dir(run_id)?
                .join("artifacts")
                .join(&stored.observation.environment.sha256),
            stored.observation.environment.size_bytes,
            &stored.observation.environment.sha256,
        )?;
        Ok((stored.observation, actual))
    }
}

fn validate_public_id(kind: &'static str, value: &str) -> Result<(), StoreError> {
    if value.len() > 4096 {
        return Err(StoreError::InvalidIdentity(kind));
    }
    validate_component(kind, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MANIFEST_SCHEMA_VERSION, RunManifest, StoreLimits};
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Root(PathBuf);

    impl Root {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("asb-verification-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn store() -> (Root, AtomicStore) {
        let root = Root::new();
        let store = AtomicStore::open(&root.0, StoreLimits::default()).unwrap();
        store
            .create_run(&RunManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                run_id: Id("run".into()),
                attempt_id: Id("attempt".into()),
                definition: serde_json::json!({}),
            })
            .unwrap();
        (root, store)
    }

    fn environment(store: &AtomicStore) -> ArtifactRef {
        store
            .put_artifact(
                "run",
                "environment.json",
                Cursor::new(br#"{"runtime":"fixture"}"#),
            )
            .unwrap()
    }

    #[test]
    fn observation_is_immutable_and_rehashes_submission() {
        let (root, store) = store();
        let submission = store
            .put_artifact("run", "submission.patch", Cursor::new(b"patch"))
            .unwrap();
        let observation = VerificationObservation::new(
            Id("attempt".into()),
            Id("original.bug-fix".into()),
            "1".repeat(64),
            environment(&store),
            Some(submission.clone()),
            VerificationOutcome::Ready,
        )
        .unwrap();
        let digest = store
            .commit_verification_observation("run", &observation)
            .unwrap();
        assert_eq!(
            store.load_verification_observation("run").unwrap(),
            (observation.clone(), digest)
        );
        assert!(matches!(
            store.commit_verification_observation("run", &observation),
            Err(StoreError::VerificationExists)
        ));
        fs::write(
            root.0.join("runs/run/artifacts").join(&submission.sha256),
            b"evil!",
        )
        .unwrap();
        assert!(matches!(
            store.load_verification_observation("run"),
            Err(StoreError::ArtifactMismatch)
        ));
    }

    #[test]
    fn observation_self_digest_and_attempt_identity_fail_closed() {
        let (root, store) = store();
        let submission = store
            .put_artifact("run", "submission.patch", Cursor::new(b"patch"))
            .unwrap();
        let stale = VerificationObservation::new(
            Id("other-attempt".into()),
            Id("original.bug-fix".into()),
            "1".repeat(64),
            environment(&store),
            Some(submission.clone()),
            VerificationOutcome::Ready,
        )
        .unwrap();
        assert!(matches!(
            store.commit_verification_observation("run", &stale),
            Err(StoreError::StaleAttempt)
        ));

        let observation = VerificationObservation::new(
            Id("attempt".into()),
            Id("original.bug-fix".into()),
            "1".repeat(64),
            environment(&store),
            Some(submission),
            VerificationOutcome::Ready,
        )
        .unwrap();
        store
            .commit_verification_observation("run", &observation)
            .unwrap();
        let path = root
            .0
            .join("runs/run/verification/verification-observation.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["observation"]["environment"]["sha256"] = serde_json::json!("4".repeat(64));
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            store.load_verification_observation("run"),
            Err(StoreError::InvalidVerification)
        ));
    }

    #[test]
    fn failure_classes_and_shape_invariants_are_strict() {
        for (outcome, has_submission, valid) in [
            (VerificationOutcome::TaskFailed, false, true),
            (VerificationOutcome::TaskFailed, true, false),
            (VerificationOutcome::PatchFailed, true, true),
            (VerificationOutcome::TimedOut, true, true),
            (VerificationOutcome::EnvironmentFailed, false, true),
            (VerificationOutcome::Ready, false, false),
        ] {
            let submission = has_submission.then(|| ArtifactRef {
                name: "submission.patch".into(),
                size_bytes: 1,
                sha256: "3".repeat(64),
            });
            let (_root, store) = store();
            assert_eq!(
                VerificationObservation::new(
                    Id("attempt".into()),
                    Id("workload".into()),
                    "1".repeat(64),
                    environment(&store),
                    submission,
                    outcome,
                )
                .is_ok(),
                valid
            );
        }
        let (_root, store) = store();
        let mut invalid_environment = environment(&store);
        invalid_environment.sha256 = "not-a-digest".into();
        assert!(matches!(
            VerificationObservation::new(
                Id("attempt".into()),
                Id("workload".into()),
                "not-a-digest".into(),
                invalid_environment,
                None,
                VerificationOutcome::TaskFailed,
            ),
            Err(StoreError::ArtifactMismatch)
        ));
    }
}
