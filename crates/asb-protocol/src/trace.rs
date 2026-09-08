// SPDX-License-Identifier: MIT
//! Stable privacy-safe causal trace contracts.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Id;

/// Current ASB trace schema generation.
pub const TRACE_SCHEMA_VERSION: u16 = 1;
/// Maximum accepted identity or label length.
pub const MAX_TRACE_LABEL_BYTES: usize = 256;

/// The causal operation represented by a span.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceKind {
    /// Complete benchmark run.
    Run,
    /// One trial within a run.
    Trial,
    /// Agent execution within a trial.
    Agent,
    /// Model request made by an agent.
    Model,
    /// Tool invocation made by an agent.
    Tool,
}

/// Privacy-safe terminal outcome.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceStatus {
    /// Operation completed successfully.
    Ok,
    /// Operation failed; details remain in the bounded run journal.
    Error,
    /// Operation was cancelled.
    Cancelled,
}

/// Opt-in content evidence containing only length and digest, never raw content.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TraceContentDigest {
    /// SHA-256 of the content, encoded as lowercase hexadecimal.
    pub sha256: String,
    /// Original byte length.
    pub byte_len: u64,
}

/// Stable ASB span independent of telemetry vendor or evolving convention.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TraceSpan {
    /// Trace schema generation.
    pub schema_version: u16,
    /// Run identity shared by the causal tree.
    pub run_id: Id,
    /// Trial identity shared below the run.
    pub trial_id: Id,
    /// Agent identity shared by agent, model, and tool spans.
    pub agent_id: Id,
    /// Stable identity of this span.
    pub span_id: Id,
    /// Stable parent identity; only a run root may omit it.
    pub parent_span_id: Option<Id>,
    /// Operation category.
    pub kind: TraceKind,
    /// Low-cardinality operation name.
    pub operation: String,
    /// Provider identifier for model spans.
    pub provider: Option<String>,
    /// Requested model identifier for model spans.
    pub model: Option<String>,
    /// Tool call identity for tool spans.
    pub tool_call_id: Option<Id>,
    /// Tool name for tool spans.
    pub tool_name: Option<String>,
    /// Wall-clock start in Unix nanoseconds.
    pub start_unix_ns: u64,
    /// Wall-clock end in Unix nanoseconds.
    pub end_unix_ns: u64,
    /// Terminal outcome.
    pub status: TraceStatus,
    /// Optional privacy-safe content evidence. Raw text is not representable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<TraceContentDigest>,
}

impl TraceSpan {
    /// Validate identities, causality, timestamps, and kind-specific fields.
    pub fn validate(&self) -> Result<(), TraceError> {
        if self.schema_version != TRACE_SCHEMA_VERSION {
            return Err(TraceError::SchemaVersion);
        }
        for value in [&self.run_id, &self.trial_id, &self.agent_id, &self.span_id] {
            valid_id(value)?;
        }
        if let Some(parent) = &self.parent_span_id {
            valid_id(parent)?;
            if parent == &self.span_id {
                return Err(TraceError::Causality);
            }
        } else if self.kind != TraceKind::Run {
            return Err(TraceError::Causality);
        }
        valid_label(&self.operation)?;
        if self.end_unix_ns < self.start_unix_ns {
            return Err(TraceError::TimestampOrder);
        }
        match self.kind {
            TraceKind::Model => {
                valid_optional_label(&self.provider)?;
                valid_optional_label(&self.model)?;
                if self.provider.is_none()
                    || self.model.is_none()
                    || self.tool_call_id.is_some()
                    || self.tool_name.is_some()
                {
                    return Err(TraceError::KindFields);
                }
            }
            TraceKind::Tool => {
                valid_id(self.tool_call_id.as_ref().ok_or(TraceError::KindFields)?)?;
                valid_optional_label(&self.tool_name)?;
                if self.tool_name.is_none() || self.provider.is_some() || self.model.is_some() {
                    return Err(TraceError::KindFields);
                }
            }
            _ if self.provider.is_some()
                || self.model.is_some()
                || self.tool_call_id.is_some()
                || self.tool_name.is_some() =>
            {
                return Err(TraceError::KindFields);
            }
            _ => {}
        }
        if let Some(content) = &self.content
            && (content.sha256.len() != 64
                || !content
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()))
        {
            return Err(TraceError::ContentDigest);
        }
        Ok(())
    }
}

fn valid_id(id: &Id) -> Result<(), TraceError> {
    if id.0.is_empty() || id.0.len() > MAX_TRACE_LABEL_BYTES || id.0.chars().any(char::is_control) {
        return Err(TraceError::Identity);
    }
    Ok(())
}

fn valid_label(label: &str) -> Result<(), TraceError> {
    if label.is_empty()
        || label.len() > MAX_TRACE_LABEL_BYTES
        || label.chars().any(char::is_control)
    {
        return Err(TraceError::Label);
    }
    Ok(())
}

fn valid_optional_label(label: &Option<String>) -> Result<(), TraceError> {
    label.as_deref().map_or(Ok(()), valid_label)
}

/// Fail-closed trace validation errors with no user content.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum TraceError {
    /// Unknown schema generation.
    #[error("unsupported trace schema")]
    SchemaVersion,
    /// Missing, oversized, or control-bearing identity.
    #[error("invalid trace identity")]
    Identity,
    /// Missing, oversized, or control-bearing label.
    #[error("invalid trace label")]
    Label,
    /// Parent relationship is invalid.
    #[error("invalid trace causality")]
    Causality,
    /// End precedes start.
    #[error("invalid trace timestamp order")]
    TimestampOrder,
    /// Fields do not match the span kind.
    #[error("invalid trace kind fields")]
    KindFields,
    /// Content digest is not canonical lowercase SHA-256.
    #[error("invalid trace content digest")]
    ContentDigest,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_span() -> TraceSpan {
        TraceSpan {
            schema_version: TRACE_SCHEMA_VERSION,
            run_id: Id("run-1".into()),
            trial_id: Id("trial-1".into()),
            agent_id: Id("agent-1".into()),
            span_id: Id("model-1".into()),
            parent_span_id: Some(Id("agent-span-1".into())),
            kind: TraceKind::Model,
            operation: "chat".into(),
            provider: Some("openai".into()),
            model: Some("model-v1".into()),
            tool_call_id: None,
            tool_name: None,
            start_unix_ns: 10,
            end_unix_ns: 20,
            status: TraceStatus::Ok,
            content: None,
        }
    }

    #[test]
    fn causal_ids_round_trip_without_content() {
        let span = model_span();
        span.validate().unwrap();
        let encoded = serde_json::to_vec(&span).unwrap();
        assert!(!String::from_utf8_lossy(&encoded).contains("content"));
        let decoded: TraceSpan = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, span);
    }

    #[test]
    fn malformed_spans_fail_closed_with_safe_errors() {
        let mut span = model_span();
        span.parent_span_id = Some(span.span_id.clone());
        assert_eq!(span.validate(), Err(TraceError::Causality));
        span.parent_span_id = Some(Id("agent-span-1".into()));
        span.end_unix_ns = 9;
        assert_eq!(span.validate(), Err(TraceError::TimestampOrder));
        span.end_unix_ns = 20;
        span.tool_name = Some("unexpected".into());
        assert_eq!(span.validate(), Err(TraceError::KindFields));
    }
}
