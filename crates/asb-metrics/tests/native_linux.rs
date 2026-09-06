// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]
#![allow(missing_docs)]

use asb_metrics::{Collection, LinuxCollector, SamplingEvidence, TargetError};
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
        .samples()
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

fn unique_root(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("asb-metrics-{label}-{unique}"))
}

#[test]
#[ignore = "invoked after an actual privilege drop by permission_denied_is_unavailable"]
fn permission_denied_child() {
    let root = std::env::var_os("ASB_PERMISSION_ROOT").expect("fixture root");
    let collection = LinuxCollector::with_roots(root, "/unused").collect_process(99, 0);
    assert_eq!(collection.evidence().available_values(), 0);
    assert_eq!(collection.evidence().unavailable_values(), 7);
    assert!(collection.samples().iter().all(|sample| {
        matches!(
            &sample.value,
            MetricValue::Unavailable { reason }
                if reason == "kernel metric source permission denied"
        )
    }));
}

#[test]
fn permission_denied_is_unavailable() {
    let root = unique_root("permission");
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
fn controlled_cgroup_fixture_preserves_values_units_and_scope() {
    let root = unique_root("cgroup");
    let group = root.join("work");
    fs::create_dir_all(&group).unwrap();
    fs::write(
        group.join("cpu.stat"),
        "usage_usec 100\nuser_usec 60\nsystem_usec 40\nnr_periods 5\n\
         nr_throttled 2\nthrottled_usec 7\n",
    )
    .unwrap();
    fs::write(group.join("memory.current"), "4096\n").unwrap();
    fs::write(group.join("memory.peak"), "8192\n").unwrap();
    fs::write(group.join("memory.stat"), "pgfault 12\npgmajfault 3\n").unwrap();
    fs::write(
        group.join("io.stat"),
        "8:0 rbytes=10 wbytes=20 rios=1\n8:16 rbytes=7 wbytes=9\n",
    )
    .unwrap();
    let pressure = "some avg10=0.00 avg60=0.00 avg300=0.00 total=4\n\
                    full avg10=0.00 avg60=0.00 avg300=0.00 total=2\n";
    for resource in ["cpu", "memory", "io"] {
        fs::write(group.join(format!("{resource}.pressure")), pressure).unwrap();
    }
    let collection = LinuxCollector::with_roots("/unused", &root)
        .collect_cgroup("work", 77)
        .unwrap();
    fs::remove_dir_all(root).unwrap();

    assert_eq!(collection.samples().len(), 18);
    assert_eq!(collection.evidence().available_values(), 18);
    assert_eq!(value(&collection, "cgroup.cpu.usage"), Some(100_000.0));
    assert_eq!(
        value(&collection, "cgroup.cpu.throttled_time"),
        Some(7_000.0)
    );
    assert_eq!(value(&collection, "cgroup.memory.current"), Some(4096.0));
    assert_eq!(value(&collection, "cgroup.io.read"), Some(17.0));
    assert_eq!(value(&collection, "cgroup.io.write"), Some(29.0));
    assert_eq!(value(&collection, "cgroup.pressure.io.full"), Some(2_000.0));
    assert!(collection.samples().iter().all(|sample| {
        sample.offset_ns == 77
            && sample.descriptor.scope == "cgroup"
            && !sample.descriptor.unit.is_empty()
            && sample.descriptor.source.starts_with("cgroup2:")
    }));
}

#[test]
fn absent_cgroup_files_are_unavailable_never_zero() {
    let root = unique_root("absent");
    fs::create_dir(&root).unwrap();
    let collection = LinuxCollector::with_roots("/unused", &root)
        .collect_cgroup("missing", 0)
        .unwrap();
    fs::remove_dir(root).unwrap();
    assert_eq!(collection.evidence().attempted_values(), 18);
    assert_eq!(collection.evidence().available_values(), 0);
    assert_eq!(collection.evidence().unavailable_values(), 18);
    assert!(collection.samples().iter().all(|sample| {
        matches!(
            &sample.value,
            MetricValue::Unavailable { reason } if reason == "kernel metric source is absent"
        )
    }));
}

#[test]
fn oversized_self_cgroup_membership_fails_closed() {
    let proc_root = unique_root("oversized-membership");
    fs::create_dir_all(proc_root.join("self")).unwrap();
    fs::write(proc_root.join("self/cgroup"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
    let result = LinuxCollector::with_roots(&proc_root, "/unused").collect_self_cgroup(0);
    fs::remove_dir_all(proc_root).unwrap();
    assert_eq!(result, Err(TargetError::UnifiedCgroupUnavailable));
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
    assert_eq!(cgroup.samples().len(), 18);
    assert!(cgroup.evidence().available_values() >= 16);
    assert!(cgroup.evidence().collection_time_ns() > 0);

    let mut evidence = SamplingEvidence::new(64);
    for offset in 0..64 {
        evidence
            .record_collection(&collector.collect_process(std::process::id(), offset))
            .unwrap();
    }
    assert!(evidence.is_complete());
    assert_eq!(evidence.lost_samples(), 0);
    assert_eq!(evidence.unavailable_values(), 0);
    assert!(evidence.collection_time_ns() > 0);
    assert!(evidence.max_collection_time_ns() > 0);
    eprintln!(
        "sampling scheduled={} collected={} lost={} available_values={} unavailable_values={} \
         total_collection_ns={} max_collection_ns={} cgroup_available={} cgroup_unavailable={}",
        evidence.scheduled_samples(),
        evidence.collected_samples(),
        evidence.lost_samples(),
        evidence.available_values(),
        evidence.unavailable_values(),
        evidence.collection_time_ns(),
        evidence.max_collection_time_ns(),
        cgroup.evidence().available_values(),
        cgroup.evidence().unavailable_values(),
    );
}
