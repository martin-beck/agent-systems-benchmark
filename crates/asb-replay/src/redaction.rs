// SPDX-License-Identifier: MIT
//! Explicit, versioned redaction before cassette serialization.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use thiserror::Error;
use url::Url;

use crate::{
    CassetteContents, Header, RecordedRequest, RecordedResponse, RedactedCassetteContents,
    ResponseBody, cassette::canonical_value_digest,
};

/// Redaction policy understood by this implementation.
pub const DEFAULT_REDACTION_POLICY_VERSION: u16 = 1;
/// Marker namespace reserved for output created by the redactor.
const MARKER_PREFIX: &str = "[ASB_REDACTED:";
/// Maximum configurable selectors in one policy.
const MAX_SELECTORS: usize = 256;
/// Maximum distinct sensitive values replaced in one redactor.
const MAX_MAPPINGS: usize = 4096;

/// Fields that must be removed before a request or response can be persisted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedactionPolicy {
    /// Policy format version.
    pub version: u16,
    /// Case-insensitive request and response header names.
    pub header_names: BTreeSet<String>,
    /// Decoded URL query parameter names.
    pub query_parameters: BTreeSet<String>,
    /// RFC 6901 pointers into the request JSON body.
    pub request_body_pointers: BTreeSet<String>,
    /// RFC 6901 pointers into buffered response JSON or event payloads.
    pub response_body_pointers: BTreeSet<String>,
}

impl Default for RedactionPolicy {
    fn default() -> Self {
        Self {
            version: DEFAULT_REDACTION_POLICY_VERSION,
            header_names: [
                "authorization",
                "cookie",
                "proxy-authorization",
                "set-cookie",
                "x-api-key",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            query_parameters: ["access_token", "api_key", "key", "token"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            request_body_pointers: BTreeSet::new(),
            response_body_pointers: BTreeSet::new(),
        }
    }
}

impl RedactionPolicy {
    fn validate(&self) -> Result<(), RedactionError> {
        if self.version != DEFAULT_REDACTION_POLICY_VERSION {
            return Err(RedactionError::UnsupportedVersion);
        }
        let count = self
            .header_names
            .len()
            .checked_add(self.query_parameters.len())
            .and_then(|count| count.checked_add(self.request_body_pointers.len()))
            .and_then(|count| count.checked_add(self.response_body_pointers.len()))
            .ok_or(RedactionError::LimitExceeded)?;
        if count > MAX_SELECTORS
            || self.header_names.iter().any(|name| {
                name.is_empty()
                    || name != &name.to_ascii_lowercase()
                    || name.bytes().any(|byte| {
                        !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                    })
            })
            || self.query_parameters.iter().any(String::is_empty)
            || self
                .request_body_pointers
                .iter()
                .chain(&self.response_body_pointers)
                .any(|pointer| !valid_pointer(pointer))
        {
            return Err(RedactionError::InvalidPolicy);
        }
        Ok(())
    }
}

/// Non-sensitive evidence describing a completed redaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RedactionReport {
    /// Applied policy version.
    pub policy_version: u16,
    /// Number of field occurrences replaced.
    pub replacements: u32,
    /// Number of distinct values mapped to distinct placeholders.
    pub distinct_values: u32,
}

/// Stateful mapping that keeps equal values stable and distinct values separate.
#[derive(Debug)]
pub struct Redactor {
    policy: RedactionPolicy,
    mappings: BTreeMap<String, String>,
    replacements: u32,
}

impl Redactor {
    /// Validate a policy and create an empty ephemeral mapping.
    pub fn new(policy: RedactionPolicy) -> Result<Self, RedactionError> {
        policy.validate()?;
        Ok(Self {
            policy,
            mappings: BTreeMap::new(),
            replacements: 0,
        })
    }

    /// Redact every interaction and return the only value accepted by sealing.
    pub fn redact_contents(
        mut self,
        mut contents: CassetteContents,
    ) -> Result<(RedactedCassetteContents, RedactionReport), RedactionError> {
        contents.redaction.version = self.policy.version;
        for interaction in &mut contents.interactions {
            self.redact_request(&mut interaction.request)?;
            self.redact_response(&mut interaction.response)?;
        }
        let report = self.report()?;
        Ok((RedactedCassetteContents(contents), report))
    }

    /// Redact a request in memory before it is sealed.
    pub fn redact_request(
        &mut self,
        request: &mut RecordedRequest,
    ) -> Result<RedactionReport, RedactionError> {
        reject_marker_injection(request)?;
        redact_headers(
            &self.policy,
            &mut self.mappings,
            &mut self.replacements,
            &mut request.headers,
        )?;
        request.path = redact_path(
            &self.policy,
            &mut self.mappings,
            &mut self.replacements,
            &request.path,
        )?;
        let pointers: Vec<String> = self.policy.request_body_pointers.iter().cloned().collect();
        for pointer in pointers {
            redact_pointer(
                &mut self.mappings,
                &mut self.replacements,
                &mut request.body,
                &pointer,
            )?;
        }
        request.body_sha256 =
            canonical_value_digest(&request.body).map_err(|_| RedactionError::Serialization)?;
        self.report()
    }

    /// Redact response headers and configured structured payload fields.
    pub fn redact_response(
        &mut self,
        response: &mut RecordedResponse,
    ) -> Result<RedactionReport, RedactionError> {
        reject_marker_injection(response)?;
        redact_headers(
            &self.policy,
            &mut self.mappings,
            &mut self.replacements,
            &mut response.headers,
        )?;
        let pointers: Vec<String> = self.policy.response_body_pointers.iter().cloned().collect();
        match &mut response.body {
            ResponseBody::Buffered {
                payload_sha256,
                payload,
                ..
            } => {
                for pointer in pointers {
                    redact_pointer(
                        &mut self.mappings,
                        &mut self.replacements,
                        payload,
                        &pointer,
                    )?;
                }
                *payload_sha256 =
                    canonical_value_digest(payload).map_err(|_| RedactionError::Serialization)?;
            }
            ResponseBody::Events { events, .. } => {
                for event in events {
                    for pointer in &pointers {
                        redact_pointer(
                            &mut self.mappings,
                            &mut self.replacements,
                            &mut event.payload,
                            pointer,
                        )?;
                    }
                    event.payload_sha256 = canonical_value_digest(&event.payload)
                        .map_err(|_| RedactionError::Serialization)?;
                }
            }
        }
        self.report()
    }

    fn report(&self) -> Result<RedactionReport, RedactionError> {
        Ok(RedactionReport {
            policy_version: self.policy.version,
            replacements: self.replacements,
            distinct_values: u32::try_from(self.mappings.len())
                .map_err(|_| RedactionError::LimitExceeded)?,
        })
    }
}

/// Redaction errors never include captured values.
#[derive(Debug, Error)]
pub enum RedactionError {
    /// The policy version is unknown.
    #[error("unsupported redaction policy version")]
    UnsupportedVersion,
    /// A selector is malformed, ambiguous, or exceeds policy.
    #[error("invalid redaction policy")]
    InvalidPolicy,
    /// A configured field did not exist or could not be selected safely.
    #[error("configured sensitive field is absent or ambiguous")]
    MissingSensitiveField,
    /// Sensitive content tried to impersonate a generated placeholder.
    #[error("reserved redaction marker occurred in input")]
    MarkerInjection,
    /// A URL was not a normalized origin-form request target.
    #[error("request target is not safely redaction-compatible")]
    InvalidRequestTarget,
    /// A finite redaction bound was exceeded.
    #[error("redaction limit exceeded")]
    LimitExceeded,
    /// Serialization used only for marker detection failed.
    #[error("redaction input is not serializable")]
    Serialization,
}

fn reject_marker_injection<T: serde::Serialize>(value: &T) -> Result<(), RedactionError> {
    let bytes = serde_json::to_vec(value).map_err(|_| RedactionError::Serialization)?;
    if bytes
        .windows(MARKER_PREFIX.len())
        .any(|window| window == MARKER_PREFIX.as_bytes())
    {
        return Err(RedactionError::MarkerInjection);
    }
    Ok(())
}

fn redact_headers(
    policy: &RedactionPolicy,
    mappings: &mut BTreeMap<String, String>,
    replacements: &mut u32,
    headers: &mut [Header],
) -> Result<(), RedactionError> {
    headers.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    let mut previous: Option<String> = None;
    for header in headers {
        header.name.make_ascii_lowercase();
        if previous.as_deref() == Some(&header.name) {
            return Err(RedactionError::MissingSensitiveField);
        }
        previous = Some(header.name.clone());
        if policy.header_names.contains(&header.name) {
            header.value = placeholder(mappings, replacements, &header.value)?;
        }
    }
    Ok(())
}

fn redact_path(
    policy: &RedactionPolicy,
    mappings: &mut BTreeMap<String, String>,
    replacements: &mut u32,
    path: &str,
) -> Result<String, RedactionError> {
    if !path.starts_with('/') || path.starts_with("//") || path.contains("%25") {
        return Err(RedactionError::InvalidRequestTarget);
    }
    let mut url = Url::parse(&format!("https://asb.invalid{path}"))
        .map_err(|_| RedactionError::InvalidRequestTarget)?;
    let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    if !pairs.is_empty() {
        url.set_query(None);
        let mut query = url.query_pairs_mut();
        for (name, value) in pairs {
            if policy.query_parameters.contains(&name) {
                query.append_pair(&name, &placeholder(mappings, replacements, &value)?);
            } else {
                query.append_pair(&name, &value);
            }
        }
    }
    let mut output = url.path().to_owned();
    if let Some(query) = url.query() {
        output.push('?');
        output.push_str(query);
    }
    Ok(output)
}

fn redact_pointer(
    mappings: &mut BTreeMap<String, String>,
    replacements: &mut u32,
    body: &mut Value,
    pointer: &str,
) -> Result<(), RedactionError> {
    let selected = body
        .pointer_mut(pointer)
        .ok_or(RedactionError::MissingSensitiveField)?;
    let key = serde_json::to_string(selected).map_err(|_| RedactionError::Serialization)?;
    *selected = Value::String(placeholder(mappings, replacements, &key)?);
    Ok(())
}

fn placeholder(
    mappings: &mut BTreeMap<String, String>,
    replacements: &mut u32,
    value: &str,
) -> Result<String, RedactionError> {
    *replacements = replacements
        .checked_add(1)
        .ok_or(RedactionError::LimitExceeded)?;
    if let Some(existing) = mappings.get(value) {
        return Ok(existing.clone());
    }
    if mappings.len() == MAX_MAPPINGS {
        return Err(RedactionError::LimitExceeded);
    }
    let ordinal = mappings.len() + 1;
    let marker = format!("{MARKER_PREFIX}{ordinal:06}]");
    mappings.insert(value.to_owned(), marker.clone());
    Ok(marker)
}

fn valid_pointer(pointer: &str) -> bool {
    pointer.starts_with('/')
        && !pointer.contains("//")
        && pointer.split('/').skip(1).all(|token| {
            let mut chars = token.chars();
            while let Some(character) = chars.next() {
                if character == '~' && !matches!(chars.next(), Some('0' | '1')) {
                    return false;
                }
            }
            !token.is_empty()
        })
}
