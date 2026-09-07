// SPDX-License-Identifier: MIT
//! Strict per-session replay matching and inbound-only HTTP delivery.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;
use thiserror::Error;

use crate::cassette::valid_header_name;
use crate::{
    CancellationToken, Cassette, CassetteLimits, Header, Interaction, PacingConfig, PacingError,
    PacingReport, ProviderDialect, RecordedRequest, RecordedResponse, ResponseBody,
    canonical_json_bytes,
};

const GEMINI_GENERATE_CONTENT_ENDPOINT: &str =
    "/v1beta/models/{model}:streamGenerateContent?alt=sse";
const MAX_HTTP_HEAD_BYTES: usize = 64 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_HTTP_HEADERS: usize = 1_024;
/// Hard ceiling for one accepted connection's read and write timeouts.
pub const MAX_IO_TIMEOUT: Duration = Duration::from_secs(30);

trait WriteTimeout {
    fn set_write_timeout_bound(&self, timeout: Option<Duration>) -> std::io::Result<()>;
}

impl WriteTimeout for TcpStream {
    fn set_write_timeout_bound(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_write_timeout(timeout)
    }
}

struct DeadlineStream<'a, S> {
    inner: &'a mut S,
    maximum: Duration,
    deadline: Option<Instant>,
}

impl<'a, S> DeadlineStream<'a, S> {
    fn new(inner: &'a mut S, maximum: Duration) -> Self {
        Self {
            inner,
            maximum,
            deadline: None,
        }
    }
}

impl<S: Read> Read for DeadlineStream<'_, S> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl<S: Write + WriteTimeout> DeadlineStream<'_, S> {
    fn remaining(&mut self) -> std::io::Result<Duration> {
        let deadline = *self.deadline.get_or_insert_with(|| {
            Instant::now()
                .checked_add(self.maximum)
                .expect("validated pacing duration fits Instant")
        });
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "paced replay segment deadline elapsed",
            ))
        } else {
            Ok(remaining)
        }
    }
}

impl<S: Write + WriteTimeout> Write for DeadlineStream<'_, S> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let remaining = self.remaining()?;
        self.inner.set_write_timeout_bound(Some(remaining))?;
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let remaining = self.remaining()?;
        self.inner.set_write_timeout_bound(Some(remaining))?;
        self.inner.flush()?;
        self.deadline = None;
        Ok(())
    }
}

/// Explicit behavior implemented for one provider request dialect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DialectCapability {
    /// Provider syntax accepted by the route.
    pub dialect: ProviderDialect,
    /// Exact origin-form endpoint or bounded model template accepted by this implementation.
    pub endpoint: &'static str,
    /// Complete JSON responses are emitted.
    pub buffered: bool,
    /// Semantic events are emitted as server-sent events.
    pub server_sent_events: bool,
    /// Tool-call payloads are retained without interpretation.
    pub tool_calls: bool,
    /// Prior response identifiers are matched.
    pub causal_response_ids: bool,
}

/// Return implemented syntax capabilities, not real-client compatibility claims.
pub fn dialect_capabilities() -> [DialectCapability; 4] {
    [
        DialectCapability {
            dialect: ProviderDialect::OpenaiChatCompletions,
            endpoint: "/v1/chat/completions",
            buffered: true,
            server_sent_events: true,
            tool_calls: true,
            causal_response_ids: false,
        },
        DialectCapability {
            dialect: ProviderDialect::OpenaiResponses,
            endpoint: "/v1/responses",
            buffered: true,
            server_sent_events: true,
            tool_calls: true,
            causal_response_ids: true,
        },
        DialectCapability {
            dialect: ProviderDialect::AnthropicMessages,
            endpoint: "/v1/messages",
            buffered: true,
            server_sent_events: true,
            tool_calls: true,
            causal_response_ids: false,
        },
        DialectCapability {
            dialect: ProviderDialect::GeminiGenerateContent,
            endpoint: GEMINI_GENERATE_CONTENT_ENDPOINT,
            buffered: true,
            server_sent_events: true,
            tool_calls: true,
            causal_response_ids: false,
        },
    ]
}

/// Session and attempt selected by the trusted benchmark coordinator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayRoute {
    /// Cassette session identity.
    pub session_id: String,
    /// Cassette attempt identity.
    pub attempt_id: String,
    /// Provider syntax expected on this isolated route.
    pub dialect: ProviderDialect,
}

/// Tightenable parser and socket bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayLimits {
    /// Maximum request-line plus header bytes.
    pub max_head_bytes: usize,
    /// Maximum request body bytes.
    pub max_body_bytes: usize,
    /// Maximum request header count.
    pub max_headers: usize,
    /// Timeout for one accepted connection.
    pub io_timeout: Duration,
}

impl Default for ReplayLimits {
    fn default() -> Self {
        Self {
            max_head_bytes: MAX_HTTP_HEAD_BYTES,
            max_body_bytes: MAX_HTTP_BODY_BYTES,
            max_headers: MAX_HTTP_HEADERS,
            io_timeout: MAX_IO_TIMEOUT,
        }
    }
}

impl ReplayLimits {
    fn validate(self) -> Result<Self, ReplayError> {
        if self.max_head_bytes == 0
            || self.max_head_bytes > MAX_HTTP_HEAD_BYTES
            || self.max_body_bytes == 0
            || self.max_body_bytes > MAX_HTTP_BODY_BYTES
            || self.max_headers == 0
            || self.max_headers > MAX_HTTP_HEADERS
            || self.io_timeout.is_zero()
            || self.io_timeout > MAX_IO_TIMEOUT
        {
            Err(ReplayError::InvalidLimits)
        } else {
            Ok(self)
        }
    }
}

/// A bounded semantic HTTP request.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayHttpRequest {
    /// Uppercase method.
    pub method: String,
    /// Origin-form target.
    pub path: String,
    /// Lowercase, unique, sorted headers.
    pub headers: Vec<Header>,
    /// Complete provider JSON body bytes.
    pub body: Vec<u8>,
}

/// Response head and independently writable semantic body segments.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayHttpResponse {
    /// HTTP status.
    pub status: u16,
    /// Captured end-to-end response headers.
    pub headers: Vec<Header>,
    /// One buffered body or one segment per SSE event.
    pub segments: Vec<Vec<u8>>,
    /// Original monotonic offset for each semantic body segment.
    pub recorded_offsets: Vec<Duration>,
}

/// Socket response status and pacing evidence, when an interaction was served.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayDeliveryReport {
    /// HTTP status written to the client.
    pub status: u16,
    /// Complete segment timing, absent for a fail-closed local error response.
    pub pacing: Option<PacingReport>,
}

/// Fail-closed errors that never contain captured values.
#[derive(Debug, Error)]
pub enum ReplayError {
    /// Service bounds are invalid.
    #[error("invalid strict replay limits")]
    InvalidLimits,
    /// Cassette metadata disagrees with provider semantics.
    #[error("cassette does not satisfy the strict dialect contract")]
    InvalidCassette,
    /// No trajectory exists for the route.
    #[error("replay route is unavailable")]
    UnknownRoute,
    /// All interactions on the route were consumed.
    #[error("replay route is exhausted")]
    Exhausted,
    /// Request differs from the next interaction.
    #[error("strict replay request mismatch")]
    Mismatch,
    /// Another socket response owns this route's next interaction.
    #[error("replay route already has an in-flight response")]
    RouteBusy,
    /// HTTP framing, syntax, JSON, or bounds are invalid.
    #[error("invalid bounded replay HTTP request")]
    InvalidHttp,
    /// Dialect is not implemented.
    #[error("provider dialect is unsupported")]
    UnsupportedDialect,
    /// Cursor mutex was poisoned.
    #[error("replay cursor state is unavailable")]
    State,
    /// Inbound local transport failed.
    #[error("local replay transport failed")]
    Io(#[source] std::io::Error),
    /// Paced delivery failed; a completely written response may already be committed.
    #[error("paced replay delivery failed")]
    Pacing(#[source] PacingError),
}

#[derive(Debug)]
struct ReplayState {
    routes: BTreeMap<(String, String), Vec<Interaction>>,
    cursors: BTreeMap<(String, String), usize>,
    reservations: BTreeMap<(String, String), usize>,
    sensitive_headers: BTreeSet<String>,
    request_body_pointers: Vec<String>,
}

#[derive(Debug)]
struct ReservedInteraction {
    key: (String, String),
    cursor: usize,
    response: ReplayHttpResponse,
}

/// Immutable cassette service with independently fenced per-session cursors.
#[derive(Debug)]
pub struct StrictReplayService {
    state: Mutex<ReplayState>,
    limits: ReplayLimits,
}

impl StrictReplayService {
    /// Validate all interactions before making the cassette available.
    pub fn new(cassette: Cassette, limits: ReplayLimits) -> Result<Self, ReplayError> {
        let limits = limits.validate()?;
        crate::cassette::validate_cassette_value(&cassette, CassetteLimits::default())
            .map_err(|_| ReplayError::InvalidCassette)?;
        let sensitive_headers = cassette
            .contents
            .redaction
            .selectors
            .header_names
            .iter()
            .cloned()
            .collect();
        let request_body_pointers = cassette
            .contents
            .redaction
            .selectors
            .request_body_pointers
            .clone();
        let mut routes: BTreeMap<(String, String), Vec<Interaction>> = BTreeMap::new();
        for interaction in cassette.contents.interactions {
            validate_dialect_contract(&interaction, &request_body_pointers)?;
            routes
                .entry((
                    interaction.session_id.clone(),
                    interaction.attempt_id.clone(),
                ))
                .or_default()
                .push(interaction);
        }
        if routes.is_empty() {
            return Err(ReplayError::InvalidCassette);
        }
        for interactions in routes.values_mut() {
            interactions.sort_by_key(|interaction| interaction.ordinal);
            validate_ordered_route_contract(interactions)?;
        }
        Ok(Self {
            state: Mutex::new(ReplayState {
                routes,
                cursors: BTreeMap::new(),
                reservations: BTreeMap::new(),
                sensitive_headers,
                request_body_pointers,
            }),
            limits,
        })
    }

    /// Match and consume one interaction, advancing only on success.
    pub fn handle(
        &self,
        route: &ReplayRoute,
        request: ReplayHttpRequest,
    ) -> Result<ReplayHttpResponse, ReplayError> {
        let reservation = self.reserve(route, request)?;
        self.finish_reservation(&reservation, true)?;
        Ok(reservation.response)
    }

    fn reserve(
        &self,
        route: &ReplayRoute,
        request: ReplayHttpRequest,
    ) -> Result<ReservedInteraction, ReplayError> {
        validate_http_request(&request, self.limits)?;
        let mut state = self.state.lock().map_err(|_| ReplayError::State)?;
        let key = (route.session_id.clone(), route.attempt_id.clone());
        let cursor = *state.cursors.get(&key).unwrap_or(&0);
        if state.reservations.contains_key(&key) {
            return Err(ReplayError::RouteBusy);
        }
        let interaction = state
            .routes
            .get(&key)
            .ok_or(ReplayError::UnknownRoute)?
            .get(cursor)
            .ok_or(ReplayError::Exhausted)?;
        if interaction.dialect != route.dialect {
            return Err(ReplayError::UnsupportedDialect);
        }
        if !request_matches(
            &request,
            &interaction.request,
            route.dialect,
            &state.sensitive_headers,
            &state.request_body_pointers,
        )? {
            return Err(ReplayError::Mismatch);
        }
        let response = encode_response(&interaction.response, route.dialect)?;
        state.reservations.insert(key.clone(), cursor);
        Ok(ReservedInteraction {
            key,
            cursor,
            response,
        })
    }

    fn finish_reservation(
        &self,
        reservation: &ReservedInteraction,
        commit: bool,
    ) -> Result<(), ReplayError> {
        let mut state = self.state.lock().map_err(|_| ReplayError::State)?;
        let reserved_cursor = state
            .reservations
            .remove(&reservation.key)
            .ok_or(ReplayError::State)?;
        let current_cursor = *state.cursors.get(&reservation.key).unwrap_or(&0);
        if reserved_cursor != reservation.cursor || current_cursor != reservation.cursor {
            return Err(ReplayError::State);
        }
        if commit {
            state
                .cursors
                .insert(reservation.key.clone(), reservation.cursor + 1);
        }
        Ok(())
    }

    #[cfg(test)]
    fn serve_connection<S: Read + Write>(
        &self,
        stream: &mut S,
        route: &ReplayRoute,
    ) -> Result<u16, ReplayError> {
        self.serve_connection_paced(
            stream,
            route,
            PacingConfig::immediate(),
            &CancellationToken::default(),
        )
        .map(|report| report.status)
    }

    fn serve_connection_paced<S: Read + Write>(
        &self,
        stream: &mut S,
        route: &ReplayRoute,
        pacing: PacingConfig,
        cancellation: &CancellationToken,
    ) -> Result<ReplayDeliveryReport, ReplayError> {
        pacing.validate().map_err(ReplayError::Pacing)?;
        let request = match read_http_request(stream, self.limits) {
            Ok(request) => request,
            Err(error) => {
                let response = error_response(&error);
                write_http_response(stream, &response).map_err(ReplayError::Io)?;
                return Ok(ReplayDeliveryReport {
                    status: response.status,
                    pacing: None,
                });
            }
        };
        let reservation = match self.reserve(route, request) {
            Ok(reservation) => reservation,
            Err(error) => {
                let response = error_response(&error);
                write_http_response(stream, &response).map_err(ReplayError::Io)?;
                return Ok(ReplayDeliveryReport {
                    status: response.status,
                    pacing: None,
                });
            }
        };
        let status = reservation.response.status;
        if let Err(error) = write_http_head(stream, &reservation.response) {
            self.finish_reservation(&reservation, false)?;
            return Err(ReplayError::Io(error));
        }
        let report = match crate::pacing::write_paced_segments_with_clock(
            stream,
            &reservation.response.segments,
            &reservation.response.recorded_offsets,
            pacing,
            cancellation,
            &crate::SystemMonotonicClock::start(),
        ) {
            Ok(report) => report,
            Err(error) => {
                let delivery_complete =
                    error.completed_segments() == reservation.response.segments.len();
                self.finish_reservation(&reservation, delivery_complete)?;
                return Err(ReplayError::Pacing(error));
            }
        };
        self.finish_reservation(&reservation, true)?;
        Ok(ReplayDeliveryReport {
            status,
            pacing: Some(report),
        })
    }

    /// Accept one inbound loopback HTTP/1.1 connection and then close it.
    ///
    /// No outbound socket is opened. The caller owns listener binding and
    /// surrounding namespace/firewall enforcement.
    pub fn serve_once(
        &self,
        listener: &TcpListener,
        route: &ReplayRoute,
    ) -> Result<u16, ReplayError> {
        self.serve_once_paced(
            listener,
            route,
            PacingConfig::immediate(),
            &CancellationToken::default(),
        )
        .map(|report| report.status)
    }

    /// Accept one loopback connection and deliver it under an explicit pacing policy.
    ///
    /// An incomplete failed write, cancellation, synthetic failure, lateness
    /// violation, or backpressure violation releases the route reservation and
    /// leaves its interaction retryable. If every semantic segment was completely
    /// written before a lateness or backpressure violation was observed, the cursor
    /// commits while this method still returns a pacing error; retrying a fully
    /// written response could duplicate delivery. The socket write timeout is never
    /// greater than the configured segment-write bound.
    pub fn serve_once_paced(
        &self,
        listener: &TcpListener,
        route: &ReplayRoute,
        pacing: PacingConfig,
        cancellation: &CancellationToken,
    ) -> Result<ReplayDeliveryReport, ReplayError> {
        let pacing = pacing.validate().map_err(ReplayError::Pacing)?;
        let (mut stream, peer) = listener.accept().map_err(ReplayError::Io)?;
        if !peer.ip().is_loopback() {
            return Err(ReplayError::InvalidHttp);
        }
        stream
            .set_read_timeout(Some(self.limits.io_timeout))
            .map_err(ReplayError::Io)?;
        let maximum = self.limits.io_timeout.min(pacing.max_segment_write);
        let mut bounded = DeadlineStream::new(&mut stream, maximum);
        self.serve_connection_paced(&mut bounded, route, pacing, cancellation)
    }
}

fn capability(dialect: ProviderDialect) -> Result<DialectCapability, ReplayError> {
    dialect_capabilities()
        .into_iter()
        .find(|capability| capability.dialect == dialect)
        .ok_or(ReplayError::UnsupportedDialect)
}

fn validate_dialect_contract(
    interaction: &Interaction,
    request_body_pointers: &[String],
) -> Result<(), ReplayError> {
    if interaction.request.method == "GET" {
        return validate_model_catalog_contract(interaction);
    }
    capability(interaction.dialect)?;
    let request = &interaction.request;
    validate_selected_request_markers(&request.body, request_body_pointers)?;
    if request.method != "POST"
        || !request_path_valid(interaction.dialect, &request.path, &request.model)
    {
        return Err(ReplayError::InvalidCassette);
    }
    let object = request
        .body
        .as_object()
        .ok_or(ReplayError::InvalidCassette)?;
    if interaction.dialect == ProviderDialect::GeminiGenerateContent {
        return validate_gemini_generate_content_contract(
            interaction,
            object,
            request_body_pointers,
        );
    }
    match interaction.dialect {
        ProviderDialect::OpenaiChatCompletions | ProviderDialect::AnthropicMessages
            if !object.get("messages").is_some_and(Value::is_array) =>
        {
            return Err(ReplayError::InvalidCassette);
        }
        ProviderDialect::OpenaiResponses if !object.contains_key("input") => {
            return Err(ReplayError::InvalidCassette);
        }
        _ => {}
    }
    if object.get("model").and_then(Value::as_str) != Some(request.model.as_str()) {
        return Err(ReplayError::InvalidCassette);
    }
    let tools = object
        .get("tools")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if tools != request.tools {
        return Err(ReplayError::InvalidCassette);
    }
    let previous = object.get("previous_response_id").and_then(Value::as_str);
    if interaction.dialect == ProviderDialect::OpenaiResponses {
        if previous != request.previous_response_id.as_deref() {
            return Err(ReplayError::InvalidCassette);
        }
    } else if previous.is_some() || request.previous_response_id.is_some() {
        return Err(ReplayError::InvalidCassette);
    }
    let excluded: &[&str] = match interaction.dialect {
        ProviderDialect::OpenaiChatCompletions | ProviderDialect::AnthropicMessages => {
            &["messages", "model", "tools"]
        }
        ProviderDialect::OpenaiResponses => &["input", "model", "previous_response_id", "tools"],
        ProviderDialect::GeminiGenerateContent => unreachable!("validated above"),
        ProviderDialect::Synthetic => return Err(ReplayError::UnsupportedDialect),
    };
    let options: BTreeMap<String, Value> = object
        .iter()
        .filter(|(name, _)| !excluded.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    if options != request.options {
        return Err(ReplayError::InvalidCassette);
    }
    let stream = match object.get("stream") {
        None => None,
        Some(Value::Bool(stream)) => Some(*stream),
        Some(_) => return Err(ReplayError::InvalidCassette),
    };
    let event_stream = matches!(interaction.response.body, ResponseBody::Events { .. });
    let exact_sse_content_type = has_exact_content_type(&interaction.response, "text/event-stream");
    let stream_contract_valid = match (stream, event_stream) {
        (Some(true), true) | (Some(false), false) | (None, false) => true,
        (None, true) => {
            interaction.dialect == ProviderDialect::OpenaiChatCompletions && exact_sse_content_type
        }
        _ => false,
    };
    if !stream_contract_valid {
        return Err(ReplayError::InvalidCassette);
    }
    if let ResponseBody::Events { events, .. } = &interaction.response.body
        && events.iter().any(|event| {
            event
                .event_type
                .bytes()
                .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
        })
    {
        return Err(ReplayError::InvalidCassette);
    }
    Ok(())
}

fn validate_gemini_generate_content_contract(
    interaction: &Interaction,
    object: &serde_json::Map<String, Value>,
    request_body_pointers: &[String],
) -> Result<(), ReplayError> {
    let request = &interaction.request;
    if !exact_object_keys(
        object,
        &["contents", "generationConfig", "systemInstruction", "tools"],
    ) || !request_body_pointers.is_empty()
        || request.previous_response_id.is_some()
        || !valid_gemini_response(&interaction.response)
    {
        return Err(ReplayError::InvalidCassette);
    }

    let contents = object
        .get("contents")
        .and_then(Value::as_array)
        .filter(|contents| !contents.is_empty())
        .ok_or(ReplayError::InvalidCassette)?;
    for content in contents {
        validate_gemini_content(content, false)?;
    }
    validate_gemini_content(
        object
            .get("systemInstruction")
            .ok_or(ReplayError::InvalidCassette)?,
        true,
    )?;

    let generation = object
        .get("generationConfig")
        .and_then(Value::as_object)
        .filter(|generation| {
            exact_object_keys(
                generation,
                &["temperature", "thinkingConfig", "topK", "topP"],
            )
        })
        .ok_or(ReplayError::InvalidCassette)?;
    if !generation.get("temperature").is_some_and(Value::is_number)
        || !generation
            .get("thinkingConfig")
            .and_then(Value::as_object)
            .is_some_and(serde_json::Map::is_empty)
        || !generation.get("topK").is_some_and(Value::is_number)
        || !generation.get("topP").is_some_and(Value::is_number)
    {
        return Err(ReplayError::InvalidCassette);
    }

    let tools = object
        .get("tools")
        .and_then(Value::as_array)
        .filter(|tools| tools.len() == 1)
        .ok_or(ReplayError::InvalidCassette)?;
    let tool = tools[0]
        .as_object()
        .filter(|tool| exact_object_keys(tool, &["functionDeclarations"]))
        .ok_or(ReplayError::InvalidCassette)?;
    let declarations = tool
        .get("functionDeclarations")
        .and_then(Value::as_array)
        .filter(|declarations| !declarations.is_empty())
        .ok_or(ReplayError::InvalidCassette)?;
    for declaration in declarations {
        let declaration = declaration
            .as_object()
            .filter(|declaration| {
                exact_object_keys(
                    declaration,
                    &["description", "name", "parametersJsonSchema"],
                )
            })
            .ok_or(ReplayError::InvalidCassette)?;
        if !declaration.get("description").is_some_and(Value::is_string)
            || !declaration.get("name").is_some_and(Value::is_string)
            || !declaration
                .get("parametersJsonSchema")
                .is_some_and(Value::is_object)
        {
            return Err(ReplayError::InvalidCassette);
        }
    }
    if request.tools != *tools {
        return Err(ReplayError::InvalidCassette);
    }
    let options: BTreeMap<String, Value> = object
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "contents" | "tools"))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    if request.options != options {
        return Err(ReplayError::InvalidCassette);
    }
    Ok(())
}

fn valid_gemini_response(response: &RecordedResponse) -> bool {
    match &response.body {
        ResponseBody::Events { events, .. } => {
            response.status == 200
                && has_exact_content_type(response, "text/event-stream")
                && events.len() == 1
                && events[0].event_type == "gemini.generate_content.chunk"
                && valid_gemini_event_payload(&events[0].payload)
        }
        ResponseBody::Buffered {
            payload,
            response_id,
            terminal,
            ..
        } => {
            response.status == 500
                && has_exact_content_type(response, "application/json")
                && response_id.is_none()
                && *terminal == crate::TerminalEvent::Failed
                && payload
                    .as_object()
                    .filter(|payload| exact_object_keys(payload, &["error"]))
                    .and_then(|payload| payload.get("error"))
                    .and_then(Value::as_object)
                    .filter(|error| exact_object_keys(error, &["code", "message", "status"]))
                    .is_some_and(|error| {
                        error.get("code").and_then(Value::as_u64) == Some(500)
                            && error
                                .get("message")
                                .and_then(Value::as_str)
                                .is_some_and(|message| !message.is_empty())
                            && error.get("status").and_then(Value::as_str) == Some("INTERNAL")
                    })
        }
    }
}

fn valid_gemini_event_payload(payload: &Value) -> bool {
    let Some(payload) = payload
        .as_object()
        .filter(|payload| exact_object_keys(payload, &["candidates", "usageMetadata"]))
    else {
        return false;
    };
    let Some(candidate) = payload
        .get("candidates")
        .and_then(Value::as_array)
        .filter(|candidates| candidates.len() == 1)
        .and_then(|candidates| candidates[0].as_object())
        .filter(|candidate| exact_object_keys(candidate, &["content", "finishReason", "index"]))
    else {
        return false;
    };
    let Some(content) = candidate
        .get("content")
        .and_then(Value::as_object)
        .filter(|content| exact_object_keys(content, &["parts", "role"]))
    else {
        return false;
    };
    let valid_part = content
        .get("parts")
        .and_then(Value::as_array)
        .filter(|parts| parts.len() == 1)
        .and_then(|parts| parts[0].as_object())
        .is_some_and(|part| {
            (exact_object_keys(part, &["text"])
                && part
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty()))
                || (exact_object_keys(part, &["functionCall"])
                    && valid_gemini_function_call(part.get("functionCall")))
        });
    let valid_usage = payload
        .get("usageMetadata")
        .and_then(Value::as_object)
        .filter(|usage| {
            exact_object_keys(
                usage,
                &[
                    "candidatesTokenCount",
                    "promptTokenCount",
                    "totalTokenCount",
                ],
            )
        })
        .is_some_and(|usage| {
            let Some(candidate_count) = usage["candidatesTokenCount"].as_u64() else {
                return false;
            };
            let Some(prompt_count) = usage["promptTokenCount"].as_u64() else {
                return false;
            };
            candidate_count.checked_add(prompt_count) == usage["totalTokenCount"].as_u64()
        });
    candidate.get("finishReason").and_then(Value::as_str) == Some("STOP")
        && candidate.get("index").and_then(Value::as_u64) == Some(0)
        && content.get("role").and_then(Value::as_str) == Some("model")
        && valid_part
        && valid_usage
}

fn validate_ordered_route_contract(interactions: &[Interaction]) -> Result<(), ReplayError> {
    for (index, interaction) in interactions.iter().enumerate() {
        if interaction.dialect != ProviderDialect::GeminiGenerateContent {
            continue;
        }
        let next = interactions.get(index + 1);
        match &interaction.response.body {
            ResponseBody::Buffered { .. } => {
                if !next.is_some_and(|next| {
                    next.dialect == ProviderDialect::GeminiGenerateContent
                        && next.request == interaction.request
                }) {
                    return Err(ReplayError::InvalidCassette);
                }
            }
            ResponseBody::Events { events, .. } => {
                if let Some((id, name)) = gemini_response_tool_call(&events[0].payload)
                    && !next.is_some_and(|next| {
                        next.dialect == ProviderDialect::GeminiGenerateContent
                            && gemini_request_contains_pair(&next.request.body, id, name)
                    })
                {
                    return Err(ReplayError::InvalidCassette);
                }
            }
        }
    }
    Ok(())
}

fn gemini_response_tool_call(payload: &Value) -> Option<(&str, &str)> {
    let call = payload.pointer("/candidates/0/content/parts/0/functionCall")?;
    Some((call.get("id")?.as_str()?, call.get("name")?.as_str()?))
}

fn gemini_request_contains_pair(body: &Value, expected_id: &str, expected_name: &str) -> bool {
    let Some(contents) = body.get("contents").and_then(Value::as_array) else {
        return false;
    };
    let mut calls = BTreeMap::<&str, &str>::new();
    for part in contents
        .iter()
        .filter_map(|content| content.get("parts").and_then(Value::as_array))
        .flatten()
    {
        if let Some(call) = part.get("functionCall") {
            let (Some(id), Some(name)) = (
                call.get("id").and_then(Value::as_str),
                call.get("name").and_then(Value::as_str),
            ) else {
                return false;
            };
            if calls.insert(id, name).is_some() {
                return false;
            }
        }
        if let Some(response) = part.get("functionResponse") {
            let (Some(id), Some(name)) = (
                response.get("id").and_then(Value::as_str),
                response.get("name").and_then(Value::as_str),
            ) else {
                return false;
            };
            if calls.remove(id) != Some(name) || id != expected_id || name != expected_name {
                return false;
            }
        }
    }
    calls.is_empty()
}

fn validate_gemini_content(value: &Value, system_instruction: bool) -> Result<(), ReplayError> {
    let content = value
        .as_object()
        .filter(|content| exact_object_keys(content, &["parts", "role"]))
        .ok_or(ReplayError::InvalidCassette)?;
    let role = content.get("role").and_then(Value::as_str);
    let valid_role = if system_instruction {
        role == Some("user")
    } else {
        matches!(role, Some("user" | "model"))
    };
    if !valid_role {
        return Err(ReplayError::InvalidCassette);
    }
    let parts = content
        .get("parts")
        .and_then(Value::as_array)
        .filter(|parts| !parts.is_empty())
        .ok_or(ReplayError::InvalidCassette)?;
    for part in parts {
        let part = part.as_object().ok_or(ReplayError::InvalidCassette)?;
        let valid = if exact_object_keys(part, &["text"]) {
            part.get("text").is_some_and(|text| {
                text.as_str().is_some_and(|text| !text.is_empty())
                    || crate::redaction::valid_marker(text)
            })
        } else if !system_instruction
            && exact_object_keys(part, &["functionCall", "thoughtSignature"])
        {
            valid_gemini_function_call(part.get("functionCall"))
                && part
                    .get("thoughtSignature")
                    .and_then(Value::as_str)
                    .is_some_and(|signature| !signature.is_empty())
        } else {
            !system_instruction
                && exact_object_keys(part, &["functionResponse"])
                && valid_gemini_function_response(part.get("functionResponse"))
        };
        if !valid {
            return Err(ReplayError::InvalidCassette);
        }
    }
    Ok(())
}

fn valid_gemini_function_call(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_object)
        .filter(|call| exact_object_keys(call, &["args", "id", "name"]))
        .is_some_and(|call| {
            call.get("args").is_some_and(Value::is_object)
                && nonempty_string_or_marker(call.get("id"))
                && call
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| !name.is_empty())
        })
}

fn valid_gemini_function_response(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_object)
        .filter(|response| exact_object_keys(response, &["id", "name", "response"]))
        .is_some_and(|response| {
            nonempty_string_or_marker(response.get("id"))
                && response
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| !name.is_empty())
                && response.get("response").is_some_and(Value::is_object)
        })
}

fn nonempty_string_or_marker(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value.as_str().is_some_and(|value| !value.is_empty())
            || crate::redaction::valid_marker(value)
    })
}

fn exact_object_keys(object: &serde_json::Map<String, Value>, expected: &[&str]) -> bool {
    object.len() == expected.len() && expected.iter().all(|name| object.contains_key(*name))
}

fn request_path_valid(dialect: ProviderDialect, path: &str, model: &str) -> bool {
    expected_request_path(dialect, model).is_ok_and(|expected| path == expected)
}

fn expected_request_path(dialect: ProviderDialect, model: &str) -> Result<String, ReplayError> {
    if dialect == ProviderDialect::GeminiGenerateContent {
        if !valid_gemini_model(model) {
            return Err(ReplayError::InvalidCassette);
        }
        Ok(format!(
            "/v1beta/models/{model}:streamGenerateContent?alt=sse"
        ))
    } else {
        Ok(capability(dialect)?.endpoint.to_owned())
    }
}

fn valid_gemini_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 256
        && model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_model_catalog_contract(interaction: &Interaction) -> Result<(), ReplayError> {
    let request = &interaction.request;
    let detail = request.path.strip_prefix("/v1/models/");
    let path_valid = request.path == "/v1/models"
        || detail.is_some_and(|model| model == request.model && valid_catalog_model(model));
    if interaction.dialect != ProviderDialect::OpenaiChatCompletions
        || !path_valid
        || !request.body.is_null()
        || !request.options.is_empty()
        || !request.tools.is_empty()
        || request.previous_response_id.is_some()
        || request.model.len() > 256
        || !valid_catalog_model(&request.model)
        || interaction.response.status != 200
        || !matches!(
            interaction.response.body,
            ResponseBody::Buffered {
                response_id: None,
                terminal: crate::TerminalEvent::Completed,
                ..
            }
        )
        || !has_exact_content_type(&interaction.response, "application/json")
    {
        return Err(ReplayError::InvalidCassette);
    }
    Ok(())
}

fn valid_catalog_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 256
        && model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

fn has_exact_content_type(response: &RecordedResponse, expected: &str) -> bool {
    response
        .headers
        .iter()
        .filter(|header| header.name == "content-type")
        .map(|header| header.value.as_str())
        .eq([expected])
}

fn validate_http_request(
    request: &ReplayHttpRequest,
    limits: ReplayLimits,
) -> Result<(), ReplayError> {
    if !matches!(request.method.as_str(), "GET" | "POST")
        || !request.path.starts_with('/')
        || request.path.starts_with("//")
        || request.path.contains(['#', '\\'])
        || request.path.chars().any(char::is_control)
        || request.body.len() > limits.max_body_bytes
        || request.headers.len() > limits.max_headers
        || request.method == "GET" && !request.body.is_empty()
    {
        return Err(ReplayError::InvalidHttp);
    }
    let mut previous: Option<&str> = None;
    let mut head_bytes = request
        .method
        .len()
        .checked_add(request.path.len())
        .and_then(|size| size.checked_add(12))
        .ok_or(ReplayError::InvalidHttp)?;
    for header in &request.headers {
        head_bytes = head_bytes
            .checked_add(header.name.len())
            .and_then(|size| size.checked_add(header.value.len()))
            .and_then(|size| size.checked_add(4))
            .ok_or(ReplayError::InvalidHttp)?;
        if header.name != header.name.to_ascii_lowercase()
            || !valid_header_name(&header.name)
            || header.value.chars().any(char::is_control)
            || previous.is_some_and(|name| name >= header.name.as_str())
        {
            return Err(ReplayError::InvalidHttp);
        }
        previous = Some(&header.name);
    }
    if head_bytes > limits.max_head_bytes {
        return Err(ReplayError::InvalidHttp);
    }
    Ok(())
}

fn request_matches(
    incoming: &ReplayHttpRequest,
    expected: &RecordedRequest,
    dialect: ProviderDialect,
    sensitive_headers: &BTreeSet<String>,
    request_body_pointers: &[String],
) -> Result<bool, ReplayError> {
    if incoming.method != expected.method || incoming.path != expected.path {
        return Ok(false);
    }
    if expected.method == "GET" {
        if !incoming.body.is_empty()
            || dialect != ProviderDialect::OpenaiChatCompletions
            || !expected.body.is_null()
        {
            return Err(ReplayError::InvalidCassette);
        }
        return normalized_headers_match(&incoming.headers, &expected.headers, sensitive_headers);
    }
    if incoming.path != expected_request_path(dialect, &expected.model)? {
        return Ok(false);
    }
    let mut value = crate::cassette::decode_json_value_no_duplicates(&incoming.body)
        .map_err(|_| ReplayError::InvalidHttp)?;
    if crate::redaction::contains_marker(&value) {
        return Err(ReplayError::InvalidHttp);
    }
    for pointer in request_body_pointers {
        let Some(incoming_selected) = value.pointer_mut(pointer) else {
            return Ok(false);
        };
        let expected_selected = expected
            .body
            .pointer(pointer)
            .ok_or(ReplayError::InvalidCassette)?;
        if !crate::redaction::valid_marker(expected_selected) {
            return Err(ReplayError::InvalidCassette);
        }
        *incoming_selected = expected_selected.clone();
    }
    if canonical_json_bytes(&value).map_err(|_| ReplayError::InvalidHttp)?
        != canonical_json_bytes(&expected.body).map_err(|_| ReplayError::InvalidCassette)?
    {
        return Ok(false);
    }
    normalized_headers_match(&incoming.headers, &expected.headers, sensitive_headers)
}

fn validate_selected_request_markers(
    body: &Value,
    request_body_pointers: &[String],
) -> Result<(), ReplayError> {
    let mut scrubbed = body.clone();
    for pointer in request_body_pointers {
        let selected = scrubbed
            .pointer_mut(pointer)
            .ok_or(ReplayError::InvalidCassette)?;
        if !crate::redaction::valid_marker(selected) {
            return Err(ReplayError::InvalidCassette);
        }
        *selected = Value::Null;
    }
    if crate::redaction::contains_marker(&scrubbed) {
        return Err(ReplayError::InvalidCassette);
    }
    Ok(())
}

fn normalized_headers_match(
    incoming: &[Header],
    expected: &[Header],
    sensitive: &BTreeSet<String>,
) -> Result<bool, ReplayError> {
    const TRANSPORT: [&str; 5] = [
        "accept-encoding",
        "connection",
        "content-length",
        "host",
        "user-agent",
    ];
    let expected_by_name: BTreeMap<&str, &str> = expected
        .iter()
        .map(|header| (header.name.as_str(), header.value.as_str()))
        .collect();
    let mut normalized = Vec::new();
    for header in incoming {
        if TRANSPORT.contains(&header.name.as_str()) {
            continue;
        }
        let value = if sensitive.contains(&header.name) {
            let Some(value) = expected_by_name.get(header.name.as_str()) else {
                return Ok(false);
            };
            (*value).to_owned()
        } else {
            header.value.clone()
        };
        normalized.push(Header {
            name: header.name.clone(),
            value,
        });
    }
    Ok(normalized == expected)
}

fn encode_response(
    response: &RecordedResponse,
    dialect: ProviderDialect,
) -> Result<ReplayHttpResponse, ReplayError> {
    let mut headers: Vec<Header> = response
        .headers
        .iter()
        .filter(|header| {
            !matches!(
                header.name.as_str(),
                "connection" | "content-length" | "transfer-encoding"
            )
        })
        .cloned()
        .collect();
    let (segments, recorded_offsets) = match &response.body {
        ResponseBody::Buffered { payload, .. } => (
            vec![canonical_json_bytes(payload).map_err(|_| ReplayError::InvalidCassette)?],
            vec![Duration::ZERO],
        ),
        ResponseBody::Events { events, .. } => {
            if !headers.iter().any(|header| header.name == "content-type") {
                headers.push(Header {
                    name: "content-type".into(),
                    value: "text/event-stream".into(),
                });
                headers.sort_by(|left, right| left.name.cmp(&right.name));
            }
            let mut encoded = Vec::with_capacity(events.len() + 1);
            let mut offsets = Vec::with_capacity(events.len() + 1);
            for event in events {
                let payload = canonical_json_bytes(&event.payload)
                    .map_err(|_| ReplayError::InvalidCassette)?;
                let mut segment = Vec::new();
                if !matches!(
                    dialect,
                    ProviderDialect::OpenaiChatCompletions | ProviderDialect::GeminiGenerateContent
                ) {
                    segment.extend_from_slice(b"event: ");
                    segment.extend_from_slice(event.event_type.as_bytes());
                    segment.push(b'\n');
                }
                segment.extend_from_slice(b"data: ");
                segment.extend_from_slice(&payload);
                segment.extend_from_slice(b"\n\n");
                encoded.push(segment);
                offsets.push(Duration::from_nanos(event.monotonic_offset_ns));
            }
            if dialect == ProviderDialect::OpenaiChatCompletions {
                encoded
                    .last_mut()
                    .ok_or(ReplayError::InvalidCassette)?
                    .extend_from_slice(b"data: [DONE]\n\n");
            }
            (encoded, offsets)
        }
    };
    Ok(ReplayHttpResponse {
        status: response.status,
        headers,
        segments,
        recorded_offsets,
    })
}

fn read_http_request<R: Read>(
    reader: &mut R,
    limits: ReplayLimits,
) -> Result<ReplayHttpRequest, ReplayError> {
    let mut bytes = Vec::with_capacity(4096);
    let head_end = loop {
        if bytes.len() >= limits.max_head_bytes {
            return Err(ReplayError::InvalidHttp);
        }
        let mut chunk = [0_u8; 4096];
        let read = reader.read(&mut chunk).map_err(ReplayError::Io)?;
        if read == 0 {
            return Err(ReplayError::InvalidHttp);
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    if head_end > limits.max_head_bytes {
        return Err(ReplayError::InvalidHttp);
    }
    let head = std::str::from_utf8(&bytes[..head_end]).map_err(|_| ReplayError::InvalidHttp)?;
    let mut lines = head[..head.len() - 4].split("\r\n");
    let request_line = lines.next().ok_or(ReplayError::InvalidHttp)?;
    let mut parts = request_line.split(' ');
    let (Some(method), Some(path), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(ReplayError::InvalidHttp);
    };
    if version != "HTTP/1.1" {
        return Err(ReplayError::InvalidHttp);
    }
    let method = method.to_owned();
    let path = path.to_owned();
    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(ReplayError::InvalidHttp)?;
        let Some(value) = value.strip_prefix(' ') else {
            return Err(ReplayError::InvalidHttp);
        };
        headers.push(Header {
            name: name.to_ascii_lowercase(),
            value: value.to_owned(),
        });
    }
    headers.sort_by(|left, right| left.name.cmp(&right.name));
    let lengths: Vec<&str> = headers
        .iter()
        .filter(|header| header.name == "content-length")
        .map(|header| header.value.as_str())
        .collect();
    if headers
        .iter()
        .any(|header| header.name == "transfer-encoding")
    {
        return Err(ReplayError::InvalidHttp);
    }
    let encoded_length = match (method.as_str(), lengths.as_slice()) {
        ("GET", []) => "0",
        ("GET" | "POST", [length]) => *length,
        _ => return Err(ReplayError::InvalidHttp),
    };
    if encoded_length.is_empty()
        || !encoded_length.bytes().all(|byte| byte.is_ascii_digit())
        || (encoded_length.starts_with('0') && encoded_length.len() != 1)
    {
        return Err(ReplayError::InvalidHttp);
    }
    let length: usize = encoded_length
        .parse()
        .map_err(|_| ReplayError::InvalidHttp)?;
    if length > limits.max_body_bytes || method == "GET" && length != 0 {
        return Err(ReplayError::InvalidHttp);
    }
    let total = head_end
        .checked_add(length)
        .ok_or(ReplayError::InvalidHttp)?;
    if bytes.len() > total {
        return Err(ReplayError::InvalidHttp);
    }
    let already = bytes.len();
    bytes.resize(total, 0);
    reader
        .read_exact(&mut bytes[already..])
        .map_err(ReplayError::Io)?;
    Ok(ReplayHttpRequest {
        method,
        path,
        headers,
        body: bytes[head_end..].to_vec(),
    })
}

fn error_response(error: &ReplayError) -> ReplayHttpResponse {
    let (status, code) = match error {
        ReplayError::UnsupportedDialect => (422, "unsupported_dialect"),
        ReplayError::Mismatch
        | ReplayError::UnknownRoute
        | ReplayError::Exhausted
        | ReplayError::RouteBusy => (409, "replay_mismatch"),
        _ => (400, "invalid_request"),
    };
    ReplayHttpResponse {
        status,
        headers: vec![Header {
            name: "content-type".into(),
            value: "application/json".into(),
        }],
        segments: vec![format!("{{\"error\":\"{code}\"}}").into_bytes()],
        recorded_offsets: vec![Duration::ZERO],
    }
}

fn write_http_response<W: Write>(
    writer: &mut W,
    response: &ReplayHttpResponse,
) -> std::io::Result<()> {
    write_http_head(writer, response)?;
    for segment in &response.segments {
        writer.write_all(segment)?;
        writer.flush()?;
    }
    Ok(())
}

fn write_http_head<W: Write>(writer: &mut W, response: &ReplayHttpResponse) -> std::io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        400 => "Bad Request",
        409 => "Conflict",
        422 => "Unprocessable Content",
        _ => "Recorded",
    };
    let length = response
        .segments
        .iter()
        .try_fold(0_usize, |total, segment| total.checked_add(segment.len()))
        .ok_or_else(|| std::io::Error::other("replay response length overflow"))?;
    write!(writer, "HTTP/1.1 {} {}\r\n", response.status, reason)?;
    for header in &response.headers {
        write!(writer, "{}: {}\r\n", header.name, header.value)?;
    }
    write!(
        writer,
        "content-length: {length}\r\nconnection: close\r\n\r\n"
    )?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, ErrorKind};
    use std::sync::{Arc, mpsc};
    use std::thread;

    struct MemoryIo {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
        fail_write: bool,
        block_write: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    }

    impl MemoryIo {
        fn new(input: Vec<u8>) -> Self {
            Self {
                input: Cursor::new(input),
                output: Vec::new(),
                fail_write: false,
                block_write: None,
            }
        }
    }

    impl Read for MemoryIo {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buffer)
        }
    }

    impl Write for MemoryIo {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            if self.fail_write {
                self.fail_write = false;
                return Err(std::io::Error::new(
                    ErrorKind::BrokenPipe,
                    "synthetic disconnected peer",
                ));
            }
            if let Some((started, release)) = self.block_write.take() {
                started
                    .send(())
                    .expect("test observer must remain available");
                release.recv().expect("test writer must be released");
            }
            self.output.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct SlowProgressIo {
        writes: usize,
    }

    impl WriteTimeout for SlowProgressIo {
        fn set_write_timeout_bound(&self, timeout: Option<Duration>) -> std::io::Result<()> {
            assert!(timeout.is_some_and(|value| !value.is_zero()));
            Ok(())
        }
    }

    impl Write for SlowProgressIo {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            thread::sleep(Duration::from_millis(2));
            Ok(1)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn deadline_writer_bounds_a_peer_that_makes_only_slow_partial_progress() {
        let mut slow = SlowProgressIo { writes: 0 };
        let started = Instant::now();
        let error = {
            let mut bounded = DeadlineStream::new(&mut slow, Duration::from_millis(5));
            bounded.write_all(&[0_u8; 1_024]).unwrap_err()
        };
        let elapsed = started.elapsed();
        assert_eq!(error.kind(), ErrorKind::TimedOut);
        assert!(elapsed < Duration::from_millis(100));
        assert!(slow.writes < 1_024);
    }

    fn body() -> Value {
        serde_json::json!({
            "messages": [{"role": "user", "content": "synthetic"}],
            "model": "messages-model",
            "stream": false,
            "tools": []
        })
    }

    fn interaction(session: &str) -> Interaction {
        Interaction {
            session_id: session.into(),
            attempt_id: "attempt-1".into(),
            interaction_id: format!("{session}-0"),
            ordinal: 0,
            dialect: ProviderDialect::AnthropicMessages,
            request: RecordedRequest {
                method: "POST".into(),
                path: "/v1/messages".into(),
                headers: vec![Header {
                    name: "content-type".into(),
                    value: "application/json".into(),
                }],
                body: body(),
                body_sha256: String::new(),
                model: "messages-model".into(),
                options: BTreeMap::from([("stream".into(), Value::Bool(false))]),
                tools: Vec::new(),
                previous_response_id: None,
            },
            response: RecordedResponse {
                status: 200,
                headers: vec![Header {
                    name: "content-type".into(),
                    value: "application/json".into(),
                }],
                body: ResponseBody::Buffered {
                    payload: serde_json::json!({"id": format!("response-{session}")}),
                    payload_sha256: String::new(),
                    response_id: Some(format!("response-{session}")),
                    terminal: crate::TerminalEvent::Completed,
                },
            },
        }
    }

    fn service() -> StrictReplayService {
        let routes = ["one", "two"]
            .into_iter()
            .map(|session| {
                (
                    (session.to_owned(), "attempt-1".to_owned()),
                    vec![interaction(session)],
                )
            })
            .collect();
        StrictReplayService {
            state: Mutex::new(ReplayState {
                routes,
                cursors: BTreeMap::new(),
                reservations: BTreeMap::new(),
                sensitive_headers: BTreeSet::new(),
                request_body_pointers: Vec::new(),
            }),
            limits: ReplayLimits::default(),
        }
    }

    fn route(session: &str) -> ReplayRoute {
        ReplayRoute {
            session_id: session.into(),
            attempt_id: "attempt-1".into(),
            dialect: ProviderDialect::AnthropicMessages,
        }
    }

    fn request() -> ReplayHttpRequest {
        let body = serde_json::to_vec(&body()).unwrap();
        ReplayHttpRequest {
            method: "POST".into(),
            path: "/v1/messages".into(),
            headers: vec![
                Header {
                    name: "content-length".into(),
                    value: body.len().to_string(),
                },
                Header {
                    name: "content-type".into(),
                    value: "application/json".into(),
                },
                Header {
                    name: "host".into(),
                    value: "127.0.0.1".into(),
                },
            ],
            body,
        }
    }

    fn raw_request() -> Vec<u8> {
        let request = request();
        format!(
            "POST /v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            request.body.len()
        )
        .into_bytes()
        .into_iter()
        .chain(request.body)
        .collect()
    }

    #[test]
    fn disconnected_writer_releases_interaction_for_exact_retry() {
        let service = service();
        let mut disconnected = MemoryIo::new(raw_request());
        disconnected.fail_write = true;
        assert!(matches!(
            service.serve_connection(&mut disconnected, &route("one")),
            Err(ReplayError::Io(error)) if error.kind() == ErrorKind::BrokenPipe
        ));

        let mut retry = MemoryIo::new(raw_request());
        assert_eq!(
            service.serve_connection(&mut retry, &route("one")).unwrap(),
            200
        );
        assert!(retry.output.starts_with(b"HTTP/1.1 200 OK\r\n"));
        let mut exhausted = MemoryIo::new(raw_request());
        assert_eq!(
            service
                .serve_connection(&mut exhausted, &route("one"))
                .unwrap(),
            409
        );
    }

    #[test]
    fn final_segment_lateness_rejection_leaves_interaction_retryable() {
        let service = service();
        let selected = route("one");
        let mut late = MemoryIo::new(raw_request());
        let error = service
            .serve_connection_paced(
                &mut late,
                &selected,
                PacingConfig {
                    mode: crate::PacingMode::Original,
                    max_segment_write: Duration::from_secs(1),
                    max_lateness: Duration::ZERO,
                    cancellation_poll: Duration::from_millis(1),
                },
                &CancellationToken::default(),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            ReplayError::Pacing(PacingError::Backpressure {
                completed_segments: 0
            })
        ));

        let mut retry = MemoryIo::new(raw_request());
        assert_eq!(
            service.serve_connection(&mut retry, &selected).unwrap(),
            200
        );
        let mut exhausted = MemoryIo::new(raw_request());
        assert_eq!(
            service.serve_connection(&mut exhausted, &selected).unwrap(),
            409
        );
    }

    #[test]
    fn completed_final_segment_over_bound_commits_interaction() {
        let service = service();
        let selected = route("one");
        let mut slow = MemoryIo::new(raw_request());
        let error = service
            .serve_connection_paced(
                &mut slow,
                &selected,
                PacingConfig {
                    mode: crate::PacingMode::Immediate,
                    max_segment_write: Duration::from_nanos(1),
                    max_lateness: Duration::from_secs(1),
                    cancellation_poll: Duration::from_millis(1),
                },
                &CancellationToken::default(),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            ReplayError::Pacing(PacingError::Backpressure {
                completed_segments: 1
            })
        ));

        let mut retry = MemoryIo::new(raw_request());
        assert_eq!(
            service.serve_connection(&mut retry, &selected).unwrap(),
            409
        );
    }

    #[test]
    fn in_flight_route_is_fenced_without_blocking_unrelated_route() {
        let service = Arc::new(service());
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let mut blocked = MemoryIo::new(raw_request());
        blocked.block_write = Some((started_tx, release_rx));
        let server = Arc::clone(&service);
        let writer = thread::spawn(move || server.serve_connection(&mut blocked, &route("one")));
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("response writer must reach deterministic fence");
        let mut same_route = MemoryIo::new(raw_request());
        assert_eq!(
            service
                .serve_connection(&mut same_route, &route("one"))
                .unwrap(),
            409
        );
        assert!(same_route.output.starts_with(b"HTTP/1.1 409 Conflict\r\n"));
        let mut unrelated = MemoryIo::new(raw_request());
        assert_eq!(
            service
                .serve_connection(&mut unrelated, &route("two"))
                .unwrap(),
            200
        );
        assert!(unrelated.output.starts_with(b"HTTP/1.1 200 OK\r\n"));
        release_tx.send(()).unwrap();
        assert_eq!(writer.join().unwrap().unwrap(), 200);
    }

    #[test]
    fn selected_marker_validation_rejects_missing_malformed_and_unselected_markers() {
        let pointers = vec!["/input/0".to_owned()];
        assert!(
            validate_selected_request_markers(
                &serde_json::json!({"input": ["[ASB_REDACTED:000001]"]}),
                &pointers,
            )
            .is_ok()
        );
        for body in [
            serde_json::json!({"input": []}),
            serde_json::json!({"input": ["[ASB_REDACTED:000000]"]}),
            serde_json::json!({"input": ["[ASB_REDACTED:000001]suffix"]}),
            serde_json::json!({
                "input": ["[ASB_REDACTED:000001]"],
                "unselected": "[ASB_REDACTED:000002]"
            }),
        ] {
            assert!(matches!(
                validate_selected_request_markers(&body, &pointers),
                Err(ReplayError::InvalidCassette)
            ));
        }
    }
}
