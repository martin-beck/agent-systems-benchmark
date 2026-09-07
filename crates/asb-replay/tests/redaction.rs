// SPDX-License-Identifier: MIT
//! Pre-persistence redaction and privacy failure tests.

mod support;

use std::collections::{BTreeMap, BTreeSet};

use asb_replay::{Header, RedactionError, RedactionPolicy, Redactor};
use serde_json::json;

#[test]
fn credentials_are_replaced_stably_without_merging_distinct_values() {
    let policy = RedactionPolicy {
        request_body_pointers: BTreeSet::from(["/credentials/key".into()]),
        ..RedactionPolicy::default()
    };
    let mut redactor = Redactor::new(policy).unwrap();
    let mut request = support::request();
    request.path = "/v1/synthetic?token=synthetic-secret-alpha&mode=test".into();
    request.headers.push(Header {
        name: "Authorization".into(),
        value: "synthetic-secret-alpha".into(),
    });
    request.body = json!({"credentials": {"key": "synthetic-secret-beta"}});

    let report = redactor.redact_request(&mut request).unwrap();
    let encoded = serde_json::to_string(&request).unwrap();
    assert!(!encoded.contains("synthetic-secret-alpha"));
    assert!(!encoded.contains("synthetic-secret-beta"));
    assert_eq!(report.replacements, 3);
    assert_eq!(report.distinct_values, 2);
    assert!(encoded.contains("[ASB_REDACTED:000001]"));
    assert!(request.path.contains("%5BASB_REDACTED%3A000001%5D"));
    assert!(encoded.contains("[ASB_REDACTED:000002]"));
}

#[test]
fn selected_nested_options_are_synchronized_without_touching_unselected_options() {
    let policy = RedactionPolicy {
        request_body_pointers: BTreeSet::from([
            "/client_metadata/session_id".into(),
            "/input".into(),
        ]),
        ..RedactionPolicy::default()
    };
    let mut request = support::request();
    request.body = json!({
        "client_metadata": {"session_id": "volatile-session", "stable": 7},
        "input": [{"role": "user", "content": "private prompt"}],
        "stream": true
    });
    request.options = BTreeMap::from([
        (
            "client_metadata".into(),
            request.body["client_metadata"].clone(),
        ),
        ("stream".into(), json!(false)),
    ]);

    Redactor::new(policy)
        .unwrap()
        .redact_request(&mut request)
        .unwrap();

    assert_eq!(
        request.options["client_metadata"],
        request.body["client_metadata"]
    );
    assert_eq!(request.options["stream"], json!(false));
    let encoded = serde_json::to_vec(&request).unwrap();
    assert!(
        !encoded
            .windows(16)
            .any(|window| window == b"volatile-session")
    );
    assert!(
        !encoded
            .windows(14)
            .any(|window| window == b"private prompt")
    );
}

#[test]
fn equal_cross_location_strings_share_a_marker_without_representation_collisions() {
    let policy = RedactionPolicy {
        request_body_pointers: BTreeSet::from(["/same".into(), "/quoted".into(), "/object".into()]),
        ..RedactionPolicy::default()
    };
    let mut redactor = Redactor::new(policy).unwrap();
    let mut request = support::request();
    request.path = "/v1/synthetic?token=secret".into();
    request.headers.push(Header {
        name: "authorization".into(),
        value: "secret".into(),
    });
    request.body = json!({
        "same": "secret",
        "quoted": "\"secret\"",
        "object": {"value": "secret"}
    });
    let report = redactor.redact_request(&mut request).unwrap();
    assert_eq!(report.replacements, 5);
    assert_eq!(report.distinct_values, 3);
    let encoded = serde_json::to_string(&request).unwrap();
    assert_eq!(encoded.matches("[ASB_REDACTED:000001]").count(), 2);
    assert!(request.path.contains("%5BASB_REDACTED%3A000001%5D"));
    assert!(encoded.contains("[ASB_REDACTED:000002]"));
    assert!(encoded.contains("[ASB_REDACTED:000003]"));
}

#[test]
fn response_headers_and_nested_events_are_redacted() {
    let policy = RedactionPolicy {
        response_body_pointers: BTreeSet::from(["/credentials/key".into()]),
        ..RedactionPolicy::default()
    };
    let mut redactor = Redactor::new(policy).unwrap();
    let mut contents = support::contents();
    let response = &mut contents.interactions[1].response;
    response.headers.push(Header {
        name: "Set-Cookie".into(),
        value: "synthetic-secret-cookie".into(),
    });
    if let asb_replay::ResponseBody::Events { events, .. } = &mut response.body {
        for event in events {
            event.payload = json!({"credentials": {"key": "synthetic-secret-body"}});
        }
    }
    redactor.redact_response(response).unwrap();
    let encoded = serde_json::to_string(response).unwrap();
    assert!(!encoded.contains("synthetic-secret-cookie"));
    assert!(!encoded.contains("synthetic-secret-body"));
}

#[test]
fn marker_injection_and_ambiguous_inputs_fail_closed() {
    let policy = RedactionPolicy::default();
    let mut redactor = Redactor::new(policy.clone()).unwrap();
    let mut injected = support::request();
    injected.body = json!({"value": "[ASB_REDACTED:000001]"});
    assert!(matches!(
        redactor.redact_request(&mut injected),
        Err(RedactionError::MarkerInjection)
    ));

    let mut duplicate = support::request();
    duplicate.headers = vec![
        Header {
            name: "X-API-Key".into(),
            value: "synthetic-a".into(),
        },
        Header {
            name: "x-api-key".into(),
            value: "synthetic-b".into(),
        },
    ];
    let error = Redactor::new(policy)
        .unwrap()
        .redact_request(&mut duplicate)
        .unwrap_err();
    assert!(matches!(error, RedactionError::MissingSensitiveField));
    assert!(!error.to_string().contains("synthetic-"));
}

#[test]
fn encoded_query_and_missing_body_pointer_fail_closed() {
    let mut encoded = support::request();
    encoded.path = "/v1/synthetic?api%255Fkey=synthetic-secret".into();
    assert!(matches!(
        Redactor::new(RedactionPolicy::default())
            .unwrap()
            .redact_request(&mut encoded),
        Err(RedactionError::InvalidRequestTarget)
    ));

    let policy = RedactionPolicy {
        request_body_pointers: BTreeSet::from(["/absent".into()]),
        ..RedactionPolicy::default()
    };
    assert!(matches!(
        Redactor::new(policy)
            .unwrap()
            .redact_request(&mut support::request()),
        Err(RedactionError::MissingSensitiveField)
    ));
}

#[test]
fn fragments_network_paths_and_header_controls_fail_before_sealing() {
    for path in ["//host/path", "/v1/synthetic#fragment", "/v1\\synthetic"] {
        let mut request = support::request();
        request.path = path.into();
        assert!(matches!(
            Redactor::new(RedactionPolicy::default())
                .unwrap()
                .redact_request(&mut request),
            Err(RedactionError::InvalidRequestTarget)
        ));
    }
    for value in ["line\r\ninjected", "nul\0byte", "tab\tvalue"] {
        let mut request = support::request();
        request.headers[0].value = value.into();
        assert!(matches!(
            Redactor::new(RedactionPolicy::default())
                .unwrap()
                .redact_request(&mut request),
            Err(RedactionError::InvalidHeaderValue)
        ));
    }
}

#[test]
fn selector_descriptors_are_complete_deterministic_and_nonsecret() {
    let left = RedactionPolicy {
        request_body_pointers: BTreeSet::from(["/z".into(), "/a".into()]),
        ..RedactionPolicy::default()
    };
    let right = RedactionPolicy {
        request_body_pointers: ["/a", "/z"].into_iter().map(str::to_owned).collect(),
        ..RedactionPolicy::default()
    };
    let left = left.descriptor().unwrap();
    let right = right.descriptor().unwrap();
    assert_eq!(left, right);
    assert_eq!(left.selectors.request_body_pointers, ["/a", "/z"]);
    assert_eq!(left.selector_sha256.len(), 64);
}

#[test]
fn unsupported_or_excessive_redaction_policy_fails_before_data() {
    let unsupported = RedactionPolicy {
        version: 2,
        ..RedactionPolicy::default()
    };
    assert!(matches!(
        Redactor::new(unsupported),
        Err(RedactionError::UnsupportedVersion)
    ));

    let excessive = RedactionPolicy {
        query_parameters: (0..257).map(|index| format!("key-{index}")).collect(),
        ..RedactionPolicy::default()
    };
    assert!(matches!(
        Redactor::new(excessive),
        Err(RedactionError::InvalidPolicy)
    ));
}

#[test]
fn buffered_payload_and_response_header_ambiguity_are_covered() {
    let policy = RedactionPolicy {
        response_body_pointers: BTreeSet::from(["/output".into()]),
        ..RedactionPolicy::default()
    };
    let mut contents = support::contents();
    let response = &mut contents.interactions[0].response;
    let report = Redactor::new(policy)
        .unwrap()
        .redact_response(response)
        .unwrap();
    assert_eq!(report.replacements, 1);
    assert!(
        !serde_json::to_string(response)
            .unwrap()
            .contains("synthetic answer")
    );

    let mut duplicate_contents = support::contents();
    let duplicate = &mut duplicate_contents.interactions[0].response;
    duplicate.headers = vec![
        Header {
            name: "Set-Cookie".into(),
            value: "synthetic-a".into(),
        },
        Header {
            name: "set-cookie".into(),
            value: "synthetic-b".into(),
        },
    ];
    assert!(matches!(
        Redactor::new(RedactionPolicy::default())
            .unwrap()
            .redact_response(duplicate),
        Err(RedactionError::MissingSensitiveField)
    ));
}

#[test]
fn path_without_query_and_invalid_pointer_are_handled() {
    let mut request = support::request();
    request.path = "/v1/synthetic".into();
    Redactor::new(RedactionPolicy::default())
        .unwrap()
        .redact_request(&mut request)
        .unwrap();
    assert_eq!(request.path, "/v1/synthetic");

    let invalid = RedactionPolicy {
        request_body_pointers: BTreeSet::from(["/bad~2pointer".into()]),
        ..RedactionPolicy::default()
    };
    assert!(matches!(
        Redactor::new(invalid),
        Err(RedactionError::InvalidPolicy)
    ));

    let mut request = support::request();
    request.body = json!({"items": ["only"]});
    let out_of_bounds = RedactionPolicy {
        request_body_pointers: BTreeSet::from(["/items/1".into()]),
        ..RedactionPolicy::default()
    };
    assert!(matches!(
        Redactor::new(out_of_bounds)
            .unwrap()
            .redact_request(&mut request),
        Err(RedactionError::MissingSensitiveField)
    ));
}

#[test]
fn mapping_table_exhaustion_fails_closed() {
    let mut redactor = Redactor::new(RedactionPolicy::default()).unwrap();
    for index in 0..4096 {
        let mut request = support::request();
        request.headers.push(Header {
            name: "authorization".into(),
            value: format!("synthetic-secret-{index}"),
        });
        redactor.redact_request(&mut request).unwrap();
    }
    let mut request = support::request();
    request.headers.push(Header {
        name: "authorization".into(),
        value: "synthetic-secret-overflow".into(),
    });
    assert!(matches!(
        redactor.redact_request(&mut request),
        Err(RedactionError::LimitExceeded)
    ));
}
