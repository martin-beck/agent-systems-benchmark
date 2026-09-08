// SPDX-License-Identifier: MIT
//! Boundary, negative, lifecycle, and reconnect conformance.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Cursor, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use asb_control::*;
use serde::Serialize;
use serde_json::{Value, json};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn limits() -> ControlLimits {
    ControlLimits {
        max_frame_bytes: 4096,
        max_timeout_ms: 1000,
        max_page_items: 3,
        max_in_flight: 2,
    }
}

fn negotiate(limits: ControlLimits) -> NegotiateParams {
    NegotiateParams {
        versions: BTreeSet::from([CONTROL_V1]),
        limits,
    }
}

fn request(id: u64, call: ControlCall) -> ControlRequest {
    ControlRequest {
        jsonrpc: JSONRPC_VERSION.into(),
        id: RequestId(id),
        timeout_ms: 500,
        call,
    }
}

fn event(revision: u64) -> ControlEvent {
    ControlEvent {
        revision: Revision(revision),
        kind: ControlEventKind::RunUpdated,
        run_id: Some(RunId("run-1".into())),
        attempt_id: Some(AttemptId("attempt-1".into())),
    }
}

#[test]
fn limits_intersect_and_reject_every_invalid_dimension() {
    let defaults = ControlLimits::default().validate().unwrap();
    assert!(defaults.max_frame_bytes <= MAX_CONTROL_FRAME_BYTES);
    assert_eq!(limits().intersect(defaults).unwrap(), limits());

    for invalid in [
        ControlLimits {
            max_frame_bytes: 0,
            ..limits()
        },
        ControlLimits {
            max_frame_bytes: MAX_CONTROL_FRAME_BYTES + 1,
            ..limits()
        },
        ControlLimits {
            max_timeout_ms: 0,
            ..limits()
        },
        ControlLimits {
            max_timeout_ms: MAX_CONTROL_TIMEOUT_MS + 1,
            ..limits()
        },
        ControlLimits {
            max_page_items: 0,
            ..limits()
        },
        ControlLimits {
            max_page_items: MAX_PAGE_ITEMS + 1,
            ..limits()
        },
        ControlLimits {
            max_in_flight: 0,
            ..limits()
        },
        ControlLimits {
            max_in_flight: MAX_CONTROL_IN_FLIGHT + 1,
            ..limits()
        },
    ] {
        assert!(matches!(
            invalid.validate(),
            Err(ProtocolError::InvalidLimit(_))
        ));
    }
}

#[test]
fn requests_enforce_envelope_deadline_page_and_mutation_keys() {
    for call in [
        ControlCall::Capabilities,
        ControlCall::ValidateSettings {
            settings: json!({}),
        },
        ControlCall::Status {
            run_id: RunId("run".into()),
        },
        ControlCall::Analyze {
            run_ids: vec![RunId("run".into())],
        },
        ControlCall::ArtifactMetadata {
            run_id: RunId("run".into()),
            digest: "a".repeat(64),
        },
    ] {
        validate_request(&request(1, call), limits()).unwrap();
    }
    let mut bad_rpc = request(1, ControlCall::Capabilities);
    bad_rpc.jsonrpc = "1.0".into();
    assert_eq!(
        validate_request(&bad_rpc, limits()),
        Err(ProtocolError::InvalidJsonRpc)
    );
    let mut bad_timeout = request(1, ControlCall::Capabilities);
    bad_timeout.timeout_ms = 0;
    assert_eq!(
        validate_request(&bad_timeout, limits()),
        Err(ProtocolError::InvalidTimeout)
    );
    bad_timeout.timeout_ms = limits().max_timeout_ms + 1;
    assert_eq!(
        validate_request(&bad_timeout, limits()),
        Err(ProtocolError::InvalidTimeout)
    );

    for call in [
        ControlCall::CreatePlan(MutationParams {
            idempotency_key: String::new(),
            definition: json!({}),
        }),
        ControlCall::Launch(LaunchParams {
            idempotency_key: String::new(),
            plan_id: "plan".into(),
        }),
        ControlCall::Cancel(CancelParams {
            run_id: RunId("run".into()),
            attempt_id: AttemptId("attempt".into()),
            idempotency_key: String::new(),
        }),
        ControlCall::Repeat(RepeatParams {
            run_id: RunId("run".into()),
            idempotency_key: String::new(),
        }),
    ] {
        assert_eq!(
            validate_request(&request(1, call), limits()),
            Err(ProtocolError::InvalidIdempotencyKey)
        );
    }
    assert_eq!(
        validate_idempotency_key(&"x".repeat(MAX_IDEMPOTENCY_KEY_BYTES + 1)),
        Err(ProtocolError::InvalidIdempotencyKey)
    );
    validate_idempotency_key("opaque-retry-key").unwrap();
    assert_eq!(
        validate_idempotency_key("line\nbreak"),
        Err(ProtocolError::InvalidIdempotencyKey)
    );
    for invalid in ["", "has space", "slash/value"] {
        assert_eq!(
            validate_identity(invalid),
            Err(ProtocolError::InvalidIdentity)
        );
    }
    assert_eq!(
        validate_identity(&"x".repeat(MAX_CONTROL_ID_BYTES + 1)),
        Err(ProtocolError::InvalidIdentity)
    );
    validate_identity("run:portable_1.example").unwrap();
    for invalid in [
        "",
        "A000000000000000000000000000000000000000000000000000000000000000",
        &"g".repeat(64),
    ] {
        assert_eq!(validate_digest(invalid), Err(ProtocolError::InvalidDigest));
    }
    validate_digest(&"f".repeat(64)).unwrap();

    for invalid_call in [
        ControlCall::Launch(LaunchParams {
            idempotency_key: "key".into(),
            plan_id: String::new(),
        }),
        ControlCall::Status {
            run_id: RunId(String::new()),
        },
        ControlCall::Cancel(CancelParams {
            run_id: RunId("run".into()),
            attempt_id: AttemptId(String::new()),
            idempotency_key: "key".into(),
        }),
        ControlCall::Repeat(RepeatParams {
            run_id: RunId("bad/id".into()),
            idempotency_key: "key".into(),
        }),
        ControlCall::Analyze { run_ids: vec![] },
        ControlCall::ArtifactMetadata {
            run_id: RunId("run".into()),
            digest: "not-a-digest".into(),
        },
    ] {
        assert!(validate_request(&request(4, invalid_call), limits()).is_err());
    }
    assert_eq!(
        validate_request(
            &request(
                5,
                ControlCall::Analyze {
                    run_ids: vec![RunId("run".into()); MAX_ANALYSIS_RUNS + 1]
                }
            ),
            limits()
        ),
        Err(ProtocolError::InvalidAnalysisSet)
    );

    for page in [
        PageParams {
            after: None,
            limit: 0,
        },
        PageParams {
            after: None,
            limit: limits().max_page_items + 1,
        },
    ] {
        assert_eq!(
            validate_page(page, limits()),
            Err(ProtocolError::InvalidPage)
        );
    }
    validate_request(
        &request(
            2,
            ControlCall::History(PageParams {
                after: None,
                limit: 1,
            }),
        ),
        limits(),
    )
    .unwrap();
    validate_request(
        &request(
            3,
            ControlCall::Events(PageParams {
                after: Some(Revision(4)),
                limit: 1,
            }),
        ),
        limits(),
    )
    .unwrap();
}

#[test]
fn responses_have_exactly_one_outcome() {
    ControlResponse::success(
        RequestId(1),
        ControlSuccess::Operation(
            BoundControlResult::new(
                &ControlCall::Cancel(CancelParams {
                    run_id: RunId("run".into()),
                    attempt_id: AttemptId("attempt".into()),
                    idempotency_key: "key".into(),
                }),
                ControlResult::Acknowledged(MutationAcknowledgement { accepted: true }),
            )
            .unwrap(),
        ),
    )
    .validate()
    .unwrap();
    ControlResponse::failure(
        RequestId(2),
        error_code::STALE_CURSOR,
        "reconnect cursor is stale",
    )
    .validate()
    .unwrap();
    for invalid in [
        json!({"jsonrpc":"2.0","id":3}),
        json!({"jsonrpc":"2.0","id":3,"result":null}),
        json!({"jsonrpc":"2.0","id":3,"error":null}),
        json!({
            "jsonrpc":"2.0",
            "id":3,
            "result":{"kind":"negotiated","value":{}},
            "error":{"code":-33006,"message":"reconnect cursor is stale"}
        }),
    ] {
        assert!(serde_json::from_value::<ControlResponse>(invalid).is_err());
    }
    let valid = ControlResponse::success(
        RequestId(5),
        ControlSuccess::Operation(
            BoundControlResult::new(
                &ControlCall::Cancel(CancelParams {
                    run_id: RunId("run".into()),
                    attempt_id: AttemptId("attempt".into()),
                    idempotency_key: "key".into(),
                }),
                ControlResult::Acknowledged(MutationAcknowledgement { accepted: true }),
            )
            .unwrap(),
        ),
    );
    let mut wrong_json = serde_json::to_value(valid).unwrap();
    wrong_json["jsonrpc"] = Value::String("2.1".into());
    let wrong_version: ControlResponse = serde_json::from_value(wrong_json).unwrap();
    assert_eq!(wrong_version.validate(), Err(ProtocolError::InvalidJsonRpc));
}

#[test]
fn frames_round_trip_and_fail_closed() {
    let value = request(7, ControlCall::Capabilities);
    let mut encoded = Vec::new();
    write_frame(&mut encoded, &value, limits()).unwrap();
    let decoded: ControlRequest = read_frame(&mut Cursor::new(encoded), limits()).unwrap();
    assert_eq!(decoded, value);

    assert!(matches!(
        read_frame::<Value>(&mut Cursor::new(0_u32.to_be_bytes()), limits()),
        Err(FrameError::InvalidLength { length: 0, .. })
    ));
    assert!(matches!(
        read_frame::<Value>(
            &mut Cursor::new((limits().max_frame_bytes + 1).to_be_bytes()),
            limits()
        ),
        Err(FrameError::InvalidLength { .. })
    ));
    assert!(matches!(
        read_frame::<Value>(&mut Cursor::new([0_u8, 0]), limits()),
        Err(FrameError::Truncated)
    ));
    let mut short = Vec::from(4_u32.to_be_bytes());
    short.extend_from_slice(b"{}");
    assert!(matches!(
        read_frame::<Value>(&mut Cursor::new(short), limits()),
        Err(FrameError::Truncated)
    ));
    let mut malformed = Vec::from(1_u32.to_be_bytes());
    malformed.push(b'{');
    assert!(matches!(
        read_frame::<Value>(&mut Cursor::new(malformed), limits()),
        Err(FrameError::MalformedJson(_))
    ));
    assert!(matches!(
        read_frame::<Value>(&mut ErrorReader(io::ErrorKind::TimedOut), limits()),
        Err(FrameError::DeadlineExceeded)
    ));
    assert!(matches!(
        read_frame::<Value>(&mut ErrorReader(io::ErrorKind::PermissionDenied), limits()),
        Err(FrameError::Io(_))
    ));
    assert!(matches!(
        write_frame(&mut Vec::new(), &"x".repeat(5000), limits()),
        Err(FrameError::InvalidLength { .. })
    ));
    assert!(matches!(
        write_frame(&mut ErrorWriter, &json!({}), limits()),
        Err(FrameError::Io(_))
    ));
    assert!(matches!(
        write_frame(&mut Vec::new(), &BrokenSerialize, limits()),
        Err(FrameError::MalformedJson(_))
    ));
    assert!(matches!(
        write_frame(
            &mut Vec::new(),
            &json!({}),
            ControlLimits {
                max_frame_bytes: 0,
                ..limits()
            }
        ),
        Err(FrameError::Protocol(_))
    ));
}

#[test]
fn absolute_stream_frames_reject_bad_lengths_truncation_and_expiry() {
    let (mut writer, mut reader) = UnixStream::pair().unwrap();
    writer.write_all(&5000_u32.to_be_bytes()).unwrap();
    assert!(matches!(
        read_frame_until::<Value>(&mut reader, limits(), RequestDeadline::start(100).unwrap()),
        Err(FrameError::InvalidLength { length: 5000, .. })
    ));

    let (mut writer, mut reader) = UnixStream::pair().unwrap();
    writer.write_all(&[0, 0]).unwrap();
    drop(writer);
    assert!(matches!(
        read_frame_until::<Value>(&mut reader, limits(), RequestDeadline::start(100).unwrap()),
        Err(FrameError::Truncated)
    ));

    let (mut writer, mut reader) = UnixStream::pair().unwrap();
    writer.write_all(&4_u32.to_be_bytes()).unwrap();
    writer.write_all(b"nu").unwrap();
    drop(writer);
    assert!(matches!(
        read_frame_until::<Value>(&mut reader, limits(), RequestDeadline::start(100).unwrap()),
        Err(FrameError::Truncated)
    ));

    let (mut writer, _reader) = UnixStream::pair().unwrap();
    assert!(matches!(
        write_frame_until(
            &mut writer,
            &"x".repeat(5000),
            limits(),
            RequestDeadline::start(100).unwrap()
        ),
        Err(FrameError::InvalidLength { .. })
    ));
    let expired = RequestDeadline::start(1).unwrap();
    thread::sleep(std::time::Duration::from_millis(3));
    assert!(matches!(
        write_frame_until(&mut writer, &json!(null), limits(), expired),
        Err(FrameError::DeadlineExceeded)
    ));
}

struct ErrorReader(io::ErrorKind);

impl Read for ErrorReader {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::from(self.0))
    }
}

struct ErrorWriter;

impl Write for ErrorWriter {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct BrokenSerialize;

impl Serialize for BrokenSerialize {
    fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom(
            "controlled serialization failure",
        ))
    }
}

#[test]
fn sessions_require_negotiation_and_bound_outstanding_requests() {
    assert!(matches!(
        ControlSession::new(ControlLimits {
            max_in_flight: 0,
            ..limits()
        }),
        Err(SessionError::Protocol(_))
    ));
    let mut session = ControlSession::new(limits()).unwrap();
    assert_eq!(
        session.admit(&request(1, ControlCall::Capabilities)),
        Err(SessionError::NegotiationRequired)
    );
    assert_eq!(
        session.negotiate(&NegotiateParams {
            versions: BTreeSet::from([ControlVersion { major: 2, minor: 0 }]),
            limits: limits(),
        }),
        Err(SessionError::IncompatibleVersion)
    );
    let offered = ControlLimits {
        max_in_flight: 1,
        ..limits()
    };
    assert_eq!(
        session.negotiate(&negotiate(offered)).unwrap(),
        (CONTROL_V1, offered)
    );
    assert_eq!(session.limits(), Some(offered));
    assert_eq!(
        session.negotiate(&negotiate(offered)),
        Err(SessionError::AlreadyNegotiated)
    );
    assert_eq!(
        session.admit(&request(9, ControlCall::Negotiate(negotiate(offered)))),
        Err(SessionError::NegotiationHandledSeparately)
    );
    let admitted = session
        .admit(&request(1, ControlCall::Capabilities))
        .unwrap();
    assert_eq!(admitted.id, RequestId(1));
    assert_eq!(admitted.timeout_ms, 500);
    assert!(admitted.deadline().remaining().is_ok());
    assert_eq!(session.in_flight(), 1);
    assert_eq!(
        session.admit(&request(1, ControlCall::Capabilities)),
        Err(SessionError::DuplicateRequest)
    );
    assert_eq!(
        session.admit(&request(2, ControlCall::Capabilities)),
        Err(SessionError::Backpressure)
    );
    session.finish(RequestId(1)).unwrap();
    assert_eq!(
        session.finish(RequestId(1)),
        Err(SessionError::UnknownRequest)
    );
    assert_eq!(session.in_flight(), 0);

    let mut invalid = request(3, ControlCall::Capabilities);
    invalid.timeout_ms = 0;
    assert!(matches!(
        session.admit(&invalid),
        Err(SessionError::Protocol(_))
    ));
}

#[test]
fn negotiation_requires_an_exact_supported_version_and_valid_envelope() {
    let mut session = ControlSession::new(limits()).unwrap();
    let unsupported = NegotiateParams {
        versions: BTreeSet::from([
            ControlVersion { major: 1, minor: 9 },
            ControlVersion { major: 2, minor: 0 },
        ]),
        limits: ControlLimits {
            max_frame_bytes: 2000,
            ..limits()
        },
    };
    assert_eq!(
        session.negotiate(&unsupported),
        Err(SessionError::IncompatibleVersion)
    );
    let offer = NegotiateParams {
        versions: BTreeSet::from([CONTROL_V1, ControlVersion { major: 1, minor: 9 }]),
        ..unsupported
    };
    let malformed = ControlRequest {
        jsonrpc: "1.0".into(),
        id: RequestId(1),
        timeout_ms: 500,
        call: ControlCall::Negotiate(offer.clone()),
    };
    assert!(matches!(
        session.negotiate_request(&malformed),
        Err(SessionError::Protocol(ProtocolError::InvalidJsonRpc))
    ));
    let envelope = request(1, ControlCall::Negotiate(offer));
    let (version, effective) = session.negotiate_request(&envelope).unwrap();
    assert_eq!(version, CONTROL_V1);
    assert_eq!(effective.max_frame_bytes, 2000);
}

#[test]
fn public_response_validation_rejects_sensitive_and_unbounded_values() {
    for value in [
        json!({"api_token": "redacted"}),
        json!({"nested": {"prompt": "private"}}),
        json!({"detail": "/home/operator/private"}),
        json!({"detail": "password=hunter2"}),
        json!({"detail": "x".repeat(MAX_PUBLIC_STRING_BYTES + 1)}),
    ] {
        assert_eq!(
            validate_public_value(&value),
            Err(ProtocolError::UnsafePublicValue)
        );
    }
    assert!(
        ControlResponse::success(
            RequestId(1),
            ControlSuccess::Operation(
                BoundControlResult::new(
                    &ControlCall::Status {
                        run_id: RunId("run-1".into())
                    },
                    ControlResult::Status(RunSummary {
                        run_id: RunId("run-1".into()),
                        attempt_id: AttemptId("attempt-1".into()),
                        state: PublicRunState::Completed,
                        revision: Revision(1),
                        plan_sha256: "a".repeat(64),
                    }),
                )
                .unwrap(),
            )
        )
        .validate()
        .is_ok()
    );

    let mut nested = json!(null);
    for _ in 0..=MAX_PUBLIC_JSON_DEPTH {
        nested = json!([nested]);
    }
    assert_eq!(
        validate_public_value(&nested),
        Err(ProtocolError::UnsafePublicValue)
    );
    assert_eq!(
        validate_public_value(&Value::Array(vec![Value::Null; MAX_PUBLIC_JSON_NODES + 1])),
        Err(ProtocolError::UnsafePublicValue)
    );

    assert_eq!(
        ControlResponse::success(
            RequestId(2),
            ControlSuccess::Operation(
                BoundControlResult::new(
                    &ControlCall::CreatePlan(MutationParams {
                        idempotency_key: "key".into(),
                        definition: json!({}),
                    }),
                    ControlResult::Plan(PlanReference {
                        plan_id: "/private/plan".into(),
                        plan_sha256: "a".repeat(64),
                    }),
                )
                .unwrap(),
            )
        )
        .validate(),
        Err(ProtocolError::InvalidIdentity)
    );
}

#[test]
fn negotiation_rejects_nonnegotiation_empty_versions_and_invalid_deadlines() {
    let ordinary = request(1, ControlCall::Capabilities);
    assert_eq!(
        validate_negotiation_request(&ordinary, limits()),
        Err(ProtocolError::InvalidResponse)
    );
    let empty = ControlRequest {
        call: ControlCall::Negotiate(NegotiateParams {
            versions: BTreeSet::new(),
            limits: limits(),
        }),
        ..ordinary
    };
    assert_eq!(
        validate_negotiation_request(&empty, limits()),
        Err(ProtocolError::InvalidLimit("versions"))
    );
    assert!(matches!(
        RequestDeadline::start(0),
        Err(SessionError::Protocol(ProtocolError::InvalidTimeout))
    ));
    let mut session = ControlSession::new(limits()).unwrap();
    assert_eq!(
        session.negotiate_request(&request(2, ControlCall::Capabilities)),
        Err(SessionError::NegotiationRequired)
    );
}

#[test]
fn durable_idempotency_survives_restore_and_conflicts_fail_closed() {
    assert_eq!(
        IdempotencyIndex::restore(0, []).unwrap_err(),
        IdempotencyError::InvalidCapacity
    );
    let record = MutationRecord {
        key: "key-1".into(),
        request_sha256: "a".repeat(64),
        result: json!({"run_id": "run-1"}),
    };
    let mut index = IdempotencyIndex::restore(2, [record.clone()]).unwrap();
    assert_eq!(index.len(), 1);
    assert!(!index.is_empty());
    assert_eq!(
        index.check("key-1", &"a".repeat(64)).unwrap(),
        IdempotencyDecision::Replay(&record.result)
    );
    assert_eq!(
        index.check("key-1", &"b".repeat(64)),
        Err(IdempotencyError::Conflict)
    );
    assert_eq!(
        index.check("key-2", &"b".repeat(64)),
        Ok(IdempotencyDecision::New)
    );
    let second = MutationRecord {
        key: "key-2".into(),
        request_sha256: "b".repeat(64),
        result: json!({"run_id": "run-2"}),
    };
    index.record_committed(second.clone()).unwrap();
    assert_eq!(index.len(), 2);
    assert_eq!(index.check("key-3", "c"), Err(IdempotencyError::Capacity));
    assert_eq!(
        index.record_committed(second),
        Err(IdempotencyError::DuplicateJournalKey)
    );
    assert_eq!(
        IdempotencyIndex::restore(2, [record.clone(), record]).unwrap_err(),
        IdempotencyError::DuplicateJournalKey
    );
    assert_eq!(
        IdempotencyIndex::restore(
            1,
            [
                MutationRecord {
                    key: "a".into(),
                    request_sha256: "a".into(),
                    result: Value::Null
                },
                MutationRecord {
                    key: "b".into(),
                    request_sha256: "b".into(),
                    result: Value::Null
                },
            ],
        )
        .unwrap_err(),
        IdempotencyError::Capacity
    );
}

#[test]
fn event_windows_resume_contiguously_and_detect_loss() {
    assert_eq!(
        EventWindow::restore(0, []).unwrap_err(),
        CursorError::InvalidCapacity
    );
    let mut empty = EventWindow::restore(3, []).unwrap();
    assert_eq!(empty.oldest_revision(), Revision(0));
    assert_eq!(empty.latest_revision(), Revision(0));
    assert_eq!(
        empty.page(None, 2).unwrap(),
        Page {
            items: vec![],
            next: None,
            has_more: false
        }
    );
    assert_eq!(empty.page(Some(Revision(1)), 2), Err(CursorError::Future));
    assert_eq!(empty.page(None, 0), Err(CursorError::InvalidPage));

    empty.append(event(10)).unwrap();
    empty.append(event(11)).unwrap();
    empty.append(event(12)).unwrap();
    let first = empty.page(None, 2).unwrap();
    assert_eq!(first.items, vec![event(10), event(11)]);
    assert_eq!(first.next, Some(Revision(11)));
    assert!(first.has_more);
    let second = empty.page(first.next, 2).unwrap();
    assert_eq!(second.items, vec![event(12)]);
    assert!(!second.has_more);
    empty.append(event(13)).unwrap();
    assert_eq!(empty.oldest_revision(), Revision(11));
    assert_eq!(empty.latest_revision(), Revision(13));
    assert_eq!(
        empty.page(Some(Revision(9)), 2),
        Err(CursorError::Stale {
            oldest: Revision(11)
        })
    );
    assert_eq!(empty.page(Some(Revision(14)), 2), Err(CursorError::Future));
    assert_eq!(empty.page(Some(Revision(13)), 2).unwrap().items, vec![]);
    assert_eq!(
        empty.append(event(15)),
        Err(CursorError::NonContiguous {
            expected: Revision(14),
            actual: Revision(15)
        })
    );
    let overflow = EventWindow::restore(2, [event(u64::MAX)]).unwrap();
    let mut overflow = overflow;
    assert_eq!(
        overflow.append(event(0)),
        Err(CursorError::RevisionOverflow)
    );
    assert!(matches!(
        EventWindow::restore(2, [event(2), event(4)]),
        Err(CursorError::NonContiguous { .. })
    ));
}

#[test]
fn owner_socket_is_private_authenticated_and_resize_independent() {
    let directory = TestDirectory::private();
    let socket_path = directory.path.join("control.sock");
    {
        let listener = OwnerSocket::bind(&socket_path, limits()).unwrap();
        let client_path = socket_path.clone();
        let client = thread::spawn(move || UnixStream::connect(client_path).unwrap());
        let (server, identity) = listener.accept().unwrap();
        let client = client.join().unwrap();
        assert_eq!(identity.uid(), rustix::process::geteuid().as_raw());
        assert_eq!(identity.gid(), rustix::process::getegid().as_raw());
        assert!(identity.pid() > 0);
        assert!(server.read_timeout().unwrap().is_some());
        assert!(server.write_timeout().unwrap().is_some());
        apply_request_deadline(&server, 25, limits()).unwrap();
        assert_eq!(
            server.read_timeout().unwrap().unwrap(),
            std::time::Duration::from_millis(25)
        );
        assert!(matches!(
            apply_request_deadline(&server, 0, limits()),
            Err(TransportError::InvalidDeadline)
        ));
        assert!(matches!(
            apply_request_deadline(&server, limits().max_timeout_ms + 1, limits()),
            Err(TransportError::InvalidDeadline)
        ));
        assert_eq!(listener.path(), socket_path);
        assert!(listener.listener().local_addr().is_ok());
        let metadata = fs::symlink_metadata(&socket_path).unwrap();
        assert!(metadata.file_type().is_socket());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        drop(client);
    }
    assert!(!socket_path.exists());
}

#[test]
fn owner_socket_refuses_unsafe_paths_and_preserves_replacements() {
    let directory = TestDirectory::private();
    let bad_dir = directory.path.join("shared");
    fs::create_dir(&bad_dir).unwrap();
    fs::set_permissions(&bad_dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        OwnerSocket::bind(bad_dir.join("control.sock"), limits()),
        Err(TransportError::UnsafeRuntimeDirectory)
    ));
    let symlink = directory.path.join("link");
    std::os::unix::fs::symlink(&bad_dir, &symlink).unwrap();
    assert!(matches!(
        OwnerSocket::bind(symlink.join("control.sock"), limits()),
        Err(TransportError::UnsafeRuntimeDirectory)
    ));
    assert!(matches!(
        OwnerSocket::bind(Path::new("/"), limits()),
        Err(TransportError::InvalidSocketPath)
    ));
    assert!(matches!(
        OwnerSocket::bind(
            directory.path.join("control.sock"),
            ControlLimits {
                max_frame_bytes: 0,
                ..limits()
            }
        ),
        Err(TransportError::Protocol(_))
    ));
    let existing = directory.path.join("existing");
    fs::write(&existing, b"do-not-replace").unwrap();
    assert!(matches!(
        OwnerSocket::bind(&existing, limits()),
        Err(TransportError::SocketPathExists)
    ));
    assert_eq!(fs::read(&existing).unwrap(), b"do-not-replace");

    let socket_path = directory.path.join("owned.sock");
    let moved_path = directory.path.join("moved.sock");
    let listener = OwnerSocket::bind(&socket_path, limits()).unwrap();
    fs::rename(&socket_path, &moved_path).unwrap();
    fs::write(&socket_path, b"replacement").unwrap();
    drop(listener);
    assert_eq!(fs::read(&socket_path).unwrap(), b"replacement");
    assert!(
        fs::symlink_metadata(&moved_path)
            .unwrap()
            .file_type()
            .is_socket()
    );
    fs::remove_file(moved_path).unwrap();

    let removed_path = directory.path.join("removed.sock");
    let removed = OwnerSocket::bind(&removed_path, limits()).unwrap();
    fs::remove_file(&removed_path).unwrap();
    drop(removed);
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn private() -> Self {
        let root = std::env::var_os("ASB_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        fs::create_dir_all(&root).unwrap();
        let path = root.join(format!(
            "asb-control-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self { path }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
