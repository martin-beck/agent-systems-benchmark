// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Typed command dependency inventory and side-effect-free preflight.
#![allow(missing_docs)]

use crate::tool_catalog;
use asb_config::{ProjectToolStatus, decode_project_config};
use serde::Serialize;
use serde_json::json;
use std::path::Path;

const VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    Tool,
    Workload,
    Project,
    Host,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Dependency {
    pub id: &'static str,
    pub kind: DependencyKind,
    pub optional: bool,
    pub remediation: &'static str,
}

/// The authoritative dependency map for public execution paths.
pub const DEPENDENCIES: &[Dependency] = &[
    Dependency {
        id: "aider",
        kind: DependencyKind::Tool,
        optional: false,
        remediation: "asb tool install aider --kind agent --source fixture://aider --version <pinned>",
    },
    Dependency {
        id: "codex",
        kind: DependencyKind::Tool,
        optional: false,
        remediation: "asb tool install codex --kind agent --source fixture://codex --version <pinned>",
    },
    Dependency {
        id: "gemini",
        kind: DependencyKind::Tool,
        optional: false,
        remediation: "asb tool install gemini --kind agent --source fixture://gemini --version <pinned>",
    },
    Dependency {
        id: "goose",
        kind: DependencyKind::Tool,
        optional: false,
        remediation: "asb tool install goose --kind agent --source fixture://goose --version <pinned>",
    },
    Dependency {
        id: "mini-swe",
        kind: DependencyKind::Tool,
        optional: false,
        remediation: "asb tool install mini-swe --kind agent --source fixture://mini-swe --version <pinned>",
    },
    Dependency {
        id: "opencode",
        kind: DependencyKind::Tool,
        optional: false,
        remediation: "asb tool install opencode --kind agent --source fixture://opencode --version <pinned>",
    },
    Dependency {
        id: "opendesk",
        kind: DependencyKind::Tool,
        optional: false,
        remediation: "asb tool install opendesk --kind agent --source fixture://opendesk --version <pinned>",
    },
    Dependency {
        id: "openhands",
        kind: DependencyKind::Tool,
        optional: false,
        remediation: "asb tool install openhands --kind agent --source fixture://openhands --version <pinned>",
    },
    Dependency {
        id: "openjiuwen",
        kind: DependencyKind::Tool,
        optional: false,
        remediation: "asb tool install openjiuwen --kind agent --source fixture://openjiuwen --version <pinned>",
    },
    Dependency {
        id: "qwen-code",
        kind: DependencyKind::Tool,
        optional: false,
        remediation: "asb tool install qwen-code --kind agent --source fixture://qwen-code --version <pinned>",
    },
    Dependency {
        id: "cli2key",
        kind: DependencyKind::Tool,
        optional: false,
        remediation: "asb tool install cli2key --kind support --source fixture://cli2key --version <pinned>",
    },
    Dependency {
        id: "perf",
        kind: DependencyKind::Host,
        optional: true,
        remediation: "install perf using the host distribution package manager",
    },
    Dependency {
        id: "bpftool",
        kind: DependencyKind::Host,
        optional: true,
        remediation: "install bpftool using the host distribution package manager",
    },
    Dependency {
        id: "linux",
        kind: DependencyKind::Host,
        optional: false,
        remediation: "run ASB on a Linux host",
    },
    Dependency {
        id: "cgroup-v2",
        kind: DependencyKind::Host,
        optional: false,
        remediation: "mount and delegate a cgroup v2 hierarchy to the user",
    },
    Dependency {
        id: "user-systemd",
        kind: DependencyKind::Host,
        optional: false,
        remediation: "enable user systemd delegation for the benchmark user",
    },
    Dependency {
        id: "bubblewrap",
        kind: DependencyKind::Host,
        optional: false,
        remediation: "install a distribution-provided bubblewrap executable",
    },
    Dependency {
        id: "kernel-btf",
        kind: DependencyKind::Host,
        optional: true,
        remediation: "boot a kernel exposing BTF at /sys/kernel/btf/vmlinux",
    },
    Dependency {
        id: "kernel-pmu",
        kind: DependencyKind::Host,
        optional: true,
        remediation: "grant the benchmark user permitted PMU access",
    },
];

/// Public execution scopes and the dependencies they may require. Keeping
/// this mapping separate from the inventory makes command coverage explicit
/// without allowing a command to silently inherit an ambient tool.
pub const DEPENDENCY_SCOPES: &[(&str, &[&str])] = &[
    (
        "run",
        &[
            "aider",
            "codex",
            "gemini",
            "goose",
            "mini-swe",
            "opencode",
            "opendesk",
            "openhands",
            "openjiuwen",
            "qwen-code",
            "cli2key",
        ],
    ),
    (
        "sweep",
        &[
            "aider",
            "codex",
            "gemini",
            "goose",
            "mini-swe",
            "opencode",
            "opendesk",
            "openhands",
            "openjiuwen",
            "qwen-code",
            "cli2key",
        ],
    ),
    (
        "benchmark-live",
        &[
            "aider",
            "codex",
            "gemini",
            "goose",
            "mini-swe",
            "opencode",
            "opendesk",
            "openhands",
            "openjiuwen",
            "qwen-code",
            "cli2key",
        ],
    ),
    (
        "preflight",
        &[
            "aider",
            "codex",
            "gemini",
            "goose",
            "mini-swe",
            "opencode",
            "opendesk",
            "openhands",
            "openjiuwen",
            "qwen-code",
            "cli2key",
            "perf",
            "bpftool",
            "linux",
            "cgroup-v2",
            "user-systemd",
            "bubblewrap",
            "kernel-btf",
            "kernel-pmu",
        ],
    ),
];

#[derive(Serialize)]
pub struct ResultView<'a> {
    pub schema_version: u16,
    pub command: &'static str,
    pub ready: bool,
    pub dependencies: Vec<Check<'a>>,
}
#[derive(Serialize)]
pub struct Check<'a> {
    pub id: &'a str,
    pub kind: DependencyKind,
    pub optional: bool,
    pub available: bool,
    pub remediation: &'a str,
}

pub fn preflight(project: &Path) -> Result<ResultView<'static>, String> {
    preflight_for("preflight", project)
}

pub fn preflight_for(command: &str, project: &Path) -> Result<ResultView<'static>, String> {
    let scope = DEPENDENCY_SCOPES
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, ids)| *ids)
        .ok_or_else(|| "dependency scope is unknown".to_owned())?;
    let catalog = tool_catalog::catalog();
    tool_catalog::validate(&catalog).map_err(str::to_owned)?;
    let config_path = project.join(".asb/project.json");
    let config = if config_path.is_file() {
        let bytes = std::fs::read(&config_path)
            .map_err(|_| "project configuration is unreadable".to_owned())?;
        Some(
            decode_project_config(&bytes)
                .map_err(|_| "project configuration is invalid".to_owned())?,
        )
    } else {
        None
    };
    let checks = DEPENDENCIES
        .iter()
        .filter(|d| scope.contains(&d.id))
        .map(|d| {
            let available = match d.kind {
                DependencyKind::Tool => {
                    catalog.entries.iter().any(|e| e.id == d.id)
                        && config.as_ref().is_some_and(|c| tool_available(c, d.id))
                }
                DependencyKind::Workload | DependencyKind::Project => true,
                DependencyKind::Host => host_available(d.id),
            };
            Check {
                id: d.id,
                kind: d.kind,
                optional: d.optional,
                available,
                remediation: d.remediation,
            }
        })
        .collect::<Vec<_>>();
    let ready = checks.iter().all(|c| c.available || c.optional);
    Ok(ResultView {
        schema_version: VERSION,
        command: "preflight",
        ready,
        dependencies: checks,
    })
}

fn tool_available(config: &asb_config::ProjectConfigV1, id: &str) -> bool {
    config
        .agents
        .get(id)
        .or_else(|| config.harnesses.get(id))
        .or_else(|| config.benchmarks.get(id))
        .or_else(|| config.workloads.get(id))
        .or_else(|| config.support_tools.get(id))
        .is_some_and(|record| record.status == ProjectToolStatus::Available)
}

fn host_available(id: &str) -> bool {
    match id {
        "linux" => std::env::consts::OS == "linux",
        "cgroup-v2" => Path::new("/sys/fs/cgroup/cgroup.controllers").is_file(),
        "user-systemd" => std::env::var_os("XDG_RUNTIME_DIR").is_some(),
        "bubblewrap" => ["/usr/bin/bwrap", "/bin/bwrap"]
            .iter()
            .any(|p| Path::new(p).is_file()),
        "perf" => ["/usr/bin/perf", "/bin/perf"]
            .iter()
            .any(|p| Path::new(p).is_file()),
        "bpftool" => ["/usr/sbin/bpftool", "/usr/bin/bpftool", "/bin/bpftool"]
            .iter()
            .any(|p| Path::new(p).is_file()),
        "kernel-btf" => Path::new("/sys/kernel/btf/vmlinux").is_file(),
        "kernel-pmu" => Path::new("/sys/bus/event_source/devices/cpu").is_dir(),
        _ => false,
    }
}

pub fn registry_json() -> serde_json::Value {
    json!({"schema_version": VERSION, "command": "dependency-registry", "dependencies": DEPENDENCIES})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_ids_are_complete() {
        for id in [
            "aider",
            "codex",
            "gemini",
            "goose",
            "mini-swe",
            "opencode",
            "opendesk",
            "openhands",
            "openjiuwen",
            "qwen-code",
            "cli2key",
            "perf",
            "bpftool",
        ] {
            assert!(DEPENDENCIES.iter().any(|d| d.id == id));
        }
    }
    #[test]
    fn host_dependencies_are_not_tools() {
        assert_eq!(
            DEPENDENCIES.iter().find(|d| d.id == "linux").unwrap().kind,
            DependencyKind::Host
        );
        assert!(
            !DEPENDENCIES
                .iter()
                .filter(|d| d.kind == DependencyKind::Host)
                .any(|d| d.remediation.starts_with("asb tool install"))
        );
        assert_eq!(
            DEPENDENCIES.iter().find(|d| d.id == "perf").unwrap().kind,
            DependencyKind::Host
        );
        assert_eq!(
            DEPENDENCIES
                .iter()
                .find(|d| d.id == "bpftool")
                .unwrap()
                .kind,
            DependencyKind::Host
        );
    }
    #[test]
    fn tool_remediation_is_exactly_installable() {
        for dependency in DEPENDENCIES
            .iter()
            .filter(|d| d.kind == DependencyKind::Tool && !d.optional)
        {
            assert!(dependency.remediation.starts_with("asb tool install "));
            assert!(dependency.remediation.contains(dependency.id));
        }
    }
    #[test]
    fn preflight_is_bounded_and_side_effect_free() {
        let result = preflight(Path::new("/nonexistent")).unwrap();
        assert_eq!(result.schema_version, VERSION);
        assert!(!result.ready);
    }

    #[test]
    fn unknown_scope_is_rejected_and_known_scope_is_filtered() {
        assert!(preflight_for("not-a-command", Path::new("/nonexistent")).is_err());
        let result = preflight_for("run", Path::new("/nonexistent")).unwrap();
        assert!(result.dependencies.iter().all(|d| d.id != "linux"));
        assert!(result.dependencies.iter().any(|d| d.id == "aider"));
    }
}
