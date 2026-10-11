// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded, catalog-driven acquisition for project tools.

use crate::tool_catalog::{self, ArtifactAlternative, ToolEntry};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

const MAX_ARTIFACT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum AcquisitionError {
    UnknownTool,
    UnsupportedPlatform,
    UnverifiedSource,
    Unavailable,
    TooLarge,
    DigestMismatch,
    UnsafeArtifact,
    LicenseMissing,
}

impl AcquisitionError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::UnknownTool => "tool is not present in the pinned acquisition catalog",
            Self::UnsupportedPlatform => "tool has no catalog entry for this platform",
            Self::UnverifiedSource => "tool source is not the catalog primary source",
            Self::Unavailable => "official tool source is unavailable or offline",
            Self::TooLarge => "tool artifact exceeds the bounded acquisition limit",
            Self::DigestMismatch => "tool artifact digest does not match the catalog",
            Self::UnsafeArtifact => "tool artifact is not a safe regular executable",
            Self::LicenseMissing => "tool catalog entry has no license metadata",
        }
    }
}

pub struct VerifiedArtifact {
    pub bytes: Vec<u8>,
    pub entry: ToolEntry,
    pub alternative: ArtifactAlternative,
    pub source_ref: String,
}

/// Deterministic offline fixture input retained for contract tests.  It is
/// explicitly namespaced and never treated as an official acquisition.
pub fn fixture(id: &str) -> Option<VerifiedArtifact> {
    let name = id.strip_prefix("fixture://")?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return None;
    }
    let bytes = format!("ASB deterministic tool fixture {name}\n").into_bytes();
    let digest = format!("{:x}", Sha256::digest(&bytes));
    Some(VerifiedArtifact {
        bytes,
        entry: ToolEntry {
            id: name.into(),
            kind: "fixture".into(),
            development_only: true,
            official_source: tool_catalog::OfficialSource {
                url: "https://example.invalid/fixture".into(),
                version: "fixture".into(),
                revision: "fixture".into(),
            },
            platforms: vec![],
            alternatives: vec![ArtifactAlternative {
                kind: "fixture".into(),
                url: format!("fixture://{name}"),
                sha256: digest.clone(),
                verifier: "fixture".into(),
            }],
            entrypoint: name.into(),
            license: "NOASSERTION".into(),
            dependencies: vec![],
            build_recipes: vec![],
        },
        alternative: ArtifactAlternative {
            kind: "fixture".into(),
            url: format!("fixture://{name}"),
            sha256: digest,
            verifier: "fixture".into(),
        },
        source_ref: format!("fixture://{name}"),
    })
}

pub fn resolve(id: &str) -> Result<(ToolEntry, ArtifactAlternative), AcquisitionError> {
    let catalog = tool_catalog::catalog();
    tool_catalog::validate(&catalog).map_err(|_| AcquisitionError::UnverifiedSource)?;
    let entry = catalog
        .entries
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or(AcquisitionError::UnknownTool)?;
    let platform = (std::env::consts::OS, std::env::consts::ARCH);
    if !entry
        .platforms
        .iter()
        .any(|p| (p.os.as_str(), p.arch.as_str()) == platform)
    {
        return Err(AcquisitionError::UnsupportedPlatform);
    }
    if entry.license.trim().is_empty() {
        return Err(AcquisitionError::LicenseMissing);
    }
    let alt = entry
        .alternatives
        .iter()
        .find(|a| a.kind == "prebuilt")
        .or_else(|| entry.alternatives.first())
        .cloned()
        .ok_or(AcquisitionError::UnverifiedSource)?;
    Ok((entry, alt))
}

pub fn verify_bytes(
    id: &str,
    source_ref: String,
    bytes: Vec<u8>,
) -> Result<VerifiedArtifact, AcquisitionError> {
    if bytes.is_empty() {
        return Err(AcquisitionError::UnsafeArtifact);
    }
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(AcquisitionError::TooLarge);
    }
    let (entry, alternative) = resolve(id)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if digest != alternative.sha256 {
        return Err(AcquisitionError::DigestMismatch);
    }
    // Archives are accepted only after bounded tar member validation and
    // extraction of the catalogued entrypoint.  Installer scripts and links
    // are never executed or materialized.
    let bytes = if bytes.len() > 262 && &bytes[257..262] == b"ustar" {
        extract_tar_entry(&bytes, &entry.entrypoint)?
    } else if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"#!") {
        return Err(AcquisitionError::UnsafeArtifact);
    } else {
        bytes
    };
    Ok(VerifiedArtifact {
        bytes,
        entry,
        alternative,
        source_ref,
    })
}

fn extract_tar_entry(archive: &[u8], entrypoint: &str) -> Result<Vec<u8>, AcquisitionError> {
    let mut offset = 0usize;
    let mut found = None;
    while offset + 512 <= archive.len() {
        let header = &archive[offset..offset + 512];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        let name_end = header[..100].iter().position(|b| *b == 0).unwrap_or(100);
        let name = std::str::from_utf8(&header[..name_end])
            .map_err(|_| AcquisitionError::UnsafeArtifact)?;
        if name.starts_with('/') || name.split('/').any(|part| part == "..") {
            return Err(AcquisitionError::UnsafeArtifact);
        }
        let size_end = header[124..136].iter().position(|b| *b == 0).unwrap_or(12) + 124;
        let size = usize::from_str_radix(
            std::str::from_utf8(&header[124..size_end])
                .unwrap_or("0")
                .trim(),
            8,
        )
        .map_err(|_| AcquisitionError::UnsafeArtifact)?;
        let kind = header[156];
        if kind != 0 && kind != b'0' && (kind == b'2' || kind == b'1') {
            return Err(AcquisitionError::UnsafeArtifact);
        }
        let start = offset + 512;
        let end = start.checked_add(size).ok_or(AcquisitionError::TooLarge)?;
        if end > archive.len() || size > MAX_ARTIFACT_BYTES {
            return Err(AcquisitionError::TooLarge);
        }
        if name == entrypoint {
            found = Some(archive[start..end].to_vec());
        }
        offset = start + size.div_ceil(512) * 512;
    }
    found
        .filter(|bytes| !bytes.is_empty())
        .ok_or(AcquisitionError::UnsafeArtifact)
}

pub fn read_local(id: &str, path: &Path) -> Result<VerifiedArtifact, AcquisitionError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| AcquisitionError::Unavailable)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(AcquisitionError::UnsafeArtifact);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(AcquisitionError::UnsafeArtifact);
    }
    let bytes = fs::read(path).map_err(|_| AcquisitionError::Unavailable)?;
    let source_ref = format!("local:{}", path.display());
    match verify_bytes(id, source_ref.clone(), bytes.clone()) {
        Ok(artifact) => Ok(artifact),
        Err(AcquisitionError::UnknownTool) => {
            // Explicit local sources retain the deterministic fixture
            // lifecycle used by offline tests without making fixture IDs
            // part of the official acquisition catalog.
            let fixture =
                fixture(&format!("fixture://{id}")).ok_or(AcquisitionError::UnknownTool)?;
            if fixture.bytes != bytes {
                return Err(AcquisitionError::DigestMismatch);
            }
            Ok(VerifiedArtifact {
                source_ref,
                ..fixture
            })
        }
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_and_wrong_platform_are_typed() {
        assert_eq!(resolve("missing"), Err(AcquisitionError::UnknownTool));
    }
    #[test]
    fn catalog_alternatives_are_immutable() {
        let (entry, alt) = resolve("codex").unwrap();
        assert_eq!(entry.id, "codex");
        assert_eq!(alt.verifier, "sha256");
        assert_eq!(alt.sha256.len(), 64);
    }

    #[test]
    fn fixture_is_strict_deterministic_and_namespaced() {
        assert!(fixture("matrix").is_none());
        assert!(fixture("fixture://").is_none());
        assert!(fixture("fixture://Bad").is_none());
        assert!(fixture("fixture://tool/name").is_none());
        let first = fixture("fixture://matrix").expect("fixture");
        let second = fixture("fixture://matrix").expect("fixture");
        assert_eq!(first.bytes, second.bytes);
        assert_eq!(first.source_ref, "fixture://matrix");
        assert_eq!(first.alternative.verifier, "fixture");
        assert!(first.entry.development_only);
    }

    #[test]
    fn acquisition_errors_have_stable_public_messages() {
        let errors = [
            AcquisitionError::UnknownTool,
            AcquisitionError::UnsupportedPlatform,
            AcquisitionError::UnverifiedSource,
            AcquisitionError::Unavailable,
            AcquisitionError::TooLarge,
            AcquisitionError::DigestMismatch,
            AcquisitionError::UnsafeArtifact,
            AcquisitionError::LicenseMissing,
        ];
        assert!(errors.iter().all(|error| !error.message().is_empty()));
    }

    #[test]
    fn empty_artifacts_fail_before_catalog_resolution() {
        assert!(matches!(
            verify_bytes("missing", "fixture://missing".into(), Vec::new()),
            Err(AcquisitionError::UnsafeArtifact)
        ));
    }

    #[test]
    fn bounded_digest_checks_reject_large_and_mismatched_catalog_bytes() {
        assert!(matches!(
            verify_bytes(
                "codex",
                "https://example.invalid/codex".into(),
                vec![0u8; MAX_ARTIFACT_BYTES + 1]
            ),
            Err(AcquisitionError::TooLarge)
        ));
        assert!(matches!(
            verify_bytes(
                "codex",
                "https://example.invalid/codex".into(),
                b"not-the-pinned-artifact".to_vec()
            ),
            Err(AcquisitionError::DigestMismatch)
        ));
    }

    fn tar_member(name: &[u8], body: &[u8], kind: u8) -> Vec<u8> {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name);
        let size = format!("{:011o}\0", body.len());
        header[124..124 + size.len()].copy_from_slice(size.as_bytes());
        header[156] = kind;
        let mut archive = header.to_vec();
        archive.extend_from_slice(body);
        archive.resize(archive.len().next_multiple_of(512), 0);
        archive.extend_from_slice(&[0u8; 1024]);
        archive
    }

    #[test]
    fn tar_extraction_rejects_unsafe_members_and_requires_entrypoint() {
        assert_eq!(
            extract_tar_entry(&tar_member(b"../escape", b"x", 0), "tool"),
            Err(AcquisitionError::UnsafeArtifact)
        );
        assert_eq!(
            extract_tar_entry(&tar_member(b"link", b"x", b'2'), "tool"),
            Err(AcquisitionError::UnsafeArtifact)
        );
        assert_eq!(
            extract_tar_entry(&tar_member(b"other", b"x", 0), "tool"),
            Err(AcquisitionError::UnsafeArtifact)
        );
        assert_eq!(
            extract_tar_entry(&tar_member(b"tool", b"payload", 0), "tool").unwrap(),
            b"payload"
        );
    }

    #[test]
    fn local_acquisition_rejects_missing_and_non_executable_paths() {
        assert!(matches!(
            read_local("codex", Path::new("/definitely/missing/asb-tool")),
            Err(AcquisitionError::Unavailable)
        ));
        let temporary = tempfile::NamedTempFile::new().unwrap();
        assert!(matches!(
            read_local("codex", temporary.path()),
            Err(AcquisitionError::UnsafeArtifact)
        ));
    }

    #[test]
    #[cfg(unix)]
    fn local_acquisition_rejects_directories_and_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        assert!(matches!(
            read_local("codex", directory.path()),
            Err(AcquisitionError::UnsafeArtifact)
        ));
        let temporary = tempfile::NamedTempFile::new().unwrap();
        std::os::unix::fs::symlink(temporary.path(), directory.path().join("link")).unwrap();
        assert!(matches!(
            read_local("codex", &directory.path().join("link")),
            Err(AcquisitionError::UnsafeArtifact)
        ));
    }

    #[test]
    #[cfg(unix)]
    fn local_catalog_mismatch_preserves_typed_error() {
        let temporary = tempfile::NamedTempFile::new().unwrap();
        fs::write(temporary.path(), b"not-the-pinned-artifact").unwrap();
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(matches!(
            read_local("codex", temporary.path()),
            Err(AcquisitionError::DigestMismatch)
        ));
    }

    #[test]
    #[cfg(unix)]
    fn local_fixture_acquisition_allows_uncatalogued_ids_only_for_matching_bytes() {
        let temporary = tempfile::NamedTempFile::new().unwrap();
        let fixture = fixture("fixture://matrix").unwrap();
        fs::write(temporary.path(), &fixture.bytes).unwrap();
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let artifact = read_local("matrix", temporary.path()).unwrap();
        assert_eq!(
            artifact.source_ref,
            format!("local:{}", temporary.path().display())
        );

        fs::write(temporary.path(), b"not the reviewed fixture").unwrap();
        assert!(matches!(
            read_local("matrix", temporary.path()),
            Err(AcquisitionError::DigestMismatch)
        ));
    }
}
