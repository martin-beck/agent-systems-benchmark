// SPDX-License-Identifier: MIT
//! Bounded nonblocking projection of stable ASB spans to OTLP-shaped JSON.

use std::collections::VecDeque;
use std::sync::Mutex;

use asb_protocol::{TraceError, TraceSpan};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Exact upstream GenAI semantic-convention source revision reviewed by ASB.
pub const OTEL_GENAI_SEMCONV_REVISION: &str = "b5d8440f6f126738fd50f927752cd669772c517b";
/// Schema URL declared by that source revision.
pub const OTEL_GENAI_SCHEMA_URL: &str = "https://opentelemetry.io/schemas/gen-ai-dev/1.42.0-dev";
/// Hard ceiling for queued projections.
pub const MAX_TRACE_QUEUE_CAPACITY: usize = 4096;

/// Minimal OTLP JSON span projection. It is transport-neutral and performs no I/O.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OtlpJsonSpan {
    /// Deterministic 16-byte trace ID as lowercase hexadecimal.
    pub trace_id: String,
    /// Deterministic 8-byte span ID as lowercase hexadecimal.
    pub span_id: String,
    /// Parent span ID, absent only for a run root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    /// Low-cardinality operation name.
    pub name: String,
    /// OTLP JSON decimal start timestamp.
    pub start_time_unix_nano: String,
    /// OTLP JSON decimal end timestamp.
    pub end_time_unix_nano: String,
    /// Deterministically ordered OTLP key/value attributes.
    pub attributes: Vec<OtlpAttribute>,
}

/// OTLP JSON key/value attribute.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OtlpAttribute {
    /// Semantic-convention or ASB extension key.
    pub key: String,
    /// Typed OTLP JSON value.
    pub value: OtlpAnyValue,
}

/// Bounded value types emitted by this projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum OtlpAnyValue {
    /// OTLP string value.
    String(OtlpStringValue),
    /// OTLP signed-integer value encoded as a decimal JSON string.
    Integer(OtlpIntegerValue),
}

/// Closed OTLP string-value object.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OtlpStringValue {
    /// UTF-8 value.
    #[serde(rename = "stringValue")]
    pub string_value: String,
}

/// Closed OTLP integer-value object.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OtlpIntegerValue {
    /// Decimal integer.
    #[serde(rename = "intValue")]
    pub int_value: String,
}

impl OtlpJsonSpan {
    /// Project a validated span. Content metadata is omitted unless explicitly enabled.
    pub fn from_asb(span: &TraceSpan, include_content_digest: bool) -> Result<Self, ExportError> {
        span.validate()?;
        let mut attributes = vec![
            string_attribute(
                "asb.run.id",
                &derived_hex(&format!("asb.run:{}", span.run_id.0), 16),
            ),
            string_attribute(
                "asb.trial.id",
                &derived_hex(&format!("asb.trial:{}", span.trial_id.0), 16),
            ),
            string_attribute(
                "asb.agent.id",
                &derived_hex(&format!("asb.agent:{}", span.agent_id.0), 16),
            ),
            string_attribute("gen_ai.operation.name", &redacted_label(&span.operation)),
            string_attribute(
                "asb.trace.status",
                &format!("{:?}", span.status).to_ascii_lowercase(),
            ),
            string_attribute("asb.semconv.revision", OTEL_GENAI_SEMCONV_REVISION),
        ];
        if let Some(provider) = &span.provider {
            attributes.push(string_attribute(
                "gen_ai.provider.name",
                &redacted_label(provider),
            ));
        }
        if let Some(model) = &span.model {
            attributes.push(string_attribute(
                "gen_ai.request.model",
                &redacted_label(model),
            ));
        }
        if let Some(call) = &span.tool_call_id {
            attributes.push(string_attribute(
                "gen_ai.tool.call.id",
                &derived_hex(&format!("asb.tool:{}", call.0), 16),
            ));
        }
        if let Some(name) = &span.tool_name {
            attributes.push(string_attribute("gen_ai.tool.name", &redacted_label(name)));
        }
        if include_content_digest && let Some(content) = &span.content {
            attributes.push(string_attribute("asb.content.sha256", &content.sha256));
            attributes.push(OtlpAttribute {
                key: "asb.content.byte_count".into(),
                value: OtlpAnyValue::Integer(OtlpIntegerValue {
                    int_value: content.byte_len.to_string(),
                }),
            });
        }
        attributes.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(Self {
            trace_id: derived_hex(&span.run_id.0, 16),
            span_id: derived_hex(&format!("{}:{}", span.run_id.0, span.span_id.0), 8),
            parent_span_id: span
                .parent_span_id
                .as_ref()
                .map(|parent| derived_hex(&format!("{}:{}", span.run_id.0, parent.0), 8)),
            name: redacted_label(&span.operation),
            start_time_unix_nano: span.start_unix_ns.to_string(),
            end_time_unix_nano: span.end_unix_ns.to_string(),
            attributes,
        })
    }
}

fn string_attribute(key: &str, value: &str) -> OtlpAttribute {
    OtlpAttribute {
        key: key.into(),
        value: OtlpAnyValue::String(OtlpStringValue {
            string_value: value.into(),
        }),
    }
}

fn redacted_label(value: &str) -> String {
    let lowercase = value.to_ascii_lowercase();
    if [
        "authorization",
        "bearer ",
        "api_key",
        "api-key",
        "token=",
        "secret=",
        "sk-",
    ]
    .iter()
    .any(|marker| lowercase.contains(marker))
    {
        "[REDACTED]".into()
    } else {
        value.into()
    }
}

/// Result of a nonblocking enqueue attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportResult {
    /// Projection was accepted.
    Enqueued,
    /// Queue was full or busy; run execution was not delayed.
    DroppedBackpressure,
    /// Span was malformed and was not queued.
    RejectedMalformed,
}

/// In-process bounded projection queue. Network transport is deliberately external.
#[derive(Debug)]
pub struct TraceExporter {
    queue: Mutex<VecDeque<OtlpJsonSpan>>,
    capacity: usize,
    include_content_digest: bool,
}

impl TraceExporter {
    /// Construct an exporter with an explicit hard-bounded capacity.
    pub fn new(capacity: usize, include_content_digest: bool) -> Result<Self, ExportError> {
        if capacity == 0 || capacity > MAX_TRACE_QUEUE_CAPACITY {
            return Err(ExportError::Capacity);
        }
        Ok(Self {
            queue: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            include_content_digest,
        })
    }

    /// Attempt projection and enqueue without waiting for another exporter user.
    pub fn try_export(&self, span: &TraceSpan) -> ExportResult {
        let projected = match OtlpJsonSpan::from_asb(span, self.include_content_digest) {
            Ok(value) => value,
            Err(_) => return ExportResult::RejectedMalformed,
        };
        let Ok(mut queue) = self.queue.try_lock() else {
            return ExportResult::DroppedBackpressure;
        };
        if queue.len() == self.capacity {
            return ExportResult::DroppedBackpressure;
        }
        queue.push_back(projected);
        ExportResult::Enqueued
    }

    /// Drain at most maximum projections for a transport worker without waiting.
    pub fn drain(&self, maximum: usize) -> Vec<OtlpJsonSpan> {
        let Ok(mut queue) = self.queue.try_lock() else {
            return Vec::new();
        };
        let count = maximum.min(queue.len());
        queue.drain(..count).collect()
    }
}

fn derived_hex(value: &str, bytes: usize) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..bytes]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Safe projection errors that never include span content.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ExportError {
    /// Capacity is zero or exceeds the implementation ceiling.
    #[error("invalid trace exporter capacity")]
    Capacity,
    /// Stable span validation failed.
    #[error("invalid trace span")]
    Span(#[from] TraceError),
}

#[cfg(test)]
mod tests {
    use asb_protocol::{
        Id, TRACE_SCHEMA_VERSION, TraceContentDigest, TraceKind, TraceSpan, TraceStatus,
    };

    use super::*;

    fn span() -> TraceSpan {
        TraceSpan {
            schema_version: TRACE_SCHEMA_VERSION,
            run_id: Id("run".into()),
            trial_id: Id("trial".into()),
            agent_id: Id("agent".into()),
            span_id: Id("model".into()),
            parent_span_id: Some(Id("agent-span".into())),
            kind: TraceKind::Model,
            operation: "chat".into(),
            provider: Some("openai".into()),
            model: Some("model-v1".into()),
            tool_call_id: None,
            tool_name: None,
            start_unix_ns: 1,
            end_unix_ns: 2,
            status: TraceStatus::Ok,
            content: Some(TraceContentDigest {
                sha256: "a".repeat(64),
                byte_len: 21,
            }),
        }
    }

    #[test]
    fn projection_round_trips_causal_ids_without_secrets() {
        let projected = OtlpJsonSpan::from_asb(&span(), false).unwrap();
        let bytes = serde_json::to_vec(&projected).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("content"));
        assert!(!text.contains(&"a".repeat(64)));
        assert_eq!(
            serde_json::from_slice::<OtlpJsonSpan>(&bytes).unwrap(),
            projected
        );
        assert!(projected.attributes.iter().any(|attribute| {
            attribute.key == "asb.run.id"
                && attribute.value
                    == OtlpAnyValue::String(OtlpStringValue {
                        string_value: derived_hex("asb.run:run", 16),
                    })
        }));
        assert!(!text.contains("\"run\""));
        assert!(!text.contains("\"trial\""));
        assert!(!text.contains("\"agent\""));
    }

    #[test]
    fn content_digest_requires_explicit_opt_in() {
        let hidden = OtlpJsonSpan::from_asb(&span(), false).unwrap();
        assert!(
            !hidden
                .attributes
                .iter()
                .any(|item| item.key == "asb.content.sha256")
        );
        let visible = OtlpJsonSpan::from_asb(&span(), true).unwrap();
        assert!(
            visible
                .attributes
                .iter()
                .any(|item| item.key == "asb.content.byte_count")
        );
    }

    #[test]
    fn malformed_and_backpressured_exports_do_not_corrupt_queue() {
        let exporter = TraceExporter::new(1, false).unwrap();
        assert_eq!(exporter.try_export(&span()), ExportResult::Enqueued);
        assert_eq!(
            exporter.try_export(&span()),
            ExportResult::DroppedBackpressure
        );
        let mut malformed = span();
        malformed.end_unix_ns = 0;
        assert_eq!(
            exporter.try_export(&malformed),
            ExportResult::RejectedMalformed
        );
        let drained = exporter.drain(2);
        assert_eq!(drained.len(), 1);
        assert!(
            drained[0]
                .attributes
                .iter()
                .any(|item| item.key == "asb.run.id")
        );
    }

    #[test]
    fn lock_contention_drops_immediately_and_preserves_queue() {
        let exporter = TraceExporter::new(1, false).unwrap();
        let guard = exporter.queue.lock().unwrap();
        assert_eq!(
            exporter.try_export(&span()),
            ExportResult::DroppedBackpressure
        );
        drop(guard);
        assert!(exporter.drain(1).is_empty());
        assert_eq!(exporter.try_export(&span()), ExportResult::Enqueued);
    }

    #[test]
    fn malformed_otlp_attribute_values_fail_closed() {
        let projected = OtlpJsonSpan::from_asb(&span(), false).unwrap();
        let mut value = serde_json::to_value(projected).unwrap();
        value["attributes"][0]["value"] = serde_json::json!({
            "stringValue": "one",
            "intValue": "2"
        });
        assert!(serde_json::from_value::<OtlpJsonSpan>(value).is_err());
    }

    #[test]
    fn secret_like_labels_are_redacted_without_changing_causality() {
        let mut sensitive = span();
        sensitive.operation = "authorization=Bearer sk-private".into();
        sensitive.model = Some("token=private".into());
        let projected = OtlpJsonSpan::from_asb(&sensitive, false).unwrap();
        let text = serde_json::to_string(&projected).unwrap();
        assert!(!text.contains("private"));
        assert!(text.contains("[REDACTED]"));
        assert_eq!(projected.trace_id, derived_hex("run", 16));
    }

    #[test]
    fn semconv_source_is_exactly_pinned() {
        assert_eq!(OTEL_GENAI_SEMCONV_REVISION.len(), 40);
        assert_eq!(
            OTEL_GENAI_SCHEMA_URL,
            "https://opentelemetry.io/schemas/gen-ai-dev/1.42.0-dev"
        );
    }
}
