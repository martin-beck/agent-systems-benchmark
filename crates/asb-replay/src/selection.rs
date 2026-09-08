// SPDX-License-Identifier: MIT
//! Explicit, privacy-safe choice between compatible replay and live execution.

use crate::{Cassette, CassetteLimits, decode_cassette};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Maximum indexed recordings in one bounded selection context.
pub const MAX_INDEXED_RECORDINGS: usize = 1_024;
/// Maximum public agent identity length.
pub const MAX_AGENT_ID_BYTES: usize = 128;

/// Public compatibility metadata bound to one authenticated cassette.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingDescriptor {
    /// Credential-free provider profile digest required by this recording.
    pub provider_profile_sha256: String,
    /// Stable selected-agent identity.
    pub agent_id: String,
    /// Authenticated cassette root digest.
    pub cassette_sha256: String,
}

/// One compatible prior recording offered without filesystem or content details.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingOffer {
    /// Stable cassette identity.
    pub cassette_id: String,
    /// Authenticated cassette root digest.
    pub cassette_sha256: String,
}

/// Required explicit source choice. There is no default variant.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceChoice {
    /// Use the live provider connection already proven by provider preflight.
    Live,
    /// Use exactly one authenticated compatible cassette.
    Replay {
        /// Exact cassette root digest selected from the offered recordings.
        cassette_sha256: String,
    },
}

/// Constructor-controlled execution source selected before launch.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionSource {
    /// Live provider execution.
    Live,
    /// Strict replay of one authenticated cassette.
    Replay {
        /// Stable cassette identity.
        cassette_id: String,
        /// Authenticated root digest.
        cassette_sha256: String,
    },
}

/// Bounded authenticated recording index.
#[derive(Clone, Debug, Default)]
pub struct RecordingIndex {
    entries: BTreeMap<(String, String), Vec<RecordingOffer>>,
    roots: BTreeSet<String>,
}

impl RecordingIndex {
    /// Construct an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Authenticate and add one recording with explicit compatibility metadata.
    pub fn insert(
        &mut self,
        descriptor: RecordingDescriptor,
        cassette: &Cassette,
    ) -> Result<(), SourceSelectionError> {
        if self.roots.len() >= MAX_INDEXED_RECORDINGS {
            return Err(SourceSelectionError::IndexFull);
        }
        validate_digest(&descriptor.provider_profile_sha256)?;
        validate_digest(&descriptor.cassette_sha256)?;
        validate_agent(&descriptor.agent_id)?;

        let encoded =
            serde_json::to_vec(cassette).map_err(|_| SourceSelectionError::InvalidCassette)?;
        let authenticated = decode_cassette(&encoded, CassetteLimits::default())
            .map_err(|_| SourceSelectionError::InvalidCassette)?;
        if authenticated.integrity.digest != descriptor.cassette_sha256 {
            return Err(SourceSelectionError::CassetteIdentityMismatch);
        }
        if !self.roots.insert(descriptor.cassette_sha256.clone()) {
            return Err(SourceSelectionError::DuplicateRecording);
        }

        let key = (descriptor.provider_profile_sha256, descriptor.agent_id);
        let offers = self.entries.entry(key).or_default();
        offers.push(RecordingOffer {
            cassette_id: authenticated.contents.cassette_id,
            cassette_sha256: descriptor.cassette_sha256,
        });
        offers.sort();
        Ok(())
    }

    /// Return deterministic compatible replay choices and explicit live availability.
    pub fn offers(
        &self,
        provider_profile_sha256: &str,
        agent_id: &str,
        live_available: bool,
    ) -> Result<(bool, &[RecordingOffer]), SourceSelectionError> {
        validate_digest(provider_profile_sha256)?;
        validate_agent(agent_id)?;
        let recordings = self
            .entries
            .get(&(provider_profile_sha256.to_owned(), agent_id.to_owned()))
            .map_or(&[][..], Vec::as_slice);
        Ok((live_available, recordings))
    }

    /// Resolve an explicit choice without silently falling back between sources.
    pub fn select(
        &self,
        provider_profile_sha256: &str,
        agent_id: &str,
        live_available: bool,
        choice: Option<&SourceChoice>,
    ) -> Result<ExecutionSource, SourceSelectionError> {
        let (live, recordings) = self.offers(provider_profile_sha256, agent_id, live_available)?;
        match choice.ok_or(SourceSelectionError::ChoiceRequired)? {
            SourceChoice::Live if live => Ok(ExecutionSource::Live),
            SourceChoice::Live => Err(SourceSelectionError::LiveUnavailable),
            SourceChoice::Replay { cassette_sha256 } => {
                validate_digest(cassette_sha256)?;
                let offer = recordings
                    .iter()
                    .find(|offer| offer.cassette_sha256 == *cassette_sha256)
                    .ok_or(SourceSelectionError::RecordingUnavailable)?;
                Ok(ExecutionSource::Replay {
                    cassette_id: offer.cassette_id.clone(),
                    cassette_sha256: offer.cassette_sha256.clone(),
                })
            }
        }
    }
}

/// Recording index or explicit source-selection failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SourceSelectionError {
    /// The recording index reached its public bound.
    #[error("recording index is full")]
    IndexFull,
    /// A provider profile or cassette digest was malformed.
    #[error("source identity digest is invalid")]
    InvalidDigest,
    /// An agent identity was malformed or unbounded.
    #[error("agent identity is invalid")]
    InvalidAgent,
    /// Cassette authentication or schema validation failed.
    #[error("recording cassette is invalid")]
    InvalidCassette,
    /// Descriptor and authenticated cassette roots differ.
    #[error("recording cassette identity does not match descriptor")]
    CassetteIdentityMismatch,
    /// The same cassette root was indexed more than once.
    #[error("recording cassette is duplicated")]
    DuplicateRecording,
    /// The caller did not explicitly choose replay or live execution.
    #[error("replay or live source choice is required")]
    ChoiceRequired,
    /// Live execution was explicitly selected but not preflighted as available.
    #[error("live provider source is unavailable")]
    LiveUnavailable,
    /// The requested cassette is not an exact compatibility match.
    #[error("compatible recording is unavailable")]
    RecordingUnavailable,
}

fn validate_digest(value: &str) -> Result<(), SourceSelectionError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(SourceSelectionError::InvalidDigest)
    }
}

fn validate_agent(value: &str) -> Result<(), SourceSelectionError> {
    if !value.is_empty()
        && value.len() <= MAX_AGENT_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        Ok(())
    } else {
        Err(SourceSelectionError::InvalidAgent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical_contents_bytes;
    use sha2::{Digest, Sha256};

    fn cassette(id: &str) -> Cassette {
        let mut cassette = decode_cassette(
            include_bytes!("../fixtures/v1/buffered.json"),
            CassetteLimits::default(),
        )
        .unwrap();
        cassette.contents.cassette_id = id.into();
        let bytes = canonical_contents_bytes(&cassette.contents).unwrap();
        cassette.integrity.digest = format!("{:x}", Sha256::digest(bytes));
        cassette
    }

    fn insert(index: &mut RecordingIndex, profile: &str, agent: &str, cassette: &Cassette) {
        index
            .insert(
                RecordingDescriptor {
                    provider_profile_sha256: profile.into(),
                    agent_id: agent.into(),
                    cassette_sha256: cassette.integrity.digest.clone(),
                },
                cassette,
            )
            .unwrap();
    }

    #[test]
    fn exact_matches_are_offered_canonically_and_require_a_choice() {
        let profile = "a".repeat(64);
        let mut index = RecordingIndex::new();
        let second = cassette("recording-b");
        let first = cassette("recording-a");
        insert(&mut index, &profile, "codex", &second);
        insert(&mut index, &profile, "codex", &first);
        let (live, offers) = index.offers(&profile, "codex", true).unwrap();
        assert!(live);
        assert_eq!(
            offers
                .iter()
                .map(|item| item.cassette_id.as_str())
                .collect::<Vec<_>>(),
            vec!["recording-a", "recording-b"]
        );
        assert_eq!(
            index.select(&profile, "codex", true, None),
            Err(SourceSelectionError::ChoiceRequired)
        );
        assert_eq!(
            index.select(&profile, "codex", true, Some(&SourceChoice::Live)),
            Ok(ExecutionSource::Live)
        );
        assert_eq!(
            index.select(
                &profile,
                "codex",
                true,
                Some(&SourceChoice::Replay {
                    cassette_sha256: first.integrity.digest.clone()
                })
            ),
            Ok(ExecutionSource::Replay {
                cassette_id: "recording-a".into(),
                cassette_sha256: first.integrity.digest
            })
        );
    }

    #[test]
    fn profile_agent_and_source_mismatches_never_fall_back() {
        let profile = "a".repeat(64);
        let recording = cassette("recording");
        let mut index = RecordingIndex::new();
        insert(&mut index, &profile, "codex", &recording);
        let replay = SourceChoice::Replay {
            cassette_sha256: recording.integrity.digest.clone(),
        };
        assert_eq!(
            index.select(&"b".repeat(64), "codex", true, Some(&replay)),
            Err(SourceSelectionError::RecordingUnavailable)
        );
        assert_eq!(
            index.select(&profile, "aider", true, Some(&replay)),
            Err(SourceSelectionError::RecordingUnavailable)
        );
        assert_eq!(
            index.select(&profile, "codex", false, Some(&SourceChoice::Live)),
            Err(SourceSelectionError::LiveUnavailable)
        );
        assert_eq!(
            index.select(
                &profile,
                "codex",
                true,
                Some(&SourceChoice::Replay {
                    cassette_sha256: "c".repeat(64)
                })
            ),
            Err(SourceSelectionError::RecordingUnavailable)
        );
    }

    #[test]
    fn malformed_forged_and_duplicate_entries_fail_closed() {
        let profile = "a".repeat(64);
        let recording = cassette("recording");
        let mut index = RecordingIndex::new();
        let mut forged = recording.clone();
        forged.integrity.digest = "b".repeat(64);
        assert_eq!(
            index.insert(
                RecordingDescriptor {
                    provider_profile_sha256: profile.clone(),
                    agent_id: "codex".into(),
                    cassette_sha256: forged.integrity.digest.clone()
                },
                &forged
            ),
            Err(SourceSelectionError::InvalidCassette)
        );
        insert(&mut index, &profile, "codex", &recording);
        assert_eq!(
            index.insert(
                RecordingDescriptor {
                    provider_profile_sha256: profile,
                    agent_id: "codex".into(),
                    cassette_sha256: recording.integrity.digest.clone()
                },
                &recording
            ),
            Err(SourceSelectionError::DuplicateRecording)
        );
    }

    #[test]
    fn public_inputs_are_bounded_and_outputs_disclose_no_contents() {
        let index = RecordingIndex::new();
        assert_eq!(
            index.offers("bad", "codex", true),
            Err(SourceSelectionError::InvalidDigest)
        );
        assert_eq!(
            index.offers(&"a".repeat(64), "../codex", true),
            Err(SourceSelectionError::InvalidAgent)
        );
        assert_eq!(
            index.offers(&"a".repeat(64), &"x".repeat(MAX_AGENT_ID_BYTES + 1), true),
            Err(SourceSelectionError::InvalidAgent)
        );
        assert!(
            !serde_json::to_string(&SourceChoice::Live)
                .unwrap()
                .contains("cassette")
        );
    }
}
