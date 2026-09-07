// SPDX-License-Identifier: MIT
//! Signed, content-addressed runtime-bundle manifests and offline verification.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use thiserror::Error;

/// Runtime-bundle manifest schema version supported by this crate.
pub const SCHEMA_VERSION: u32 = 1;
/// SSH signature namespace used only for ASB runtime bundles.
pub const SIGNATURE_NAMESPACE: &str = "asb-runtime-bundle-v1";
/// Maximum accepted manifest size.
pub const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
/// Maximum accepted SBOM size per document.
pub const MAX_SBOM_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum total payload size accepted from one runtime bundle.
pub const MAX_TOTAL_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024 * 1024;
/// Maximum accepted number of bundle artifacts.
pub const MAX_ARTIFACTS: usize = 100_000;
const MAX_SIGNATURE_BYTES: u64 = 1024 * 1024;
const MAX_ALLOWED_SIGNERS_BYTES: u64 = 1024 * 1024;
const MAX_STRING_BYTES: usize = 4096;

/// A signed runtime bundle complete content-addressed inventory.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeBundleManifest {
    /// Manifest schema version; currently exactly one.
    #[schemars(schema_with = "schema_version")]
    pub schema_version: u32,
    /// Stable bounded bundle identifier.
    #[schemars(length(min = 1, max = 4096))]
    pub bundle_id: String,
    /// Immutable upstream or application version.
    #[schemars(length(min = 1, max = 4096))]
    pub bundle_version: String,
    /// Exact supported execution target.
    pub target: RuntimeTarget,
    /// Relative executable entrypoint, present in artifacts and executable.
    #[schemars(length(min = 1, max = 4096))]
    pub entrypoint: String,
    /// Complete sorted inventory excluding manifest, signature, and SBOM files.
    #[schemars(length(min = 1, max = 100_000))]
    pub artifacts: Vec<BundleArtifact>,
    /// SHA-256 over the canonical ordered artifact tuple stream.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"))]
    pub content_sha256: String,
    /// SPDX JSON document and its signed content hash.
    pub spdx: SbomDocument,
    /// CycloneDX JSON document and its signed content hash.
    pub cyclonedx: SbomDocument,
}

/// Exact runtime platform identity.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTarget {
    /// Operating-system identifier, such as linux.
    #[schemars(length(min = 1, max = 4096))]
    pub operating_system: String,
    /// CPU architecture, such as x86_64 or aarch64.
    #[schemars(length(min = 1, max = 4096))]
    pub architecture: String,
    /// libc family, such as glibc or musl.
    #[schemars(length(min = 1, max = 4096))]
    pub libc: String,
    /// Exact libc ABI version required by the bundle.
    #[schemars(length(min = 1, max = 4096))]
    pub libc_version: String,
}

/// One regular file in the runtime bundle.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BundleArtifact {
    /// Normalized UTF-8 path relative to the bundle root.
    #[schemars(length(min = 1, max = 4096))]
    pub path: String,
    /// Exact file length.
    pub size: u64,
    /// Lowercase SHA-256 of the file bytes.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"))]
    pub sha256: String,
    /// Whether any executable permission bit is set.
    pub executable: bool,
    /// SPDX license expression attributed to this file.
    #[schemars(length(min = 1, max = 4096))]
    pub license_expression: String,
    /// Non-empty inventoried paths containing license evidence.
    #[schemars(length(min = 1, max = 100_000))]
    pub license_evidence: Vec<String>,
}

/// Signed identity of one SBOM document.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SbomDocument {
    /// Normalized relative path to the JSON document.
    #[schemars(length(min = 1, max = 4096))]
    pub path: String,
    /// Lowercase SHA-256 of its exact bytes.
    #[schemars(regex(pattern = r"^[0-9a-f]{64}$"))]
    pub sha256: String,
}

/// Expected platform identity supplied by the installer or launcher.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedTarget<'a> {
    /// Expected operating system.
    pub operating_system: &'a str,
    /// Expected architecture.
    pub architecture: &'a str,
    /// Expected libc family.
    pub libc: &'a str,
    /// Exact expected libc ABI version.
    pub libc_version: &'a str,
}

/// Offline verifier configuration and explicit trust root.
#[derive(Clone, Debug)]
pub struct VerifierConfig {
    /// Exact ssh-keygen executable used for SSHSIG verification.
    pub ssh_keygen: PathBuf,
    /// Lowercase SHA-256 of the trusted ssh-keygen executable.
    pub ssh_keygen_sha256: String,
    /// OpenSSH allowed-signers file containing trusted release keys.
    pub allowed_signers: PathBuf,
    /// Principal required in the allowed-signers file.
    pub principal: String,
}

/// Successful immutable verification evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedBundle {
    /// SHA-256 of the exact signed manifest bytes.
    pub manifest_sha256: String,
    /// Verified bundle identity.
    pub bundle_id: String,
    /// Verified bundle version.
    pub bundle_version: String,
    /// Verified content digest.
    pub content_sha256: String,
    /// Number of verified artifacts.
    pub artifact_count: usize,
}

/// Fail-closed runtime-bundle verification error.
#[derive(Debug, Error)]
pub enum VerifyError {
    /// Bundle path or file topology is unsafe.
    #[error("unsafe bundle topology: {0}")]
    Topology(String),
    /// A bounded input limit was exceeded.
    #[error("bundle input exceeds limit: {0}")]
    Limit(String),
    /// Manifest or SBOM syntax or semantics is invalid.
    #[error("invalid bundle metadata: {0}")]
    Metadata(String),
    /// Detached signature is absent, invalid, or untrusted.
    #[error("runtime bundle signature verification failed")]
    Signature,
    /// Content differs from the signed inventory.
    #[error("runtime bundle content mismatch: {0}")]
    Content(String),
    /// Runtime target differs from the callers required target.
    #[error("runtime bundle target mismatch: {0}")]
    Target(String),
    /// Local I/O failed without exposing private file contents.
    #[error("runtime bundle I/O failed")]
    Io(#[source] std::io::Error),
}

impl From<std::io::Error> for VerifyError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

fn schema_version(_: &mut SchemaGenerator) -> Schema {
    json_schema!({"type": "integer", "const": SCHEMA_VERSION})
}

/// Verify a bundle entirely from local files and an explicit offline trust root.
pub fn verify_bundle(
    bundle_root: &Path,
    config: &VerifierConfig,
    expected: &ExpectedTarget<'_>,
) -> Result<VerifiedBundle, VerifyError> {
    let root = validate_root(bundle_root)?;
    let manifest_path = root.join("manifest.json");
    let signature_path = root.join("manifest.json.sig");
    let manifest_bytes = read_regular_bounded(&manifest_path, MAX_MANIFEST_BYTES)?;
    validate_regular_file_bounded(&signature_path, MAX_SIGNATURE_BYTES)?;
    verify_signature(&manifest_bytes, &signature_path, config)?;

    let manifest: RuntimeBundleManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| VerifyError::Metadata("manifest is not strict schema-v1 JSON".into()))?;
    validate_manifest(&manifest, expected)?;
    if expected_inventory(&manifest)? != enumerate_files(&root)? {
        return Err(VerifyError::Content(
            "signed inventory does not equal bundle files".into(),
        ));
    }

    let artifacts: BTreeMap<&str, &BundleArtifact> = manifest
        .artifacts
        .iter()
        .map(|artifact| (artifact.path.as_str(), artifact))
        .collect();
    let total_bytes = manifest
        .artifacts
        .iter()
        .try_fold(0_u64, |total, artifact| total.checked_add(artifact.size));
    if total_bytes.is_none_or(|total| total > MAX_TOTAL_ARTIFACT_BYTES) {
        return Err(VerifyError::Limit("total artifact bytes".into()));
    }
    for (path, artifact) in &artifacts {
        let full_path = root.join(path);
        let (size, hash) = hash_regular_file(&full_path)?;
        if size != artifact.size || hash != artifact.sha256 {
            return Err(VerifyError::Content(format!("artifact {path} differs")));
        }
        let executable = fs::metadata(&full_path)?.permissions().mode() & 0o111 != 0;
        if executable != artifact.executable {
            return Err(VerifyError::Content(format!(
                "artifact {path} executable mode differs"
            )));
        }
    }
    let calculated_content = content_digest(&manifest.artifacts);
    if calculated_content != manifest.content_sha256 {
        return Err(VerifyError::Content("content digest differs".into()));
    }

    let spdx = read_and_hash_sbom(&root, &manifest.spdx)?;
    let cyclonedx = read_and_hash_sbom(&root, &manifest.cyclonedx)?;
    validate_spdx(&spdx, &artifacts)?;
    validate_cyclonedx(&cyclonedx, &artifacts)?;

    Ok(VerifiedBundle {
        manifest_sha256: sha256(&manifest_bytes),
        bundle_id: manifest.bundle_id,
        bundle_version: manifest.bundle_version,
        content_sha256: manifest.content_sha256,
        artifact_count: artifacts.len(),
    })
}

fn validate_root(path: &Path) -> Result<PathBuf, VerifyError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(VerifyError::Topology(
            "bundle root is not a real directory".into(),
        ));
    }
    fs::canonicalize(path).map_err(VerifyError::Io)
}

fn validate_manifest(
    manifest: &RuntimeBundleManifest,
    expected: &ExpectedTarget<'_>,
) -> Result<(), VerifyError> {
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(VerifyError::Metadata("unsupported schema version".into()));
    }
    for (name, value) in [
        ("bundle_id", manifest.bundle_id.as_str()),
        ("bundle_version", manifest.bundle_version.as_str()),
        ("entrypoint", manifest.entrypoint.as_str()),
        (
            "operating_system",
            manifest.target.operating_system.as_str(),
        ),
        ("architecture", manifest.target.architecture.as_str()),
        ("libc", manifest.target.libc.as_str()),
        ("libc_version", manifest.target.libc_version.as_str()),
    ] {
        bounded_nonempty(name, value)?;
    }
    if manifest.artifacts.is_empty() || manifest.artifacts.len() > MAX_ARTIFACTS {
        return Err(VerifyError::Limit("artifact count".into()));
    }
    let target = &manifest.target;
    if target.operating_system != expected.operating_system
        || target.architecture != expected.architecture
        || target.libc != expected.libc
        || target.libc_version != expected.libc_version
    {
        return Err(VerifyError::Target(
            "OS, architecture, or libc differs".into(),
        ));
    }
    validate_hash(&manifest.content_sha256)?;
    validate_relative_path(&manifest.entrypoint)?;
    validate_sbom(&manifest.spdx)?;
    validate_sbom(&manifest.cyclonedx)?;
    if manifest.spdx.path == manifest.cyclonedx.path {
        return Err(VerifyError::Metadata("SBOM paths collide".into()));
    }

    let mut prior: Option<&str> = None;
    let mut paths = BTreeSet::new();
    for artifact in &manifest.artifacts {
        validate_relative_path(&artifact.path)?;
        if matches!(
            artifact.path.as_str(),
            "manifest.json" | "manifest.json.sig"
        ) {
            return Err(VerifyError::Metadata(
                "artifact collides with signed metadata".into(),
            ));
        }
        validate_hash(&artifact.sha256)?;
        bounded_nonempty("license_expression", &artifact.license_expression)?;
        if prior.is_some_and(|value| value >= artifact.path.as_str()) {
            return Err(VerifyError::Metadata(
                "artifacts are not strictly path-sorted".into(),
            ));
        }
        prior = Some(&artifact.path);
        paths.insert(artifact.path.as_str());
        if artifact.license_evidence.is_empty() {
            return Err(VerifyError::Metadata("license evidence is empty".into()));
        }
        for evidence in &artifact.license_evidence {
            validate_relative_path(evidence)?;
        }
    }
    for artifact in &manifest.artifacts {
        if artifact
            .license_evidence
            .iter()
            .any(|evidence| !paths.contains(evidence.as_str()))
        {
            return Err(VerifyError::Metadata(
                "license evidence is not inventoried".into(),
            ));
        }
    }
    let entrypoint = manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.path == manifest.entrypoint)
        .ok_or_else(|| VerifyError::Metadata("entrypoint is not inventoried".into()))?;
    if !entrypoint.executable {
        return Err(VerifyError::Metadata("entrypoint is not executable".into()));
    }
    Ok(())
}

fn validate_sbom(sbom: &SbomDocument) -> Result<(), VerifyError> {
    validate_relative_path(&sbom.path)?;
    validate_hash(&sbom.sha256)
}

fn expected_inventory(manifest: &RuntimeBundleManifest) -> Result<BTreeSet<String>, VerifyError> {
    let mut paths: BTreeSet<String> = manifest
        .artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect();
    for path in [&manifest.spdx.path, &manifest.cyclonedx.path] {
        if !paths.insert(path.clone()) {
            return Err(VerifyError::Metadata("metadata path collides".into()));
        }
    }
    paths.insert("manifest.json".into());
    paths.insert("manifest.json.sig".into());
    Ok(paths)
}

fn enumerate_files(root: &Path) -> Result<BTreeSet<String>, VerifyError> {
    fn visit(root: &Path, dir: &Path, result: &mut BTreeSet<String>) -> Result<(), VerifyError> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(VerifyError::Topology("symlink in bundle".into()));
            }
            if metadata.is_dir() {
                visit(root, &path, result)?;
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| VerifyError::Topology("bundle entry escaped root".into()))?;
                let value = relative
                    .to_str()
                    .ok_or_else(|| VerifyError::Topology("bundle path is not UTF-8".into()))?;
                validate_relative_path(value)?;
                result.insert(value.to_owned());
                if result.len() > MAX_ARTIFACTS + 4 {
                    return Err(VerifyError::Limit("file count".into()));
                }
            } else {
                return Err(VerifyError::Topology("non-regular bundle entry".into()));
            }
        }
        Ok(())
    }

    let mut result = BTreeSet::new();
    visit(root, root, &mut result)?;
    Ok(result)
}

fn read_regular_bounded(path: &Path, max: u64) -> Result<Vec<u8>, VerifyError> {
    let mut file = open_regular(path)?;
    let metadata = file.metadata()?;
    if metadata.len() > max {
        return Err(VerifyError::Limit("file size".into()));
    }
    let capacity = usize::try_from(metadata.len())
        .map_err(|_| VerifyError::Limit("file size is not addressable".into()))?;
    let mut bytes = Vec::with_capacity(capacity);
    Read::by_ref(&mut file)
        .take(max + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(VerifyError::Limit("file grew while reading".into()));
    }
    Ok(bytes)
}

fn hash_regular_file(path: &Path) -> Result<(u64, String), VerifyError> {
    let mut file = open_regular(path)?;
    let before = file.metadata()?;
    let mut hasher = Sha256::new();
    let copied = std::io::copy(&mut file, &mut hasher)?;
    let after = file.metadata()?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || copied != after.len()
    {
        return Err(VerifyError::Content(
            "file changed while it was verified".into(),
        ));
    }
    Ok((copied, format!("{:x}", hasher.finalize())))
}

fn open_regular(path: &Path) -> Result<File, VerifyError> {
    let mut options = fs::OpenOptions::new();
    options.read(true).custom_flags(no_follow_flags());
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(VerifyError::Topology("expected regular file".into()));
    }
    Ok(file)
}

fn validate_regular_file_bounded(path: &Path, max: u64) -> Result<(), VerifyError> {
    let file = open_regular(path)?;
    if file.metadata()?.len() > max {
        Err(VerifyError::Limit("trusted metadata file size".into()))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
const fn no_follow_flags() -> i32 {
    0o400000 | 0o2000000
}

#[cfg(not(target_os = "linux"))]
const fn no_follow_flags() -> i32 {
    0
}

fn verify_signature(
    manifest: &[u8],
    signature: &Path,
    config: &VerifierConfig,
) -> Result<(), VerifyError> {
    bounded_nonempty("principal", &config.principal)?;
    validate_hash(&config.ssh_keygen_sha256)?;
    let (_, ssh_keygen_sha256) =
        hash_regular_file(&config.ssh_keygen).map_err(|_| VerifyError::Signature)?;
    if ssh_keygen_sha256 != config.ssh_keygen_sha256 {
        return Err(VerifyError::Signature);
    }
    validate_regular_file_bounded(&config.allowed_signers, MAX_ALLOWED_SIGNERS_BYTES)?;
    let mut child = Command::new(&config.ssh_keygen)
        .args([
            OsStr::new("-Y"),
            OsStr::new("verify"),
            OsStr::new("-f"),
            config.allowed_signers.as_os_str(),
            OsStr::new("-I"),
            OsStr::new(&config.principal),
            OsStr::new("-n"),
            OsStr::new(SIGNATURE_NAMESPACE),
            OsStr::new("-s"),
            signature.as_os_str(),
        ])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| VerifyError::Signature)?;
    child
        .stdin
        .take()
        .ok_or(VerifyError::Signature)?
        .write_all(manifest)
        .map_err(|_| VerifyError::Signature)?;
    if child.wait().map_err(|_| VerifyError::Signature)?.success() {
        Ok(())
    } else {
        Err(VerifyError::Signature)
    }
}

fn read_and_hash_sbom(root: &Path, document: &SbomDocument) -> Result<Value, VerifyError> {
    let bytes = read_regular_bounded(&root.join(&document.path), MAX_SBOM_BYTES)?;
    if sha256(&bytes) != document.sha256 {
        return Err(VerifyError::Content("SBOM digest differs".into()));
    }
    serde_json::from_slice(&bytes).map_err(|_| VerifyError::Metadata("SBOM is not JSON".into()))
}

fn validate_spdx(
    document: &Value,
    artifacts: &BTreeMap<&str, &BundleArtifact>,
) -> Result<(), VerifyError> {
    if document.get("spdxVersion").and_then(Value::as_str) != Some("SPDX-2.3") {
        return Err(VerifyError::Metadata("SPDX version is not 2.3".into()));
    }
    let files = document
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| VerifyError::Metadata("SPDX files are absent".into()))?;
    let mut seen = BTreeSet::new();
    for file in files {
        let path = required_string(file, "fileName", "SPDX file")?;
        let artifact = artifacts
            .get(path)
            .ok_or_else(|| VerifyError::Metadata("SPDX has an unknown file".into()))?;
        if !seen.insert(path) {
            return Err(VerifyError::Metadata("SPDX repeats a file".into()));
        }
        let checksum = file
            .get("checksums")
            .and_then(Value::as_array)
            .and_then(|checksums| {
                checksums.iter().find_map(|item| {
                    (item.get("algorithm")?.as_str()? == "SHA256")
                        .then(|| item.get("checksumValue")?.as_str())
                        .flatten()
                })
            });
        if checksum != Some(artifact.sha256.as_str())
            || file.get("licenseConcluded").and_then(Value::as_str)
                != Some(artifact.license_expression.as_str())
        {
            return Err(VerifyError::Metadata("SPDX file identity differs".into()));
        }
    }
    require_exact_parity("SPDX", seen, artifacts)
}

fn validate_cyclonedx(
    document: &Value,
    artifacts: &BTreeMap<&str, &BundleArtifact>,
) -> Result<(), VerifyError> {
    if document.get("bomFormat").and_then(Value::as_str) != Some("CycloneDX")
        || document.get("specVersion").and_then(Value::as_str) != Some("1.6")
    {
        return Err(VerifyError::Metadata(
            "CycloneDX identity is not 1.6".into(),
        ));
    }
    let components = document
        .get("components")
        .and_then(Value::as_array)
        .ok_or_else(|| VerifyError::Metadata("CycloneDX components are absent".into()))?;
    let mut seen = BTreeSet::new();
    for component in components {
        if component.get("type").and_then(Value::as_str) != Some("file") {
            return Err(VerifyError::Metadata(
                "CycloneDX component is not a file".into(),
            ));
        }
        let path = required_string(component, "bom-ref", "CycloneDX component")?;
        let artifact = artifacts
            .get(path)
            .ok_or_else(|| VerifyError::Metadata("CycloneDX has an unknown file".into()))?;
        if !seen.insert(path) {
            return Err(VerifyError::Metadata("CycloneDX repeats a file".into()));
        }
        let checksum = component
            .get("hashes")
            .and_then(Value::as_array)
            .and_then(|hashes| {
                hashes.iter().find_map(|item| {
                    (item.get("alg")?.as_str()? == "SHA-256")
                        .then(|| item.get("content")?.as_str())
                        .flatten()
                })
            });
        let license = component
            .pointer("/licenses/0/expression")
            .and_then(Value::as_str);
        if checksum != Some(artifact.sha256.as_str())
            || license != Some(artifact.license_expression.as_str())
        {
            return Err(VerifyError::Metadata(
                "CycloneDX file identity differs".into(),
            ));
        }
    }
    require_exact_parity("CycloneDX", seen, artifacts)
}

fn require_exact_parity(
    name: &str,
    seen: BTreeSet<&str>,
    artifacts: &BTreeMap<&str, &BundleArtifact>,
) -> Result<(), VerifyError> {
    if seen.len() == artifacts.len() && artifacts.keys().all(|path| seen.contains(path)) {
        Ok(())
    } else {
        Err(VerifyError::Metadata(format!(
            "{name} does not cover the exact artifact inventory"
        )))
    }
}

fn required_string<'a>(
    value: &'a Value,
    field: &str,
    context: &str,
) -> Result<&'a str, VerifyError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| VerifyError::Metadata(format!("{context} lacks required {field}")))
}

fn update_content_digest(hasher: &mut Sha256, artifact: &BundleArtifact) {
    let size = artifact.size.to_string();
    for value in [
        artifact.path.as_bytes(),
        size.as_bytes(),
        artifact.sha256.as_bytes(),
        if artifact.executable {
            b"1".as_slice()
        } else {
            b"0".as_slice()
        },
        artifact.license_expression.as_bytes(),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    hasher.update((artifact.license_evidence.len() as u64).to_be_bytes());
    for evidence in &artifact.license_evidence {
        hasher.update((evidence.len() as u64).to_be_bytes());
        hasher.update(evidence.as_bytes());
    }
}

/// Calculate the canonical content identity for a strictly path-sorted artifact inventory.
#[must_use]
pub fn content_digest(artifacts: &[BundleArtifact]) -> String {
    let mut hasher = Sha256::new();
    for artifact in artifacts {
        update_content_digest(&mut hasher, artifact);
    }
    format!("{:x}", hasher.finalize())
}

fn validate_relative_path(value: &str) -> Result<(), VerifyError> {
    bounded_nonempty("path", value)?;
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(VerifyError::Topology(
            "path is not normalized relative UTF-8".into(),
        ));
    }
    Ok(())
}

fn validate_hash(value: &str) -> Result<(), VerifyError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(VerifyError::Metadata(
            "SHA-256 is not lowercase hexadecimal".into(),
        ))
    }
}

fn bounded_nonempty(name: &str, value: &str) -> Result<(), VerifyError> {
    if value.is_empty() || value.len() > MAX_STRING_BYTES || value.chars().any(char::is_control) {
        Err(VerifyError::Metadata(format!(
            "{name} is empty, oversized, or contains controls"
        )))
    } else {
        Ok(())
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Generate the canonical JSON Schema for runtime bundle manifests.
#[must_use]
pub fn manifest_schema() -> schemars::Schema {
    schemars::schema_for!(RuntimeBundleManifest)
}
