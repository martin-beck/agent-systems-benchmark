// SPDX-License-Identifier: MIT
//! Cassette integrity, bounds, chunking, and causality tests.

mod support;

use asb_replay::{
    CassetteError, CassetteLimits, RedactionPolicy, Redactor, ResponseBody,
    canonical_contents_bytes, canonical_json_bytes, decode_cassette, decode_cassette_chunks,
    seal_cassette,
};
use sha2::{Digest, Sha256};

#[test]
fn round_trips_across_every_chunk_split() {
    let bytes = seal_cassette(support::redacted_contents(), CassetteLimits::default()).unwrap();
    let expected = decode_cassette(&bytes, CassetteLimits::default()).unwrap();
    for split in 0..=bytes.len() {
        let actual = decode_cassette_chunks(
            [&bytes[..split], &bytes[split..]],
            CassetteLimits::default(),
        )
        .unwrap();
        assert_eq!(actual, expected, "split {split}");
    }
}

#[test]
fn integrity_corruption_and_truncation_fail_closed() {
    let bytes = seal_cassette(support::redacted_contents(), CassetteLimits::default()).unwrap();
    let mut corrupted = bytes.clone();
    let position = corrupted
        .windows("synthetic answer".len())
        .position(|window| window == b"synthetic answer")
        .unwrap();
    corrupted[position] = b'S';
    assert!(matches!(
        decode_cassette(&corrupted, CassetteLimits::default()),
        Err(CassetteError::Integrity)
    ));
    for cut in [0, 1, bytes.len() / 2, bytes.len() - 1] {
        assert!(decode_cassette(&bytes[..cut], CassetteLimits::default()).is_err());
    }
}

#[test]
fn payload_digest_rejects_tampering_even_with_recomputed_root() {
    let bytes = seal_cassette(support::redacted_contents(), CassetteLimits::default()).unwrap();
    let mut envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    envelope["contents"]["interactions"][0]["response"]["body"]["payload"]["output"] =
        "changed synthetic answer".into();
    let changed = authenticate_envelope(&mut envelope);
    assert!(matches!(
        decode_cassette(&changed, CassetteLimits::default()),
        Err(CassetteError::Integrity)
    ));
}

fn authenticate_envelope(envelope: &mut serde_json::Value) -> Vec<u8> {
    let contents: asb_replay::CassetteContents =
        serde_json::from_value(envelope["contents"].clone()).unwrap();
    envelope["integrity"]["digest"] = hex_digest(&Sha256::digest(
        canonical_contents_bytes(&contents).unwrap(),
    ))
    .into();
    serde_json::to_vec(envelope).unwrap()
}

#[test]
fn reordered_nested_objects_have_stable_canonical_bytes_and_digest() {
    let left: serde_json::Value =
        serde_json::from_str(r#"{"z":{"b":2,"a":"é"},"a":[true,null,1.5]}"#).unwrap();
    let right: serde_json::Value =
        serde_json::from_str(r#"{"a":[true,null,1.5],"z":{"a":"é","b":2}}"#).unwrap();
    let expected = r#"{"a":[true,null,1.5],"z":{"a":"é","b":2}}"#.as_bytes();
    let left = canonical_json_bytes(&left).unwrap();
    let right = canonical_json_bytes(&right).unwrap();
    assert_eq!(left, expected);
    assert_eq!(right, expected);
    assert_eq!(Sha256::digest(left), Sha256::digest(right));
}

#[test]
fn authenticated_unsafe_targets_and_header_values_fail_decode() {
    let bytes = seal_cassette(support::redacted_contents(), CassetteLimits::default()).unwrap();
    for path in ["//host/path", "/v1/synthetic#fragment", "/v1\\synthetic"] {
        let mut envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        envelope["contents"]["interactions"][0]["request"]["path"] = path.into();
        assert!(matches!(
            decode_cassette(
                &authenticate_envelope(&mut envelope),
                CassetteLimits::default()
            ),
            Err(CassetteError::NotNormalized)
        ));
    }
    for value in ["line\r\ninjected", "nul\0byte", "tab\tvalue"] {
        let mut envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        envelope["contents"]["interactions"][0]["request"]["headers"][0]["value"] = value.into();
        assert!(matches!(
            decode_cassette(
                &authenticate_envelope(&mut envelope),
                CassetteLimits::default()
            ),
            Err(CassetteError::NotNormalized)
        ));
    }
}

#[test]
fn authenticated_redaction_descriptor_tampering_fails_independently() {
    let bytes = seal_cassette(support::redacted_contents(), CassetteLimits::default()).unwrap();
    let mut envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    envelope["contents"]["redaction"]["selectors"]["query_parameters"][0] =
        "different-selector".into();
    let changed = authenticate_envelope(&mut envelope);
    assert!(matches!(
        decode_cassette(&changed, CassetteLimits::default()),
        Err(CassetteError::InvalidRedactionPolicy)
    ));

    let mut envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    envelope["contents"]["redaction"]["selector_sha256"] = "0".repeat(64).into();
    let changed = authenticate_envelope(&mut envelope);
    assert!(matches!(
        decode_cassette(&changed, CassetteLimits::default()),
        Err(CassetteError::InvalidRedactionPolicy)
    ));
}

#[test]
fn unknown_versions_and_fields_are_rejected() {
    let bytes = seal_cassette(support::redacted_contents(), CassetteLimits::default()).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["contents"]["schema_version"] = 2.into();
    let unknown_version = serde_json::to_vec(&value).unwrap();
    assert!(matches!(
        decode_cassette(&unknown_version, CassetteLimits::default()),
        Err(CassetteError::UnsupportedVersion { .. })
    ));
    value["unexpected"] = true.into();
    let unknown_field = serde_json::to_vec(&value).unwrap();
    assert!(matches!(
        decode_cassette(&unknown_field, CassetteLimits::default()),
        Err(CassetteError::InvalidJson(_))
    ));
}

#[test]
fn duplicate_json_members_are_rejected_before_interpretation() {
    let bytes = seal_cassette(support::redacted_contents(), CassetteLimits::default()).unwrap();
    let encoded = String::from_utf8(bytes).unwrap();
    let duplicate = encoded.replacen(
        "{\"contents\":",
        "{\"integrity\":{\"algorithm\":\"sha256\",\"digest\":\"0000000000000000000000000000000000000000000000000000000000000000\"},\"contents\":",
        1,
    );
    assert!(matches!(
        decode_cassette(duplicate.as_bytes(), CassetteLimits::default()),
        Err(CassetteError::InvalidJson(_))
    ));
}

#[test]
fn limits_apply_before_unbounded_chunk_growth() {
    let bytes = seal_cassette(support::redacted_contents(), CassetteLimits::default()).unwrap();
    let limits = CassetteLimits {
        max_cassette_bytes: 32,
        ..CassetteLimits::default()
    };
    assert!(matches!(
        decode_cassette_chunks([bytes.as_slice()], limits),
        Err(CassetteError::TooLarge { kind: "cassette" })
    ));
    let invalid = CassetteLimits {
        max_events: 0,
        ..CassetteLimits::default()
    };
    assert!(matches!(
        seal_cassette(support::redacted_contents(), invalid),
        Err(CassetteError::InvalidLimit { kind: "events", .. })
    ));
}

#[test]
fn every_configured_limit_rejects_zero_and_above_ceiling() {
    let defaults = CassetteLimits::default();
    let cases = [
        (
            CassetteLimits {
                max_cassette_bytes: 0,
                ..defaults
            },
            "cassette bytes",
        ),
        (
            CassetteLimits {
                max_request_bytes: 0,
                ..defaults
            },
            "request bytes",
        ),
        (
            CassetteLimits {
                max_response_bytes: 0,
                ..defaults
            },
            "response bytes",
        ),
        (
            CassetteLimits {
                max_event_bytes: 0,
                ..defaults
            },
            "event bytes",
        ),
        (
            CassetteLimits {
                max_interactions: 0,
                ..defaults
            },
            "interactions",
        ),
        (
            CassetteLimits {
                max_headers: 0,
                ..defaults
            },
            "headers",
        ),
        (
            CassetteLimits {
                max_cassette_bytes: defaults.max_cassette_bytes + 1,
                ..defaults
            },
            "cassette bytes",
        ),
        (
            CassetteLimits {
                max_request_bytes: defaults.max_request_bytes + 1,
                ..defaults
            },
            "request bytes",
        ),
        (
            CassetteLimits {
                max_response_bytes: defaults.max_response_bytes + 1,
                ..defaults
            },
            "response bytes",
        ),
        (
            CassetteLimits {
                max_event_bytes: defaults.max_event_bytes + 1,
                ..defaults
            },
            "event bytes",
        ),
        (
            CassetteLimits {
                max_events: defaults.max_events + 1,
                ..defaults
            },
            "events",
        ),
        (
            CassetteLimits {
                max_interactions: defaults.max_interactions + 1,
                ..defaults
            },
            "interactions",
        ),
        (
            CassetteLimits {
                max_headers: defaults.max_headers + 1,
                ..defaults
            },
            "headers",
        ),
    ];
    for (limits, kind) in cases {
        assert!(matches!(
            limits.validate(),
            Err(CassetteError::InvalidLimit { kind: actual, .. }) if actual == kind
        ));
    }
}

#[test]
fn malformed_integrity_and_trailing_json_fail_closed() {
    let bytes = seal_cassette(support::redacted_contents(), CassetteLimits::default()).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for digest in ["0".to_owned(), "A".repeat(64)] {
        value["integrity"]["digest"] = digest.into();
        assert!(matches!(
            decode_cassette(
                &serde_json::to_vec(&value).unwrap(),
                CassetteLimits::default()
            ),
            Err(CassetteError::Integrity)
        ));
    }
    value["integrity"]["algorithm"] = "sha512".into();
    assert!(matches!(
        decode_cassette(
            &serde_json::to_vec(&value).unwrap(),
            CassetteLimits::default()
        ),
        Err(CassetteError::Integrity)
    ));
    let mut trailing = bytes;
    trailing.extend_from_slice(b" null");
    assert!(matches!(
        decode_cassette(&trailing, CassetteLimits::default()),
        Err(CassetteError::InvalidJson(_))
    ));
}

#[test]
fn policy_identity_request_and_response_invariants_fail_closed() {
    let mut normalization = support::contents();
    normalization.normalization.version = 2;
    let normalization = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(normalization)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(normalization, CassetteLimits::default()),
        Err(CassetteError::UnsupportedVersion {
            kind: "normalization policy",
            ..
        })
    ));

    let bytes = seal_cassette(support::redacted_contents(), CassetteLimits::default()).unwrap();
    let mut redaction: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    redaction["contents"]["redaction"]["version"] = 2.into();
    assert!(matches!(
        decode_cassette(
            &serde_json::to_vec(&redaction).unwrap(),
            CassetteLimits::default()
        ),
        Err(CassetteError::UnsupportedVersion {
            kind: "redaction policy",
            ..
        })
    ));

    let mut duplicate = support::contents();
    duplicate.interactions[1].interaction_id = duplicate.interactions[0].interaction_id.clone();
    let duplicate = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(duplicate)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(duplicate, CassetteLimits::default()),
        Err(CassetteError::InvalidIdentity)
    ));

    let mut identity = support::contents();
    identity.interactions[0].session_id.clear();
    let identity = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(identity)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(identity, CassetteLimits::default()),
        Err(CassetteError::InvalidIdentity)
    ));

    let mut method = support::contents();
    method.interactions[0].request.method = "post".into();
    let method = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(method)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(method, CassetteLimits::default()),
        Err(CassetteError::NotNormalized)
    ));

    let mut status = support::contents();
    status.interactions[0].response.status = 99;
    let status = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(status)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(status, CassetteLimits::default()),
        Err(CassetteError::InvalidEventStream)
    ));

    let bytes = seal_cassette(support::redacted_contents(), CassetteLimits::default()).unwrap();
    let mut headers: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    headers["contents"]["interactions"][0]["request"]["headers"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"name": "content-type", "value": "duplicate"}));
    assert!(matches!(
        decode_cassette(
            &serde_json::to_vec(&headers).unwrap(),
            CassetteLimits::default()
        ),
        Err(CassetteError::NotNormalized)
    ));
}

#[test]
fn event_payload_and_reference_failures_are_independent() {
    let mut contents = support::contents();
    if let ResponseBody::Events {
        transport_chunk_bytes,
        ..
    } = &mut contents.interactions[1].response.body
    {
        transport_chunk_bytes[0] = 0;
    }
    let redacted = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(contents)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(redacted, CassetteLimits::default()),
        Err(CassetteError::InvalidEventStream)
    ));

    let mut contents = support::contents();
    if let ResponseBody::Events { events, .. } = &mut contents.interactions[1].response.body {
        events[0].previous_response_id = Some("missing".into());
    }
    let redacted = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(contents)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(redacted, CassetteLimits::default()),
        Err(CassetteError::InvalidReference)
    ));

    let mut contents = support::contents();
    if let ResponseBody::Events { events, .. } = &mut contents.interactions[1].response.body {
        events[0].tool_call_id = Some("bad identity!".into());
    }
    let redacted = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(contents)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(redacted, CassetteLimits::default()),
        Err(CassetteError::InvalidIdentity)
    ));
}

#[test]
fn redacted_contents_are_inspectable_without_bypassing_sealing() {
    let contents = support::redacted_contents();
    assert_eq!(contents.as_contents().schema_version, 1);
}

#[test]
fn every_semantic_size_and_count_limit_has_a_negative() {
    let cases = [
        (
            CassetteLimits {
                max_request_bytes: 1,
                ..CassetteLimits::default()
            },
            "request",
        ),
        (
            CassetteLimits {
                max_response_bytes: 1,
                ..CassetteLimits::default()
            },
            "response",
        ),
        (
            CassetteLimits {
                max_event_bytes: 1,
                ..CassetteLimits::default()
            },
            "event",
        ),
        (
            CassetteLimits {
                max_events: 1,
                ..CassetteLimits::default()
            },
            "events",
        ),
        (
            CassetteLimits {
                max_interactions: 1,
                ..CassetteLimits::default()
            },
            "interactions",
        ),
    ];
    for (limits, kind) in cases {
        assert!(matches!(
            seal_cassette(support::redacted_contents(), limits),
            Err(CassetteError::TooLarge { kind: actual }) if actual == kind
        ));
    }

    let mut headers = support::contents();
    headers.interactions[0]
        .request
        .headers
        .push(asb_replay::Header {
            name: "x-synthetic".into(),
            value: "value".into(),
        });
    let headers = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(headers)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(
            headers,
            CassetteLimits {
                max_headers: 1,
                ..CassetteLimits::default()
            },
        ),
        Err(CassetteError::TooLarge { kind: "headers" })
    ));
}

#[test]
fn causal_and_terminal_invariants_are_enforced() {
    let mut dangling = support::contents();
    dangling.interactions[0].request.previous_response_id = Some("missing-response".into());
    let dangling = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(dangling)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(dangling, CassetteLimits::default()),
        Err(CassetteError::InvalidReference)
    ));

    let mut incomplete = support::contents();
    if let asb_replay::ResponseBody::Events { events, .. } =
        &mut incomplete.interactions[1].response.body
    {
        events.last_mut().unwrap().terminal = None;
    }
    let incomplete = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(incomplete)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(incomplete, CassetteLimits::default()),
        Err(CassetteError::InvalidEventStream)
    ));
}

#[test]
fn ordering_normalization_and_duplicate_responses_fail_closed() {
    let mut unordered = support::contents();
    unordered.interactions[1].ordinal = 2;
    let unordered = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(unordered)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(unordered, CassetteLimits::default()),
        Err(CassetteError::InvalidReference)
    ));

    let mut backwards = support::contents();
    if let asb_replay::ResponseBody::Events { events, .. } =
        &mut backwards.interactions[1].response.body
    {
        events[1].monotonic_offset_ns = events[0].monotonic_offset_ns - 1;
    }
    let backwards = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(backwards)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(backwards, CassetteLimits::default()),
        Err(CassetteError::InvalidEventStream)
    ));

    let mut duplicate = support::contents();
    if let asb_replay::ResponseBody::Events { events, .. } =
        &mut duplicate.interactions[1].response.body
    {
        events[0].response_id = Some("response-alpha".into());
    }
    let duplicate = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(duplicate)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(duplicate, CassetteLimits::default()),
        Err(CassetteError::InvalidReference)
    ));
}

#[test]
fn identical_provider_ids_are_scoped_to_session_and_attempt() {
    let mut independent = support::contents();
    let mut clone = independent.interactions[0].clone();
    clone.session_id = "session-beta".into();
    clone.attempt_id = "attempt-beta".into();
    clone.interaction_id = "interaction-beta-0".into();
    independent.interactions.push(clone);
    let independent = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(independent)
        .unwrap()
        .0;
    seal_cassette(independent, CassetteLimits::default()).unwrap();

    let mut cross_scope = support::contents();
    cross_scope.interactions[0].session_id = "session-other".into();
    cross_scope.interactions[0].attempt_id = "attempt-other".into();
    let cross_scope = Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_contents(cross_scope)
        .unwrap()
        .0;
    assert!(matches!(
        seal_cassette(cross_scope, CassetteLimits::default()),
        Err(CassetteError::InvalidReference)
    ));
}

fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(output, "{byte:02x}").unwrap();
    }
    output
}
