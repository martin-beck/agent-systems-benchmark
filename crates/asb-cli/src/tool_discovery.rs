//! Deterministic, read-only discovery of ASB tools.
//!
//! Discovery is deliberately kept separate from installation and project
//! configuration.  It only inspects a bounded list of known executable names
//! and never changes `.asb/project.json`.

use asb_config::{ProjectConfigV1, ProjectToolKind, ProjectToolRecordV1, ProjectToolStatus};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Version of the machine-readable discovery response.
pub const DISCOVERY_SCHEMA_VERSION: u16 = 1;
const MAX_PATH_ENTRIES: usize = 128;
const MAX_OUTPUT_BYTES: usize = 8 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_millis(750);

#[derive(Clone, Copy)]
struct KnownTool {
    name: &'static str,
    kind: ProjectToolKind,
    capabilities: &'static [&'static str],
}

// This is the complete probe allowlist.  A scan never executes a configured
// or PATH-discovered name outside this table.
const KNOWN_TOOLS: &[KnownTool] = &[
    KnownTool {
        name: "asb",
        kind: ProjectToolKind::Harness,
        capabilities: &["benchmark", "plan"],
    },
    KnownTool {
        name: "aider",
        kind: ProjectToolKind::Agent,
        capabilities: &["agent"],
    },
    KnownTool {
        name: "codex",
        kind: ProjectToolKind::Agent,
        capabilities: &["agent"],
    },
    KnownTool {
        name: "goose",
        kind: ProjectToolKind::Agent,
        capabilities: &["agent"],
    },
    KnownTool {
        name: "opencode",
        kind: ProjectToolKind::Agent,
        capabilities: &["agent"],
    },
    KnownTool {
        name: "opendesk",
        kind: ProjectToolKind::Agent,
        capabilities: &["agent"],
    },
    KnownTool {
        name: "qwen",
        kind: ProjectToolKind::Agent,
        capabilities: &["agent"],
    },
    KnownTool {
        name: "asb-benchmark",
        kind: ProjectToolKind::Benchmark,
        capabilities: &["benchmark"],
    },
    KnownTool {
        name: "swe-bench",
        kind: ProjectToolKind::Workload,
        capabilities: &["workload"],
    },
    KnownTool {
        name: "core-bench",
        kind: ProjectToolKind::Workload,
        capabilities: &["workload"],
    },
    KnownTool {
        name: "cargo",
        kind: ProjectToolKind::SupportTool,
        capabilities: &["build", "test"],
    },
    KnownTool {
        name: "rustc",
        kind: ProjectToolKind::SupportTool,
        capabilities: &["compile"],
    },
    KnownTool {
        name: "git",
        kind: ProjectToolKind::SupportTool,
        capabilities: &["source"],
    },
    KnownTool {
        name: "make",
        kind: ProjectToolKind::SupportTool,
        capabilities: &["build"],
    },
    KnownTool {
        name: "qemu-system-aarch64",
        kind: ProjectToolKind::SupportTool,
        capabilities: &["emulated_aarch64"],
    },
];

/// Origin and precedence class for a discovered tool.
#[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverySource {
    /// A record already present in `.asb/project.json`.
    Configured,
    /// An executable under a project-local tool root.
    ProjectLocal,
    /// An executable resolved from the bounded system `PATH` scan.
    Path,
}

/// One discovered agent, harness, benchmark, workload, or support tool.
#[derive(Debug, Serialize)]
pub struct DiscoveredTool {
    /// Stable tool name.
    pub name: String,
    /// Inventory class.
    pub kind: ProjectToolKind,
    /// Discovery source and precedence class.
    pub source: DiscoverySource,
    /// Whether this candidate wins deterministic precedence selection.
    pub selected: bool,
    /// Canonical executable path, when the candidate can be resolved.
    pub canonical_path: Option<String>,
    /// First bounded version line, when an allowlisted probe succeeds.
    pub version: Option<String>,
    /// Public capability labels.
    pub capabilities: Vec<String>,
    /// Availability classification.
    pub status: ProjectToolStatus,
    /// Stable diagnostic reason for unavailable or unselected candidates.
    pub reason: Option<String>,
}

/// Complete deterministic discovery response.
#[derive(Debug, Serialize)]
pub struct DiscoveryReport {
    /// Response schema version.
    pub schema_version: u16,
    /// True when the bounded scan completed, even if entries are unavailable.
    pub ok: bool,
    /// Canonical command identifier.
    pub command: &'static str,
    /// Canonical project root inspected by the scan.
    pub project_root: String,
    /// Sorted inventory entries.
    pub tools: Vec<DiscoveredTool>,
    /// Non-blocking development warnings.
    pub warnings: Vec<String>,
}

/// Run the bounded, read-only inventory scan.
///
/// `path_env` is injectable for deterministic fixtures; passing `None` uses
/// the process environment.  No configuration or project files are modified.
pub fn discover(
    root: &Path,
    config: Option<&ProjectConfigV1>,
    path_env: Option<OsString>,
) -> DiscoveryReport {
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut tools = Vec::new();
    let mut selected = BTreeSet::new();
    let mut configured = BTreeMap::new();
    if let Some(config) = config {
        for (name, record) in all_records(config) {
            let key = (record.kind, name.clone());
            configured.insert(key.clone(), record.clone());
            let item = inspect_record(&name, &record, &canonical_root);
            selected.insert(key);
            tools.push(item);
        }
    }

    let local_roots = [
        canonical_root.join(".asb/tools"),
        canonical_root.join("tools"),
        canonical_root.join("bin"),
    ];
    let mut seen = BTreeSet::new();
    for known in KNOWN_TOOLS {
        for local_root in &local_roots {
            let candidate = local_root.join(known.name);
            if !candidate_exists(&candidate) {
                continue;
            }
            let key = (known.kind, known.name.to_owned());
            if configured.contains_key(&key) {
                continue;
            }
            let identity = fs::canonicalize(&candidate).ok();
            if !seen.insert((key.0, key.1.clone(), identity.clone())) {
                continue;
            }
            tools.push(inspect_candidate(
                known,
                &candidate,
                DiscoverySource::ProjectLocal,
                &canonical_root,
                true,
            ));
            break;
        }
    }

    let path_value = path_env.unwrap_or_else(|| std::env::var_os("PATH").unwrap_or_default());
    for directory in std::env::split_paths(&path_value).take(MAX_PATH_ENTRIES) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        for known in KNOWN_TOOLS {
            let candidate = directory.join(known.name);
            if !candidate_exists(&candidate) {
                continue;
            }
            let key = (known.kind, known.name.to_owned());
            let identity = fs::canonicalize(&candidate).ok();
            if !seen.insert((key.0, key.1.clone(), identity.clone())) {
                continue;
            }
            let mut item = inspect_candidate(
                known,
                &candidate,
                DiscoverySource::Path,
                &canonical_root,
                false,
            );
            if configured.contains_key(&key) {
                item.selected = false;
                item.reason = Some("configured_record_takes_precedence".into());
            } else if !selected.insert(key) {
                item.selected = false;
                item.reason = Some("duplicate_lower_precedence".into());
            }
            tools.push(item);
        }
    }
    tools.sort_by(|left, right| {
        (&left.kind, &left.name, &left.source, &left.canonical_path).cmp(&(
            &right.kind,
            &right.name,
            &right.source,
            &right.canonical_path,
        ))
    });
    DiscoveryReport {
        schema_version: DISCOVERY_SCHEMA_VERSION,
        ok: true,
        command: "tool discover",
        project_root: canonical_root.display().to_string(),
        tools,
        warnings: vec!["development authentication, signatures, and keys are warning-only; discovery does not require them".into()],
    }
}

fn all_records(config: &ProjectConfigV1) -> Vec<(String, ProjectToolRecordV1)> {
    let mut records = Vec::new();
    for inventory in [
        &config.agents,
        &config.harnesses,
        &config.benchmarks,
        &config.workloads,
        &config.support_tools,
    ] {
        records.extend(
            inventory
                .iter()
                .map(|(name, record)| (name.clone(), record.clone())),
        );
    }
    records.sort_by(|left, right| (&left.1.kind, &left.0).cmp(&(&right.1.kind, &right.0)));
    records
}

fn inspect_record(name: &str, record: &ProjectToolRecordV1, root: &Path) -> DiscoveredTool {
    let Some(relative) = record.path.as_deref() else {
        return DiscoveredTool {
            name: name.into(),
            kind: record.kind,
            source: DiscoverySource::Configured,
            selected: true,
            canonical_path: None,
            version: Some(record.version.clone()),
            capabilities: record.capabilities.clone(),
            status: record.status,
            reason: Some("configured_record_has_no_local_path".into()),
        };
    };
    inspect_candidate_with(
        name,
        record.kind,
        &record.capabilities,
        &root.join(relative),
        DiscoverySource::Configured,
        root,
        true,
        Some(record.version.clone()),
    )
}

fn inspect_candidate(
    known: &KnownTool,
    path: &Path,
    source: DiscoverySource,
    root: &Path,
    project_local: bool,
) -> DiscoveredTool {
    inspect_candidate_with(
        known.name,
        known.kind,
        known.capabilities,
        path,
        source,
        root,
        project_local,
        None,
    )
}

fn inspect_candidate_with<T: ToString>(
    name: &str,
    kind: ProjectToolKind,
    capabilities: &[T],
    path: &Path,
    source: DiscoverySource,
    root: &Path,
    project_local: bool,
    configured_version: Option<String>,
) -> DiscoveredTool {
    let capabilities = capabilities
        .iter()
        .map(|item| item.to_string())
        .collect::<Vec<_>>();
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return unavailable(name, kind, source, capabilities, "missing"),
    };
    let canonical = match fs::canonicalize(path) {
        Ok(canonical) => canonical,
        Err(_) => return unavailable(name, kind, source, capabilities, "unsafe_path"),
    };
    if !metadata.is_file() && !metadata.file_type().is_symlink() {
        return unavailable_with_path(
            name,
            kind,
            source,
            capabilities,
            &canonical,
            "not_regular_file",
        );
    }
    if project_local && !canonical.starts_with(root) {
        return unavailable_with_path(name, kind, source, capabilities, &canonical, "unsafe_path");
    }
    if canonical
        .metadata()
        .map(|m| m.permissions().mode() & 0o111 == 0)
        .unwrap_or(true)
    {
        return unavailable_with_path(
            name,
            kind,
            source,
            capabilities,
            &canonical,
            "not_executable",
        );
    }
    let version = configured_version.or_else(|| probe_version(&canonical));
    let reason = if version.is_none() {
        Some("version_probe_unavailable".into())
    } else {
        None
    };
    DiscoveredTool {
        name: name.into(),
        kind,
        source,
        selected: true,
        canonical_path: Some(canonical.display().to_string()),
        version,
        capabilities,
        status: ProjectToolStatus::Available,
        reason,
    }
}

fn unavailable(
    name: &str,
    kind: ProjectToolKind,
    source: DiscoverySource,
    capabilities: Vec<String>,
    reason: &str,
) -> DiscoveredTool {
    DiscoveredTool {
        name: name.into(),
        kind,
        source,
        selected: true,
        canonical_path: None,
        version: None,
        capabilities,
        status: if reason == "missing" {
            ProjectToolStatus::Missing
        } else {
            ProjectToolStatus::Error
        },
        reason: Some(reason.into()),
    }
}
fn unavailable_with_path(
    name: &str,
    kind: ProjectToolKind,
    source: DiscoverySource,
    capabilities: Vec<String>,
    path: &Path,
    reason: &str,
) -> DiscoveredTool {
    let mut item = unavailable(name, kind, source, capabilities, reason);
    item.canonical_path = Some(path.display().to_string());
    item
}
fn candidate_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn probe_version(path: &Path) -> Option<String> {
    let mut child = Command::new(path)
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        if child.try_wait().ok()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let mut output = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut output);
    }
    if output.len() > MAX_OUTPUT_BYTES {
        output.truncate(MAX_OUTPUT_BYTES);
    }
    let line = String::from_utf8_lossy(&output)
        .lines()
        .next()?
        .trim()
        .to_owned();
    if line.is_empty() || line.len() > 256 {
        None
    } else {
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    #[test]
    fn path_discovery_is_deterministic_and_bounded() {
        let dir = tempdir().unwrap();
        let cargo = dir.path().join("cargo");
        fs::write(&cargo, b"#!/bin/sh\nprintf 'cargo 1.0\\n'").unwrap();
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
        let report = discover(
            dir.path(),
            None,
            Some(dir.path().as_os_str().to_os_string()),
        );
        assert!(
            report
                .tools
                .iter()
                .any(|tool| tool.name == "cargo" && tool.status == ProjectToolStatus::Available)
        );
        assert_eq!(
            serde_json::to_vec(&report).unwrap(),
            serde_json::to_vec(&discover(
                dir.path(),
                None,
                Some(dir.path().as_os_str().to_os_string())
            ))
            .unwrap()
        );
    }

    #[test]
    fn configured_missing_record_is_retained_without_mutation() {
        let dir = tempdir().unwrap();
        let mut config = ProjectConfigV1::empty();
        config.support_tools.insert(
            "cargo".into(),
            ProjectToolRecordV1 {
                kind: ProjectToolKind::SupportTool,
                source_ref: "https://example.invalid/cargo".into(),
                version: "1".into(),
                platform: "linux-x86_64".into(),
                path: Some(".asb/tools/cargo".into()),
                digest_sha256: None,
                capabilities: vec!["build".into()],
                status: ProjectToolStatus::Available,
            },
        );
        let report = discover(dir.path(), Some(&config), Some(OsString::new()));
        assert_eq!(report.tools[0].status, ProjectToolStatus::Missing);
        assert!(report.tools[0].selected);
    }

    #[test]
    fn project_local_symlink_escape_is_rejected() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let target = outside.path().join("cargo");
        fs::write(&target, b"cargo 1.0\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
        let local = dir.path().join("tools");
        fs::create_dir(&local).unwrap();
        symlink(&target, local.join("cargo")).unwrap();
        let report = discover(dir.path(), None, Some(OsString::new()));
        let cargo = report
            .tools
            .iter()
            .find(|tool| tool.name == "cargo" && tool.source == DiscoverySource::ProjectLocal)
            .unwrap();
        assert_eq!(cargo.status, ProjectToolStatus::Error);
        assert_eq!(cargo.reason.as_deref(), Some("unsafe_path"));
    }

    #[test]
    fn duplicate_path_entries_have_stable_precedence() {
        let first = tempdir().unwrap();
        let second = tempdir().unwrap();
        for (directory, version) in [(&first, "one"), (&second, "two")] {
            let cargo = directory.path().join("cargo");
            fs::write(&cargo, format!("#!/bin/sh\nprintf 'cargo {version}\\n'")).unwrap();
            fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();
        let report = discover(first.path(), None, Some(path));
        let cargo = report
            .tools
            .iter()
            .filter(|tool| tool.name == "cargo" && tool.source == DiscoverySource::Path)
            .collect::<Vec<_>>();
        assert_eq!(cargo.len(), 2);
        assert!(
            cargo
                .iter()
                .any(|tool| tool.selected && tool.version.as_deref() == Some("cargo one"))
        );
        assert!(
            cargo.iter().any(|tool| !tool.selected
                && tool.reason.as_deref() == Some("duplicate_lower_precedence"))
        );
    }
}
