// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Development-only cli2key provider identities.
#![allow(missing_docs)]
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub const CLI2KEY_SCHEMA_VERSION: u16 = 1;
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cli2KeyModel {
    pub model_id: String,
    pub revision: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cli2KeyCatalog {
    pub schema_version: u16,
    pub provider_id: String,
    pub classification: String,
    pub catalog_generation: u64,
    pub bridge_revision: String,
    pub bridge_tree_sha256: String,
    pub executable_sha256: String,
    pub endpoint_sha256: String,
    pub models: Vec<Cli2KeyModel>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cli2KeySelection {
    pub schema_version: u16,
    pub provider_id: String,
    pub catalog_generation: u64,
    pub catalog_sha256: String,
    pub model_id: String,
    pub bridge_revision: String,
    pub bridge_tree_sha256: String,
    pub executable_sha256: String,
    pub endpoint_sha256: String,
    pub credential_reference_sha256: String,
    pub agent_id: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cli2KeyLaunch {
    pub schema_version: u16,
    pub selection: Cli2KeySelection,
    pub api_mode: String,
    pub endpoint_host: String,
    pub endpoint_port: u16,
    pub generation: String,
}
fn digest(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b"asb-cli2key-catalog-v1\0");
    h.update(bytes);
    format!("{:x}", h.finalize())
}
fn valid_digest(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
impl Cli2KeyCatalog {
    pub fn digest(&self) -> Result<String, &'static str> {
        self.validate()?;
        Ok(digest(
            &serde_json::to_vec(self).map_err(|_| "catalog encoding")?,
        ))
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != 1
            || self.provider_id != "cli2key"
            || self.classification != "development-only-unofficial"
            || self.catalog_generation == 0
            || self.bridge_revision.is_empty()
            || !valid_digest(&self.bridge_tree_sha256)
            || !valid_digest(&self.executable_sha256)
            || !valid_digest(&self.endpoint_sha256)
            || self.models.is_empty()
            || self
                .models
                .iter()
                .any(|m| m.model_id.is_empty() || m.revision.is_empty())
        {
            return Err("invalid cli2key catalog");
        }
        Ok(())
    }
}
impl Cli2KeySelection {
    pub fn validate_against(&self, c: &Cli2KeyCatalog) -> Result<(), &'static str> {
        c.validate()?;
        if self.schema_version != 1
            || self.provider_id != "cli2key"
            || self.catalog_generation != c.catalog_generation
            || self.bridge_revision != c.bridge_revision
            || self.bridge_tree_sha256 != c.bridge_tree_sha256
            || self.executable_sha256 != c.executable_sha256
            || self.endpoint_sha256 != c.endpoint_sha256
            || self.agent_id != "codex"
            || !valid_digest(&self.catalog_sha256)
            || !valid_digest(&self.credential_reference_sha256)
            || !c.models.iter().any(|m| m.model_id == self.model_id)
        {
            return Err("cli2key selection is stale or unsupported");
        }
        if self.catalog_sha256 != c.digest()? {
            return Err("cli2key catalog digest mismatch");
        }
        Ok(())
    }
}
impl Cli2KeyLaunch {
    /// Validate the secret-free launch shape before a catalog is available.
    pub fn validate_shape(&self) -> Result<(), &'static str> {
        if self.schema_version != 1
            || self.api_mode != "responses"
            || !matches!(self.endpoint_host.as_str(), "127.0.0.1" | "::1")
            || self.endpoint_port == 0
            || self.generation.is_empty()
            || self.generation.len() > 256
            || !self
                .generation
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return Err("cli2key launch is not loopback Responses");
        }
        if self.selection.provider_id != "cli2key"
            || self.selection.agent_id != "codex"
            || !valid_digest(&self.selection.catalog_sha256)
            || !valid_digest(&self.selection.credential_reference_sha256)
            || !valid_digest(&self.selection.endpoint_sha256)
        {
            return Err("cli2key selection is invalid");
        }
        Ok(())
    }

    pub fn validate_against(&self, c: &Cli2KeyCatalog) -> Result<(), &'static str> {
        self.validate_shape()?;
        self.selection.validate_against(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn catalog() -> Cli2KeyCatalog {
        Cli2KeyCatalog {
            schema_version: 1,
            provider_id: "cli2key".into(),
            classification: "development-only-unofficial".into(),
            catalog_generation: 7,
            bridge_revision: "bridge-r1".into(),
            bridge_tree_sha256: "a".repeat(64),
            executable_sha256: "b".repeat(64),
            endpoint_sha256: "c".repeat(64),
            models: vec![Cli2KeyModel {
                model_id: "codex-local".into(),
                revision: "m1".into(),
            }],
        }
    }
    #[test]
    fn codex_responses_loopback_selection_is_bound() {
        let c = catalog();
        let s = Cli2KeySelection {
            schema_version: 1,
            provider_id: "cli2key".into(),
            catalog_generation: 7,
            catalog_sha256: c.digest().unwrap(),
            model_id: "codex-local".into(),
            bridge_revision: "bridge-r1".into(),
            bridge_tree_sha256: "a".repeat(64),
            executable_sha256: "b".repeat(64),
            endpoint_sha256: "c".repeat(64),
            credential_reference_sha256: "d".repeat(64),
            agent_id: "codex".into(),
        };
        let l = Cli2KeyLaunch {
            schema_version: 1,
            selection: s,
            api_mode: "responses".into(),
            endpoint_host: "127.0.0.1".into(),
            endpoint_port: 1234,
            generation: "generation-7".into(),
        };
        assert!(l.validate_against(&c).is_ok());
    }
    #[test]
    fn stale_or_official_or_remote_inputs_fail_closed() {
        let c = catalog();
        let mut s = Cli2KeySelection {
            schema_version: 1,
            provider_id: "openai".into(),
            catalog_generation: 7,
            catalog_sha256: c.digest().unwrap(),
            model_id: "codex-local".into(),
            bridge_revision: "bridge-r1".into(),
            bridge_tree_sha256: "a".repeat(64),
            executable_sha256: "b".repeat(64),
            endpoint_sha256: "c".repeat(64),
            credential_reference_sha256: "d".repeat(64),
            agent_id: "codex".into(),
        };
        assert!(s.validate_against(&c).is_err());
        s.provider_id = "cli2key".into();
        s.catalog_generation = 6;
        assert!(s.validate_against(&c).is_err());
        let l = Cli2KeyLaunch {
            schema_version: 1,
            selection: s,
            api_mode: "responses".into(),
            endpoint_host: "203.0.113.9".into(),
            endpoint_port: 1234,
            generation: "generation-6".into(),
        };
        assert!(l.validate_against(&c).is_err());
    }
    #[test]
    fn unknown_fields_are_rejected() {
        let bad = r#"{"schema_version":1,"provider_id":"cli2key","classification":"development-only-unofficial","catalog_generation":1,"bridge_revision":"r","bridge_tree_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","executable_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","endpoint_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","models":[],"secret":"must-not-parse"}"#;
        assert!(serde_json::from_str::<Cli2KeyCatalog>(bad).is_err());
    }
}
