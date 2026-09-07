// SPDX-License-Identifier: MIT
#![no_main]

use asb_protocol::Id;
use asb_store::{AtomicStore, MANIFEST_SCHEMA_VERSION, RunManifest, StoreLimits};
use libfuzzer_sys::fuzz_target;
use serde_json::json;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Option<Self> {
        let base = std::env::var_os("ASB_FUZZ_SCRATCH").map(PathBuf::from)?;
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!("store-path-{}-{sequence}", std::process::id()));
        fs::create_dir_all(&path).ok()?;
        Some(Self(path))
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fuzz_target!(|data: &[u8]| {
    let Some(root) = TestRoot::new() else {
        return;
    };
    let Ok(store) = AtomicStore::open(&root.0, StoreLimits::default()) else {
        return;
    };
    let manifest = RunManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        run_id: Id("run".into()),
        attempt_id: Id("attempt".into()),
        definition: json!({"source": "fuzz"}),
    };
    if store.create_run(&manifest).is_err() {
        return;
    }
    let name = String::from_utf8_lossy(data.get(..data.len().min(512)).unwrap_or_default());
    let _ = store.put_artifact("run", &name, Cursor::new(b"bounded"));
    assert!(!root.0.parent().unwrap_or(&root.0).join("escape").exists());
});
