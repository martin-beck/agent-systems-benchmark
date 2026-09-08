// SPDX-License-Identifier: MIT
//! Constructor-controlled offline scoring over immutable observations.

use asb_protocol::ArtifactRef;
use asb_store::{AtomicStore, StoreError, VerificationObservation, VerificationOutcome};
use asb_workloads::{GradeReport, VerifierSpec};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt;
use std::io::Cursor;

/// Current append-only score-ledger schema.
pub const SCORE_LEDGER_SCHEMA_VERSION: u16 = 1;
/// Maximum encoded score ledger.
pub const MAX_SCORE_LEDGER_BYTES: usize = 1024 * 1024;
/// Maximum revisions for one immutable observation.
pub const MAX_SCORE_REVISIONS: usize = 4096;

/// Protected verifier terminal result.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreOutcome {
    /// Every protected check passed.
    Passed,
    /// One or more protected checks rejected the submission.
    Failed,
    /// The independent verifier failed; reward remains absent, never zero.
    VerifierFailed,
}

/// One constructor-controlled immutable score revision.
///
/// Fields are private so downstream code cannot forge a passing reward.
///
/// ```compile_fail
/// use asb_analysis::{ScoreOutcome, ScoreRevision};
///
/// let _forged = ScoreRevision {
///     outcome: ScoreOutcome::Passed,
///     reward_millionths: Some(1_000_000),
/// };
/// ```
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreRevision {
    schema_version: u16,
    revision: u32,
    parent_sha256: Option<String>,
    observation_sha256: String,
    scorer_revision: String,
    verifier_contract_sha256: String,
    verifier_artifact_sha256: String,
    outcome: ScoreOutcome,
    reward_millionths: Option<u32>,
}

impl ScoreRevision {
    /// Zero-based revision index.
    pub const fn revision(&self) -> u32 {
        self.revision
    }

    /// Digest of the prior revision, absent only for revision zero.
    pub fn parent_sha256(&self) -> Option<&str> {
        self.parent_sha256.as_deref()
    }

    /// Exact immutable observation digest.
    pub fn observation_sha256(&self) -> &str {
        &self.observation_sha256
    }

    /// Independently versioned scorer identity.
    pub fn scorer_revision(&self) -> &str {
        &self.scorer_revision
    }

    /// Protected verifier contract digest.
    pub fn verifier_contract_sha256(&self) -> &str {
        &self.verifier_contract_sha256
    }

    /// Independently re-hashed verifier artifact digest.
    pub fn verifier_artifact_sha256(&self) -> &str {
        &self.verifier_artifact_sha256
    }

    /// Protected terminal classification.
    pub const fn outcome(&self) -> ScoreOutcome {
        self.outcome
    }

    /// Fixed-point binary reward; absent only for verifier failure.
    pub const fn reward_millionths(&self) -> Option<u32> {
        self.reward_millionths
    }

    fn validate(
        &self,
        index: usize,
        parent: Option<&str>,
        observation: &str,
    ) -> Result<(), ScoreError> {
        if self.schema_version != SCORE_LEDGER_SCHEMA_VERSION
            || usize::try_from(self.revision).ok() != Some(index)
            || self.parent_sha256.as_deref() != parent
            || self.observation_sha256 != observation
            || !token(&self.scorer_revision)
            || !digest(&self.verifier_contract_sha256)
            || !digest(&self.verifier_artifact_sha256)
        {
            return Err(ScoreError::InvalidLedger);
        }
        match (self.outcome, self.reward_millionths) {
            (ScoreOutcome::Passed, Some(1_000_000))
            | (ScoreOutcome::Failed, Some(0))
            | (ScoreOutcome::VerifierFailed, None) => Ok(()),
            _ => Err(ScoreError::InvalidLedger),
        }
    }
}

/// Append-only offline score history for one immutable observation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreLedger {
    schema_version: u16,
    observation_sha256: String,
    revisions: Vec<ScoreRevision>,
    #[serde(skip)]
    authoritative: bool,
}

impl ScoreLedger {
    /// Begin a ledger from the exact observation currently committed for a run.
    pub fn open(store: &AtomicStore, run_id: &str) -> Result<Self, ScoreError> {
        let (observation, observation_sha256) = store.load_verification_observation(run_id)?;
        if observation.outcome() != VerificationOutcome::Ready {
            return Err(ScoreError::ObservationNotScorable);
        }
        Ok(Self {
            schema_version: SCORE_LEDGER_SCHEMA_VERSION,
            observation_sha256,
            revisions: Vec::new(),
            authoritative: true,
        })
    }

    /// Decode and fully validate a bounded prior ledger.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ScoreError> {
        if bytes.len() > MAX_SCORE_LEDGER_BYTES {
            return Err(ScoreError::LedgerTooLarge);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| ScoreError::InvalidLedger)?;
        value.validate()?;
        Ok(value)
    }

    /// Encode canonical struct-order JSON for content-addressed storage.
    pub fn to_json(&self) -> Result<Vec<u8>, ScoreError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| ScoreError::InvalidLedger)?;
        if bytes.len() > MAX_SCORE_LEDGER_BYTES {
            return Err(ScoreError::LedgerTooLarge);
        }
        Ok(bytes)
    }

    /// Immutable observation digest shared by every revision.
    pub fn observation_sha256(&self) -> &str {
        &self.observation_sha256
    }

    /// Contiguous score revisions.
    pub fn revisions(&self) -> &[ScoreRevision] {
        &self.revisions
    }

    /// Append a result that only a protected workload grader can construct.
    pub fn append_grade(
        &mut self,
        store: &AtomicStore,
        run_id: &str,
        verifier: &VerifierSpec,
        verifier_artifact: &ArtifactRef,
        report: &GradeReport,
    ) -> Result<&ScoreRevision, ScoreError> {
        self.require_authoritative()?;
        let (observation, digest) =
            self.verify_inputs(store, run_id, verifier, verifier_artifact)?;
        if report.workload_id() != observation.workload_id().0
            || report.workload_sha256() != observation.workload_sha256()
            || report.scoring_version() != verifier.scoring_version()
            || report.verifier_contract_sha256() != verifier.contract_sha256()
        {
            return Err(ScoreError::EvidenceMismatch);
        }
        let (outcome, reward_millionths) = if report.passed() {
            (ScoreOutcome::Passed, Some(1_000_000))
        } else {
            (ScoreOutcome::Failed, Some(0))
        };
        self.append(
            digest,
            report.scoring_version(),
            verifier.contract_sha256(),
            &verifier_artifact.sha256,
            outcome,
            reward_millionths,
        )
    }

    /// Append a content-addressed verifier failure without inventing zero reward.
    pub fn append_verifier_failure(
        &mut self,
        store: &AtomicStore,
        run_id: &str,
        verifier: &VerifierSpec,
        verifier_artifact: &ArtifactRef,
    ) -> Result<&ScoreRevision, ScoreError> {
        self.require_authoritative()?;
        let (_, digest) = self.verify_inputs(store, run_id, verifier, verifier_artifact)?;
        self.append(
            digest,
            verifier.scoring_version(),
            verifier.contract_sha256(),
            &verifier_artifact.sha256,
            ScoreOutcome::VerifierFailed,
            None,
        )
    }

    /// Persist the complete validated ledger as a new content-addressed artifact.
    ///
    /// Existing score artifacts remain immutable, so rescoring adds evidence
    /// rather than overwriting an earlier reward.
    pub fn persist(&self, store: &AtomicStore, run_id: &str) -> Result<ArtifactRef, ScoreError> {
        self.require_authoritative()?;
        let bytes = self.to_json()?;
        let name = format!("score-ledger-{:04}.json", self.revisions.len());
        store
            .put_artifact(run_id, &name, Cursor::new(bytes))
            .map_err(ScoreError::Store)
    }

    fn verify_inputs(
        &self,
        store: &AtomicStore,
        run_id: &str,
        verifier: &VerifierSpec,
        verifier_artifact: &ArtifactRef,
    ) -> Result<(VerificationObservation, String), ScoreError> {
        self.validate()?;
        let (observation, observation_sha256) = store.load_verification_observation(run_id)?;
        store.verify_artifact_ref(run_id, verifier_artifact)?;
        if observation.outcome() != VerificationOutcome::Ready {
            return Err(ScoreError::ObservationNotScorable);
        }
        if observation_sha256 != self.observation_sha256
            || observation.workload_id().0 != verifier.workload_id()
            || observation.workload_sha256() != verifier.workload_sha256()
        {
            return Err(ScoreError::EvidenceMismatch);
        }
        Ok((observation, observation_sha256))
    }

    fn require_authoritative(&self) -> Result<(), ScoreError> {
        if self.authoritative {
            Ok(())
        } else {
            Err(ScoreError::UntrustedLedger)
        }
    }

    fn append(
        &mut self,
        observation_sha256: String,
        scorer_revision: &str,
        verifier_contract_sha256: &str,
        verifier_artifact_sha256: &str,
        outcome: ScoreOutcome,
        reward_millionths: Option<u32>,
    ) -> Result<&ScoreRevision, ScoreError> {
        if self.revisions.len() >= MAX_SCORE_REVISIONS {
            return Err(ScoreError::TooManyRevisions);
        }
        let revision =
            u32::try_from(self.revisions.len()).map_err(|_| ScoreError::TooManyRevisions)?;
        let parent_sha256 = self.revisions.last().map(revision_digest).transpose()?;
        let score = ScoreRevision {
            schema_version: SCORE_LEDGER_SCHEMA_VERSION,
            revision,
            parent_sha256,
            observation_sha256,
            scorer_revision: scorer_revision.to_owned(),
            verifier_contract_sha256: verifier_contract_sha256.to_owned(),
            verifier_artifact_sha256: verifier_artifact_sha256.to_owned(),
            outcome,
            reward_millionths,
        };
        score.validate(
            self.revisions.len(),
            self.revisions
                .last()
                .map(revision_digest)
                .transpose()?
                .as_deref(),
            &self.observation_sha256,
        )?;
        self.revisions.push(score);
        Ok(self.revisions.last().expect("score just appended"))
    }

    fn validate(&self) -> Result<(), ScoreError> {
        if self.schema_version != SCORE_LEDGER_SCHEMA_VERSION
            || !digest(&self.observation_sha256)
            || self.revisions.len() > MAX_SCORE_REVISIONS
        {
            return Err(ScoreError::InvalidLedger);
        }
        let mut parent = None;
        for (index, revision) in self.revisions.iter().enumerate() {
            revision.validate(index, parent.as_deref(), &self.observation_sha256)?;
            parent = Some(revision_digest(revision)?);
        }
        Ok(())
    }
}

/// Fail-closed scoring or persistence failure without submitted content.
#[derive(Debug)]
pub enum ScoreError {
    /// Durable storage or artifact verification failed.
    Store(StoreError),
    /// Observation is a task, patch, timeout, or environment failure.
    ObservationNotScorable,
    /// Workload, verifier, observation, or protected result identities disagree.
    EvidenceMismatch,
    /// Ledger JSON, chain, reward, or identity is inconsistent.
    InvalidLedger,
    /// Encoded ledger exceeds the hard byte ceiling.
    LedgerTooLarge,
    /// Revision count reached the hard ceiling.
    TooManyRevisions,
    /// Decoded unkeyed JSON is inspectable but cannot become authoritative.
    UntrustedLedger,
}

impl fmt::Display for ScoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Store(_) => "score storage failed",
            Self::ObservationNotScorable => "observation is not scorable",
            Self::EvidenceMismatch => "score evidence identity mismatch",
            Self::InvalidLedger => "score ledger is invalid",
            Self::LedgerTooLarge => "score ledger exceeds the byte bound",
            Self::TooManyRevisions => "score ledger exceeds the revision bound",
            Self::UntrustedLedger => "decoded score ledger is not authoritative",
        })
    }
}

impl Error for ScoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StoreError> for ScoreError {
    fn from(value: StoreError) -> Self {
        Self::Store(value)
    }
}

fn revision_digest(revision: &ScoreRevision) -> Result<String, ScoreError> {
    let bytes = serde_json::to_vec(revision).map_err(|_| ScoreError::InvalidLedger)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_protocol::Id;
    use asb_store::{MANIFEST_SCHEMA_VERSION, RunManifest, StoreLimits};
    use asb_workloads::OriginalWorkloads;
    use std::fs;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Root(PathBuf);

    impl Root {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("asb-score-{label}-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&root).expect("private scratch");
            Self(root)
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn ready() -> (
        Root,
        AtomicStore,
        VerifierSpec,
        ArtifactRef,
        asb_workloads::PreparedWorkload,
    ) {
        let root = Root::new("ready");
        let store = AtomicStore::open(root.0.join("store"), StoreLimits::default()).unwrap();
        store
            .create_run(&RunManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                run_id: Id("run".into()),
                attempt_id: Id("attempt".into()),
                definition: serde_json::json!({"fixture": "original.bug-fix"}),
            })
            .unwrap();
        let submission = store
            .put_artifact("run", "submission.patch", Cursor::new(b"bounded patch"))
            .unwrap();
        let environment = store
            .put_artifact(
                "run",
                "environment.json",
                Cursor::new(br#"{"runtime":"fixture"}"#),
            )
            .unwrap();
        let verifier = OriginalWorkloads::verifier_spec("original.bug-fix").unwrap();
        let observation = VerificationObservation::new(
            Id("attempt".into()),
            Id(verifier.workload_id().into()),
            verifier.workload_sha256().into(),
            environment,
            Some(submission),
            VerificationOutcome::Ready,
        )
        .unwrap();
        store
            .commit_verification_observation("run", &observation)
            .unwrap();
        let verifier_artifact = store
            .put_artifact(
                "run",
                "protected-verifier.bin",
                Cursor::new(b"asb protected verifier fixture"),
            )
            .unwrap();
        let prepared =
            OriginalWorkloads::prepare("original.bug-fix", root.0.join("prepared")).unwrap();
        (root, store, verifier, verifier_artifact, prepared)
    }

    #[test]
    fn protected_grades_append_immutable_score_revisions() {
        let (_root, store, verifier, verifier_artifact, prepared) = ready();
        let failed = prepared.evaluate_with_verifier(&verifier).unwrap();
        assert!(!failed.passed());
        let mut ledger = ScoreLedger::open(&store, "run").unwrap();
        let first = ledger
            .append_grade(&store, "run", &verifier, &verifier_artifact, &failed)
            .unwrap();
        assert_eq!(first.outcome(), ScoreOutcome::Failed);
        assert_eq!(first.reward_millionths(), Some(0));
        assert_eq!(first.revision(), 0);
        let first_artifact = ledger.persist(&store, "run").unwrap();

        fs::write(
            prepared.workspace().join("parser.py"),
            "def parse_line(line):\n    if line.endswith(\"\\r\"):\n        line = line[:-1]\n    return line\n",
        )
        .unwrap();
        let passed = prepared.evaluate_with_verifier(&verifier).unwrap();
        assert!(passed.passed());
        let second = ledger
            .append_grade(&store, "run", &verifier, &verifier_artifact, &passed)
            .unwrap();
        assert_eq!(second.outcome(), ScoreOutcome::Passed);
        assert_eq!(second.reward_millionths(), Some(1_000_000));
        assert_eq!(second.revision(), 1);
        assert!(second.parent_sha256().is_some());
        let second_artifact = ledger.persist(&store, "run").unwrap();
        assert_ne!(first_artifact.sha256, second_artifact.sha256);
        store
            .verify_artifact_ref("run", &first_artifact)
            .expect("prior score remains immutable");
        let mut round_trip = ScoreLedger::from_json(&ledger.to_json().unwrap()).unwrap();
        assert_eq!(round_trip.observation_sha256(), ledger.observation_sha256());
        assert_eq!(round_trip.revisions(), ledger.revisions());
        assert!(matches!(
            round_trip.append_grade(&store, "run", &verifier, &verifier_artifact, &passed),
            Err(ScoreError::UntrustedLedger)
        ));
        assert!(matches!(
            round_trip.persist(&store, "run"),
            Err(ScoreError::UntrustedLedger)
        ));
    }

    #[test]
    fn verifier_failures_and_identity_or_artifact_tampering_fail_closed() {
        let (root, store, verifier, verifier_artifact, prepared) = ready();
        let report = prepared.evaluate_with_verifier(&verifier).unwrap();
        let mut ledger = ScoreLedger::open(&store, "run").unwrap();
        assert_eq!(
            ledger
                .append_verifier_failure(&store, "run", &verifier, &verifier_artifact)
                .unwrap()
                .reward_millionths(),
            None
        );

        let wrong = OriginalWorkloads::verifier_spec("original.feature-addition").unwrap();
        assert!(matches!(
            ledger.append_grade(&store, "run", &wrong, &verifier_artifact, &report),
            Err(ScoreError::EvidenceMismatch)
        ));

        fs::write(
            root.0
                .join("store/runs/run/artifacts")
                .join(&verifier_artifact.sha256),
            b"replaced",
        )
        .unwrap();
        assert!(matches!(
            ledger.append_grade(&store, "run", &verifier, &verifier_artifact, &report),
            Err(ScoreError::Store(StoreError::ArtifactMismatch))
        ));
    }

    #[test]
    fn ledger_decode_rejects_chain_reward_shape_and_size_tampering() {
        let (_root, store, verifier, verifier_artifact, prepared) = ready();
        let report = prepared.evaluate_with_verifier(&verifier).unwrap();
        let mut ledger = ScoreLedger::open(&store, "run").unwrap();
        ledger
            .append_grade(&store, "run", &verifier, &verifier_artifact, &report)
            .unwrap();
        let bytes = ledger.to_json().unwrap();

        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["revisions"][0]["reward_millionths"] = serde_json::json!(1_000_000);
        assert!(matches!(
            ScoreLedger::from_json(&serde_json::to_vec(&value).unwrap()),
            Err(ScoreError::InvalidLedger)
        ));

        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["revisions"][0]["parent_sha256"] = serde_json::json!("a".repeat(64));
        assert!(matches!(
            ScoreLedger::from_json(&serde_json::to_vec(&value).unwrap()),
            Err(ScoreError::InvalidLedger)
        ));
        assert!(matches!(
            ScoreLedger::from_json(&vec![b' '; MAX_SCORE_LEDGER_BYTES + 1]),
            Err(ScoreError::LedgerTooLarge)
        ));
    }

    #[test]
    fn pre_verifier_terminal_causes_remain_distinct_and_unscorable() {
        for (index, outcome) in [
            VerificationOutcome::TaskFailed,
            VerificationOutcome::PatchFailed,
            VerificationOutcome::TimedOut,
            VerificationOutcome::EnvironmentFailed,
        ]
        .into_iter()
        .enumerate()
        {
            let root = Root::new("terminal");
            let store = AtomicStore::open(root.0.join("store"), StoreLimits::default()).unwrap();
            let run_id = format!("run-{index}");
            store
                .create_run(&RunManifest {
                    schema_version: MANIFEST_SCHEMA_VERSION,
                    run_id: Id(run_id.clone()),
                    attempt_id: Id("attempt".into()),
                    definition: serde_json::json!({}),
                })
                .unwrap();
            let submission = if matches!(
                outcome,
                VerificationOutcome::PatchFailed | VerificationOutcome::TimedOut
            ) {
                Some(
                    store
                        .put_artifact(&run_id, "submission.patch", Cursor::new(b"patch"))
                        .unwrap(),
                )
            } else {
                None
            };
            let environment = store
                .put_artifact(
                    &run_id,
                    "environment.json",
                    Cursor::new(br#"{"runtime":"fixture"}"#),
                )
                .unwrap();
            let observation = VerificationObservation::new(
                Id("attempt".into()),
                Id("original.bug-fix".into()),
                "a".repeat(64),
                environment,
                submission,
                outcome,
            )
            .unwrap();
            store
                .commit_verification_observation(&run_id, &observation)
                .unwrap();
            let (loaded, _) = store.load_verification_observation(&run_id).unwrap();
            assert_eq!(loaded.outcome(), outcome);
            assert!(matches!(
                ScoreLedger::open(&store, &run_id),
                Err(ScoreError::ObservationNotScorable)
            ));
        }
    }
}
