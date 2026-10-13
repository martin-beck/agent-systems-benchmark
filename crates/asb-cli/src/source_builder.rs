// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Policy and execution boundary for catalog-pinned source builds.
//!
//! This module deliberately accepts an argv vector and a caller-owned source
//! tree.  It never invokes a shell, package manager, or network client.  The
//! caller is responsible for acquiring the catalog-pinned source artifact;
//! this boundary verifies its revision, builds in a transaction directory,
//! and publishes only the declared entrypoint.
#![allow(missing_docs)]

use crate::source_checkout::SourceCheckout;
use crate::tool_catalog::{BuildRecipe, ToolEntry};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const MAX_TIMEOUT_MS: u64 = 30 * 60 * 1000;
pub const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;
pub const MAX_ARTIFACT_BYTES: u64 = 1024 * 1024 * 1024;

/// Build outcome classes are intentionally stable: callers can provide an
/// actionable recovery without parsing host diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuildFailure {
    ToolchainUnavailable,
    PlatformUnsupported,
    DependencyBuildFailed,
    PolicyRejected,
    OutputMismatch,
    ResourceLimit,
    TimedOut,
    Cancelled,
    CleanupFailed,
}

impl BuildFailure {
    pub const fn code(self) -> &'static str {
        match self {
            Self::ToolchainUnavailable => "toolchain_unavailable",
            Self::PlatformUnsupported => "platform_unsupported",
            Self::DependencyBuildFailed => "dependency_build_failed",
            Self::PolicyRejected => "build_policy_rejected",
            Self::OutputMismatch => "build_output_mismatch",
            Self::ResourceLimit => "build_resource_limit",
            Self::TimedOut => "build_timeout",
            Self::Cancelled => "build_cancelled",
            Self::CleanupFailed => "build_cleanup_failed",
        }
    }
}

/// Bounded execution policy recorded with every build plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildLimits {
    pub timeout: Duration,
    pub max_output_bytes: u64,
    pub max_artifact_bytes: u64,
}

impl Default for BuildLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(15 * 60),
            max_output_bytes: MAX_OUTPUT_BYTES,
            max_artifact_bytes: MAX_ARTIFACT_BYTES,
        }
    }
}

/// A source build request.  `source_revision` is compared with the checked
/// out source before any command is started.
#[derive(Clone, Debug)]
pub struct BuildRequest {
    pub source: PathBuf,
    pub project_prefix: PathBuf,
    pub tool: ToolEntry,
    pub recipe: BuildRecipe,
    pub limits: BuildLimits,
}

/// Safe, content-minimized result suitable for a project receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildReceipt {
    pub tool_id: String,
    pub source_revision: String,
    pub output_digest_sha256: String,
    pub output_bytes: u64,
    pub dependency_ids: Vec<String>,
    pub staging_cleaned: bool,
}

/// Consume a verified catalog checkout with the existing pinned source
/// builder.  This is the integration seam used by the supported installer:
/// callers cannot accidentally pass an archive or an unverified local tree.
pub fn build_from_checkout(
    checkout: SourceCheckout,
    mut request: BuildRequest,
) -> Result<BuildReceipt, BuildFailure> {
    request.source = checkout.path().to_path_buf();
    let result = build(&request);
    if checkout.cleanup().is_err() {
        return Err(BuildFailure::CleanupFailed);
    }
    result
}

/// Validate a catalog recipe before execution.  This rejects shell syntax,
/// ambient package-manager commands, credentials, and unbounded limits.
pub fn validate_recipe(tool: &ToolEntry, recipe: &BuildRecipe) -> Result<(), BuildFailure> {
    if recipe.source_revision != tool.official_source.revision
        || recipe.argv.is_empty()
        || recipe.output.is_empty()
        || recipe.toolchain.is_empty()
        || recipe.timeout_ms == 0
        || recipe.timeout_ms > MAX_TIMEOUT_MS
        || recipe.max_output_bytes == 0
        || recipe.max_output_bytes > MAX_OUTPUT_BYTES
        || recipe.max_artifact_bytes == 0
        || recipe.max_artifact_bytes > MAX_ARTIFACT_BYTES
        || tool.id.is_empty()
        || tool.id.contains(['/', '\\'])
        || tool.entrypoint.is_empty()
        || Path::new(&tool.entrypoint).is_absolute()
        || Path::new(&tool.entrypoint)
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(BuildFailure::PolicyRejected);
    }
    let forbidden = [
        "sudo", "su", "sh", "bash", "zsh", "fish", "curl", "wget", "apt", "dnf", "yum", "pacman",
    ];
    for arg in &recipe.argv {
        if arg.is_empty()
            || arg.len() > 256
            || arg.contains([';', '|', '&', '`', '$', '\n', '\r'])
            || arg.to_ascii_lowercase().contains("token")
            || arg.to_ascii_lowercase().contains("password")
        {
            return Err(BuildFailure::PolicyRejected);
        }
    }
    if recipe
        .argv
        .first()
        .is_some_and(|program| forbidden.iter().any(|item| program == item))
    {
        return Err(BuildFailure::PolicyRejected);
    }
    let program = recipe.argv.first().map(String::as_str).unwrap_or_default();
    if (program == "pip" || program == "python")
        && !recipe.argv.iter().any(|arg| arg == "--no-isolation")
    {
        return Err(BuildFailure::PolicyRejected);
    }
    if program == "npm" && !recipe.argv.iter().any(|arg| arg == "--ignore-scripts") {
        return Err(BuildFailure::PolicyRejected);
    }
    Ok(())
}

/// Resolve a system dependency only when its catalog identity is exact.
pub fn validate_system_dependency(
    expected_id: &str,
    candidate_id: &str,
    candidate_version: &str,
    expected_version: &str,
) -> Result<(), BuildFailure> {
    if expected_id.is_empty()
        || candidate_id != expected_id
        || candidate_version != expected_version
    {
        Err(BuildFailure::DependencyBuildFailed)
    } else {
        Ok(())
    }
}

/// Execute one recipe in an isolated project-owned transaction directory.
/// The environment is rebuilt from a tiny allowlist and all standard output
/// is bounded and discarded rather than entering the receipt.
pub fn build(request: &BuildRequest) -> Result<BuildReceipt, BuildFailure> {
    validate_recipe(&request.tool, &request.recipe)?;
    if request.limits.timeout.is_zero()
        || request.limits.timeout > Duration::from_millis(MAX_TIMEOUT_MS)
        || request.limits.max_output_bytes == 0
        || request.limits.max_output_bytes > MAX_OUTPUT_BYTES
        || request.limits.max_artifact_bytes == 0
        || request.limits.max_artifact_bytes > MAX_ARTIFACT_BYTES
    {
        return Err(BuildFailure::PolicyRejected);
    }
    if !request.source.is_dir() {
        return Err(BuildFailure::PlatformUnsupported);
    }
    let revision = Command::new("git")
        .args([
            "-C",
            request.source.to_string_lossy().as_ref(),
            "rev-parse",
            "HEAD",
        ])
        .env_clear()
        .env("PATH", &request.recipe.toolchain)
        .output()
        .map_err(|_| BuildFailure::ToolchainUnavailable)?;
    if !revision.status.success()
        || String::from_utf8_lossy(&revision.stdout).trim() != request.recipe.source_revision
    {
        return Err(BuildFailure::PolicyRejected);
    }
    let staging_root = request.project_prefix.join(".staging");
    fs::create_dir_all(&staging_root).map_err(|_| BuildFailure::CleanupFailed)?;
    let staging = staging_root.join(format!("{}-{}", request.tool.id, std::process::id()));
    if staging.exists() {
        return Err(BuildFailure::CleanupFailed);
    }
    fs::create_dir(&staging).map_err(|_| BuildFailure::CleanupFailed)?;
    let result = build_inner(request, &staging);
    let cleanup = fs::remove_dir_all(&staging).is_ok();
    match (result, cleanup) {
        (Ok(mut receipt), true) => {
            receipt.staging_cleaned = true;
            Ok(receipt)
        }
        (Err(error), true) => Err(error),
        (_, false) => Err(BuildFailure::CleanupFailed),
    }
}

fn build_inner(request: &BuildRequest, staging: &Path) -> Result<BuildReceipt, BuildFailure> {
    let mut command = Command::new(&request.recipe.argv[0]);
    command
        .args(&request.recipe.argv[1..])
        .current_dir(&request.source)
        .env_clear()
        .env("PATH", &request.recipe.toolchain)
        .env("HOME", staging.join("home"))
        .env("TMPDIR", staging.join("tmp"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    fs::create_dir_all(staging.join("home")).map_err(|_| BuildFailure::CleanupFailed)?;
    fs::create_dir_all(staging.join("tmp")).map_err(|_| BuildFailure::CleanupFailed)?;
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|_| BuildFailure::ToolchainUnavailable)?;
    loop {
        if started.elapsed() > request.limits.timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(BuildFailure::TimedOut);
        }
        if child
            .try_wait()
            .map_err(|_| BuildFailure::Cancelled)?
            .is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child
        .wait_with_output()
        .map_err(|_| BuildFailure::Cancelled)?;
    if output.stdout.len() as u64 > request.limits.max_output_bytes
        || output.stderr.len() as u64 > request.limits.max_output_bytes
    {
        return Err(BuildFailure::ResourceLimit);
    }
    if !output.status.success() {
        return Err(BuildFailure::DependencyBuildFailed);
    }
    let relative = Path::new(&request.recipe.output);
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| c == std::path::Component::ParentDir)
    {
        return Err(BuildFailure::PolicyRejected);
    }
    let produced = resolve_output(&request.source, relative)?;
    let metadata = fs::symlink_metadata(&produced).map_err(|_| BuildFailure::OutputMismatch)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > request.limits.max_artifact_bytes
    {
        return Err(BuildFailure::OutputMismatch);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o111 == 0 {
        // Package archives are not adapter entrypoints.  A source recipe must
        // publish the executable produced by its pinned build, never the
        // wheel/tarball used to produce it.
        return Err(BuildFailure::OutputMismatch);
    }
    let mut file = fs::File::open(&produced).map_err(|_| BuildFailure::OutputMismatch)?;
    let mut header = [0_u8; 4];
    file.read_exact(&mut header)
        .map_err(|_| BuildFailure::OutputMismatch)?;
    if !is_executable_adapter_header(&header) {
        return Err(BuildFailure::OutputMismatch);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| BuildFailure::OutputMismatch)?;
    let mut digest = Sha256::new();
    let mut bytes = 0_u64;
    let mut buf = [0_u8; 8192];
    loop {
        let count = file
            .read(&mut buf)
            .map_err(|_| BuildFailure::OutputMismatch)?;
        if count == 0 {
            break;
        }
        bytes = bytes.saturating_add(count as u64);
        digest.update(&buf[..count]);
    }
    let destination = request.project_prefix.join(&request.tool.id);
    fs::create_dir_all(&destination).map_err(|_| BuildFailure::CleanupFailed)?;
    let staged_output = staging.join(&request.tool.entrypoint);
    fs::copy(&produced, &staged_output).map_err(|_| BuildFailure::OutputMismatch)?;
    #[cfg(unix)]
    fs::set_permissions(&staged_output, fs::Permissions::from_mode(0o700))
        .map_err(|_| BuildFailure::OutputMismatch)?;
    let staged_bytes = fs::read(&staged_output).map_err(|_| BuildFailure::OutputMismatch)?;
    let staged_digest = Sha256::digest(&staged_bytes);
    if staged_bytes.len() as u64 != bytes
        || format!("{:x}", staged_digest) != format!("{:x}", digest.clone().finalize())
    {
        return Err(BuildFailure::OutputMismatch);
    }
    let final_output = destination.join(&request.tool.entrypoint);
    if final_output.exists() {
        return Err(BuildFailure::OutputMismatch);
    }
    fs::rename(&staged_output, &final_output).map_err(|_| BuildFailure::CleanupFailed)?;
    Ok(BuildReceipt {
        tool_id: request.tool.id.clone(),
        source_revision: request.recipe.source_revision.clone(),
        output_digest_sha256: format!("{:x}", digest.finalize()),
        output_bytes: bytes,
        dependency_ids: request.tool.dependencies.clone(),
        staging_cleaned: false,
    })
}

fn is_executable_adapter_header(header: &[u8; 4]) -> bool {
    // A source recipe may publish a native executable or a launcher with an
    // interpreter directive. Package archives and arbitrary executable-bit
    // files are not adapters.
    header == b"\x7fELF"
        || header[0..2] == [b'#', b'!']
        || matches!(header, [0xcf, 0xfa, 0xed, 0xfe] | [0xfe, 0xed, 0xfa, 0xcf])
}

fn resolve_output(source: &Path, pattern: &Path) -> Result<PathBuf, BuildFailure> {
    let text = pattern.to_str().ok_or(BuildFailure::PolicyRejected)?;
    if !text.contains('*') {
        return Ok(source.join(pattern));
    }
    if text.matches('*').count() != 1 {
        return Err(BuildFailure::PolicyRejected);
    }
    let (parent, suffix) = text.rsplit_once('*').ok_or(BuildFailure::PolicyRejected)?;
    let directory = source.join(parent);
    let matches = fs::read_dir(directory).map_err(|_| BuildFailure::OutputMismatch)?;
    let mut found = None;
    for entry in matches {
        let entry = entry.map_err(|_| BuildFailure::OutputMismatch)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(suffix) {
            if found.is_some() {
                return Err(BuildFailure::OutputMismatch);
            }
            found = Some(entry.path());
        }
    }
    found.ok_or(BuildFailure::OutputMismatch)
}

/// Return a deterministic environment projection for receipt tests.
pub fn safe_environment() -> BTreeMap<&'static str, &'static str> {
    [("LC_ALL", "C"), ("LANG", "C"), ("TZ", "UTC")]
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_catalog::{OfficialSource, Platform};
    use tempfile::tempdir;

    fn fixture() -> (ToolEntry, BuildRecipe) {
        let tool = ToolEntry {
            id: "fixture".into(),
            kind: "support".into(),
            development_only: true,
            official_source: OfficialSource {
                url: "https://example.invalid/source".into(),
                version: "1.0.0".into(),
                revision: "abc123".into(),
            },
            platforms: vec![Platform {
                os: "linux".into(),
                arch: "x86_64".into(),
            }],
            alternatives: Vec::new(),
            entrypoint: "fixture".into(),
            license: "MIT".into(),
            dependencies: vec!["compiler>=1".into()],
            build_recipes: Vec::new(),
        };
        let recipe = BuildRecipe {
            id: "fixture-build".into(),
            source_revision: "abc123".into(),
            argv: vec!["true".into()],
            output: "fixture".into(),
            toolchain: "/usr/bin:/bin".into(),
            timeout_ms: 1000,
            max_output_bytes: 1024,
            max_artifact_bytes: 1024,
        };
        (tool, recipe)
    }

    #[test]
    fn recipe_rejects_shell_and_package_manager_commands() {
        let (tool, mut recipe) = fixture();
        recipe.argv[0] = "sh".into();
        assert_eq!(
            validate_recipe(&tool, &recipe),
            Err(BuildFailure::PolicyRejected)
        );
        recipe.argv[0] = "true".into();
        recipe.argv.push("$(secret)".into());
        assert_eq!(
            validate_recipe(&tool, &recipe),
            Err(BuildFailure::PolicyRejected)
        );
        recipe.argv = vec!["npm".into(), "install".into()];
        assert_eq!(
            validate_recipe(&tool, &recipe),
            Err(BuildFailure::PolicyRejected)
        );
        let mut unsafe_tool = tool;
        unsafe_tool.entrypoint = "../escape".into();
        recipe.argv = vec!["true".into()];
        assert_eq!(
            validate_recipe(&unsafe_tool, &recipe),
            Err(BuildFailure::PolicyRejected)
        );
    }

    #[test]
    fn dependency_reuse_requires_exact_catalog_identity() {
        assert!(validate_system_dependency("python", "python", "3.11", "3.11").is_ok());
        assert_eq!(
            validate_system_dependency("python", "python", "3.12", "3.11"),
            Err(BuildFailure::DependencyBuildFailed)
        );
    }

    #[test]
    fn environment_projection_contains_no_credentials_or_host_paths() {
        let environment = safe_environment();
        assert_eq!(environment.get("TZ"), Some(&"UTC"));
        assert!(
            !environment
                .keys()
                .any(|key| key.contains("TOKEN") || key.contains("HOME"))
        );
    }

    #[test]
    fn source_output_accepts_native_or_launcher_headers_only() {
        assert!(is_executable_adapter_header(b"\x7fELF"));
        assert!(is_executable_adapter_header(b"#!/b"));
        assert!(is_executable_adapter_header(b"\xcf\xfa\xed\xfe"));
        assert!(!is_executable_adapter_header(b"PK\x03\x04"));
        assert!(!is_executable_adapter_header(b"\x1f\x8b\x08\0"));
    }

    #[test]
    fn source_builder_compiles_and_publishes_a_pinned_elf_adapter() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source");
        let staging = directory.path().join("staging");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(source.join("fixture.c"), b"int main(void) { return 0; }\n").unwrap();
        let (tool, mut recipe) = fixture();
        recipe.argv = vec![
            "cc".into(),
            "fixture.c".into(),
            "-o".into(),
            "fixture".into(),
        ];
        let request = BuildRequest {
            source,
            project_prefix: directory.path().join("published"),
            tool,
            recipe,
            limits: BuildLimits::default(),
        };
        let receipt = build_inner(&request, &staging).unwrap();
        assert_eq!(receipt.tool_id, "fixture");
        assert!(receipt.output_bytes > 0);
        assert!(!receipt.output_digest_sha256.is_empty());
        assert!(directory.path().join("published/fixture/fixture").is_file());
    }

    #[test]
    fn source_builder_rejects_failed_builds_and_ambiguous_outputs() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("broken.c"), b"not valid C\n").unwrap();
        let staging = directory.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        let (tool, mut recipe) = fixture();
        recipe.argv = vec![
            "cc".into(),
            "broken.c".into(),
            "-o".into(),
            "fixture".into(),
        ];
        let request = BuildRequest {
            source: source.clone(),
            project_prefix: directory.path().join("published"),
            tool,
            recipe,
            limits: BuildLimits::default(),
        };
        assert!(matches!(
            build_inner(&request, &staging),
            Err(BuildFailure::DependencyBuildFailed)
        ));
        fs::write(source.join("one.bin"), b"one").unwrap();
        fs::write(source.join("two.bin"), b"two").unwrap();
        assert_eq!(
            resolve_output(&source, Path::new("*.bin")),
            Err(BuildFailure::OutputMismatch)
        );
        assert_eq!(
            resolve_output(&source, Path::new("*.missing")),
            Err(BuildFailure::OutputMismatch)
        );
        assert_eq!(
            resolve_output(&source, Path::new("one*two*")),
            Err(BuildFailure::PolicyRejected)
        );
    }

    #[test]
    fn build_failure_codes_are_stable_and_exhaustive() {
        let cases = [
            (BuildFailure::ToolchainUnavailable, "toolchain_unavailable"),
            (BuildFailure::PlatformUnsupported, "platform_unsupported"),
            (
                BuildFailure::DependencyBuildFailed,
                "dependency_build_failed",
            ),
            (BuildFailure::PolicyRejected, "build_policy_rejected"),
            (BuildFailure::OutputMismatch, "build_output_mismatch"),
            (BuildFailure::ResourceLimit, "build_resource_limit"),
            (BuildFailure::TimedOut, "build_timeout"),
            (BuildFailure::Cancelled, "build_cancelled"),
            (BuildFailure::CleanupFailed, "build_cleanup_failed"),
        ];
        for (failure, code) in cases {
            assert_eq!(failure.code(), code);
        }
    }
}
