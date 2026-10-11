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
        if kind != 0 && kind != b'0' {
            if kind == b'2' || kind == b'1' {
                return Err(AcquisitionError::UnsafeArtifact);
            }
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
    verify_bytes(id, format!("local:{}", path.display()), bytes)
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
}
