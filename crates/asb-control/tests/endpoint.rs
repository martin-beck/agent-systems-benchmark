// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! End-to-end local endpoint, disconnect, recovery, privacy, and deadline tests.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use asb_control::*;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn limits(timeout_ms: u64) -> ControlLimits {
    ControlLimits {
        max_frame_bytes: 4096,
        max_timeout_ms: timeout_ms,
        max_page_items: 8,
        max_in_flight: 2,
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let path = std::env::var_os("ASB_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!(
                "asb-control-endpoint-{label}-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
        let _ = fs::remove_dir_all(&path);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("private test directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct DurableState {
    launches: usize,
    catalog_reads: usize,
    by_key: BTreeMap<String, (String, String)>,
    run_states: BTreeMap<String, String>,
}

#[derive(Clone, Default)]
struct DurableBackend {
    state: Arc<Mutex<DurableState>>,
}

impl DurableBackend {
    fn launches(&self) -> usize {
        self.state.lock().expect("state lock").launches
    }

    fn catalog_reads(&self) -> usize {
        self.state.lock().expect("state lock").catalog_reads
    }
}

impl ControlBackend for DurableBackend {
    fn runner_instance_id(&self) -> &str {
        "runner-test-1"
    }

    fn oldest_revision(&self) -> Revision {
        Revision(1)
    }

    fn latest_revision(&self) -> Revision {
        Revision(3)
    }

    fn execute(
        &self,
        call: &ControlCall,
        deadline: RequestDeadline,
    ) -> Result<BoundControlResult, BackendFailure> {
        deadline
            .check()
            .map_err(|_| BackendFailure::NeedsReconciliation)?;
        match call {
            ControlCall::Capabilities => Ok(BoundControlResult::new(
                call,
                ControlResult::Capabilities(Capabilities {
                    validate_settings: true,
                    run_control: true,
                    repeat: true,
                    analysis: true,
                    events: true,
                }),
            )
            .expect("bind result")),
            ControlCall::MeasurementCatalog => {
                self.state.lock().expect("state lock").catalog_reads += 1;
                Ok(BoundControlResult::new(
                    call,
                    ControlResult::MeasurementCatalog(MeasurementCatalogPublication::built_in(
                        asb_protocol::baseline_measurement_catalog(),
                    )),
                )
                .expect("bind result"))
            }
            ControlCall::Launch(params) => {
                let mut state = self.state.lock().expect("state lock");
                if let Some((plan_id, run_id)) = state.by_key.get(&params.idempotency_key) {
                    return if plan_id == &params.plan_id {
                        Ok(BoundControlResult::new(
                            call,
                            ControlResult::Launch(summary(run_id, PublicRunState::Running)),
                        )
                        .expect("bind result"))
                    } else {
                        Err(BackendFailure::NeedsReconciliation)
                    };
                }
                state.launches += 1;
                let run_id = format!("run-{}", state.launches);
                state.by_key.insert(
                    params.idempotency_key.clone(),
                    (params.plan_id.clone(), run_id.clone()),
                );
                state.run_states.insert(run_id.clone(), "running".into());
                let durable = Arc::clone(&self.state);
                let completed = run_id.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(30));
                    durable
                        .lock()
                        .expect("state lock")
                        .run_states
                        .insert(completed, "completed".into());
                });
                Ok(BoundControlResult::new(
                    call,
                    ControlResult::Launch(summary(&run_id, PublicRunState::Running)),
                )
                .expect("bind result"))
            }
            ControlCall::Status { run_id } => {
                let state = self.state.lock().expect("state lock");
                state
                    .run_states
                    .get(&run_id.0)
                    .map(|status| {
                        let state = if status == "completed" {
                            PublicRunState::Completed
                        } else {
                            PublicRunState::Running
                        };
                        BoundControlResult::new(
                            call,
                            ControlResult::Status(summary(&run_id.0, state)),
                        )
                        .expect("bind result")
                    })
                    .ok_or(BackendFailure::NotFound)
            }
            _ => Ok(BoundControlResult::new(
                call,
                ControlResult::Acknowledged(MutationAcknowledgement { accepted: true }),
            )
            .expect("bind result")),
        }
    }
}

fn summary(run_id: &str, state: PublicRunState) -> RunSummary {
    RunSummary {
        run_id: RunId(run_id.into()),
        attempt_id: AttemptId(format!("{run_id}-attempt")),
        state,
        created_revision: Revision(1),
        revision: Revision(3),
        plan_sha256: "0".repeat(64),
    }
}

fn launch_server(
    socket: &Path,
    backend: DurableBackend,
) -> thread::JoinHandle<Result<(), EndpointError>> {
    let mut server = ControlServer::bind(socket, limits(500), backend).expect("bind server");
    assert_eq!(server.path(), socket);
    thread::spawn(move || server.serve_one())
}

#[test]
fn client_disconnect_does_not_cancel_and_restart_retry_is_idempotent() {
    let root = Scratch::new("lifecycle");
    let backend = DurableBackend::default();
    let first_socket = root.0.join("first.sock");
    let first = launch_server(&first_socket, backend.clone());
    let mut client = ControlClient::connect(&first_socket, limits(500)).expect("connect");
    assert_eq!(client.negotiated().runner_instance_id, "runner-test-1");
    let launched = client
        .call(
            ControlCall::Launch(LaunchParams {
                idempotency_key: "launch-key-1".into(),
                plan_id: "plan-1".into(),
            }),
            200,
        )
        .expect("launch response");
    assert!(matches!(
        launched.into_result(),
        Some(ControlSuccess::Operation(BoundControlResult {
            result: ControlResult::Launch(RunSummary {
                run_id: RunId(ref value),
                ..
            }),
            ..
        })) if value == "run-1"
    ));
    drop(client);
    first.join().expect("server thread").expect("serve first");

    thread::sleep(Duration::from_millis(50));
    let second_socket = root.0.join("second.sock");
    let second = launch_server(&second_socket, backend.clone());
    let mut reconnected = ControlClient::connect(&second_socket, limits(500)).expect("reconnect");
    let replayed = reconnected
        .call(
            ControlCall::Launch(LaunchParams {
                idempotency_key: "launch-key-1".into(),
                plan_id: "plan-1".into(),
            }),
            200,
        )
        .expect("retry response");
    assert!(matches!(
        replayed.into_result(),
        Some(ControlSuccess::Operation(BoundControlResult {
            result: ControlResult::Launch(RunSummary {
                run_id: RunId(ref value),
                ..
            }),
            ..
        })) if value == "run-1"
    ));
    let status = reconnected
        .call(
            ControlCall::Status {
                run_id: RunId("run-1".into()),
            },
            200,
        )
        .expect("status");
    assert!(matches!(
        status.into_result(),
        Some(ControlSuccess::Operation(BoundControlResult {
            result: ControlResult::Status(RunSummary {
                state: PublicRunState::Completed,
                ..
            }),
            ..
        }))
    ));
    assert_eq!(backend.launches(), 1);
    drop(reconnected);
    second.join().expect("server thread").expect("serve second");
}

#[test]
fn trickle_input_cannot_reset_the_absolute_ingress_deadline() {
    let root = Scratch::new("trickle");
    let socket = root.0.join("control.sock");
    let mut server =
        ControlServer::bind(&socket, limits(40), DurableBackend::default()).expect("bind server");
    let started = Instant::now();
    let thread = thread::spawn(move || server.serve_one());
    let mut stream = UnixStream::connect(&socket).expect("connect raw peer");
    stream.write_all(&[0]).expect("first trickle byte");
    thread::sleep(Duration::from_millis(70));
    let _ = stream.write_all(&[0, 0, 1]);
    drop(stream);
    assert!(matches!(
        thread.join().expect("server thread"),
        Err(EndpointError::Frame(FrameError::DeadlineExceeded))
    ));
    assert!(started.elapsed() < Duration::from_millis(250));
}

#[derive(Clone)]
struct NonCooperativeBackend;

impl ControlBackend for NonCooperativeBackend {
    fn runner_instance_id(&self) -> &str {
        "runner-slow-test"
    }

    fn oldest_revision(&self) -> Revision {
        Revision(0)
    }

    fn latest_revision(&self) -> Revision {
        Revision(0)
    }

    fn execute(
        &self,
        _call: &ControlCall,
        _deadline: RequestDeadline,
    ) -> Result<BoundControlResult, BackendFailure> {
        thread::sleep(Duration::from_millis(200));
        Ok(BoundControlResult::new(
            _call,
            ControlResult::Capabilities(Capabilities {
                validate_settings: true,
                run_control: true,
                repeat: true,
                analysis: true,
                events: true,
            }),
        )
        .expect("bind result"))
    }
}

#[test]
fn noncooperative_backend_is_joined_before_deadline_failure() {
    let root = Scratch::new("backend-deadline");
    let socket = root.0.join("control.sock");
    let mut server =
        ControlServer::bind(&socket, limits(30), NonCooperativeBackend).expect("bind server");
    let thread = thread::spawn(move || server.serve_one());
    let mut client = ControlClient::connect(&socket, limits(30)).expect("connect");
    let started = Instant::now();
    assert!(client.call(ControlCall::Capabilities, 20).is_err());
    drop(client);
    assert!(matches!(
        thread.join().expect("server thread"),
        Err(EndpointError::Session(SessionError::DeadlineExceeded))
    ));
    assert!(started.elapsed() >= Duration::from_millis(180));
}

#[derive(Clone)]
struct UnsafeBackend;

impl ControlBackend for UnsafeBackend {
    fn runner_instance_id(&self) -> &str {
        "runner-unsafe-test"
    }

    fn oldest_revision(&self) -> Revision {
        Revision(0)
    }

    fn latest_revision(&self) -> Revision {
        Revision(0)
    }

    fn execute(
        &self,
        _call: &ControlCall,
        _deadline: RequestDeadline,
    ) -> Result<BoundControlResult, BackendFailure> {
        Ok(BoundControlResult::new(
            _call,
            ControlResult::Plan(PlanReference {
                plan_id: "/private/plan".into(),
                plan_sha256: "0".repeat(64),
            }),
        )
        .expect("bind result"))
    }
}

#[test]
fn endpoint_refuses_backend_sensitive_output_before_writing_it() {
    let root = Scratch::new("privacy");
    let socket = root.0.join("control.sock");
    let mut server = ControlServer::bind(&socket, limits(500), UnsafeBackend).expect("bind server");
    let thread = thread::spawn(move || server.serve_one());
    let mut client = ControlClient::connect(&socket, limits(500)).expect("connect");
    assert!(client.call(ControlCall::Capabilities, 200).is_err());
    drop(client);
    assert!(matches!(
        thread.join().expect("server thread"),
        Err(EndpointError::Protocol(ProtocolError::InvalidIdentity))
    ));
}

#[test]
fn malformed_initial_envelope_fails_before_backend_work() {
    let root = Scratch::new("negotiation");
    let socket = root.0.join("control.sock");
    let backend = DurableBackend::default();
    let mut server =
        ControlServer::bind(&socket, limits(500), backend.clone()).expect("bind server");
    let thread = thread::spawn(move || server.serve_one());
    let mut stream = UnixStream::connect(&socket).expect("connect raw peer");
    let request = ControlRequest {
        jsonrpc: "1.0".into(),
        id: RequestId(7),
        timeout_ms: 200,
        call: ControlCall::Negotiate(NegotiateParams {
            versions: [CONTROL_V1].into_iter().collect(),
            limits: limits(500),
        }),
    };
    write_frame(&mut stream, &request, limits(500)).expect("write malformed request");
    drop(stream);
    assert!(matches!(
        thread.join().expect("server thread"),
        Err(EndpointError::Session(SessionError::Protocol(
            ProtocolError::InvalidJsonRpc
        )))
    ));
    assert_eq!(backend.launches(), 0);
}

#[test]
fn catalog_is_isolated_from_v1_0_and_available_only_after_exact_v1_2_negotiation() {
    let root = Scratch::new("catalog-versions");
    let catalog_limits = ControlLimits {
        max_frame_bytes: 64 * 1024,
        ..limits(500)
    };

    let v1_socket = root.0.join("v1.sock");
    let v1_backend = DurableBackend::default();
    let mut v1_server =
        ControlServer::bind(&v1_socket, catalog_limits, v1_backend.clone()).expect("bind v1");
    let v1_thread = thread::spawn(move || v1_server.serve_one());
    let mut v1_peer = UnixStream::connect(&v1_socket).expect("connect raw v1 peer");
    let negotiate_request = ControlRequest {
        jsonrpc: JSONRPC_VERSION.into(),
        id: RequestId(1),
        timeout_ms: 200,
        call: ControlCall::Negotiate(NegotiateParams {
            versions: [CONTROL_V1].into_iter().collect(),
            limits: catalog_limits,
        }),
    };
    write_frame(&mut v1_peer, &negotiate_request, catalog_limits).expect("write negotiation");
    let negotiated: ControlResponse =
        read_frame(&mut v1_peer, catalog_limits).expect("read negotiation");
    assert!(matches!(
        negotiated.result(),
        Some(ControlSuccess::Negotiated(Negotiated {
            version: CONTROL_V1,
            ..
        }))
    ));
    let catalog_request = ControlRequest {
        jsonrpc: JSONRPC_VERSION.into(),
        id: RequestId(2),
        timeout_ms: 200,
        call: ControlCall::MeasurementCatalog,
    };
    write_frame(&mut v1_peer, &catalog_request, catalog_limits).expect("write v1 catalog call");
    let rejected: ControlResponse =
        read_frame(&mut v1_peer, catalog_limits).expect("read v1 catalog rejection");
    assert_eq!(
        rejected.error().expect("capability error").code,
        error_code::CAPABILITY_UNAVAILABLE
    );
    assert!(rejected.result().is_none());
    assert_eq!(v1_backend.catalog_reads(), 0);
    drop(v1_peer);
    v1_thread.join().expect("join v1").expect("serve v1");

    let v1_2_socket = root.0.join("v1-2.sock");
    let v1_2_backend = DurableBackend::default();
    let mut v1_2_server =
        ControlServer::bind(&v1_2_socket, catalog_limits, v1_2_backend.clone()).expect("bind v1.2");
    let v1_2_thread = thread::spawn(move || v1_2_server.serve_one());
    let mut v1_2_client = ControlClient::connect_with_versions(
        &v1_2_socket,
        catalog_limits,
        [CONTROL_V1, CONTROL_MEASUREMENT_CATALOG_V1],
    )
    .expect("connect v1.2");
    assert_eq!(
        v1_2_client.negotiated().version,
        CONTROL_MEASUREMENT_CATALOG_V1
    );
    let response = v1_2_client
        .call(ControlCall::MeasurementCatalog, 200)
        .expect("catalog call");
    assert!(matches!(
        response.result(),
        Some(ControlSuccess::Operation(BoundControlResult {
            result: ControlResult::MeasurementCatalog(_),
            ..
        }))
    ));
    assert_eq!(v1_2_backend.catalog_reads(), 1);
    drop(v1_2_client);
    v1_2_thread.join().expect("join v1.2").expect("serve v1.2");
}

#[test]
fn endpoint_rejects_a_types_only_v1_1_offer() {
    let root = Scratch::new("v1-1-only");
    let socket = root.0.join("control.sock");
    let backend = DurableBackend::default();
    let mut server = ControlServer::bind(&socket, limits(500), backend.clone()).expect("bind");
    let server_thread = thread::spawn(move || server.serve_one());
    let mut peer = UnixStream::connect(&socket).expect("connect raw v1.1 peer");
    write_frame(
        &mut peer,
        &ControlRequest {
            jsonrpc: JSONRPC_VERSION.into(),
            id: RequestId(1),
            timeout_ms: 200,
            call: ControlCall::Negotiate(NegotiateParams {
                versions: [CONTROL_HISTORY_ANALYSIS_V1].into_iter().collect(),
                limits: limits(500),
            }),
        },
        limits(500),
    )
    .expect("write v1.1 offer");
    drop(peer);
    assert!(matches!(
        server_thread.join().expect("join"),
        Err(EndpointError::Session(SessionError::IncompatibleVersion))
    ));
    assert_eq!(backend.catalog_reads(), 0);
}
