// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Portable, fail-closed Linux process and cgroup-v2 metrics.

use asb_protocol::{Aggregation, Id, MetricDescriptor, MetricSample, MetricValue};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

const NS_PER_SECOND: u128 = 1_000_000_000;

/// Why a configured collection target cannot be used safely.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetError {
    /// A cgroup target was absolute rather than relative to the configured mount.
    AbsoluteCgroupPath,
    /// A cgroup target contained a parent, prefix, or other unsafe component.
    UnsafeCgroupPath,
    /// No unambiguous unified cgroup-v2 membership was exposed.
    UnifiedCgroupUnavailable,
}

impl fmt::Display for TargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AbsoluteCgroupPath => "cgroup path must be relative",
            Self::UnsafeCgroupPath => "cgroup path has an unsafe component",
            Self::UnifiedCgroupUnavailable => "unified cgroup v2 path is unavailable",
        })
    }
}

impl std::error::Error for TargetError {}

/// Accounting for one collector invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CollectionEvidence {
    /// Number of metric values attempted.
    attempted_values: u64,
    /// Number backed by kernel evidence.
    available_values: u64,
    /// Number explicitly unavailable.
    unavailable_values: u64,
    /// Monotonic time spent reading and parsing.
    collection_time_ns: u64,
}

impl CollectionEvidence {
    /// Number of metric values attempted.
    #[must_use]
    pub const fn attempted_values(self) -> u64 {
        self.attempted_values
    }

    /// Number of values backed by kernel evidence.
    #[must_use]
    pub const fn available_values(self) -> u64 {
        self.available_values
    }

    /// Number of explicitly unavailable values.
    #[must_use]
    pub const fn unavailable_values(self) -> u64 {
        self.unavailable_values
    }

    /// Monotonic time spent reading and parsing.
    #[must_use]
    pub const fn collection_time_ns(self) -> u64 {
        self.collection_time_ns
    }
}

/// A complete collector response.
#[derive(Clone, Debug, PartialEq)]
pub struct Collection {
    /// Metric samples in stable metric-ID order.
    pub samples: Vec<MetricSample>,
    /// Overhead and availability evidence.
    pub evidence: CollectionEvidence,
}

impl Collection {
    fn new(mut samples: Vec<MetricSample>, started: Instant) -> Self {
        samples.sort_by(|a, b| a.descriptor.metric_id.0.cmp(&b.descriptor.metric_id.0));
        let attempted = u64::try_from(samples.len()).unwrap_or(u64::MAX);
        let available = u64::try_from(
            samples
                .iter()
                .filter(|sample| matches!(sample.value, MetricValue::Available { .. }))
                .count(),
        )
        .unwrap_or(u64::MAX);
        Self {
            samples,
            evidence: CollectionEvidence {
                attempted_values: attempted,
                available_values: available,
                unavailable_values: attempted.saturating_sub(available),
                collection_time_ns: u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
            },
        }
    }
}

/// Explicit accounting across scheduler sampling slots.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SamplingEvidence {
    /// Declared slots in the schedule.
    scheduled_samples: u64,
    /// Slots with completed collection.
    collected_samples: u64,
    /// Slots explicitly skipped or lost.
    lost_samples: u64,
    /// Available values across completed collections.
    available_values: u64,
    /// Unavailable values across completed collections.
    unavailable_values: u64,
    /// Sum of measured collector execution time.
    collection_time_ns: u64,
    /// Largest collector execution time.
    max_collection_time_ns: u64,
}

/// A sampling event would violate the declared schedule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SamplingError {
    /// Collected and lost samples already cover every scheduled slot.
    ScheduleExhausted,
}

impl fmt::Display for SamplingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sampling schedule is exhausted")
    }
}

impl std::error::Error for SamplingError {}

impl SamplingEvidence {
    /// Declared slots in the schedule.
    #[must_use]
    pub const fn scheduled_samples(self) -> u64 {
        self.scheduled_samples
    }

    /// Slots with completed collection.
    #[must_use]
    pub const fn collected_samples(self) -> u64 {
        self.collected_samples
    }

    /// Slots explicitly skipped or lost.
    #[must_use]
    pub const fn lost_samples(self) -> u64 {
        self.lost_samples
    }

    /// Available values across completed collections.
    #[must_use]
    pub const fn available_values(self) -> u64 {
        self.available_values
    }

    /// Unavailable values across completed collections.
    #[must_use]
    pub const fn unavailable_values(self) -> u64 {
        self.unavailable_values
    }

    /// Sum of measured collector execution time.
    #[must_use]
    pub const fn collection_time_ns(self) -> u64 {
        self.collection_time_ns
    }

    /// Largest collector execution time.
    #[must_use]
    pub const fn max_collection_time_ns(self) -> u64 {
        self.max_collection_time_ns
    }
}

impl SamplingEvidence {
    /// Start accounting for a fixed number of scheduled slots.
    #[must_use]
    pub const fn new(scheduled_samples: u64) -> Self {
        Self {
            scheduled_samples,
            collected_samples: 0,
            lost_samples: 0,
            available_values: 0,
            unavailable_values: 0,
            collection_time_ns: 0,
            max_collection_time_ns: 0,
        }
    }

    /// Record a completed collection.
    pub fn record_collection(&mut self, collection: &Collection) -> Result<(), SamplingError> {
        self.reserve_slot()?;
        self.collected_samples = self.collected_samples.saturating_add(1);
        self.available_values = self
            .available_values
            .saturating_add(collection.evidence.available_values);
        self.unavailable_values = self
            .unavailable_values
            .saturating_add(collection.evidence.unavailable_values);
        self.collection_time_ns = self
            .collection_time_ns
            .saturating_add(collection.evidence.collection_time_ns);
        self.max_collection_time_ns = self
            .max_collection_time_ns
            .max(collection.evidence.collection_time_ns);
        Ok(())
    }

    /// Record a schedule slot without a collection.
    pub fn record_lost_sample(&mut self) -> Result<(), SamplingError> {
        self.reserve_slot()?;
        self.lost_samples = self.lost_samples.saturating_add(1);
        Ok(())
    }

    /// Whether collected and lost slots exactly cover the declared schedule.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        self.collected_samples.saturating_add(self.lost_samples) == self.scheduled_samples
    }

    fn reserve_slot(self) -> Result<(), SamplingError> {
        if self.collected_samples.saturating_add(self.lost_samples) >= self.scheduled_samples {
            Err(SamplingError::ScheduleExhausted)
        } else {
            Ok(())
        }
    }
}

/// Linux procfs and cgroup-v2 collector.
#[derive(Clone, Debug)]
pub struct LinuxCollector {
    proc_root: PathBuf,
    cgroup_root: PathBuf,
    page_size_bytes: u64,
    clock_ticks_per_second: u64,
}

impl LinuxCollector {
    /// Use conventional Linux procfs and unified-cgroup mount points.
    #[must_use]
    pub fn host() -> Self {
        Self::with_roots("/proc", "/sys/fs/cgroup")
    }

    /// Use alternate roots for isolated mount namespaces or deterministic fixtures.
    #[must_use]
    pub fn with_roots(proc_root: impl Into<PathBuf>, cgroup_root: impl Into<PathBuf>) -> Self {
        Self {
            proc_root: proc_root.into(),
            cgroup_root: cgroup_root.into(),
            page_size_bytes: u64::try_from(rustix::param::page_size()).unwrap_or(u64::MAX),
            clock_ticks_per_second: rustix::param::clock_ticks_per_second(),
        }
    }

    /// Collect CPU, resident memory, faults, and physical I/O for one process.
    #[must_use]
    pub fn collect_process(&self, pid: u32, offset_ns: u64) -> Collection {
        let started = Instant::now();
        let stat = read_text(self.proc_root.join(pid.to_string()).join("stat")).and_then(|text| {
            parse_process_stat(&text, self.page_size_bytes, self.clock_ticks_per_second)
        });
        let process_io = read_text(self.proc_root.join(pid.to_string()).join("io"))
            .and_then(|text| parse_key_values(&text));
        let mut samples = Vec::with_capacity(7);
        let stat_metrics = [
            (
                "process.cpu.user_time",
                "ns",
                stat.as_ref().map(|v| v.user_ns),
            ),
            (
                "process.cpu.system_time",
                "ns",
                stat.as_ref().map(|v| v.system_ns),
            ),
            (
                "process.memory.resident",
                "By",
                stat.as_ref().map(|v| v.rss_bytes),
            ),
            (
                "process.faults.minor",
                "{fault}",
                stat.as_ref().map(|v| v.minor_faults),
            ),
            (
                "process.faults.major",
                "{fault}",
                stat.as_ref().map(|v| v.major_faults),
            ),
        ];
        for (id, unit, value) in stat_metrics {
            let source = "procfs:/proc/[pid]/stat";
            let resolution = if id.starts_with("process.cpu") {
                cpu_resolution(self.clock_ticks_per_second)
            } else {
                0
            };
            push(
                &mut samples,
                descriptor(id, unit, "process", source, resolution),
                offset_ns,
                value.map_err(Clone::clone),
            );
        }
        for (id, key) in [
            ("process.io.read", "read_bytes"),
            ("process.io.write", "write_bytes"),
        ] {
            let value = process_io
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|v| required(v, key));
            push(
                &mut samples,
                descriptor(id, "By", "process", "procfs:/proc/[pid]/io", 0),
                offset_ns,
                value,
            );
        }
        Collection::new(samples, started)
    }

    /// Collect CPU, memory, fault, I/O, pressure, and throttling for one cgroup.
    pub fn collect_cgroup(
        &self,
        relative_path: impl AsRef<Path>,
        offset_ns: u64,
    ) -> Result<Collection, TargetError> {
        let path = validated_relative(relative_path.as_ref())?;
        let started = Instant::now();
        let root = self.cgroup_root.join(path);
        let cpu = read_map(root.join("cpu.stat"));
        let memory_current = read_number(root.join("memory.current"));
        let memory_peak = read_number(root.join("memory.peak"));
        let memory = read_map(root.join("memory.stat"));
        let io = read_text(root.join("io.stat")).and_then(|text| parse_io_stat(&text));
        let cpu_pressure =
            read_text(root.join("cpu.pressure")).and_then(|text| parse_pressure(&text));
        let memory_pressure =
            read_text(root.join("memory.pressure")).and_then(|text| parse_pressure(&text));
        let io_pressure =
            read_text(root.join("io.pressure")).and_then(|text| parse_pressure(&text));
        let mut samples = Vec::with_capacity(18);
        for (id, key) in [
            ("cgroup.cpu.usage", "usage_usec"),
            ("cgroup.cpu.user", "user_usec"),
            ("cgroup.cpu.system", "system_usec"),
            ("cgroup.cpu.throttled_time", "throttled_usec"),
        ] {
            let value = cpu
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|v| required(v, key))
                .and_then(usec_to_ns);
            push(
                &mut samples,
                descriptor(id, "ns", "cgroup", "cgroup2:cpu.stat", 1_000),
                offset_ns,
                value,
            );
        }
        for (id, key) in [
            ("cgroup.cpu.periods", "nr_periods"),
            ("cgroup.cpu.throttled_periods", "nr_throttled"),
        ] {
            let value = cpu
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|v| required(v, key));
            push(
                &mut samples,
                descriptor(id, "{event}", "cgroup", "cgroup2:cpu.stat", 0),
                offset_ns,
                value,
            );
        }
        push(
            &mut samples,
            descriptor(
                "cgroup.memory.current",
                "By",
                "cgroup",
                "cgroup2:memory.current",
                0,
            ),
            offset_ns,
            memory_current,
        );
        push(
            &mut samples,
            descriptor(
                "cgroup.memory.peak",
                "By",
                "cgroup",
                "cgroup2:memory.peak",
                0,
            ),
            offset_ns,
            memory_peak,
        );
        for (id, key) in [
            ("cgroup.faults.minor", "pgfault"),
            ("cgroup.faults.major", "pgmajfault"),
        ] {
            let value = memory
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|v| required(v, key));
            push(
                &mut samples,
                descriptor(id, "{fault}", "cgroup", "cgroup2:memory.stat", 0),
                offset_ns,
                value,
            );
        }
        for (id, key) in [("cgroup.io.read", "rbytes"), ("cgroup.io.write", "wbytes")] {
            let value = io
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|v| required(v, key));
            push(
                &mut samples,
                descriptor(id, "By", "cgroup", "cgroup2:io.stat", 0),
                offset_ns,
                value,
            );
        }
        for (resource, pressure) in [
            ("cpu", &cpu_pressure),
            ("memory", &memory_pressure),
            ("io", &io_pressure),
        ] {
            for kind in ["some", "full"] {
                let value = pressure
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|v| required(v, kind))
                    .and_then(usec_to_ns);
                push(
                    &mut samples,
                    descriptor(
                        &format!("cgroup.pressure.{resource}.{kind}"),
                        "ns",
                        "cgroup",
                        &format!("cgroup2:{resource}.pressure"),
                        1_000,
                    ),
                    offset_ns,
                    value,
                );
            }
        }
        Ok(Collection::new(samples, started))
    }

    /// Resolve and collect the calling process's unified cgroup.
    pub fn collect_self_cgroup(&self, offset_ns: u64) -> Result<Collection, TargetError> {
        let membership = fs::read_to_string(self.proc_root.join("self/cgroup"))
            .map_err(|_| TargetError::UnifiedCgroupUnavailable)?;
        self.collect_cgroup(parse_unified_cgroup(&membership)?, offset_ns)
    }
}

#[derive(Clone, Copy)]
struct ProcessStat {
    minor_faults: u64,
    major_faults: u64,
    user_ns: u64,
    system_ns: u64,
    rss_bytes: u64,
}

fn parse_process_stat(
    text: &str,
    page_size: u64,
    ticks_per_second: u64,
) -> Result<ProcessStat, String> {
    if ticks_per_second == 0 {
        return Err("invalid zero clock-tick frequency".into());
    }
    let close = text
        .rfind(')')
        .ok_or_else(|| "malformed proc stat".to_owned())?;
    let fields: Vec<_> = text[close + 1..].split_whitespace().collect();
    if fields.len() < 22 {
        return Err("malformed proc stat".into());
    }
    Ok(ProcessStat {
        minor_faults: parse_u64(fields[7], "proc minor faults")?,
        major_faults: parse_u64(fields[9], "proc major faults")?,
        user_ns: ticks_to_ns(parse_u64(fields[11], "proc user ticks")?, ticks_per_second)?,
        system_ns: ticks_to_ns(
            parse_u64(fields[12], "proc system ticks")?,
            ticks_per_second,
        )?,
        rss_bytes: parse_u64(fields[21], "proc resident pages")?
            .checked_mul(page_size)
            .ok_or_else(|| "proc resident byte count overflow".to_owned())?,
    })
}

fn parse_key_values(text: &str) -> Result<BTreeMap<String, u64>, String> {
    let mut values = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 2 {
            return Err("malformed or duplicate key-value line".into());
        }
        let key = fields[0].strip_suffix(':').unwrap_or(fields[0]);
        if key.is_empty() || values.contains_key(key) {
            return Err("malformed or duplicate key-value line".into());
        }
        values.insert(key.to_owned(), parse_u64(fields[1], "counter")?);
    }
    Ok(values)
}

fn parse_io_stat(text: &str) -> Result<BTreeMap<String, u64>, String> {
    let mut totals = BTreeMap::from([("rbytes".to_owned(), 0_u64), ("wbytes".to_owned(), 0)]);
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let device = fields
            .next()
            .ok_or_else(|| "malformed io.stat line".to_owned())?;
        let (major, minor) = device
            .split_once(':')
            .ok_or_else(|| "malformed io.stat device".to_owned())?;
        parse_u64(major, "device major")?;
        parse_u64(minor, "device minor")?;
        let mut seen = BTreeMap::new();
        for field in fields {
            let (key, raw) = field
                .split_once('=')
                .ok_or_else(|| "malformed io.stat counter".to_owned())?;
            if seen.insert(key, ()).is_some() {
                return Err("duplicate io.stat counter".into());
            }
            let value = parse_u64(raw, "io.stat counter")?;
            if let Some(total) = totals.get_mut(key) {
                *total = total
                    .checked_add(value)
                    .ok_or_else(|| "io.stat counter overflow".to_owned())?;
            }
        }
    }
    Ok(totals)
}

fn parse_pressure(text: &str) -> Result<BTreeMap<String, u64>, String> {
    let mut totals = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let kind = fields
            .next()
            .ok_or_else(|| "malformed pressure line".to_owned())?;
        if !matches!(kind, "some" | "full") {
            return Err("unknown pressure class".into());
        }
        let mut total = None;
        for field in fields {
            let (key, raw) = field
                .split_once('=')
                .ok_or_else(|| "malformed pressure field".to_owned())?;
            match key {
                "total" if total.is_none() => total = Some(parse_u64(raw, "pressure total")?),
                "avg10" | "avg60" | "avg300" if raw.parse::<f64>().is_ok_and(f64::is_finite) => {}
                "total" => return Err("duplicate pressure total".into()),
                "avg10" | "avg60" | "avg300" => return Err("malformed pressure average".into()),
                _ => return Err("unknown pressure field".into()),
            }
        }
        if totals
            .insert(
                kind.to_owned(),
                total.ok_or_else(|| "missing pressure total".to_owned())?,
            )
            .is_some()
        {
            return Err("duplicate pressure class".into());
        }
    }
    Ok(totals)
}

fn parse_unified_cgroup(text: &str) -> Result<PathBuf, TargetError> {
    let mut found = text.lines().filter_map(|line| line.strip_prefix("0::"));
    let raw = found.next().ok_or(TargetError::UnifiedCgroupUnavailable)?;
    if found.next().is_some() {
        return Err(TargetError::UnifiedCgroupUnavailable);
    }
    let relative = raw
        .strip_prefix('/')
        .ok_or(TargetError::UnifiedCgroupUnavailable)?;
    validated_relative(Path::new(relative))
}

fn validated_relative(path: &Path) -> Result<PathBuf, TargetError> {
    if path.is_absolute() {
        return Err(TargetError::AbsoluteCgroupPath);
    }
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => result.push(value),
            _ => return Err(TargetError::UnsafeCgroupPath),
        }
    }
    Ok(result)
}

fn read_text(path: PathBuf) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| unavailable_reason(&error))
}

fn read_map(path: PathBuf) -> Result<BTreeMap<String, u64>, String> {
    read_text(path).and_then(|text| parse_key_values(&text))
}

fn read_number(path: PathBuf) -> Result<u64, String> {
    read_text(path).and_then(|text| parse_u64(text.trim(), "unsigned integer"))
}

fn unavailable_reason(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound => "kernel metric source is absent".into(),
        io::ErrorKind::PermissionDenied => "kernel metric source permission denied".into(),
        kind => format!("kernel metric source read failed: {kind:?}"),
    }
}

fn parse_u64(raw: &str, label: &str) -> Result<u64, String> {
    raw.parse().map_err(|_| format!("malformed {label}"))
}

fn required(values: &BTreeMap<String, u64>, key: &str) -> Result<u64, String> {
    values
        .get(key)
        .copied()
        .ok_or_else(|| format!("missing {key} counter"))
}

fn ticks_to_ns(ticks: u64, frequency: u64) -> Result<u64, String> {
    let value = u128::from(ticks) * NS_PER_SECOND / u128::from(frequency);
    u64::try_from(value).map_err(|_| "CPU time overflow".into())
}

fn usec_to_ns(value: u64) -> Result<u64, String> {
    value
        .checked_mul(1_000)
        .ok_or_else(|| "microsecond counter overflow".into())
}

fn cpu_resolution(frequency: u64) -> u64 {
    if frequency == 0 {
        0
    } else {
        u64::try_from(NS_PER_SECOND / u128::from(frequency)).unwrap_or(u64::MAX)
    }
}

fn descriptor(
    id: &str,
    unit: &str,
    scope: &str,
    source: &str,
    resolution_ns: u64,
) -> MetricDescriptor {
    MetricDescriptor {
        metric_id: Id(id.into()),
        unit: unit.into(),
        aggregation: if id == "process.memory.resident" || id.starts_with("cgroup.memory.") {
            Aggregation::Gauge
        } else {
            Aggregation::Counter
        },
        scope: scope.into(),
        source: source.into(),
        resolution_ns,
    }
}

fn push(
    samples: &mut Vec<MetricSample>,
    descriptor: MetricDescriptor,
    offset_ns: u64,
    result: Result<u64, String>,
) {
    let value = match result {
        Ok(raw) => MetricValue::Available {
            #[allow(clippy::cast_precision_loss)]
            value: raw as f64,
        },
        Err(reason) => MetricValue::Unavailable { reason },
    };
    samples.push(MetricSample {
        descriptor,
        offset_ns,
        value,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_stat_name_and_exact_units() {
        let text = "42 (worker ) name) R 1 2 3 4 5 6 7 11 8 13 9 17 19 20 21 22 23 24 25 26 27 29";
        let parsed = parse_process_stat(text, 4096, 100).unwrap();
        assert_eq!(parsed.minor_faults, 7);
        assert_eq!(parsed.major_faults, 8);
        assert_eq!(parsed.user_ns, 90_000_000);
        assert_eq!(parsed.system_ns, 170_000_000);
        assert_eq!(parsed.rss_bytes, 27 * 4096);
    }

    #[test]
    fn parsers_fail_closed_on_malformed_duplicate_and_overflow() {
        assert!(parse_process_stat("bad", 4096, 100).is_err());
        assert!(parse_key_values("a 1\na 2\n").is_err());
        assert!(parse_io_stat("bad rbytes=1\n").is_err());
        assert!(parse_io_stat("8:0 rbytes=18446744073709551615\n8:1 rbytes=1\n").is_err());
        assert!(parse_pressure("partial avg10=0 total=1\n").is_err());
        assert!(parse_pressure("some avg10=nan total=1\n").is_err());
        assert!(usec_to_ns(u64::MAX).is_err());
        assert!(ticks_to_ns(u64::MAX, 1).is_err());
    }

    #[test]
    fn io_and_pressure_aggregate_as_documented() {
        let proc_io = parse_key_values("read_bytes: 3\nwrite_bytes: 5\n").unwrap();
        assert_eq!(proc_io["read_bytes"], 3);
        let io = parse_io_stat("8:0 rbytes=10 wbytes=20 rios=1\n8:16 rbytes=7 wbytes=9\n").unwrap();
        assert_eq!(io["rbytes"], 17);
        assert_eq!(io["wbytes"], 29);
        let pressure = parse_pressure(
            "some avg10=0.0 avg60=0 avg300=0 total=4\nfull avg10=0 avg60=0 avg300=0 total=2\n",
        )
        .unwrap();
        assert_eq!(pressure["some"], 4);
        assert_eq!(pressure["full"], 2);
    }

    #[test]
    fn unavailable_reasons_are_typed_and_redacted() {
        assert_eq!(
            unavailable_reason(&io::Error::from(io::ErrorKind::NotFound)),
            "kernel metric source is absent"
        );
        assert_eq!(
            unavailable_reason(&io::Error::from(io::ErrorKind::PermissionDenied)),
            "kernel metric source permission denied"
        );
        assert_eq!(
            unavailable_reason(&io::Error::other("private path")),
            "kernel metric source read failed: Other"
        );
    }

    #[test]
    fn cgroup_paths_and_membership_fail_closed() {
        assert_eq!(
            validated_relative(Path::new("../escape")),
            Err(TargetError::UnsafeCgroupPath)
        );
        assert_eq!(
            validated_relative(Path::new("/absolute")),
            Err(TargetError::AbsoluteCgroupPath)
        );
        assert_eq!(
            parse_unified_cgroup("0::/team/job\n"),
            Ok(PathBuf::from("team/job"))
        );
        assert_eq!(
            parse_unified_cgroup("2:cpu:/old\n"),
            Err(TargetError::UnifiedCgroupUnavailable)
        );
        assert_eq!(
            parse_unified_cgroup("0::/one\n0::/two\n"),
            Err(TargetError::UnifiedCgroupUnavailable)
        );
    }

    #[test]
    fn absent_process_is_unavailable_never_zero() {
        let collector = LinuxCollector::with_roots("/definitely-absent-asb-proc", "/unused");
        let collection = collector.collect_process(7, 55);
        assert_eq!(collection.evidence.attempted_values, 7);
        assert_eq!(collection.evidence.available_values, 0);
        assert_eq!(collection.evidence.unavailable_values, 7);
        assert!(
            collection
                .samples
                .iter()
                .all(|sample| matches!(sample.value, MetricValue::Unavailable { .. }))
        );
        assert!(
            collection
                .samples
                .iter()
                .all(|sample| sample.offset_ns == 55)
        );
    }

    #[test]
    fn sampling_accounting_exposes_loss_and_unavailable_values() {
        let collection = Collection {
            samples: Vec::new(),
            evidence: CollectionEvidence {
                attempted_values: 3,
                available_values: 2,
                unavailable_values: 1,
                collection_time_ns: 17,
            },
        };
        let mut evidence = SamplingEvidence::new(2);
        evidence.record_collection(&collection).unwrap();
        evidence.record_lost_sample().unwrap();
        assert!(evidence.is_complete());
        assert_eq!(evidence.unavailable_values(), 1);
        assert_eq!(evidence.collection_time_ns(), 17);
        assert_eq!(evidence.max_collection_time_ns(), 17);
        assert_eq!(
            evidence.record_lost_sample(),
            Err(SamplingError::ScheduleExhausted)
        );
    }
}
