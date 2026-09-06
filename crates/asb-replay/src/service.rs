// SPDX-License-Identifier: MIT
//! Strict per-session replay matching and inbound-only HTTP delivery.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;
use thiserror::Error;

use crate::{
    Cassette, CassetteLimits, Header, Interaction, ProviderDialect, RecordedRequest,
    RecordedResponse, ResponseBody, canonical_json_bytes,
};

const MAX_HTTP_HEAD_BYTES: usize = 64 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_HTTP_HEADERS: usize = 1_024;
/// Hard ceiling for one accepted connection's read and write timeouts.
pub const MAX_IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Explicit behavior implemented for one provider request dialect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DialectCapability {
    /// Provider syntax accepted by the route.
    pub dialect: ProviderDialect,
    /// Exact origin-form endpoint accepted by this implementation.
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
pub fn dialect_capabilities() -> [DialectCapability; 3] {
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
}

#[derive(Debug)]
struct ReplayState {
    routes: BTreeMap<(String, String), Vec<Interaction>>,
    cursors: BTreeMap<(String, String), usize>,
    reservations: BTreeMap<(String, String), usize>,
    sensitive_headers: BTreeSet<String>,
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
        let mut routes: BTreeMap<(String, String), Vec<Interaction>> = BTreeMap::new();
        for interaction in cassette.contents.interactions {
            validate_dialect_contract(&interaction)?;
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
        }
        Ok(Self {
            state: Mutex::new(ReplayState {
                routes,
                cursors: BTreeMap::new(),
                reservations: BTreeMap::new(),
                sensitive_headers,
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

    fn serve_connection<S: Read + Write>(
        &self,
        stream: &mut S,
        route: &ReplayRoute,
    ) -> Result<u16, ReplayError> {
        let request = match read_http_request(stream, self.limits) {
            Ok(request) => request,
            Err(error) => {
                let response = error_response(&error);
                write_http_response(stream, &response).map_err(ReplayError::Io)?;
                return Ok(response.status);
            }
        };
        let reservation = match self.reserve(route, request) {
            Ok(reservation) => reservation,
            Err(error) => {
                let response = error_response(&error);
                write_http_response(stream, &response).map_err(ReplayError::Io)?;
                return Ok(response.status);
            }
        };
        let status = reservation.response.status;
        if let Err(error) = write_http_response(stream, &reservation.response) {
            self.finish_reservation(&reservation, false)?;
            return Err(ReplayError::Io(error));
        }
        self.finish_reservation(&reservation, true)?;
        Ok(status)
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
        let (mut stream, peer) = listener.accept().map_err(ReplayError::Io)?;
        if !peer.ip().is_loopback() {
            return Err(ReplayError::InvalidHttp);
        }
        stream
            .set_read_timeout(Some(self.limits.io_timeout))
            .map_err(ReplayError::Io)?;
        stream
            .set_write_timeout(Some(self.limits.io_timeout))
            .map_err(ReplayError::Io)?;
        self.serve_connection(&mut stream, route)
    }
}

fn capability(dialect: ProviderDialect) -> Result<DialectCapability, ReplayError> {
    dialect_capabilities()
        .into_iter()
        .find(|capability| capability.dialect == dialect)
        .ok_or(ReplayError::UnsupportedDialect)
}

fn validate_dialect_contract(interaction: &Interaction) -> Result<(), ReplayError> {
    let capability = capability(interaction.dialect)?;
    let request = &interaction.request;
    if request.method != "POST" || request.path != capability.endpoint {
        return Err(ReplayError::InvalidCassette);
    }
    let object = request
        .body
        .as_object()
        .ok_or(ReplayError::InvalidCassette)?;
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
    let stream = object
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if stream != matches!(interaction.response.body, ResponseBody::Events { .. }) {
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

fn validate_http_request(
    request: &ReplayHttpRequest,
    limits: ReplayLimits,
) -> Result<(), ReplayError> {
    if request.method != "POST"
        || !request.path.starts_with('/')
        || request.path.starts_with("//")
        || request.path.contains(['#', '\\'])
        || request.path.chars().any(char::is_control)
        || request.body.len() > limits.max_body_bytes
        || request.headers.len() > limits.max_headers
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
        if header.name.is_empty()
            || header.name != header.name.to_ascii_lowercase()
            || header
                .name
                .bytes()
                .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
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
) -> Result<bool, ReplayError> {
    if incoming.method != expected.method || incoming.path != expected.path {
        return Ok(false);
    }
    if incoming.path != capability(dialect)?.endpoint {
        return Ok(false);
    }
    let value = crate::cassette::decode_json_value_no_duplicates(&incoming.body)
        .map_err(|_| ReplayError::InvalidHttp)?;
    if canonical_json_bytes(&value).map_err(|_| ReplayError::InvalidHttp)?
        != canonical_json_bytes(&expected.body).map_err(|_| ReplayError::InvalidCassette)?
    {
        return Ok(false);
    }
    normalized_headers_match(&incoming.headers, &expected.headers, sensitive_headers)
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
    let segments = match &response.body {
        ResponseBody::Buffered { payload, .. } => {
            vec![canonical_json_bytes(payload).map_err(|_| ReplayError::InvalidCassette)?]
        }
        ResponseBody::Events { events, .. } => {
            if !headers.iter().any(|header| header.name == "content-type") {
                headers.push(Header {
                    name: "content-type".into(),
                    value: "text/event-stream".into(),
                });
                headers.sort_by(|left, right| left.name.cmp(&right.name));
            }
            let mut encoded = Vec::with_capacity(events.len() + 1);
            for event in events {
                let payload = canonical_json_bytes(&event.payload)
                    .map_err(|_| ReplayError::InvalidCassette)?;
                let mut segment = Vec::new();
                if dialect != ProviderDialect::OpenaiChatCompletions {
                    segment.extend_from_slice(b"event: ");
                    segment.extend_from_slice(event.event_type.as_bytes());
                    segment.push(b'\n');
                }
                segment.extend_from_slice(b"data: ");
                segment.extend_from_slice(&payload);
                segment.extend_from_slice(b"\n\n");
                encoded.push(segment);
            }
            if dialect == ProviderDialect::OpenaiChatCompletions {
                encoded.push(b"data: [DONE]\n\n".to_vec());
            }
            encoded
        }
    };
    Ok(ReplayHttpResponse {
        status: response.status,
        headers,
        segments,
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
    if lengths.len() != 1
        || headers
            .iter()
            .any(|header| header.name == "transfer-encoding")
    {
        return Err(ReplayError::InvalidHttp);
    }
    let encoded_length = lengths[0];
    if encoded_length.is_empty()
        || !encoded_length.bytes().all(|byte| byte.is_ascii_digit())
        || (encoded_length.starts_with('0') && encoded_length.len() != 1)
    {
        return Err(ReplayError::InvalidHttp);
    }
    let length: usize = encoded_length
        .parse()
        .map_err(|_| ReplayError::InvalidHttp)?;
    if length > limits.max_body_bytes {
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
    }
}

fn write_http_response<W: Write>(
    writer: &mut W,
    response: &ReplayHttpResponse,
) -> std::io::Result<()> {
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
    for segment in &response.segments {
        writer.write_all(segment)?;
        writer.flush()?;
    }
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
}
