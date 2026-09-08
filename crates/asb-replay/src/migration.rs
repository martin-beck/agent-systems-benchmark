// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Fail-closed invariants for future cassette schema migrations.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::{CassetteContents, ResponseBody};

/// Multiset of stable identities and directed causal references.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferenceGraph {
    identities: BTreeMap<String, u32>,
    references: BTreeMap<(String, String), u32>,
}

impl ReferenceGraph {
    /// Capture the complete identity/reference graph without payload content.
    pub fn capture(contents: &CassetteContents) -> Result<Self, MigrationError> {
        let mut graph = Self {
            identities: BTreeMap::new(),
            references: BTreeMap::new(),
        };
        for interaction in &contents.interactions {
            let scope = format!("{}/{}", interaction.session_id, interaction.attempt_id);
            graph.identity(format!("session:{}", interaction.session_id))?;
            graph.identity(format!("{scope}/attempt"))?;
            let source = format!("interaction:{}", interaction.interaction_id);
            graph.identity(source.clone())?;
            if let Some(target) = &interaction.request.previous_response_id {
                graph.reference(source.clone(), format!("{scope}/response:{target}"))?;
            }
            match &interaction.response.body {
                ResponseBody::Buffered { response_id, .. } => {
                    if let Some(response_id) = response_id {
                        graph.identity(format!("{scope}/response:{response_id}"))?;
                    }
                }
                ResponseBody::Events { events, .. } => {
                    for event in events {
                        let event_source = format!("{source}/event: {}", event.sequence);
                        graph.identity(event_source.clone())?;
                        if let Some(response_id) = &event.response_id {
                            graph.identity(format!("{scope}/response:{response_id}"))?;
                        }
                        if let Some(target) = &event.previous_response_id {
                            graph.reference(
                                event_source.clone(),
                                format!("{scope}/response:{target}"),
                            )?;
                        }
                        if let Some(tool_call_id) = &event.tool_call_id {
                            graph.identity(format!("{scope}/tool:{tool_call_id}"))?;
                            graph
                                .reference(event_source, format!("{scope}/tool:{tool_call_id}"))?;
                        }
                    }
                }
            }
        }
        Ok(graph)
    }

    fn identity(&mut self, identity: String) -> Result<(), MigrationError> {
        increment(&mut self.identities, identity)
    }

    fn reference(&mut self, source: String, target: String) -> Result<(), MigrationError> {
        increment(&mut self.references, (source, target))
    }
}

/// Require a migration to preserve references and all recorded trajectories.
pub fn verify_migration_references(
    before: &CassetteContents,
    after: &CassetteContents,
) -> Result<(), MigrationError> {
    if ReferenceGraph::capture(before)? != ReferenceGraph::capture(after)?
        || before.cassette_id != after.cassette_id
        || before.normalization != after.normalization
        || before.redaction != after.redaction
        || before.interactions != after.interactions
    {
        return Err(MigrationError::InvariantViolation);
    }
    Ok(())
}

/// Migration validation failed without exposing cassette values.
#[derive(Debug, Error)]
pub enum MigrationError {
    /// Counting an identity or edge overflowed.
    #[error("migration reference count overflowed")]
    CountOverflow,
    /// A migration changed identity, causality, policy, or trajectory content.
    #[error("migration did not preserve cassette references and trajectories")]
    InvariantViolation,
}

fn increment<K: Ord>(counts: &mut BTreeMap<K, u32>, key: K) -> Result<(), MigrationError> {
    let value = counts.entry(key).or_default();
    *value = value.checked_add(1).ok_or(MigrationError::CountOverflow)?;
    Ok(())
}
