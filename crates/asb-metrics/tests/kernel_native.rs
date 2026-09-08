// SPDX-License-Identifier: MIT
//! Opt-in native evidence for exact digest-pinned perf and bpftool probes.

use asb_metrics::kernel::{KernelDiagnostics, PinnedTool, UnavailableReason};
use std::path::PathBuf;
use std::time::Duration;

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required for the native diagnostic"))
}

#[test]
fn exact_pinned_native_probes_are_available_or_explicitly_permission_denied() {
    if std::env::var_os("ASB_NATIVE_KERNEL_DIAGNOSTICS").is_none() {
        return;
    }
    let scratch = PathBuf::from(required("ASB_KERNEL_SCRATCH"));
    let diagnostics = KernelDiagnostics::new(scratch.clone(), Duration::from_secs(10)).unwrap();
    let perf = PinnedTool::new(
        PathBuf::from(required("ASB_PERF_PATH")),
        required("ASB_PERF_SHA256"),
        "--version".into(),
        required("ASB_PERF_VERSION"),
    )
    .unwrap();
    let bpftool = PinnedTool::new(
        PathBuf::from(required("ASB_BPFTOOL_PATH")),
        required("ASB_BPFTOOL_SHA256"),
        "version".into(),
        required("ASB_BPFTOOL_VERSION"),
    )
    .unwrap();

    let perf_result = diagnostics.perf_task_clock(&perf, 10);
    assert!(
        perf_result.value().is_some_and(|value| value > 0)
            || perf_result.unavailable() == Some(UnavailableReason::PermissionDenied),
        "unexpected redacted perf classification: {:?}",
        perf_result.unavailable()
    );
    let ebpf_result = diagnostics.ebpf_feature_count(&bpftool);
    assert!(
        ebpf_result.value().is_some_and(|value| value > 0)
            || ebpf_result.unavailable() == Some(UnavailableReason::PermissionDenied),
        "unexpected redacted bpftool classification: {:?}",
        ebpf_result.unavailable()
    );
    assert_eq!(std::fs::read_dir(scratch).unwrap().count(), 0);
}
