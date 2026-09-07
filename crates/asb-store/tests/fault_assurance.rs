// SPDX-License-Identifier: MIT
//! Real process-crash and storage-reader fault regressions.

use asb_protocol::Id;
use asb_store::{
    AtomicStore, ExecutionState, JOURNAL_SCHEMA_VERSION, JournalEvent, MANIFEST_SCHEMA_VERSION,
    RecoveryDecision, RunManifest, StoreError, StoreLimits,
};
use serde_json::json;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestRoot(PathBuf);
impl TestRoot {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let base = std::env::var_os("CARGO_TARGET_TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = base.join(format!("asb-fault-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("create test root");
        Self(path)
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn manifest() -> RunManifest {
    RunManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        run_id: Id("run".into()),
        attempt_id: Id("attempt".into()),
        definition: json!({"source": "fault-assurance"}),
    }
}
fn event(sequence: u64, state: ExecutionState) -> JournalEvent {
    JournalEvent {
        schema_version: JOURNAL_SCHEMA_VERSION,
        sequence,
        attempt_id: Id("attempt".into()),
        monotonic_offset_ns: sequence,
        state,
        evidence: json!({}),
    }
}

#[test]
fn crash_child_after_durable_running_intent() {
    let Some(root) = std::env::var_os("ASB_CRASH_CHILD_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    assert_eq!(
        std::env::current_dir().expect("read crash child working directory"),
        root
    );
    fs::write("crash-child-cwd", b"root").expect("write crash child cwd proof");
    let store = AtomicStore::open(&root, StoreLimits::default()).expect("open child store");
    store.create_run(&manifest()).expect("create child run");
    for (sequence, state) in [
        ExecutionState::Planned,
        ExecutionState::Prepared,
        ExecutionState::Running,
    ]
    .into_iter()
    .enumerate()
    {
        store
            .append("run", &event(sequence as u64, state))
            .expect("persist child state");
    }
    std::process::abort();
}

#[test]
fn real_process_crash_preserves_uncertain_recovery_outcome() {
    let root = TestRoot::new("process-crash");
    let status = Command::new(std::env::current_exe().expect("test executable"))
        .arg("--exact")
        .arg("crash_child_after_durable_running_intent")
        .arg("--nocapture")
        .current_dir(&root.0)
        .env("ASB_CRASH_CHILD_ROOT", &root.0)
        .status()
        .expect("launch crash child");
    assert!(!status.success());
    assert_eq!(
        fs::read(root.0.join("crash-child-cwd")).expect("read crash child cwd proof"),
        b"root"
    );
    let store = AtomicStore::open(&root.0, StoreLimits::default()).expect("reopen store");
    assert_eq!(
        store.recovery_decision("run").expect("decision"),
        RecoveryDecision::NeedsReconciliation
    );
    assert_eq!(store.load_journal("run").expect("journal").len(), 3);
}

struct StorageFullReader {
    returned_data: bool,
}
impl Read for StorageFullReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.returned_data {
            Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "synthetic storage-full boundary",
            ))
        } else {
            self.returned_data = true;
            buffer[..4].copy_from_slice(b"part");
            Ok(4)
        }
    }
}

#[test]
fn storage_fault_removes_partial_artifact_and_preserves_manifest() {
    let root = TestRoot::new("storage-full");
    let store = AtomicStore::open(&root.0, StoreLimits::default()).expect("open store");
    store.create_run(&manifest()).expect("create run");
    let result = store.put_artifact(
        "run",
        "artifact",
        StorageFullReader {
            returned_data: false,
        },
    );
    assert!(matches!(result, Err(StoreError::Io(_))));
    assert_eq!(
        fs::read_dir(root.0.join("runs/run/artifacts"))
            .expect("artifact directory")
            .count(),
        0
    );
    assert_eq!(store.load_manifest("run").expect("manifest"), manifest());
    assert!(store.load_journal("run").expect("journal").is_empty());
}
