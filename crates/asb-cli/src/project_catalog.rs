// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Deterministic, content-addressed project tool catalogs.

use super::*;
use asb_config::GeneratedCatalogReferenceV1;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const CATALOG_SCHEMA: &str = "asb.project-catalog.v1";
const CATALOG_SOURCE: &str = "https://agent-systems-benchmark.dev/project-inventory-v1";
const MAX_CATALOG_BYTES: usize = 256 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogArtifactV1 {
    schema_version: u16,
    id: String,
    kind: ProjectToolKind,
    source_ref: String,
    compatibility: Vec<String>,
    entries: Vec<CatalogEntryV1>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogEntryV1 {
    id: String,
    source_ref: String,
    version: String,
    platform: String,
    digest_sha256: Option<String>,
    capabilities: Vec<String>,
    status: ProjectToolStatus,
}

/// Dispatch the project catalog subcommands.
pub(super) fn dispatch(args: &[String], output: &mut dyn Write) -> Result<(), CliError> {
    let operation = args
        .first()
        .map(String::as_str)
        .ok_or_else(|| CliError::legacy_usage("catalog requires generate|list|show|select"))?;
    match operation {
        "generate" => generate(&args[1..], output),
        "list" => list(&args[1..], output),
        "show" => show(&args[1..], output),
        "select" => select(&args[1..], output),
        _ => Err(CliError::legacy_usage(
            "catalog requires generate|list|show|select",
        )),
    }
}

fn parse_project_only(args: &[String]) -> Result<PathBuf, CliError> {
    super::parse_tool_project_only(args).map_err(|_| {
        CliError::legacy_usage("catalog accepts only --project PATH after its required arguments")
    })
}

fn kind_name(kind: ProjectToolKind) -> &'static str {
    match kind {
        ProjectToolKind::Agent => "agent",
        ProjectToolKind::Harness => "harness",
        ProjectToolKind::Benchmark => "benchmark",
        ProjectToolKind::Workload => "workload",
        ProjectToolKind::SupportTool => "support-tool",
    }
}

fn catalog_id(kind: ProjectToolKind) -> String {
    format!("project-{}-catalog-v1", kind_name(kind))
}

fn kinds() -> [ProjectToolKind; 5] {
    [
        ProjectToolKind::Agent,
        ProjectToolKind::Harness,
        ProjectToolKind::Benchmark,
        ProjectToolKind::Workload,
        ProjectToolKind::SupportTool,
    ]
}

fn canonical_artifact(config: &ProjectConfigV1, kind: ProjectToolKind) -> CatalogArtifactV1 {
    let entries = super::tool_kind_map(config, kind)
        .iter()
        .map(|(id, record)| CatalogEntryV1 {
            id: id.clone(),
            source_ref: record.source_ref.clone(),
            version: record.version.clone(),
            platform: record.platform.clone(),
            digest_sha256: record.digest_sha256.clone(),
            capabilities: record.capabilities.clone(),
            status: record.status,
        })
        .collect::<Vec<_>>();
    let compatibility = entries
        .iter()
        .filter(|entry| entry.status == ProjectToolStatus::Available)
        .flat_map(|entry| entry.capabilities.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    CatalogArtifactV1 {
        schema_version: 1,
        id: catalog_id(kind),
        kind,
        source_ref: CATALOG_SOURCE.into(),
        compatibility,
        entries,
    }
}

fn artifact_bytes(artifact: &CatalogArtifactV1) -> Result<Vec<u8>, CliError> {
    serde_json::to_vec(artifact)
        .map_err(|_| CliError::legacy_operation("generated catalog cannot be encoded"))
}

fn generated_at() -> Result<String, CliError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| CliError::legacy_operation("catalog generation timestamp cannot be encoded"))
}

fn catalog_path(root: &Path, config: &ProjectConfigV1, id: &str, digest: &str) -> PathBuf {
    root.join(&config.roots.catalogs)
        .join(format!("{id}-{digest}.json"))
}

fn generate(args: &[String], output: &mut dyn Write) -> Result<(), CliError> {
    let root = parse_project_only(args)?;
    let mut config = super::load_tool_project(&root)?;
    let mut generated = Vec::new();
    for kind in kinds() {
        let artifact = canonical_artifact(&config, kind);
        let bytes = artifact_bytes(&artifact)?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let path = catalog_path(&root, &config, &artifact.id, &digest);
        super::write_atomic_private(&path, &bytes, DirectoryPurpose::Catalogs, None)?;
        for reference in config.catalogs.values_mut() {
            if reference.kind == kind {
                reference.active = false;
            }
        }
        let reference = GeneratedCatalogReferenceV1 {
            id: artifact.id.clone(),
            kind,
            schema: CATALOG_SCHEMA.into(),
            source_ref: CATALOG_SOURCE.into(),
            digest_sha256: digest.clone(),
            generated_at: generated_at()?,
            compatibility: artifact.compatibility.clone(),
            active: true,
        };
        config.catalogs.insert(artifact.id.clone(), reference);
        generated.push(serde_json::json!({
            "id": artifact.id,
            "kind": kind,
            "digest_sha256": digest,
            "entries": artifact.entries.len(),
        }));
    }
    super::save_tool_project(&root, &config)?;
    write_json(
        output,
        &serde_json::json!({
            "schema_version": OUTPUT_SCHEMA_VERSION,
            "ok": true,
            "command": "catalog generate",
            "catalogs": generated,
            "tools": generated.clone(),
        }),
    )
}

fn list(args: &[String], output: &mut dyn Write) -> Result<(), CliError> {
    let root = parse_project_only(args)?;
    let config = super::load_tool_project(&root)?;
    let catalogs = config
        .catalogs
        .iter()
        .map(|(name, reference)| serde_json::json!({"name": name, "reference": reference}))
        .collect::<Vec<_>>();
    write_json(
        output,
        &serde_json::json!({
            "schema_version": OUTPUT_SCHEMA_VERSION,
            "ok": true,
            "command": "catalog list",
            "catalogs": catalogs,
            "tools": catalogs.clone(),
        }),
    )
}

fn parse_id_and_project<'a>(
    args: &'a [String],
    action: &'static str,
) -> Result<(&'a str, PathBuf), CliError> {
    let (id, rest) = args
        .split_first()
        .ok_or_else(|| CliError::legacy_usage(action))?;
    if !super::valid_tool_id(id) {
        return Err(CliError::legacy_validation("catalog ID is invalid"));
    }
    Ok((id, parse_project_only(rest)?))
}

fn checked_artifact(
    root: &Path,
    config: &ProjectConfigV1,
    id: &str,
) -> Result<(GeneratedCatalogReferenceV1, CatalogArtifactV1), CliError> {
    let reference = config.catalogs.get(id).cloned().ok_or_else(|| {
        CliError::legacy_validation("catalog ID is not generated for this project")
    })?;
    let path = catalog_path(root, config, id, &reference.digest_sha256);
    let bytes = super::read_bounded_json(
        &path,
        MAX_CATALOG_BYTES,
        "catalog artifact is missing, unsafe, or unreadable; rerun catalog generate",
    )?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if digest != reference.digest_sha256 {
        return Err(CliError::legacy_validation(
            "catalog artifact digest mismatches project provenance; rerun catalog generate",
        ));
    }
    let artifact = serde_json::from_slice::<CatalogArtifactV1>(&bytes).map_err(|_| {
        CliError::legacy_validation("catalog artifact is invalid; rerun catalog generate")
    })?;
    if artifact.schema_version != 1
        || artifact.id != reference.id
        || artifact.kind != reference.kind
        || artifact.source_ref != reference.source_ref
        || artifact.compatibility != reference.compatibility
    {
        return Err(CliError::legacy_validation(
            "catalog artifact is incompatible with project provenance; rerun catalog generate",
        ));
    }
    Ok((reference, artifact))
}

/// Validate that every project inventory class has one usable active catalog.
///
/// Execution consumes these artifacts rather than treating the mutable tool
/// inventory as an implicit catalog.  This keeps a tool install, discovery, or
/// selection change from silently changing the benchmark inputs after a
/// catalog was generated.
pub(super) fn validate_active_execution_catalogs(
    root: &Path,
    config: &ProjectConfigV1,
) -> Result<(), CliError> {
    for kind in kinds() {
        let reference = config
            .catalogs
            .values()
            .find(|reference| reference.kind == kind && reference.active)
            .ok_or_else(|| {
                CliError::legacy_validation(
                    "project active catalog is missing; run `asb catalog generate --project PATH` then select the catalog",
                )
            })?;
        let (_, artifact) = checked_artifact(root, config, &reference.id)?;
        if artifact
            .entries
            .iter()
            .any(|entry| entry.status != ProjectToolStatus::Available)
        {
            return Err(CliError::legacy_validation(
                "project active catalog contains unavailable tools; run `asb tool discover PATH`, repair or install the tools, then regenerate and select the catalog",
            ));
        }
    }
    Ok(())
}

/// Bind a selected inventory record to the exact entry in its active catalog.
///
/// Inventory discovery is mutable, whereas a generated catalog is the
/// execution input.  A selection therefore cannot be used after installation,
/// discovery, or replacement changes its public identity; the operator must
/// regenerate the catalog and select it again.
pub(super) fn validate_selected_catalog_entry(
    root: &Path,
    config: &ProjectConfigV1,
    kind: ProjectToolKind,
    id: &str,
    record: &ProjectToolRecordV1,
) -> Result<(), CliError> {
    let reference = config
        .catalogs
        .values()
        .find(|reference| reference.kind == kind && reference.active)
        .ok_or_else(|| {
            CliError::legacy_validation(
                "project active catalog is missing; run `asb catalog generate --project PATH` then select the catalog",
            )
        })?;
    let (_, artifact) = checked_artifact(root, config, &reference.id)?;
    let entry = artifact.entries.iter().find(|entry| entry.id == id).ok_or_else(|| {
        CliError::legacy_validation(
            "project selection is absent from its active catalog; rerun `asb catalog generate --project PATH` and select the regenerated catalog",
        )
    })?;
    if entry.source_ref != record.source_ref
        || entry.version != record.version
        || entry.platform != record.platform
        || entry.digest_sha256 != record.digest_sha256
        || entry.capabilities != record.capabilities
        || entry.status != record.status
    {
        return Err(CliError::legacy_validation(
            "project selected tool drifted from its active catalog; rerun `asb catalog generate --project PATH` and select the regenerated catalog",
        ));
    }
    Ok(())
}

fn show(args: &[String], output: &mut dyn Write) -> Result<(), CliError> {
    let (id, root) = parse_id_and_project(args, "catalog show requires an ID")?;
    let config = super::load_tool_project(&root)?;
    let (reference, artifact) = checked_artifact(&root, &config, id)?;
    write_json(
        output,
        &serde_json::json!({
            "schema_version": OUTPUT_SCHEMA_VERSION,
            "ok": true,
            "command": "catalog show",
            "reference": reference,
            "catalog": artifact,
            "tools": artifact.entries.clone(),
        }),
    )
}

fn select(args: &[String], output: &mut dyn Write) -> Result<(), CliError> {
    let (id, rest) = args.split_first().ok_or_else(|| {
        CliError::legacy_usage("catalog select requires an ID, --kind, and --digest-sha256")
    })?;
    if !super::valid_tool_id(id) {
        return Err(CliError::legacy_validation("catalog ID is invalid"));
    }
    let mut kind = None;
    let mut expected_digest = None;
    let mut project_args = Vec::new();
    let mut index = 0;
    while index < rest.len() {
        match rest[index].as_str() {
            "--kind" => {
                let value = rest.get(index + 1).ok_or_else(|| {
                    CliError::legacy_usage("catalog select --kind requires a value")
                })?;
                kind = Some(super::tool_kind(value)?);
                index += 2;
            }
            "--digest-sha256" => {
                let value = rest.get(index + 1).ok_or_else(|| {
                    CliError::legacy_usage("catalog select --digest-sha256 requires a value")
                })?;
                if !super::valid_sha256(value) {
                    return Err(CliError::legacy_validation(
                        "catalog digest must be 64 lowercase hexadecimal characters",
                    ));
                }
                expected_digest = Some(value.clone());
                index += 2;
            }
            "--project" => {
                project_args.push(rest[index].clone());
                project_args.push(
                    rest.get(index + 1)
                        .ok_or_else(|| {
                            CliError::legacy_usage("catalog select --project requires a path")
                        })?
                        .clone(),
                );
                index += 2;
            }
            _ => {
                return Err(CliError::legacy_usage(
                    "catalog select accepts --kind, --digest-sha256, and --project",
                ));
            }
        }
    }
    let kind = kind.ok_or_else(|| CliError::legacy_usage("catalog select requires --kind"))?;
    let expected_digest = expected_digest
        .ok_or_else(|| CliError::legacy_usage("catalog select requires --digest-sha256"))?;
    let root = parse_project_only(&project_args)?;
    let mut config = super::load_tool_project(&root)?;
    let (reference, artifact) = checked_artifact(&root, &config, id)?;
    if reference.kind != kind || reference.digest_sha256 != expected_digest {
        return Err(CliError::legacy_validation(
            "catalog selection kind or digest is stale or incompatible; rerun catalog list",
        ));
    }
    if artifact
        .entries
        .iter()
        .any(|entry| entry.status != ProjectToolStatus::Available)
    {
        return Err(CliError::legacy_validation(
            "catalog selection contains unavailable inventory entries; resolve them then rerun catalog generate",
        ));
    }
    for other in config.catalogs.values_mut() {
        if other.kind == kind {
            other.active = false;
        }
    }
    config
        .catalogs
        .get_mut(id)
        .expect("checked catalog reference must remain present")
        .active = true;
    super::save_tool_project(&root, &config)?;
    write_json(
        output,
        &serde_json::json!({
            "schema_version": OUTPUT_SCHEMA_VERSION,
            "ok": true,
            "command": "catalog select",
            "id": id,
            "kind": kind,
            "digest_sha256": expected_digest,
            "active": true,
            "tools": [],
        }),
    )
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

    fn install_fixture(directory: &Path, id: &str, kind: &str) {
        super::super::tool(
            &[
                "install".into(),
                id.into(),
                "--kind".into(),
                kind.into(),
                "--source".into(),
                format!("fixture://{id}"),
                "--version".into(),
                "1.0.0".into(),
                "--project".into(),
                directory.display().to_string(),
            ],
            &mut Vec::new(),
        )
        .unwrap();
    }

    #[test]
    fn generation_is_content_addressed_and_secret_free() {
        let directory = project();
        install_fixture(directory.path(), "fixture-agent", "agent");
        let args = ["--project".into(), directory.path().display().to_string()];
        let mut first = Vec::new();
        generate(&args, &mut first).unwrap();
        let first_config = super::super::load_tool_project(directory.path()).unwrap();
        let first_digest = first_config
            .catalogs
            .get("project-agent-catalog-v1")
            .unwrap()
            .digest_sha256
            .clone();
        let mut second = Vec::new();
        generate(&args, &mut second).unwrap();
        let config = super::super::load_tool_project(directory.path()).unwrap();
        let reference = config.catalogs.get("project-agent-catalog-v1").unwrap();
        let path = catalog_path(
            directory.path(),
            &config,
            &reference.id,
            &reference.digest_sha256,
        );
        let bytes = fs::read(path).unwrap();
        assert_eq!(first_digest, reference.digest_sha256);
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            reference.digest_sha256
        );
        let rendered = String::from_utf8(bytes).unwrap();
        assert!(!rendered.contains(directory.path().to_str().unwrap()));
        assert!(!rendered.contains("api_key"));
    }

    #[test]
    fn show_and_select_reject_digest_and_availability_mismatches_without_mutation() {
        let directory = project();
        install_fixture(directory.path(), "fixture-agent", "agent");
        let project_args = ["--project".into(), directory.path().display().to_string()];
        generate(&project_args, &mut Vec::new()).unwrap();
        let before = super::super::load_tool_project(directory.path()).unwrap();
        let reference = before.catalogs.get("project-agent-catalog-v1").unwrap();
        let bad = [
            "project-agent-catalog-v1".into(),
            "--kind".into(),
            "agent".into(),
            "--digest-sha256".into(),
            "0".repeat(64),
            "--project".into(),
            directory.path().display().to_string(),
        ];
        assert!(select(&bad, &mut Vec::new()).is_err());
        assert_eq!(
            before,
            super::super::load_tool_project(directory.path()).unwrap()
        );
        let good = [
            "project-agent-catalog-v1".into(),
            "--kind".into(),
            "agent".into(),
            "--digest-sha256".into(),
            reference.digest_sha256.clone(),
            "--project".into(),
            directory.path().display().to_string(),
        ];
        select(&good, &mut Vec::new()).unwrap();
        let mut config = super::super::load_tool_project(directory.path()).unwrap();
        config.agents.get_mut("fixture-agent").unwrap().status = ProjectToolStatus::Missing;
        super::super::save_tool_project(directory.path(), &config).unwrap();
        generate(&project_args, &mut Vec::new()).unwrap();
        let digest = super::super::load_tool_project(directory.path())
            .unwrap()
            .catalogs
            .get("project-agent-catalog-v1")
            .unwrap()
            .digest_sha256
            .clone();
        let unavailable = [
            "project-agent-catalog-v1".into(),
            "--kind".into(),
            "agent".into(),
            "--digest-sha256".into(),
            digest,
            "--project".into(),
            directory.path().display().to_string(),
        ];
        assert!(select(&unavailable, &mut Vec::new()).is_err());
        assert!(
            show(
                &[
                    "project-agent-catalog-v1".into(),
                    "--project".into(),
                    directory.path().display().to_string()
                ],
                &mut Vec::new()
            )
            .is_ok()
        );
    }

    #[test]
    fn catalog_list_has_human_and_json_presentations() {
        let directory = project();
        install_fixture(directory.path(), "fixture-agent", "agent");
        let project = directory.path().display().to_string();
        generate(&["--project".into(), project.clone()], &mut Vec::new()).unwrap();

        let arguments = [
            OsString::from("catalog"),
            OsString::from("list"),
            OsString::from("--project"),
            OsString::from(&project),
        ];
        let mut human = Vec::new();
        assert_eq!(
            super::super::run_with_default_mode_and_stdin(
                &arguments,
                &mut human,
                &mut Vec::new(),
                true,
                None,
            ),
            0
        );
        let human = String::from_utf8(human).unwrap();
        assert!(human.contains("catalog"));
        assert!(!human.contains("schema_version"));

        let json_args = [
            OsString::from("--json"),
            OsString::from("catalog"),
            OsString::from("list"),
            OsString::from("--project"),
            OsString::from(project),
        ];
        let mut json = Vec::new();
        assert_eq!(super::super::run(&json_args, &mut json, &mut Vec::new()), 0);
        let value: Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value["command"], "catalog list");
        assert!(value["catalogs"].is_array());
    }
}
