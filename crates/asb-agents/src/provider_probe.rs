// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded provider authentication probe result classification.

use crate::authenticated_request::{
    AuthRequestError, AuthenticatedRequest, Cancellation, HeaderSink,
};
use crate::credential::ResolvedCredential;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use url::Url;

/// Maximum response body accepted by a provider authentication probe.
pub const MAX_PROBE_BODY_BYTES: usize = 16 * 1024;
/// Maximum probe deadline in milliseconds.
pub const MAX_PROBE_TIMEOUT_MS: u64 = 30_000;

/// Execute a credential-free, loopback-only HTTP probe with bounded I/O.
///
/// The endpoint is bound to the request's SHA-256 identity and is never
/// redirected. Callers provide the current enrollment generation so a stale
/// response cannot be accepted after rotation.
pub fn execute_loopback_probe<F>(
    request: &ProbeRequest,
    endpoint: &str,
    current_generation: F,
) -> Result<ProbeOutcome, ProbeTransportError>
where
    F: Fn() -> u64,
{
    execute_loopback_probe_with_header(request, endpoint, current_generation, None, None)
}

/// Execute a loopback probe after injecting the qualified provider credential.
/// The header is written only to the bounded transport request and wiped after use.
pub fn execute_authenticated_loopback_probe<F, C>(
    request: &ProbeRequest,
    auth_request: AuthenticatedRequest,
    endpoint: &str,
    current_generation: F,
    now_ms: u64,
    credential: ResolvedCredential,
    cancelled: C,
) -> Result<ProbeOutcome, ProbeTransportError>
where
    F: Fn() -> u64 + Copy,
    C: Cancellation + Copy,
{
    let provider_matches = matches!(
        (request.provider, auth_request.provider),
        (
            ProbeProvider::OpenAi,
            crate::authenticated_request::AuthProvider::OpenAi
        ) | (
            ProbeProvider::Gemini,
            crate::authenticated_request::AuthProvider::Gemini
        ) | (
            ProbeProvider::Ollama,
            crate::authenticated_request::AuthProvider::Ollama
        )
    );
    if !provider_matches {
        return Err(ProbeTransportError::ProviderMismatch);
    }
    let auth_budget = auth_request.deadline_ms.saturating_sub(now_ms);
    if auth_budget == 0 {
        return Err(ProbeTransportError::Authentication(
            AuthRequestError::DeadlineExceeded,
        ));
    }
    let mut header = CapturedHeader::default();
    let injection = inject_probe_auth(
        auth_request,
        endpoint,
        current_generation,
        now_ms,
        credential,
        &mut header,
        cancelled,
    );
    if let Err(error) = injection {
        header.wipe();
        return Err(ProbeTransportError::Authentication(error));
    }
    let result = execute_loopback_probe_with_header(
        request,
        endpoint,
        current_generation,
        Some(header.bytes.as_slice()),
        Some((
            &cancelled,
            Instant::now(),
            Duration::from_millis(request.timeout_ms.min(auth_budget)),
        )),
    );
    header.wipe();
    result
}

fn execute_loopback_probe_with_header<F>(
    request: &ProbeRequest,
    endpoint: &str,
    current_generation: F,
    auth_header: Option<&[u8]>,
    budget: Option<(&dyn Cancellation, Instant, Duration)>,
) -> Result<ProbeOutcome, ProbeTransportError>
where
    F: Fn() -> u64,
{
    let budget_error = |budget: &Option<(&dyn Cancellation, Instant, Duration)>| {
        budget.as_ref().and_then(|(cancelled, started, limit)| {
            if cancelled.is_cancelled() {
                Some(AuthRequestError::CancelledOrStale)
            } else if started.elapsed() >= *limit {
                Some(AuthRequestError::DeadlineExceeded)
            } else {
                None
            }
        })
    };
    let remaining_budget = |budget: &Option<(&dyn Cancellation, Instant, Duration)>| {
        budget
            .as_ref()
            .map(|(_, started, limit)| limit.saturating_sub(started.elapsed()))
    };
    request
        .validate()
        .map_err(ProbeTransportError::InvalidRequest)?;
    if current_generation() != request.generation {
        return Err(ProbeTransportError::StaleGeneration);
    }
    let url = Url::parse(endpoint).map_err(|_| ProbeTransportError::InvalidEndpoint)?;
    if url.scheme() != "http" || url.username() != "" || url.password().is_some() {
        return Err(ProbeTransportError::UnqualifiedTransport);
    }
    let host = url.host_str().ok_or(ProbeTransportError::InvalidEndpoint)?;
    let ip = match host {
        "localhost" => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        _ => host
            .parse::<IpAddr>()
            .map_err(|_| ProbeTransportError::UnqualifiedTransport)?,
    };
    if !ip.is_loopback() {
        return Err(ProbeTransportError::UnqualifiedTransport);
    }
    let port = url.port().ok_or(ProbeTransportError::InvalidEndpoint)?;
    let expected = format!("{:x}", Sha256::digest(endpoint.as_bytes()));
    if expected != request.endpoint_identity_sha256 {
        return Err(ProbeTransportError::EndpointIdentityMismatch);
    }
    let address = SocketAddr::new(ip, port);
    let timeout = Duration::from_millis(request.timeout_ms);
    if let Some(error) = budget_error(&budget) {
        return Err(ProbeTransportError::Authentication(error));
    }
    let connect_timeout = remaining_budget(&budget).unwrap_or(timeout);
    let mut stream = TcpStream::connect_timeout(&address, connect_timeout)
        .map_err(|_| ProbeTransportError::Unavailable)?;
    let socket_timeout = remaining_budget(&budget).unwrap_or(timeout);
    stream
        .set_read_timeout(Some(socket_timeout))
        .and_then(|_| stream.set_write_timeout(Some(socket_timeout)))
        .map_err(|_| ProbeTransportError::Unavailable)?;
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let target = url
        .query()
        .map_or_else(|| path.to_owned(), |query| format!("{path}?{query}"));
    let host_header = if url.port() == Some(80) {
        host.to_owned()
    } else {
        format!("{host}:{port}")
    };
    if let Some(error) = budget_error(&budget) {
        return Err(ProbeTransportError::Authentication(error));
    }
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: {host_header}\r\nConnection: close\r\n"
    )
    .map_err(|_| ProbeTransportError::Unavailable)?;
    if let Some(header) = auth_header {
        stream
            .write_all(header)
            .and_then(|_| stream.write_all(b"\r\n\r\n"))
            .map_err(|_| ProbeTransportError::Unavailable)?;
    } else {
        stream
            .write_all(b"\r\n")
            .map_err(|_| ProbeTransportError::Unavailable)?;
    }
    let mut response = Vec::with_capacity(request.max_response_bytes.min(4096));
    let mut chunk = [0_u8; 1024];
    loop {
        if let Some(error) = budget_error(&budget) {
            return Err(ProbeTransportError::Authentication(error));
        }
        if let Some(remaining) = remaining_budget(&budget) {
            stream
                .set_read_timeout(Some(remaining))
                .map_err(|_| ProbeTransportError::Unavailable)?;
        }
        let read = stream.read(&mut chunk).map_err(|_| {
            budget_error(&budget).map_or(
                ProbeTransportError::Unavailable,
                ProbeTransportError::Authentication,
            )
        })?;
        if read == 0 {
            break;
        }
        if response.len() + read > request.max_response_bytes {
            return Err(ProbeTransportError::ResponseTooLarge);
        }
        response.extend_from_slice(&chunk[..read]);
    }
    if let Some(error) = budget_error(&budget) {
        return Err(ProbeTransportError::Authentication(error));
    }
    if current_generation() != request.generation {
        return Err(ProbeTransportError::StaleGeneration);
    }
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(ProbeTransportError::MalformedResponse)?;
    let headers = &response[..header_end];
    let body_len = response.len() - header_end - 4;
    let mut lines = headers.split(|byte| *byte == b'\n');
    let status = lines
        .next()
        .and_then(|line| line.strip_suffix(b"\r"))
        .and_then(|line| line.split(|byte| *byte == b' ').nth(1))
        .and_then(|code| std::str::from_utf8(code).ok())
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or(ProbeTransportError::MalformedResponse)?;
    let content_type_json = lines.any(|line| {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        line.len() >= 13
            && line[..13].eq_ignore_ascii_case(b"content-type:")
            && std::str::from_utf8(&line[13..]).is_ok_and(|value| {
                value
                    .trim()
                    .to_ascii_lowercase()
                    .starts_with("application/json")
            })
    });
    Ok(classify_probe(
        request.provider,
        status,
        body_len,
        content_type_json,
    ))
}

/// Failures from the qualified bounded probe transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeTransportError {
    /// Request validation failed before opening a socket.
    InvalidRequest(ProbeRequestError),
    /// Endpoint syntax or required port is invalid.
    InvalidEndpoint,
    /// Endpoint is not an explicitly loopback HTTP transport.
    UnqualifiedTransport,
    /// Endpoint bytes do not match the enrolled identity.
    EndpointIdentityMismatch,
    /// The enrollment generation changed during the probe.
    StaleGeneration,
    /// The endpoint did not respond within the bounded transport.
    Unavailable,
    /// The response exceeded the enrolled body bound.
    ResponseTooLarge,
    /// The response did not contain a parseable status/header block.
    MalformedResponse,
    /// The authenticated request provider does not match the probe provider.
    ProviderMismatch,
    /// Credential injection failed before transport.
    Authentication(AuthRequestError),
}

#[derive(Default)]
struct CapturedHeader {
    bytes: Vec<u8>,
}

impl HeaderSink for CapturedHeader {
    fn write_header(
        &mut self,
        name: &[u8],
        prefix: &[u8],
        value: &[u8],
    ) -> Result<(), crate::authenticated_request::HeaderWriteError> {
        self.bytes.extend_from_slice(name);
        self.bytes.extend_from_slice(b": ");
        self.bytes.extend_from_slice(prefix);
        self.bytes.extend_from_slice(value);
        Ok(())
    }
}

impl CapturedHeader {
    fn wipe(&mut self) {
        self.bytes.fill(0);
        self.bytes.clear();
    }
}

/// Bind an opaque credential to the already-qualified probe transport sink.
pub fn inject_probe_auth<C: Cancellation>(
    request: AuthenticatedRequest,
    endpoint: &str,
    current_generation: impl Fn() -> u64,
    now_ms: u64,
    credential: ResolvedCredential,
    sink: &mut impl HeaderSink,
    cancelled: C,
) -> Result<(), AuthRequestError> {
    request.inject(
        endpoint,
        current_generation,
        now_ms,
        credential,
        sink,
        cancelled,
    )
}

/// Bounded, endpoint-pinned probe request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeRequest {
    /// Provider protocol family.
    pub provider: ProbeProvider,
    /// Credential-free endpoint identity digest.
    pub endpoint_identity_sha256: String,
    /// Enrollment generation being probed.
    pub generation: u64,
    /// End-to-end deadline.
    pub timeout_ms: u64,
    /// Maximum response body the transport may buffer.
    pub max_response_bytes: usize,
}

impl ProbeRequest {
    /// Validate bounds before a transport is opened.
    pub fn validate(&self) -> Result<(), ProbeRequestError> {
        if self.endpoint_identity_sha256.len() != 64
            || !self
                .endpoint_identity_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ProbeRequestError::InvalidEndpointIdentity);
        }
        if self.generation == 0 {
            return Err(ProbeRequestError::InvalidGeneration);
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_PROBE_TIMEOUT_MS {
            return Err(ProbeRequestError::InvalidTimeout);
        }
        if self.max_response_bytes == 0 || self.max_response_bytes > MAX_PROBE_BODY_BYTES {
            return Err(ProbeRequestError::InvalidResponseLimit);
        }
        Ok(())
    }
}

/// Probe request validation failure without endpoint or provider disclosure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeRequestError {
    /// Endpoint identity is not a lowercase SHA-256 digest.
    InvalidEndpointIdentity,
    /// Generation zero is never a valid enrollment.
    InvalidGeneration,
    /// Deadline is outside the bounded range.
    InvalidTimeout,
    /// Response limit is outside the bounded range.
    InvalidResponseLimit,
}

/// Provider protocol family whose authentication response is being classified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeProvider {
    /// OpenAI-compatible JSON endpoint.
    OpenAi,
    /// Gemini JSON endpoint.
    Gemini,
    /// Ollama local API endpoint.
    Ollama,
}

/// Public, body-free probe outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeOutcome {
    /// Authentication and protocol identity were accepted.
    Connected,
    /// The provider explicitly rejected authentication.
    Rejected,
    /// The credential is expired or no longer usable.
    Expired,
    /// The endpoint or qualified transport was unavailable.
    Unavailable,
}

/// Classify one already-bounded HTTP probe response.
///
/// Callers must perform endpoint pinning, TLS/transport policy, timeout and
/// response-size enforcement before invoking this function. The response body
/// is represented only by its length, so no provider text can enter evidence.
pub fn classify_probe(
    provider: ProbeProvider,
    status: u16,
    body_len: usize,
    content_type_json: bool,
) -> ProbeOutcome {
    if body_len == 0 || body_len > MAX_PROBE_BODY_BYTES || !content_type_json {
        return ProbeOutcome::Unavailable;
    }
    match provider {
        ProbeProvider::OpenAi | ProbeProvider::Gemini => match status {
            200..=299 => ProbeOutcome::Connected,
            401 | 403 => ProbeOutcome::Rejected,
            300..=399 => ProbeOutcome::Unavailable,
            408 | 425 | 429 | 500..=599 => ProbeOutcome::Unavailable,
            _ => ProbeOutcome::Rejected,
        },
        ProbeProvider::Ollama => match status {
            200..=299 => ProbeOutcome::Connected,
            401 | 403 => ProbeOutcome::Rejected,
            404 => ProbeOutcome::Unavailable,
            300..=399 => ProbeOutcome::Unavailable,
            408 | 425 | 429 | 500..=599 => ProbeOutcome::Unavailable,
            _ => ProbeOutcome::Rejected,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authenticated_request::{AuthPolicy, AuthProvider, HeaderWriteError};
    use std::net::TcpListener;
    use std::thread;

    struct AuthSink {
        name: Vec<u8>,
        value: Vec<u8>,
    }

    impl HeaderSink for AuthSink {
        fn write_header(
            &mut self,
            name: &[u8],
            prefix: &[u8],
            value: &[u8],
        ) -> Result<(), HeaderWriteError> {
            self.name.extend_from_slice(name);
            self.value.extend_from_slice(prefix);
            self.value.extend_from_slice(value);
            Ok(())
        }
    }

    #[test]
    fn provider_specific_success_and_auth_failures_are_body_free() {
        assert_eq!(
            classify_probe(ProbeProvider::OpenAi, 200, 128, true),
            ProbeOutcome::Connected
        );
        assert_eq!(
            classify_probe(ProbeProvider::Gemini, 401, 12, true),
            ProbeOutcome::Rejected
        );
        assert_eq!(
            classify_probe(ProbeProvider::Ollama, 404, 12, true),
            ProbeOutcome::Unavailable
        );
    }

    #[test]
    fn malformed_or_oversized_responses_fail_closed() {
        assert_eq!(
            classify_probe(ProbeProvider::OpenAi, 200, MAX_PROBE_BODY_BYTES + 1, true),
            ProbeOutcome::Unavailable
        );
        assert_eq!(
            classify_probe(ProbeProvider::Gemini, 200, 0, false),
            ProbeOutcome::Unavailable
        );
        assert_eq!(
            classify_probe(ProbeProvider::OpenAi, 200, 0, true),
            ProbeOutcome::Unavailable
        );
        assert_eq!(
            classify_probe(ProbeProvider::Ollama, 503, 10, true),
            ProbeOutcome::Unavailable
        );
    }

    #[test]
    fn auth_wrapper_applies_provider_policy_at_pinned_endpoint() {
        let endpoint = "http://127.0.0.1:11434/v1/models";
        let mut sink = AuthSink {
            name: Vec::new(),
            value: Vec::new(),
        };
        let request = AuthenticatedRequest {
            provider: AuthProvider::OpenAi,
            endpoint_identity_sha256: crate::authenticated_request::endpoint_identity_sha256(
                endpoint,
            ),
            generation: 4,
            timeout_ms: 30_000,
            deadline_ms: 32_000,
            max_response_bytes: 1_024,
            policy: AuthPolicy::Bearer,
        };
        inject_probe_auth(
            request,
            endpoint,
            || 4,
            2_000,
            crate::credential::ResolvedCredential::from_test(b"token"),
            &mut sink,
            || false,
        )
        .unwrap();
        assert_eq!(sink.name, b"authorization");
        assert_eq!(sink.value, b"Bearer token");
    }

    #[test]
    fn auth_wrapper_rejects_policy_and_endpoint_mismatch_before_sink() {
        let endpoint = "http://127.0.0.1:11434/v1/models";
        let mut sink = AuthSink {
            name: Vec::new(),
            value: Vec::new(),
        };
        let unsupported = AuthenticatedRequest {
            provider: AuthProvider::Gemini,
            endpoint_identity_sha256: crate::authenticated_request::endpoint_identity_sha256(
                endpoint,
            ),
            generation: 4,
            timeout_ms: 5_000,
            deadline_ms: 6_000,
            max_response_bytes: 1_024,
            policy: AuthPolicy::Bearer,
        };
        assert_eq!(
            inject_probe_auth(
                unsupported,
                endpoint,
                || 4,
                10_000,
                crate::credential::ResolvedCredential::from_test(b"token"),
                &mut sink,
                || false,
            ),
            Err(AuthRequestError::UnsupportedPolicy)
        );
        let mismatch = AuthenticatedRequest {
            provider: AuthProvider::OpenAi,
            endpoint_identity_sha256: crate::authenticated_request::endpoint_identity_sha256(
                endpoint,
            ),
            generation: 4,
            timeout_ms: 5_000,
            deadline_ms: 6_000,
            max_response_bytes: 1_024,
            policy: AuthPolicy::Bearer,
        };
        assert_eq!(
            inject_probe_auth(
                mismatch,
                "http://127.0.0.1:11434/v1/chat",
                || 4,
                2_000,
                crate::credential::ResolvedCredential::from_test(b"token"),
                &mut sink,
                || false,
            ),
            Err(AuthRequestError::EndpointIdentityMismatch)
        );
        assert!(sink.name.is_empty() && sink.value.is_empty());
    }

    #[test]
    fn auth_wrapper_uses_gemini_key_and_ollama_no_auth_policies() {
        let endpoint = "http://127.0.0.1:11434/v1/models";
        let metadata = |provider, policy| AuthenticatedRequest {
            provider,
            endpoint_identity_sha256: crate::authenticated_request::endpoint_identity_sha256(
                endpoint,
            ),
            generation: 4,
            timeout_ms: 1_000,
            deadline_ms: 2_000,
            max_response_bytes: 1_024,
            policy,
        };
        let mut sink = AuthSink {
            name: Vec::new(),
            value: Vec::new(),
        };
        inject_probe_auth(
            metadata(AuthProvider::Gemini, AuthPolicy::ApiKey),
            endpoint,
            || 4,
            1_500,
            crate::credential::ResolvedCredential::from_test(b"key"),
            &mut sink,
            || false,
        )
        .unwrap();
        assert_eq!(sink.name, b"x-api-key");
        assert_eq!(sink.value, b"key");
        sink.name.clear();
        sink.value.clear();
        inject_probe_auth(
            metadata(AuthProvider::Ollama, AuthPolicy::None),
            endpoint,
            || 4,
            1_500,
            crate::credential::ResolvedCredential::from_test(b""),
            &mut sink,
            || false,
        )
        .unwrap();
        assert!(sink.name.is_empty() && sink.value.is_empty());
    }

    #[test]
    fn authenticated_probe_fails_closed_on_cancel_and_expired_deadline() {
        let endpoint = "http://127.0.0.1:11434/v1/models";
        let probe = ProbeRequest {
            provider: ProbeProvider::OpenAi,
            endpoint_identity_sha256: crate::authenticated_request::endpoint_identity_sha256(
                endpoint,
            ),
            generation: 4,
            timeout_ms: 1_000,
            max_response_bytes: 1_024,
        };
        let request = || AuthenticatedRequest {
            provider: AuthProvider::OpenAi,
            endpoint_identity_sha256: probe.endpoint_identity_sha256.clone(),
            generation: 4,
            timeout_ms: 1_000,
            deadline_ms: 2_000,
            max_response_bytes: 1_024,
            policy: AuthPolicy::Bearer,
        };
        assert_eq!(
            execute_authenticated_loopback_probe(
                &probe,
                request(),
                endpoint,
                || 4,
                1_500,
                crate::credential::ResolvedCredential::from_test(b"token"),
                || true,
            ),
            Err(ProbeTransportError::Authentication(
                AuthRequestError::CancelledOrStale
            ))
        );
        assert_eq!(
            execute_authenticated_loopback_probe(
                &probe,
                request(),
                endpoint,
                || 4,
                2_000,
                crate::credential::ResolvedCredential::from_test(b"token"),
                || false,
            ),
            Err(ProbeTransportError::Authentication(
                AuthRequestError::DeadlineExceeded
            ))
        );
    }

    #[test]
    fn authenticated_probe_writes_provider_header_to_pinned_transport() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let endpoint = format!(
            "http://127.0.0.1:{}/v1/models",
            listener.local_addr().unwrap().port()
        );
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 128];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let size = stream.read(&mut chunk).unwrap();
                assert!(size > 0);
                request.extend_from_slice(&chunk[..size]);
                assert!(request.len() <= 512);
            }
            assert!(
                request
                    .windows(b"authorization: Bearer token".len())
                    .any(|window| window == b"authorization: Bearer token")
            );
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                )
                .unwrap();
        });
        let probe = ProbeRequest {
            provider: ProbeProvider::OpenAi,
            endpoint_identity_sha256: crate::authenticated_request::endpoint_identity_sha256(
                &endpoint,
            ),
            generation: 4,
            timeout_ms: 30_000,
            max_response_bytes: 1_024,
        };
        let auth = AuthenticatedRequest {
            provider: AuthProvider::OpenAi,
            endpoint_identity_sha256: probe.endpoint_identity_sha256.clone(),
            generation: 4,
            timeout_ms: 30_000,
            deadline_ms: 40_000,
            max_response_bytes: 1_024,
            policy: AuthPolicy::Bearer,
        };
        assert_eq!(
            execute_authenticated_loopback_probe(
                &probe,
                auth,
                &endpoint,
                || 4,
                10_000,
                crate::credential::ResolvedCredential::from_test(b"token"),
                || false,
            ),
            Ok(ProbeOutcome::Connected)
        );
        server.join().unwrap();
    }

    #[test]
    fn probe_requests_validate_before_transport() {
        let valid = ProbeRequest {
            provider: ProbeProvider::OpenAi,
            endpoint_identity_sha256: "a".repeat(64),
            generation: 1,
            timeout_ms: 1_000,
            max_response_bytes: 4096,
        };
        assert_eq!(valid.validate(), Ok(()));
        let mut invalid = valid.clone();
        invalid.generation = 0;
        assert_eq!(
            invalid.validate(),
            Err(ProbeRequestError::InvalidGeneration)
        );
        invalid = valid.clone();
        invalid.timeout_ms = MAX_PROBE_TIMEOUT_MS + 1;
        assert_eq!(invalid.validate(), Err(ProbeRequestError::InvalidTimeout));
        invalid = valid.clone();
        invalid.max_response_bytes = MAX_PROBE_BODY_BYTES + 1;
        assert_eq!(
            invalid.validate(),
            Err(ProbeRequestError::InvalidResponseLimit)
        );
    }

    #[test]
    fn loopback_transport_is_pinned_bounded_and_generation_checked() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let endpoint = format!(
            "http://127.0.0.1:{}/health",
            listener.local_addr().unwrap().port()
        );
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            std::io::Write::write_all(
                &mut stream,
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
            )
            .unwrap();
            stream.shutdown(std::net::Shutdown::Write).unwrap();
        });
        let request = ProbeRequest {
            provider: ProbeProvider::OpenAi,
            endpoint_identity_sha256: format!("{:x}", Sha256::digest(endpoint.as_bytes())),
            generation: 7,
            timeout_ms: 1_000,
            max_response_bytes: 4096,
        };
        assert_eq!(
            execute_loopback_probe(&request, &endpoint, || 7),
            Ok(ProbeOutcome::Connected)
        );
        server.join().unwrap();
        assert_eq!(
            execute_loopback_probe(&request, &endpoint, || 8),
            Err(ProbeTransportError::StaleGeneration)
        );
    }

    #[test]
    fn loopback_transport_rejects_redirects_and_non_loopback_endpoints() {
        let endpoint = "http://127.0.0.1:1/health";
        let request = ProbeRequest {
            provider: ProbeProvider::Gemini,
            endpoint_identity_sha256: format!("{:x}", Sha256::digest(endpoint.as_bytes())),
            generation: 1,
            timeout_ms: 100,
            max_response_bytes: 1024,
        };
        assert_eq!(
            execute_loopback_probe(&request, endpoint, || 1),
            Err(ProbeTransportError::Unavailable)
        );
        let external = "http://192.0.2.1:80/health";
        let external_request = ProbeRequest {
            endpoint_identity_sha256: format!("{:x}", Sha256::digest(external.as_bytes())),
            ..request
        };
        assert_eq!(
            execute_loopback_probe(&external_request, external, || 1),
            Err(ProbeTransportError::UnqualifiedTransport)
        );
    }
}
