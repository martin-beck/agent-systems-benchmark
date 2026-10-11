// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! The checked-in authority for supported executable acquisition.
//!
//! This catalog is intentionally data-only: it never downloads, invokes a
//! package manager, or trusts a PATH entry.  Install and discovery callers
//! must verify a candidate against one of these identities before reuse.
#![allow(missing_docs)]

use serde::{Deserialize, Serialize};

pub const CATALOG_SCHEMA_VERSION: u16 = 1;
pub const CATALOG_ID: &str = "asb-supported-tools-v1";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolCatalog {
    pub schema_version: u16,
    pub catalog_id: String,
    pub entries: Vec<ToolEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolEntry {
    pub id: String,
    pub kind: String,
    pub development_only: bool,
    pub official_source: OfficialSource,
    pub platforms: Vec<Platform>,
    pub alternatives: Vec<ArtifactAlternative>,
    pub entrypoint: String,
    pub license: String,
    pub dependencies: Vec<String>,
    pub build_recipes: Vec<BuildRecipe>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OfficialSource {
    pub url: String,
    pub version: String,
    pub revision: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Platform {
    pub os: String,
    pub arch: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactAlternative {
    pub kind: String,
    pub url: String,
    pub sha256: String,
    pub verifier: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildRecipe {
    pub id: String,
    pub source_revision: String,
    pub argv: Vec<String>,
    pub output: String,
    /// Immutable toolchain PATH used for the build; ambient PATH is ignored.
    #[serde(default = "default_toolchain")]
    pub toolchain: String,
    /// Maximum wall-clock duration for the recipe.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// Maximum combined diagnostic bytes retained by the runner.
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: u64,
    /// Maximum produced entrypoint size.
    #[serde(default = "default_max_artifact_bytes")]
    pub max_artifact_bytes: u64,
}

fn default_toolchain() -> String {
    "/usr/bin:/bin".into()
}
fn default_timeout_ms() -> u64 {
    15 * 60 * 1000
}
fn default_max_output_bytes() -> u64 {
    1024 * 1024
}
fn default_max_artifact_bytes() -> u64 {
    1024 * 1024 * 1024
}

/// Return the immutable catalog shipped with this ASB version.
pub fn catalog() -> ToolCatalog {
    serde_json::from_str(include_str!("../fixtures/v1/tool-acquisition-catalog.json"))
        .expect("checked-in tool acquisition catalog must be valid")
}

/// Validate a catalog before it can influence installation or reuse.
/// Unknown identities, missing verification, floating versions and unsafe
/// command recipes are rejected rather than downgraded to a best effort.
pub fn validate(catalog: &ToolCatalog) -> Result<(), &'static str> {
    if catalog.schema_version != CATALOG_SCHEMA_VERSION || catalog.catalog_id != CATALOG_ID {
        return Err("unsupported catalog identity");
    }
    if catalog.entries.is_empty() || catalog.entries.len() > 32 {
        return Err("catalog entry count is out of bounds");
    }
    let mut ids = std::collections::BTreeSet::new();
    for entry in &catalog.entries {
        if !ids.insert(&entry.id) || !valid_id(&entry.id) || entry.kind.is_empty() {
            return Err("catalog contains duplicate or invalid tool identity");
        }
        if !entry.official_source.url.starts_with("https://")
            || entry.official_source.version.is_empty()
            || entry.official_source.revision.is_empty()
        {
            return Err("catalog source is not pinned to an official identity");
        }
        if entry.platforms.is_empty() || entry.alternatives.is_empty() {
            return Err("catalog entry has no bounded platform or artifact alternative");
        }
        for artifact in &entry.alternatives {
            if !artifact.url.starts_with("https://")
                || !is_sha256(&artifact.sha256)
                || artifact.verifier.is_empty()
            {
                return Err("catalog artifact lacks immutable verification");
            }
        }
        if entry.entrypoint.is_empty() || entry.license.is_empty() {
            return Err("catalog entry lacks executable or license identity");
        }
        for recipe in &entry.build_recipes {
            if recipe.id.is_empty()
                || recipe.source_revision != entry.official_source.revision
                || recipe.argv.is_empty()
                || recipe
                    .argv
                    .iter()
                    .any(|arg| arg.contains('|') || arg.contains(';'))
                || recipe.output.is_empty()
                || recipe.toolchain.is_empty()
                || recipe.timeout_ms == 0
                || recipe.timeout_ms > 30 * 60 * 1000
                || recipe.max_output_bytes == 0
                || recipe.max_output_bytes > 1024 * 1024
                || recipe.max_artifact_bytes == 0
                || recipe.max_artifact_bytes > 1024 * 1024 * 1024
            {
                return Err("catalog build recipe is unbounded or mismatched");
            }
        }
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_catalog_is_complete_and_valid() {
        let catalog = catalog();
        validate(&catalog).unwrap();
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
            assert!(
                catalog.entries.iter().any(|entry| entry.id == id),
                "missing {id}"
            );
        }
    }

    #[test]
    fn missing_verifier_and_recipe_mismatch_fail_closed() {
        let mut value = catalog();
        value.entries[0].alternatives[0].sha256 = "0".into();
        assert_eq!(
            validate(&value),
            Err("catalog artifact lacks immutable verification")
        );
        let mut value = catalog();
        value.entries[0].build_recipes[0].source_revision = "wrong".into();
        assert_eq!(
            validate(&value),
            Err("catalog build recipe is unbounded or mismatched")
        );
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let result = serde_json::from_str::<ToolCatalog>(
            r#"{"schema_version":1,"catalog_id":"asb-supported-tools-v1","entries":[],"extra":true}"#,
        );
        assert!(result.is_err());
    }
}
