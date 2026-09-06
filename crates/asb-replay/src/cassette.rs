// SPDX-License-Identifier: MIT
//! Versioned cassette values, integrity, bounds, and causal validation.

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use schemars::JsonSchema;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// The only cassette schema accepted by the normal decoder.
pub const CASSETTE_SCHEMA_VERSION: u16 = 1;
/// Absolute ceiling for an encoded cassette.
pub const MAX_CASSETTE_BYTES: u64 = 256 * 1024 * 1024;
/// Absolute ceiling for one normalized request.
pub const MAX_REQUEST_BYTES: u64 = 16 * 1024 * 1024;
/// Absolute ceiling for one buffered response.
pub const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;
/// Absolute ceiling for one semantic response event.
pub const MAX_EVENT_BYTES: u64 = 16 * 1024 * 1024;
/// Absolute number of events in one cassette.
pub const MAX_EVENTS: u32 = 65_536;
/// Absolute number of interactions in one cassette.
pub const MAX_INTERACTIONS: u32 = 16_384;
/// Absolute number of headers in one request or response.
pub const MAX_HEADERS: u16 = 1_024;

/// Caller-selectable limits that may only tighten implementation ceilings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CassetteLimits {
    /// Largest encoded cassette accepted or produced.
    pub max_cassette_bytes: u64,
    /// Largest normalized request.
    pub max_request_bytes: u64,
    /// Largest buffered response.
    pub max_response_bytes: u64,
    /// Largest semantic event.
    pub max_event_bytes: u64,
    /// Most semantic events in a cassette.
    pub max_events: u32,
    /// Most interactions in a cassette.
    pub max_interactions: u32,
    /// Most headers in a request or response.
    pub max_headers: u16,
}

impl Default for CassetteLimits {
    fn default() -> Self {
        Self {
            max_cassette_bytes: MAX_CASSETTE_BYTES,
            max_request_bytes: MAX_REQUEST_BYTES,
            max_response_bytes: MAX_RESPONSE_BYTES,
            max_event_bytes: MAX_EVENT_BYTES,
            max_events: MAX_EVENTS,
            max_interactions: MAX_INTERACTIONS,
            max_headers: MAX_HEADERS,
        }
    }
}

impl CassetteLimits {
    /// Reject zero limits and limits above the implementation ceilings.
    pub fn validate(self) -> Result<Self, CassetteError> {
        validate_limit(
            "cassette bytes",
            self.max_cassette_bytes,
            MAX_CASSETTE_BYTES,
        )?;
        validate_limit("request bytes", self.max_request_bytes, MAX_REQUEST_BYTES)?;
        validate_limit(
            "response bytes",
            self.max_response_bytes,
            MAX_RESPONSE_BYTES,
        )?;
        validate_limit("event bytes", self.max_event_bytes, MAX_EVENT_BYTES)?;
        validate_limit("events", u64::from(self.max_events), u64::from(MAX_EVENTS))?;
        validate_limit(
            "interactions",
            u64::from(self.max_interactions),
            u64::from(MAX_INTERACTIONS),
        )?;
        validate_limit(
            "headers",
            u64::from(self.max_headers),
            u64::from(MAX_HEADERS),
        )?;
        Ok(self)
    }
}

/// Version of a normalization or redaction policy applied before persistence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyVersion {
    /// Policy schema generation.
    pub version: u16,
}

/// Provider syntax named by a capture; this is not a support claim.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderDialect {
    /// OpenAI Chat Completions-shaped capture.
    OpenaiChatCompletions,
    /// OpenAI Responses-shaped capture.
    OpenaiResponses,
    /// Anthropic Messages-shaped capture.
    AnthropicMessages,
    /// Synthetic dialect used by public fixtures.
    Synthetic,
}

/// A normalized HTTP header. Names must be lowercase and entries sorted.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    /// Lowercase header name.
    pub name: String,
    /// Privacy-reviewed value.
    pub value: String,
}

/// Strict normalized request stored for later matching.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedRequest {
    /// Uppercase HTTP method.
    pub method: String,
    /// Origin-form path and normalized query.
    pub path: String,
    /// Sorted, lowercase, privacy-reviewed headers.
    pub headers: Vec<Header>,
    /// Canonical semantic JSON body.
    pub body: Value,
    /// SHA-256 of the canonical privacy-reviewed body.
    pub body_sha256: String,
    /// Model name as sent to the synthetic/provider boundary.
    pub model: String,
    /// Response-affecting provider options.
    pub options: BTreeMap<String, Value>,
    /// Tool definitions in request order.
    pub tools: Vec<Value>,
    /// Causal provider response identity, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
}

/// Required completion classification for a captured response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalEvent {
    /// Provider declared successful completion.
    Completed,
    /// Provider declared a failure response.
    Failed,
    /// Capture ended after an explicit cancellation.
    Cancelled,
}

/// One ordered provider event independent of transport read boundaries.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CassetteEvent {
    /// Zero-based contiguous semantic-event sequence.
    pub sequence: u32,
    /// Monotonic offset from the start of this response.
    pub monotonic_offset_ns: u64,
    /// Provider event type or synthetic fixture name.
    pub event_type: String,
    /// Privacy-reviewed provider payload.
    pub payload: Value,
    /// SHA-256 of the canonical privacy-reviewed payload.
    pub payload_sha256: String,
    /// Provider response identity established by this event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    /// Prior response identity referenced by this event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    /// Tool-call identity established or continued by this event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Present only on the final event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalEvent>,
}

/// Captured response representation.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseBody {
    /// Complete provider JSON response.
    Buffered {
        /// Privacy-reviewed semantic payload.
        payload: Value,
        /// SHA-256 of the canonical privacy-reviewed payload.
        payload_sha256: String,
        /// Provider response identity established by the payload.
        #[serde(skip_serializing_if = "Option::is_none")]
        response_id: Option<String>,
        /// Required terminal classification.
        terminal: TerminalEvent,
    },
    /// Ordered SSE/provider semantic events.
    Events {
        /// Events after transport chunks have been reassembled.
        events: Vec<CassetteEvent>,
        /// Sizes of observed transport chunks, retained separately from events.
        transport_chunk_bytes: Vec<u32>,
    },
}

/// Recorded HTTP response metadata and content.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedResponse {
    /// HTTP response status.
    pub status: u16,
    /// Sorted, lowercase, privacy-reviewed headers.
    pub headers: Vec<Header>,
    /// Buffered or event-stream body.
    pub body: ResponseBody,
}

/// One immutable request/response trajectory.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Interaction {
    /// Session identity used only for isolation and references.
    pub session_id: String,
    /// Attempt identity used only for isolation and references.
    pub attempt_id: String,
    /// Stable interaction identity.
    pub interaction_id: String,
    /// Zero-based sequence within this session and attempt.
    pub ordinal: u32,
    /// Captured provider syntax.
    pub dialect: ProviderDialect,
    /// Strict normalized request.
    pub request: RecordedRequest,
    /// Captured response.
    pub response: RecordedResponse,
}

/// Integrity-protected cassette content.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CassetteContents {
    /// Must equal [`CASSETTE_SCHEMA_VERSION`].
    pub schema_version: u16,
    /// Stable content-set identity.
    pub cassette_id: String,
    /// Exact normalization policy used before hashing.
    pub normalization: PolicyVersion,
    /// Exact redaction policy used before hashing.
    pub redaction: PolicyVersion,
    /// Ordered immutable interactions.
    pub interactions: Vec<Interaction>,
}

/// Cassette contents that passed the crate's complete pre-persistence redaction.
///
/// The inner value is deliberately private. Only the redactor can create this
/// wrapper, so callers cannot seal arbitrary unredacted content.
#[derive(Clone, Debug, PartialEq)]
pub struct RedactedCassetteContents(pub(crate) CassetteContents);

impl RedactedCassetteContents {
    /// Inspect the redacted value before sealing.
    pub fn as_contents(&self) -> &CassetteContents {
        &self.0
    }

    /// Consume the wrapper without weakening the sealing boundary.
    pub fn into_contents(self) -> CassetteContents {
        self.0
    }
}

/// Digest of canonical cassette contents.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CassetteIntegrity {
    /// Digest algorithm, fixed to `sha256` in v1.
    pub algorithm: String,
    /// Lowercase hexadecimal digest of canonical content JSON.
    pub digest: String,
}

/// Complete immutable cassette envelope.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cassette {
    /// Bounded content protected by [`Self::integrity`].
    pub contents: CassetteContents,
    /// Content digest checked before use.
    pub integrity: CassetteIntegrity,
}

/// Fail-closed cassette error without including captured values.
#[derive(Debug, Error)]
pub enum CassetteError {
    /// A configured bound is zero or above its ceiling.
    #[error("invalid {kind} limit {value}; ceiling is {ceiling}")]
    InvalidLimit {
        /// Limit class.
        kind: &'static str,
        /// Supplied value.
        value: u64,
        /// Absolute ceiling.
        ceiling: u64,
    },
    /// Encoded or semantic content crossed a bound.
    #[error("{kind} exceeds its configured bound")]
    TooLarge {
        /// Bounded object class.
        kind: &'static str,
    },
    /// Cassette JSON is malformed, truncated, or has unknown fields.
    #[error("cassette JSON is invalid")]
    InvalidJson(#[source] serde_json::Error),
    /// The schema or policy version is unsupported.
    #[error("unsupported {kind} version {actual}")]
    UnsupportedVersion {
        /// Versioned object class.
        kind: &'static str,
        /// Observed version.
        actual: u16,
    },
    /// A stable identity is empty or unsafe.
    #[error("invalid cassette identity")]
    InvalidIdentity,
    /// Request normalization invariants do not hold.
    #[error("request is not strictly normalized")]
    NotNormalized,
    /// Response metadata or event sequence is invalid.
    #[error("response event stream is invalid")]
    InvalidEventStream,
    /// A causal identifier is duplicated, dangling, or forward-referenced.
    #[error("cassette causal references are invalid")]
    InvalidReference,
    /// Integrity metadata is malformed or does not match content.
    #[error("cassette integrity check failed")]
    Integrity,
    /// Checked arithmetic failed.
    #[error("cassette size arithmetic overflowed")]
    SizeOverflow,
}

/// Validate content, compute integrity, and serialize a cassette.
pub fn seal_cassette(
    contents: RedactedCassetteContents,
    limits: CassetteLimits,
) -> Result<Vec<u8>, CassetteError> {
    let limits = limits.validate()?;
    let contents = contents.into_contents();
    validate_contents(&contents, limits)?;
    let canonical = encode_bounded(&contents, limits.max_cassette_bytes, "cassette")?;
    let cassette = Cassette {
        contents,
        integrity: CassetteIntegrity {
            algorithm: "sha256".to_owned(),
            digest: hex_digest(&Sha256::digest(canonical)),
        },
    };
    encode_bounded(&cassette, limits.max_cassette_bytes, "cassette")
}

/// Decode, validate, and authenticate a complete cassette.
pub fn decode_cassette(bytes: &[u8], limits: CassetteLimits) -> Result<Cassette, CassetteError> {
    let limits = limits.validate()?;
    enforce_size("cassette", bytes.len() as u64, limits.max_cassette_bytes)?;
    let mut duplicate_check = serde_json::Deserializer::from_slice(bytes);
    NoDuplicateJson::deserialize(&mut duplicate_check).map_err(CassetteError::InvalidJson)?;
    duplicate_check.end().map_err(CassetteError::InvalidJson)?;
    let cassette: Cassette = serde_json::from_slice(bytes).map_err(CassetteError::InvalidJson)?;
    validate_contents(&cassette.contents, limits)?;
    if cassette.integrity.algorithm != "sha256" || !valid_digest(&cassette.integrity.digest) {
        return Err(CassetteError::Integrity);
    }
    let canonical = encode_bounded(&cassette.contents, limits.max_cassette_bytes, "cassette")?;
    if cassette.integrity.digest != hex_digest(&Sha256::digest(canonical)) {
        return Err(CassetteError::Integrity);
    }
    Ok(cassette)
}

/// Read arbitrary transport chunks into the same bounded cassette decoder.
pub fn decode_cassette_chunks<I, B>(
    chunks: I,
    limits: CassetteLimits,
) -> Result<Cassette, CassetteError>
where
    I: IntoIterator<Item = B>,
    B: AsRef<[u8]>,
{
    let limits = limits.validate()?;
    let maximum =
        usize::try_from(limits.max_cassette_bytes).map_err(|_| CassetteError::SizeOverflow)?;
    let mut bytes = Vec::with_capacity(maximum.min(4096));
    for chunk in chunks {
        let chunk = chunk.as_ref();
        if chunk.len() > maximum.saturating_sub(bytes.len()) {
            return Err(CassetteError::TooLarge { kind: "cassette" });
        }
        bytes.extend_from_slice(chunk);
    }
    decode_cassette(&bytes, limits)
}

fn validate_contents(
    contents: &CassetteContents,
    limits: CassetteLimits,
) -> Result<(), CassetteError> {
    if contents.schema_version != CASSETTE_SCHEMA_VERSION {
        return Err(CassetteError::UnsupportedVersion {
            kind: "cassette schema",
            actual: contents.schema_version,
        });
    }
    if contents.normalization.version != 1 {
        return Err(CassetteError::UnsupportedVersion {
            kind: "normalization policy",
            actual: contents.normalization.version,
        });
    }
    if contents.redaction.version != 1 {
        return Err(CassetteError::UnsupportedVersion {
            kind: "redaction policy",
            actual: contents.redaction.version,
        });
    }
    validate_id(&contents.cassette_id)?;
    enforce_count(
        "interactions",
        contents.interactions.len(),
        limits.max_interactions as usize,
    )?;
    let mut interaction_ids = BTreeSet::new();
    let mut ordinals = BTreeMap::<(&str, &str), u32>::new();
    let mut response_ids = BTreeSet::new();
    let mut total_events = 0_u32;
    for interaction in &contents.interactions {
        validate_id(&interaction.session_id)?;
        validate_id(&interaction.attempt_id)?;
        validate_id(&interaction.interaction_id)?;
        if !interaction_ids.insert(interaction.interaction_id.as_str()) {
            return Err(CassetteError::InvalidIdentity);
        }
        let expected = ordinals
            .entry((&interaction.session_id, &interaction.attempt_id))
            .or_default();
        if interaction.ordinal != *expected {
            return Err(CassetteError::InvalidReference);
        }
        *expected = expected.checked_add(1).ok_or(CassetteError::SizeOverflow)?;
        validate_request(&interaction.request, limits)?;
        if let Some(reference) = &interaction.request.previous_response_id
            && !response_ids.contains(&(
                interaction.session_id.clone(),
                interaction.attempt_id.clone(),
                reference.clone(),
            ))
        {
            return Err(CassetteError::InvalidReference);
        }
        validate_response(
            &interaction.response,
            &interaction.session_id,
            &interaction.attempt_id,
            limits,
            &mut response_ids,
            &mut total_events,
        )?;
        if total_events > limits.max_events {
            return Err(CassetteError::TooLarge { kind: "events" });
        }
    }
    Ok(())
}

fn validate_request(
    request: &RecordedRequest,
    limits: CassetteLimits,
) -> Result<(), CassetteError> {
    if request.method.is_empty()
        || request
            .method
            .bytes()
            .any(|byte| !byte.is_ascii_uppercase())
        || !request.path.starts_with('/')
        || request.model.is_empty()
    {
        return Err(CassetteError::NotNormalized);
    }
    validate_headers(&request.headers, limits)?;
    if !valid_digest(&request.body_sha256)
        || request.body_sha256 != canonical_value_digest(&request.body)?
    {
        return Err(CassetteError::Integrity);
    }
    encode_bounded(request, limits.max_request_bytes, "request")?;
    Ok(())
}

fn validate_response(
    response: &RecordedResponse,
    session_id: &str,
    attempt_id: &str,
    limits: CassetteLimits,
    response_ids: &mut BTreeSet<(String, String, String)>,
    total_events: &mut u32,
) -> Result<(), CassetteError> {
    if !(100..=599).contains(&response.status) {
        return Err(CassetteError::InvalidEventStream);
    }
    validate_headers(&response.headers, limits)?;
    match &response.body {
        ResponseBody::Buffered {
            payload,
            payload_sha256,
            response_id,
            ..
        } => {
            encode_bounded(response, limits.max_response_bytes, "response")?;
            if !valid_digest(payload_sha256) || payload_sha256 != &canonical_value_digest(payload)?
            {
                return Err(CassetteError::Integrity);
            }
            if let Some(response_id) = response_id {
                validate_id(response_id)?;
                if !response_ids.insert((
                    session_id.to_owned(),
                    attempt_id.to_owned(),
                    response_id.clone(),
                )) {
                    return Err(CassetteError::InvalidReference);
                }
            }
        }
        ResponseBody::Events {
            events,
            transport_chunk_bytes,
        } => {
            if events.is_empty() || transport_chunk_bytes.contains(&0) {
                return Err(CassetteError::InvalidEventStream);
            }
            let count = u32::try_from(events.len()).map_err(|_| CassetteError::SizeOverflow)?;
            if count > limits.max_events.saturating_sub(*total_events) {
                return Err(CassetteError::TooLarge { kind: "events" });
            }
            *total_events = total_events
                .checked_add(count)
                .ok_or(CassetteError::SizeOverflow)?;
            let mut prior_offset = 0;
            for (position, event) in events.iter().enumerate() {
                let expected = u32::try_from(position).map_err(|_| CassetteError::SizeOverflow)?;
                if event.sequence != expected
                    || (position != 0 && event.monotonic_offset_ns < prior_offset)
                    || event.event_type.is_empty()
                    || event.terminal.is_some() != (position + 1 == events.len())
                {
                    return Err(CassetteError::InvalidEventStream);
                }
                prior_offset = event.monotonic_offset_ns;
                encode_bounded(event, limits.max_event_bytes, "event")?;
                if !valid_digest(&event.payload_sha256)
                    || event.payload_sha256 != canonical_value_digest(&event.payload)?
                {
                    return Err(CassetteError::Integrity);
                }
                if let Some(reference) = &event.previous_response_id
                    && !response_ids.contains(&(
                        session_id.to_owned(),
                        attempt_id.to_owned(),
                        reference.clone(),
                    ))
                {
                    return Err(CassetteError::InvalidReference);
                }
                if let Some(response_id) = &event.response_id {
                    validate_id(response_id)?;
                    if !response_ids.insert((
                        session_id.to_owned(),
                        attempt_id.to_owned(),
                        response_id.clone(),
                    )) {
                        return Err(CassetteError::InvalidReference);
                    }
                }
                if let Some(tool_call_id) = &event.tool_call_id {
                    validate_id(tool_call_id)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_headers(headers: &[Header], limits: CassetteLimits) -> Result<(), CassetteError> {
    enforce_count("headers", headers.len(), usize::from(limits.max_headers))?;
    let mut previous: Option<&str> = None;
    for header in headers {
        if header.name.is_empty()
            || header
                .name
                .bytes()
                .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
            || previous.is_some_and(|name| name >= header.name.as_str())
        {
            return Err(CassetteError::NotNormalized);
        }
        previous = Some(&header.name);
    }
    Ok(())
}

fn validate_id(value: &str) -> Result<(), CassetteError> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(CassetteError::InvalidIdentity);
    }
    Ok(())
}

fn validate_limit(kind: &'static str, value: u64, ceiling: u64) -> Result<(), CassetteError> {
    if value == 0 || value > ceiling {
        Err(CassetteError::InvalidLimit {
            kind,
            value,
            ceiling,
        })
    } else {
        Ok(())
    }
}

fn enforce_count(kind: &'static str, actual: usize, maximum: usize) -> Result<(), CassetteError> {
    if actual > maximum {
        Err(CassetteError::TooLarge { kind })
    } else {
        Ok(())
    }
}

fn enforce_size(kind: &'static str, actual: u64, maximum: u64) -> Result<(), CassetteError> {
    if actual > maximum {
        Err(CassetteError::TooLarge { kind })
    } else {
        Ok(())
    }
}

fn encode_bounded<T: Serialize>(
    value: &T,
    maximum: u64,
    kind: &'static str,
) -> Result<Vec<u8>, CassetteError> {
    let maximum = usize::try_from(maximum).map_err(|_| CassetteError::SizeOverflow)?;
    let mut writer = BoundedWriter::new(maximum);
    if let Err(error) = serde_json::to_writer(&mut writer, value) {
        return if writer.exceeded {
            Err(CassetteError::TooLarge { kind })
        } else {
            Err(CassetteError::InvalidJson(error))
        };
    }
    Ok(writer.bytes)
}

struct BoundedWriter {
    bytes: Vec<u8>,
    maximum: usize,
    exceeded: bool,
}

impl BoundedWriter {
    fn new(maximum: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(maximum.min(4096)),
            maximum,
            exceeded: false,
        }
    }
}

impl io::Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.maximum.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("bounded cassette serialization failed"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn canonical_value_digest(value: &Value) -> Result<String, CassetteError> {
    let bytes = serde_json::to_vec(value).map_err(CassetteError::InvalidJson)?;
    Ok(hex_digest(&Sha256::digest(bytes)))
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}

struct NoDuplicateJson;

impl<'de> Deserialize<'de> for NoDuplicateJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateVisitor)
    }
}

struct NoDuplicateVisitor;

impl<'de> Visitor<'de> for NoDuplicateVisitor {
    type Value = NoDuplicateJson;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("JSON without duplicate object members")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut names = BTreeSet::new();
        while let Some(name) = map.next_key::<String>()? {
            if !names.insert(name) {
                return Err(de::Error::custom("duplicate JSON object member"));
            }
            map.next_value::<NoDuplicateJson>()?;
        }
        Ok(NoDuplicateJson)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element::<NoDuplicateJson>()?.is_some() {}
        Ok(NoDuplicateJson)
    }

    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }

    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(NoDuplicateJson)
    }
}

#[cfg(test)]
mod tests {
    use super::NoDuplicateJson;

    #[test]
    fn duplicate_scanner_accepts_every_json_scalar_kind() {
        for value in ["true", "-1", "1", "1.5", "\"text\"", "null", "[]", "{}"] {
            serde_json::from_str::<NoDuplicateJson>(value).unwrap();
        }
    }
}
