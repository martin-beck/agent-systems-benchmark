// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]

use asb_metrics::{Collection, LinuxCollector, SamplingEvidence};
use asb_protocol::MetricValue;
use std::fs::{self, File};
use std::hint::black_box;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime};

fn value(collection: &Collection, id: &str) -> Option<f64> {
    collection
        .samples
        .iter()
        .find(|sample| sample.descriptor.metric_id.0 == id)
        .and_then(|sample| match sample.value {
            MetricValue::Available { value } => Some(value),
            MetricValue::Unavailable { .. } => None,
        })
}

fn private_fixture(root: &Path) {
    let process = root.join("99");
    fs::create_dir_all(&process).unwrap();
    fs::write(
        process.join("stat"),
        "99 (private) R 1 2 3 4 5 6 7 11 8 13 9 17 19 20 21 22 23 24 25 26 27 29",
    )
    .unwrap();
    fs::write(process.join("io"), "read_bytes: 1\nwrite_bytes: 2\n").unwrap();
    fs::set_permissions(process.join("stat"), fs::Permissions::from_mode(0o0)).unwrap();
    fs::set_permissions(process.join("io"), fs::Permissions::from_mode(0o0)).unwrap();
}

#[test]
#[ignore = "invoked after an actual privilege drop by permission_denied_is_unavailable"]
fn permission_denied_child() {
    let root = std::env::var_os("ASB_PERMISSION_ROOT").expect("fixture root");
    let collection = LinuxCollector::with_roots(root, "/unused").collect_process(99, 0);
    assert_eq!(collection.evidence.available_values, 0);
    assert_eq!(collection.evidence.unavailable_values, 7);
    assert!(collection.samples.iter().all(|sample| {
        matches!(
            &sample.value,
            MetricValue::Unavailable { reason }
                if reason == "kernel metric source permission denied"
        )
    }));
}

#[test]
fn permission_denied_is_unavailable() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("asb-metrics-permission-{unique}"));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    private_fixture(&root);
    let uid = Command::new("id").arg("-u").output().unwrap();
    let uid = String::from_utf8(uid.stdout).unwrap();
    let status = if uid.trim() == "0" {
        Command::new("setpriv")
            .args(["--reuid=65534", "--regid=65534", "--clear-groups"])
            .arg(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "permission_denied_child",
                "--ignored",
                "--nocapture",
            ])
            .env("ASB_PERMISSION_ROOT", &root)
            .status()
            .expect("setpriv from util-linux is required for the root-run negative")
    } else {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "permission_denied_child",
                "--ignored",
                "--nocapture",
            ])
            .env("ASB_PERMISSION_ROOT", &root)
            .status()
            .unwrap()
    };
    fs::set_permissions(root.join("99/stat"), fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(root.join("99/io"), fs::Permissions::from_mode(0o600)).unwrap();
    fs::remove_dir_all(&root).unwrap();
    assert!(status.success());
}

#[test]
fn real_procfs_observes_controlled_cpu_memory_fault_and_io_work() {
    let collector = LinuxCollector::host();
    let pid = std::process::id();
    let before = collector.collect_process(pid, 0);
    let cpu_before = value(&before, "process.cpu.user_time").expect("CPU available");
    let rss_before = value(&before, "process.memory.resident").expect("RSS available");
    let faults_before = value(&before, "process.faults.minor").expect("faults available");
    let write_before = value(&before, "process.io.write").expect("I/O available");

    let mut allocation = vec![0_u8; 32 * 1024 * 1024];
    for page in allocation.chunks_mut(4096) {
        page[0] = 1;
    }
    let started = std::time::Instant::now();
    let mut accumulator = 0_u64;
    while started.elapsed() < Duration::from_millis(150) {
        for number in 0_u64..20_000 {
            accumulator = accumulator.wrapping_add(black_box(number).rotate_left(7));
        }
    }
    black_box(accumulator);

    let path = std::env::temp_dir().join(format!("asb-metrics-io-{pid}"));
    let payload = vec![0x5a_u8; 4 * 1024 * 1024];
    let mut file = File::create(&path).unwrap();
    file.write_all(&payload).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let mut readback = Vec::new();
    File::open(&path)
        .unwrap()
        .read_to_end(&mut readback)
        .unwrap();
    fs::remove_file(path).unwrap();
    black_box(allocation.as_slice());
    assert_eq!(readback.len(), payload.len());

    let after = collector.collect_process(pid, 1);
    let cpu_delta = value(&after, "process.cpu.user_time").unwrap() - cpu_before;
    let rss_delta = value(&after, "process.memory.resident").unwrap() - rss_before;
    let fault_delta = value(&after, "process.faults.minor").unwrap() - faults_before;
    let write_delta = value(&after, "process.io.write").unwrap() - write_before;
    eprintln!(
        "controlled_process cpu_delta_ns={cpu_delta:.0} rss_delta_bytes={rss_delta:.0} \
         minor_fault_delta={fault_delta:.0} write_delta_bytes={write_delta:.0}"
    );
    assert!(cpu_delta > 0.0);
    assert!(rss_delta >= 16.0 * 1024.0 * 1024.0);
    assert!(fault_delta > 0.0);
    assert!(write_delta > 0.0);
}

#[test]
fn real_cgroup_and_overhead_evidence_are_observable() {
    let collector = LinuxCollector::host();
    let cgroup = collector.collect_self_cgroup(0).expect("unified cgroup v2");
    assert_eq!(cgroup.samples.len(), 18);
    assert!(cgroup.evidence.available_values >= 16);
    assert!(cgroup.evidence.collection_time_ns > 0);

    let mut evidence = SamplingEvidence::new(64);
    for offset in 0..64 {
        evidence.record_collection(&collector.collect_process(std::process::id(), offset));
    }
    assert!(evidence.is_complete());
    assert_eq!(evidence.lost_samples, 0);
    assert_eq!(evidence.unavailable_values, 0);
    assert!(evidence.collection_time_ns > 0);
    assert!(evidence.max_collection_time_ns > 0);
    eprintln!(
        "sampling scheduled={} collected={} lost={} available_values={} unavailable_values={} \
         total_collection_ns={} max_collection_ns={} cgroup_available={} cgroup_unavailable={}",
        evidence.scheduled_samples,
        evidence.collected_samples,
        evidence.lost_samples,
        evidence.available_values,
        evidence.unavailable_values,
        evidence.collection_time_ns,
        evidence.max_collection_time_ns,
        cgroup.evidence.available_values,
        cgroup.evidence.unavailable_values,
    );
}
