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
    root: PathBuf,
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
        validate_required_selections(&root, &config)?;

        let result_root = root.join(&config.roots.results);
        let work_root = root.join(".asb").join("work");
        ensure_project_child(&root, &result_root, "project results directory")?;
        ensure_project_child(&root, &work_root, "project work directory")?;
        Ok(Self {
            root,
            result_root,
            work_root,
        })
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    /// Replace caller-selected output roots with the initialized project roots.
    pub(super) fn bind_plan(&self, mut plan: PlanFile) -> Result<PlanFile, CliError> {
        self.validate_plan_identity(&plan)?;
        plan.result_root.clone_from(&self.result_root);
        plan.work_root.clone_from(&self.work_root);
        Ok(plan)
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

fn validate_required_selections(root: &Path, config: &ProjectConfigV1) -> Result<(), CliError> {
    for (kind, selected, inventory) in [
        (
            ProjectToolKind::Agent,
            config.selections.agent.as_ref(),
            &config.agents,
        ),
        (
            ProjectToolKind::Harness,
            config.selections.harness.as_ref(),
            &config.harnesses,
        ),
        (
            ProjectToolKind::Benchmark,
            config.selections.benchmark.as_ref(),
            &config.benchmarks,
        ),
        (
            ProjectToolKind::Workload,
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
        super::project_catalog::validate_selected_catalog_entry(root, config, kind, name, record)?;
    }
    Ok(())
}

impl ProjectExecution {
    fn validate_plan_identity(&self, plan: &PlanFile) -> Result<(), CliError> {
        self.validate_workload_identity(
            &plan.workload,
            &plan.experiment.workload.workload_revision,
            &plan.experiment.workload.workload_sha256,
        )
    }

    /// Validate that a plan consumes the selected installed workload receipt.
    ///
    /// Project inventory records are mutable discovery state; execution must
    /// additionally bind the immutable workload receipt and prepared adapter
    /// tree. This rejects stale, revoked, manually substituted, or ambient
    /// workload identities before any run roots are prepared.
    pub(super) fn validate_workload_identity(
        &self,
        workload_id: &str,
        workload_revision: &str,
        workload_digest: &str,
    ) -> Result<(), CliError> {
        let config = super::load_tool_project(&self.root)?;
        let Some(record) = config.workloads.get(workload_id) else {
            // Standalone/built-in workload plans have no project inventory
            // record and retain the pre-existing project execution path only
            // when this project has no selected workload. Once a project
            // workload is selected, an unknown plan identity must fail closed.
            if config.selections.workload.is_some()
                && OriginalWorkloads::describe(workload_id).is_err()
            {
                return Err(CliError::legacy_validation(
                    "plan workload is not registered in the selected project inventory",
                ));
            }
            return Ok(());
        };
        // Legacy project tool fixtures remain valid workload records. The
        // receipt fence applies to bundles installed through `asb workload`,
        // which are explicitly marked as external-workload.
        if !record
            .capabilities
            .iter()
            .any(|capability| capability == "external-workload")
        {
            return Ok(());
        }
        if config.selections.workload.as_deref() != Some(workload_id) {
            return Err(CliError::legacy_validation(
                "plan workload is not the selected installed project workload",
            ));
        }
        if record.version != workload_revision
            || !record
                .capabilities
                .iter()
                .any(|capability| capability == "adapter-ready")
        {
            return Err(CliError::legacy_validation(
                "selected installed workload adapter metadata is stale or incomplete",
            ));
        }
        if record.status != ProjectToolStatus::Available
            || record.kind != ProjectToolKind::Workload
            || record.platform != super::platform_label()
            || record.digest_sha256.as_deref() != Some(workload_digest)
        {
            return Err(CliError::legacy_validation(
                "selected installed workload identity is stale or unavailable",
            ));
        }
        let relative = record.path.as_deref().ok_or_else(|| {
            CliError::legacy_validation("selected installed workload has no project-local path")
        })?;
        let root = self.root.join(relative);
        if !root.starts_with(&self.root) {
            return Err(CliError::legacy_validation(
                "selected workload path escapes the project",
            ));
        }
        let root_meta = fs::symlink_metadata(&root).map_err(|_| {
            CliError::legacy_validation("selected workload installation is missing")
        })?;
        if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
            return Err(CliError::legacy_validation(
                "selected workload installation is not a real directory",
            ));
        }
        let prepared = root.join("prepared");
        let prepared_meta = fs::symlink_metadata(&prepared)
            .map_err(|_| CliError::legacy_validation("selected workload preparation is missing"))?;
        if prepared_meta.file_type().is_symlink() || !prepared_meta.is_dir() {
            return Err(CliError::legacy_validation(
                "selected workload preparation is not usable",
            ));
        }
        let bundle_path = root.join("bundle.archive");
        let bundle_meta = fs::symlink_metadata(&bundle_path)
            .map_err(|_| CliError::legacy_validation("selected workload archive is missing"))?;
        if bundle_meta.file_type().is_symlink() || !bundle_meta.is_file() {
            return Err(CliError::legacy_validation(
                "selected workload archive is not a regular file",
            ));
        }
        let bundle = fs::read(&bundle_path)
            .map_err(|_| CliError::legacy_validation("selected workload archive is unreadable"))?;
        if format!("{:x}", Sha256::digest(&bundle)) != workload_digest {
            return Err(CliError::legacy_validation(
                "selected workload archive digest does not match the selected identity",
            ));
        }
        let receipt_path = root.join("receipt.json");
        let receipt = fs::read(&receipt_path)
            .map_err(|_| CliError::legacy_validation("selected workload receipt is missing"))?;
        if receipt.len() > 64 * 1024 {
            return Err(CliError::legacy_validation(
                "selected workload receipt is too large",
            ));
        }
        let receipt: serde_json::Value = serde_json::from_slice(&receipt)
            .map_err(|_| CliError::legacy_validation("selected workload receipt is invalid"))?;
        let text = |key: &str| receipt.get(key).and_then(serde_json::Value::as_str);
        if receipt
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            != Some(1)
            || text("workload_id") != Some(workload_id)
            || text("catalog_revision") != Some(workload_revision)
            || text("archive_digest_sha256") != Some(workload_digest)
            || text("status") != Some("verified")
            || text("adapter_version") != Some("asb-workload-adapter-v1")
            || text("license").is_none_or(|license| {
                license.is_empty() || license.eq_ignore_ascii_case("NOASSERTION")
            })
            || text("preparation") != Some("bounded-extracted; no installer executed")
            || text("evaluator_entrypoint").is_none()
        {
            return Err(CliError::legacy_validation(
                "selected workload receipt is stale, revoked, or incomplete",
            ));
        }
        Ok(())
    }
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
            root: directory.path().to_path_buf(),
            result_root: fs::canonicalize(&result).unwrap(),
            work_root: directory.path().join(".asb/work"),
        };
        assert!(execution.validate_run_reference(outside.path()).is_err());
    }

    #[test]
    fn installed_workload_identity_requires_verified_prepared_receipt() {
        let directory = project();
        let digest = format!("{:x}", Sha256::digest(b"fixture archive"));
        let root = directory.path().join(".asb/workloads/fixture");
        fs::create_dir_all(root.join("prepared")).unwrap();
        fs::write(root.join("bundle.archive"), b"fixture archive").unwrap();
        fs::write(
            root.join("receipt.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 1,
                "workload_id": "fixture",
                "catalog_revision": "fixture-v1",
                "archive_digest_sha256": digest,
                "status": "verified",
                "adapter_version": "asb-workload-adapter-v1",
                "license": "MIT",
                "preparation": "bounded-extracted; no installer executed",
                "evaluator_entrypoint": "fixture/manifest.json"
            }))
            .unwrap(),
        )
        .unwrap();
        let mut config = super::load_tool_project(directory.path()).unwrap();
        config.selections.workload = Some("fixture".into());
        config.workloads.insert(
            "fixture".into(),
            ProjectToolRecordV1 {
                kind: ProjectToolKind::Workload,
                source_ref: "fixture://fixture".into(),
                version: "fixture-v1".into(),
                platform: "linux-x86_64".into(),
                path: Some(".asb/workloads/fixture".into()),
                digest_sha256: Some(digest.clone()),
                capabilities: vec!["external-workload".into(), "adapter-ready".into()],
                status: ProjectToolStatus::Available,
            },
        );
        super::save_tool_project(directory.path(), &config).unwrap();
        let execution = ProjectExecution {
            root: directory.path().to_path_buf(),
            result_root: directory.path().join("results"),
            work_root: directory.path().join(".asb/work"),
        };
        execution
            .validate_workload_identity("fixture", "fixture-v1", &digest)
            .unwrap();
        assert!(
            execution
                .validate_workload_identity("fixture", "stale", &digest)
                .is_err()
        );
        fs::write(root.join("receipt.json"), b"{}\n").unwrap();
        assert!(
            execution
                .validate_workload_identity("fixture", "fixture-v1", &digest)
                .is_err()
        );
    }

    #[test]
    fn installed_workload_identity_rejects_unsafe_or_stale_installations() {
        fn fixture() -> (tempfile::TempDir, ProjectExecution, String, PathBuf) {
            let directory = project();
            let digest = format!("{:x}", Sha256::digest(b"fixture archive"));
            let root = directory.path().join(".asb/workloads/fixture");
            fs::create_dir_all(root.join("prepared")).unwrap();
            fs::write(root.join("bundle.archive"), b"fixture archive").unwrap();
            fs::write(
                root.join("receipt.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schema_version": 1,
                    "workload_id": "fixture",
                    "catalog_revision": "fixture-v1",
                    "archive_digest_sha256": digest,
                    "status": "verified",
                    "adapter_version": "asb-workload-adapter-v1",
                    "license": "Apache-2.0",
                    "preparation": "bounded-extracted; no installer executed",
                    "evaluator_entrypoint": "fixture/run"
                }))
                .unwrap(),
            )
            .unwrap();
            let mut config = super::load_tool_project(directory.path()).unwrap();
            config.selections.workload = Some("fixture".into());
            config.workloads.insert(
                "fixture".into(),
                ProjectToolRecordV1 {
                    kind: ProjectToolKind::Workload,
                    source_ref: "fixture://fixture".into(),
                    version: "fixture-v1".into(),
                    platform: super::platform_label(),
                    path: Some(".asb/workloads/fixture".into()),
                    digest_sha256: Some(digest.clone()),
                    capabilities: vec!["external-workload".into(), "adapter-ready".into()],
                    status: ProjectToolStatus::Available,
                },
            );
            super::save_tool_project(directory.path(), &config).unwrap();
            let execution = ProjectExecution {
                root: directory.path().to_path_buf(),
                result_root: directory.path().join("results"),
                work_root: directory.path().join(".asb/work"),
            };
            (directory, execution, digest, root)
        }

        // A built-in or unregistered workload keeps the standalone path.
        let (directory, execution, digest, _) = fixture();
        assert!(
            execution
                .validate_workload_identity("not-installed", "v1", &digest)
                .is_err()
        );
        drop(directory);

        // Without a project workload selection, standalone/built-in plans
        // retain their established execution path.
        let directory = project();
        let execution = ProjectExecution {
            root: directory.path().to_path_buf(),
            result_root: directory.path().join("results"),
            work_root: directory.path().join(".asb/work"),
        };
        assert!(
            execution
                .validate_workload_identity("built-in", "v1", &"c".repeat(64))
                .is_ok()
        );
        drop(directory);

        // Every installation precondition must fail closed before execution.
        let (directory, execution, digest, _root) = fixture();
        let mut config = super::load_tool_project(directory.path()).unwrap();
        config.selections.workload = None;
        super::save_tool_project(directory.path(), &config).unwrap();
        assert!(
            execution
                .validate_workload_identity("fixture", "fixture-v1", &digest)
                .is_err()
        );
        drop(directory);

        let (directory, execution, digest, _root) = fixture();
        let mut config = super::load_tool_project(directory.path()).unwrap();
        config.workloads.get_mut("fixture").unwrap().status = ProjectToolStatus::Missing;
        super::save_tool_project(directory.path(), &config).unwrap();
        assert!(
            execution
                .validate_workload_identity("fixture", "fixture-v1", &digest)
                .is_err()
        );
        drop(directory);

        let (directory, execution, digest, _root) = fixture();
        let mut config = super::load_tool_project(directory.path()).unwrap();
        config.workloads.get_mut("fixture").unwrap().path = None;
        super::save_tool_project(directory.path(), &config).unwrap();
        assert!(
            execution
                .validate_workload_identity("fixture", "fixture-v1", &digest)
                .is_err()
        );
        drop(directory);

        let (directory, execution, digest, root) = fixture();
        fs::remove_dir_all(&root).unwrap();
        assert!(
            execution
                .validate_workload_identity("fixture", "fixture-v1", &digest)
                .is_err()
        );
        drop(directory);

        let (directory, execution, digest, root) = fixture();
        fs::remove_dir_all(root.join("prepared")).unwrap();
        assert!(
            execution
                .validate_workload_identity("fixture", "fixture-v1", &digest)
                .is_err()
        );
        drop(directory);

        let (directory, execution, digest, root) = fixture();
        fs::write(root.join("receipt.json"), b"not-json").unwrap();
        assert!(
            execution
                .validate_workload_identity("fixture", "fixture-v1", &digest)
                .is_err()
        );
        drop(directory);

        let (directory, execution, digest, root) = fixture();
        fs::write(root.join("receipt.json"), vec![b'x'; 64 * 1024 + 1]).unwrap();
        assert!(
            execution
                .validate_workload_identity("fixture", "fixture-v1", &digest)
                .is_err()
        );
        drop(directory);
    }
}
