// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Explicit, versioned redaction before cassette serialization.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use url::Url;

use crate::{
    CassetteContents, Header, RecordedRequest, RecordedResponse, RedactedCassetteContents,
    RedactionDescriptor, RedactionSelectors, RequestBodyRedactionRule, ResponseBody,
    cassette::{
        canonical_json_bytes, canonical_value_digest, valid_header_name, valid_header_value,
        valid_origin_form,
    },
};

/// Redaction policy understood by this implementation.
pub const DEFAULT_REDACTION_POLICY_VERSION: u16 = 1;
/// Interaction-scoped request redaction policy generation.
pub const INTERACTION_REDACTION_POLICY_VERSION: u16 = 2;
/// Marker namespace reserved for output created by the redactor.
pub(crate) const MARKER_PREFIX: &str = "[ASB_REDACTED:";
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
    /// V2 selectors bound to exact interaction identities and methods.
    pub request_body_rules: Vec<RequestBodyRedactionRule>,
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
            request_body_rules: Vec::new(),
            response_body_pointers: BTreeSet::new(),
        }
    }
}

impl RedactionPolicy {
    fn validate(&self) -> Result<(), RedactionError> {
        if !matches!(
            self.version,
            DEFAULT_REDACTION_POLICY_VERSION | INTERACTION_REDACTION_POLICY_VERSION
        ) {
            return Err(RedactionError::UnsupportedVersion);
        }
        let count = self
            .header_names
            .len()
            .checked_add(self.query_parameters.len())
            .and_then(|count| count.checked_add(self.request_body_pointers.len()))
            .and_then(|count| {
                self.request_body_rules
                    .iter()
                    .try_fold(count, |count, rule| count.checked_add(rule.pointers.len()))
            })
            .and_then(|count| count.checked_add(self.response_body_pointers.len()))
            .ok_or(RedactionError::LimitExceeded)?;
        let version_shape_valid = match self.version {
            DEFAULT_REDACTION_POLICY_VERSION => self.request_body_rules.is_empty(),
            INTERACTION_REDACTION_POLICY_VERSION => {
                self.request_body_pointers.is_empty() && !self.request_body_rules.is_empty()
            }
            _ => false,
        };
        let mut previous_rule: Option<(&str, &str)> = None;
        let rules_valid = self.request_body_rules.iter().all(|rule| {
            let key = (rule.interaction_id.as_str(), rule.method.as_str());
            let ordered = previous_rule.is_none_or(|previous| previous < key);
            previous_rule = Some(key);
            ordered
                && valid_interaction_id(&rule.interaction_id)
                && rule.method == "POST"
                && !rule.pointers.is_empty()
                && rule.pointers.len() <= MAX_SELECTORS
                && rule.pointers.windows(2).all(|pair| pair[0] < pair[1])
                && rule.pointers.iter().all(|pointer| valid_pointer(pointer))
        });
        if count > MAX_SELECTORS
            || !version_shape_valid
            || !rules_valid
            || self.header_names.iter().any(|name| {
                name.is_empty() || name != &name.to_ascii_lowercase() || !valid_header_name(name)
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

    /// Produce the complete deterministic descriptor persisted in a cassette.
    pub fn descriptor(&self) -> Result<RedactionDescriptor, RedactionError> {
        self.validate()?;
        descriptor_unchecked(self)
    }
}

/// Non-sensitive evidence describing a completed redaction.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
    mappings: BTreeMap<SensitiveValueKey, String>,
    replacements: u32,
}

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd)]
enum SensitiveValueKey {
    Text(String),
    Json(Vec<u8>),
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
        contents.redaction = self.policy.descriptor()?;
        for interaction in &mut contents.interactions {
            let pointers = request_body_pointers_for_descriptor(
                &contents.redaction,
                &interaction.interaction_id,
                &interaction.request.method,
            )
            .ok_or(RedactionError::InvalidPolicy)?
            .to_vec();
            self.redact_request_with_pointers(&mut interaction.request, &pointers)?;
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
        if !self.policy.request_body_rules.is_empty() {
            return Err(RedactionError::InteractionContextRequired);
        }
        let pointers: Vec<String> = self.policy.request_body_pointers.iter().cloned().collect();
        self.redact_request_with_pointers(request, &pointers)
    }

    fn redact_request_with_pointers(
        &mut self,
        request: &mut RecordedRequest,
        pointers: &[String],
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
        for pointer in pointers {
            redact_pointer(
                &mut self.mappings,
                &mut self.replacements,
                &mut request.body,
                pointer,
            )?;
        }
        synchronize_request_options(request, pointers);
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

fn synchronize_request_options(request: &mut RecordedRequest, pointers: &[String]) {
    let Some(body) = request.body.as_object() else {
        return;
    };
    for (name, value) in &mut request.options {
        let escaped = name.replace('~', "~0").replace('/', "~1");
        let selected = format!("/{escaped}");
        if pointers.iter().any(|pointer| {
            pointer == &selected
                || pointer
                    .strip_prefix(&selected)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        }) && let Some(body_value) = body.get(name)
        {
            *value = body_value.clone();
        }
    }
}

pub(crate) fn contains_marker(value: &Value) -> bool {
    serde_json::to_vec(value).is_ok_and(|bytes| {
        bytes
            .windows(MARKER_PREFIX.len())
            .any(|window| window == MARKER_PREFIX.as_bytes())
    })
}

pub(crate) fn valid_marker(value: &Value) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    let Some(ordinal) = text
        .strip_prefix(MARKER_PREFIX)
        .and_then(|suffix| suffix.strip_suffix(']'))
    else {
        return false;
    };
    ordinal.len() == 6
        && ordinal.bytes().all(|byte| byte.is_ascii_digit())
        && ordinal
            .parse::<usize>()
            .is_ok_and(|ordinal| (1..=MAX_MAPPINGS).contains(&ordinal))
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
    /// Interaction-scoped policy was used without an interaction identity.
    #[error("interaction context is required by redaction policy")]
    InteractionContextRequired,
    /// A header contains control characters unsafe for persistence or replay.
    #[error("header value contains a forbidden control character")]
    InvalidHeaderValue,
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

fn descriptor_unchecked(policy: &RedactionPolicy) -> Result<RedactionDescriptor, RedactionError> {
    let selectors = RedactionSelectors {
        header_names: policy.header_names.iter().cloned().collect(),
        query_parameters: policy.query_parameters.iter().cloned().collect(),
        request_body_pointers: policy.request_body_pointers.iter().cloned().collect(),
        request_body_rules: policy.request_body_rules.clone(),
        response_body_pointers: policy.response_body_pointers.iter().cloned().collect(),
    };
    let value = serde_json::to_value(&selectors).map_err(|_| RedactionError::Serialization)?;
    let selector_sha256 =
        canonical_value_digest(&value).map_err(|_| RedactionError::Serialization)?;
    Ok(RedactionDescriptor {
        version: policy.version,
        selectors,
        selector_sha256,
    })
}

pub(crate) fn validate_redaction_descriptor(descriptor: &RedactionDescriptor) -> bool {
    let selectors = &descriptor.selectors;
    let policy = RedactionPolicy {
        version: descriptor.version,
        header_names: selectors.header_names.iter().cloned().collect(),
        query_parameters: selectors.query_parameters.iter().cloned().collect(),
        request_body_pointers: selectors.request_body_pointers.iter().cloned().collect(),
        request_body_rules: selectors.request_body_rules.clone(),
        response_body_pointers: selectors.response_body_pointers.iter().cloned().collect(),
    };
    let no_duplicates = policy.header_names.len() == selectors.header_names.len()
        && policy.query_parameters.len() == selectors.query_parameters.len()
        && policy.request_body_pointers.len() == selectors.request_body_pointers.len()
        && policy.response_body_pointers.len() == selectors.response_body_pointers.len();
    no_duplicates && descriptor_unchecked(&policy).is_ok_and(|expected| expected == *descriptor)
}

pub(crate) fn request_body_pointers_for_descriptor<'a>(
    descriptor: &'a RedactionDescriptor,
    interaction_id: &str,
    method: &str,
) -> Option<&'a [String]> {
    match descriptor.version {
        DEFAULT_REDACTION_POLICY_VERSION => Some(&descriptor.selectors.request_body_pointers),
        INTERACTION_REDACTION_POLICY_VERSION => descriptor
            .selectors
            .request_body_rules
            .iter()
            .find(|rule| rule.interaction_id == interaction_id && rule.method == method)
            .map_or(Some(&[]), |rule| Some(rule.pointers.as_slice())),
        _ => None,
    }
}

pub(crate) fn validate_interaction_redaction(contents: &CassetteContents) -> bool {
    if contents.redaction.version != INTERACTION_REDACTION_POLICY_VERSION {
        return true;
    }
    contents
        .redaction
        .selectors
        .request_body_rules
        .iter()
        .all(|rule| {
            contents.interactions.iter().any(|interaction| {
                interaction.interaction_id == rule.interaction_id
                    && interaction.request.method == rule.method
            })
        })
}

fn valid_interaction_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
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
    mappings: &mut BTreeMap<SensitiveValueKey, String>,
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
        if !valid_header_value(&header.value) {
            return Err(RedactionError::InvalidHeaderValue);
        }
        previous = Some(header.name.clone());
        if policy.header_names.contains(&header.name) {
            header.value = placeholder_text(mappings, replacements, &header.value)?;
        }
    }
    Ok(())
}

fn redact_path(
    policy: &RedactionPolicy,
    mappings: &mut BTreeMap<SensitiveValueKey, String>,
    replacements: &mut u32,
    path: &str,
) -> Result<String, RedactionError> {
    if !valid_origin_form(path) || path.contains("%25") {
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
                query.append_pair(&name, &placeholder_text(mappings, replacements, &value)?);
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
    mappings: &mut BTreeMap<SensitiveValueKey, String>,
    replacements: &mut u32,
    body: &mut Value,
    pointer: &str,
) -> Result<(), RedactionError> {
    let selected = body
        .pointer_mut(pointer)
        .ok_or(RedactionError::MissingSensitiveField)?;
    let key = match &*selected {
        Value::String(string) => SensitiveValueKey::Text(string.clone()),
        value => SensitiveValueKey::Json(
            canonical_json_bytes(value).map_err(|_| RedactionError::Serialization)?,
        ),
    };
    *selected = Value::String(placeholder(mappings, replacements, key)?);
    Ok(())
}

fn placeholder_text(
    mappings: &mut BTreeMap<SensitiveValueKey, String>,
    replacements: &mut u32,
    value: &str,
) -> Result<String, RedactionError> {
    placeholder(
        mappings,
        replacements,
        SensitiveValueKey::Text(value.to_owned()),
    )
}

fn placeholder(
    mappings: &mut BTreeMap<SensitiveValueKey, String>,
    replacements: &mut u32,
    value: SensitiveValueKey,
) -> Result<String, RedactionError> {
    *replacements = replacements
        .checked_add(1)
        .ok_or(RedactionError::LimitExceeded)?;
    if let Some(existing) = mappings.get(&value) {
        return Ok(existing.clone());
    }
    if mappings.len() == MAX_MAPPINGS {
        return Err(RedactionError::LimitExceeded);
    }
    let ordinal = mappings.len() + 1;
    let marker = format!("{MARKER_PREFIX}{ordinal:06}]");
    mappings.insert(value, marker.clone());
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
