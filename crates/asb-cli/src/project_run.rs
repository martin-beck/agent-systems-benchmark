// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Project-bound benchmark execution inputs and result references.
//!
//! The CLI keeps project discovery and catalog generation separate from
//! execution.  This module is the narrow point where an explicit project is
//! resolved into a checked inventory/catalog snapshot and bounded run roots.

use super::*;

/// Checked, project-local execution locations.
#[derive(Debug)]
pub(super) struct ProjectExecution {
    result_root: PathBuf,
    work_root: PathBuf,
}

impl ProjectExecution {
    /// Confirm that a setup operation targets an initialized real project.
    pub(super) fn validate_setup_project(path: &Path) -> Result<(), CliError> {
        let root = fs::canonicalize(path).map_err(|_| {
            CliError::legacy_validation(
                "ASB project path is unavailable; run `asb project init PATH` first",
            )
        })?;
        let metadata = fs::symlink_metadata(&root)
            .map_err(|_| CliError::legacy_validation("ASB project path cannot be inspected"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(CliError::legacy_validation(
                "ASB project path must be a real directory",
            ));
        }
        super::load_tool_project(&root).map(|_| ())
    }

    /// Resolve an initialized project and its catalog-selected execution inputs.
    pub(super) fn resolve(path: &Path) -> Result<Self, CliError> {
        let root = fs::canonicalize(path).map_err(|_| {
            CliError::legacy_validation(
                "ASB project path is unavailable; run `asb project init PATH` first",
            )
        })?;
        let metadata = fs::symlink_metadata(&root)
            .map_err(|_| CliError::legacy_validation("ASB project path cannot be inspected"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(CliError::legacy_validation(
                "ASB project path must be a real directory",
            ));
        }
        let config = super::load_tool_project(&root)?;
        super::project_catalog::validate_active_execution_catalogs(&root, &config)?;
        validate_required_selections(&config)?;

        let result_root = root.join(&config.roots.results);
        let work_root = root.join(".asb").join("work");
        ensure_project_child(&root, &result_root, "project results directory")?;
        ensure_project_child(&root, &work_root, "project work directory")?;
        Ok(Self {
            result_root,
            work_root,
        })
    }

    /// Replace caller-selected output roots with the initialized project roots.
    pub(super) fn bind_plan(&self, mut plan: PlanFile) -> PlanFile {
        plan.result_root.clone_from(&self.result_root);
        plan.work_root.clone_from(&self.work_root);
        plan
    }

    /// Reject a report/compare run reference that is outside project results.
    pub(super) fn validate_run_reference(&self, path: &Path) -> Result<(), CliError> {
        let canonical = fs::canonicalize(path)
            .map_err(|_| CliError::legacy_validation("project run reference is unavailable"))?;
        if !canonical.starts_with(&self.result_root) {
            return Err(CliError::legacy_validation(
                "run reference is outside this project's results; use `asb report RUN --project PATH` with a project-owned run",
            ));
        }
        Ok(())
    }
}

fn ensure_project_child(root: &Path, child: &Path, label: &'static str) -> Result<(), CliError> {
    if !child.starts_with(root) {
        return Err(CliError::legacy_validation("project layout is unsafe"));
    }
    if let Some(parent) = child.parent() {
        let canonical_parent = fs::canonicalize(parent)
            .map_err(|_| CliError::legacy_validation("project layout parent is unavailable"))?;
        if !canonical_parent.starts_with(root) {
            return Err(CliError::legacy_validation(
                "project layout parent escapes project",
            ));
        }
    }
    let metadata = match fs::symlink_metadata(child) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            return Err(CliError::legacy_validation(
                "project layout directory is unavailable",
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CliError::legacy_validation(label));
    }
    Ok(())
}

fn validate_required_selections(config: &ProjectConfigV1) -> Result<(), CliError> {
    for (kind, selected, inventory) in [
        ("agent", config.selections.agent.as_ref(), &config.agents),
        (
            "harness",
            config.selections.harness.as_ref(),
            &config.harnesses,
        ),
        (
            "benchmark",
            config.selections.benchmark.as_ref(),
            &config.benchmarks,
        ),
        (
            "workload",
            config.selections.workload.as_ref(),
            &config.workloads,
        ),
    ] {
        let name = selected.ok_or_else(|| {
            CliError::legacy_validation(
                "project selection is missing; run `asb tool select ID --kind KIND --project PATH`, then regenerate and select catalogs",
            )
        })?;
        let record = inventory.get(name).ok_or_else(|| {
            CliError::legacy_validation(
                "project selection is stale; run `asb tool discover PATH`, repair or install the tool, then regenerate and select catalogs",
            )
        })?;
        if record.status != ProjectToolStatus::Available {
            return Err(CliError::legacy_validation(
                "project selected tool is unavailable; run `asb tool discover PATH`, repair or install the tool, then regenerate and select catalogs",
            ));
        }
        let _ = kind;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn project() -> tempfile::TempDir {
        let directory = tempdir().unwrap();
        super::super::project_init_with_progress(
            &[directory.path().display().to_string()],
            &mut Vec::new(),
            &mut Vec::new(),
            false,
        )
        .unwrap();
        directory
    }

    #[test]
    fn project_execution_requires_catalogs_and_selected_tools() {
        let directory = project();
        let error = ProjectExecution::resolve(directory.path()).unwrap_err();
        assert!(error.message.contains("active catalog is missing"));
    }

    #[test]
    fn project_run_reference_must_be_under_project_results() {
        let directory = project();
        let result = directory.path().join("results");
        let outside = tempdir().unwrap();
        let execution = ProjectExecution {
            result_root: fs::canonicalize(&result).unwrap(),
            work_root: directory.path().join(".asb/work"),
        };
        assert!(execution.validate_run_reference(outside.path()).is_err());
    }
}
