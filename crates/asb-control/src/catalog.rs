// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Renderer-neutral, authenticated local-agent catalog types.

use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::{ProtocolError, Revision, validate_digest, validate_identity};

const MAX_CATALOG_AGENTS: usize = 32;
const MAX_CAPABILITIES: usize = 32;
const MAX_CATALOG_STRING_BYTES: usize = 128;

/// Read-only catalog operation requested by a frontend.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCatalogAction {
    /// Return the current verified snapshot without external effects.
    Status,
    /// Refresh the snapshot from locally configured, authorized sources.
    Refresh,
}

/// Request for a runner-generation-bound catalog operation.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCatalogRequest {
    /// Operation to perform.
    pub action: AgentCatalogAction,
    /// Generation identity received during protocol negotiation.
    pub runner_instance_id: String,
    /// Last catalog generation observed by the frontend, if any.
    pub known_generation: Option<Revision>,
}

/// Explicit reason an agent cannot currently be selected.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AgentUnavailableReason {
    /// No compatible package exists for the current target.
    IncompatibleTarget,
    /// Package or dependency closure is not complete.
    IncompleteProvenance,
    /// Package signature or digest could not be verified.
    UnverifiedArtifact,
    /// Required local capability is absent.
    MissingCapability,
    /// Catalog data is older than its freshness bound.
    StaleCatalog,
    /// The agent is not permitted by runner policy.
    PolicyDenied,
}

/// Availability projection for one catalog entry.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(
    tag = "status",
    content = "reason",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AgentAvailability {
    /// The pinned artifact can be selected by a subsequent operation.
    Available,
    /// The entry is visible for explanation but must not be selected.
    Unavailable(AgentUnavailableReason),
}

/// Exact target constraints for one agent package.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTarget {
    /// Operating-system identity.
    pub operating_system: String,
    /// CPU architecture identity.
    pub architecture: String,
    /// libc family identity.
    pub libc: String,
    /// Exact libc ABI identity.
    pub libc_version: String,
}

/// Signed package identity exposed by the catalog.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPackage {
    /// Stable package identity, not a filesystem path or URL.
    pub package_id: String,
    /// Exact package version.
    pub version: String,
    /// SHA-256 of the package bytes.
    pub sha256: String,
    /// SHA-256 of the detached signature bytes.
    pub signature_sha256: String,
    /// Public signing-key identity, never the key material itself.
    pub signer: AgentSigner,
}

/// Public identity of the key and principal that signed the package.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSigner {
    /// Stable operator-provisioned key identity.
    pub key_id: String,
    /// Bounded public signing principal.
    pub principal: String,
}

/// Immutable source and runtime provenance for one package.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenance {
    /// Exact source revision producing the package.
    pub source_revision: String,
    /// SHA-256 of the signed runtime manifest.
    pub manifest_sha256: String,
    /// SHA-256 of the SBOM describing the complete package inventory.
    pub sbom_sha256: String,
    /// SPDX license identifier or expression for the package.
    pub license_ref: String,
}

/// One stable agent entry. It is safe to expose and contains no credentials.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCatalogEntry {
    /// Stable canonical adapter identifier.
    pub agent_id: String,
    /// Exact compatible target.
    pub target: AgentTarget,
    /// Pinned package identity when the artifact has been verified.
    pub package: Option<AgentPackage>,
    /// Immutable source/runtime provenance when the closure is complete.
    pub provenance: Option<AgentProvenance>,
    /// Sorted capability identifiers supported by this package.
    pub capabilities: Vec<String>,
    /// Explicit selection decision and, when unavailable, its reason.
    pub availability: AgentAvailability,
}

/// Authenticated, target-bound catalog snapshot returned by the runner.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCatalog {
    /// Runner instance/generation identity from negotiation.
    pub runner_instance_id: String,
    /// Monotonic catalog generation.
    pub generation: Revision,
    /// SHA-256 over the canonical catalog snapshot.
    pub catalog_sha256: String,
    /// Target for which every entry was evaluated.
    pub target: AgentTarget,
    /// Complete sorted catalog, including unavailable entries.
    pub agents: Vec<AgentCatalogEntry>,
    /// True only when this response performed a refresh.
    pub refreshed: bool,
}

impl AgentCatalogRequest {
    /// Validate request shape and the generation binding supplied by the client.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.runner_instance_id)
    }
}

impl AgentCatalog {
    /// Validate all bounds, canonical ordering, integrity identities, and target consistency.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_identity(&self.runner_instance_id)?;
        validate_digest(&self.catalog_sha256)?;
        validate_target(&self.target)?;
        if self.agents.is_empty() || self.agents.len() > MAX_CATALOG_AGENTS {
            return Err(ProtocolError::InvalidResponse);
        }
        let mut prior = None;
        let mut ids = BTreeSet::new();
        for entry in &self.agents {
            validate_identifier(&entry.agent_id)?;
            if prior.is_some_and(|value: &str| value >= entry.agent_id.as_str())
                || !ids.insert(entry.agent_id.as_str())
            {
                return Err(ProtocolError::InvalidResponse);
            }
            prior = Some(entry.agent_id.as_str());
            if entry.target != self.target {
                return Err(ProtocolError::InvalidResponse);
            }
            if entry.capabilities.len() > MAX_CAPABILITIES {
                return Err(ProtocolError::InvalidResponse);
            }
            match entry.availability {
                AgentAvailability::Available => {
                    validate_package(
                        entry
                            .package
                            .as_ref()
                            .ok_or(ProtocolError::InvalidResponse)?,
                    )?;
                    validate_provenance(
                        entry
                            .provenance
                            .as_ref()
                            .ok_or(ProtocolError::InvalidResponse)?,
                    )?;
                    if entry.capabilities.is_empty() {
                        return Err(ProtocolError::InvalidResponse);
                    }
                }
                AgentAvailability::Unavailable(_) => {
                    if let Some(package) = &entry.package {
                        validate_package(package)?;
                    }
                    if let Some(provenance) = &entry.provenance {
                        validate_provenance(provenance)?;
                    }
                    if entry.package.is_some() != entry.provenance.is_some()
                        && entry.availability
                            != AgentAvailability::Unavailable(
                                AgentUnavailableReason::IncompleteProvenance,
                            )
                    {
                        return Err(ProtocolError::InvalidResponse);
                    }
                }
            }
            let mut capabilities = BTreeSet::new();
            let mut prior_capability = None;
            for capability in &entry.capabilities {
                validate_identifier(capability)?;
                if prior_capability.is_some_and(|value: &str| value >= capability.as_str())
                    || !capabilities.insert(capability.as_str())
                {
                    return Err(ProtocolError::InvalidResponse);
                }
                prior_capability = Some(capability.as_str());
            }
        }
        if self.catalog_sha256 != self.computed_sha256()? {
            return Err(ProtocolError::InvalidResponse);
        }
        Ok(())
    }

    /// Compute the authenticated SHA-256 identity of this catalog snapshot.
    ///
    /// The `catalog_sha256` and response-only `refreshed` members are deliberately excluded from the hashed
    /// representation, preventing a self-referential digest. Callers should
    /// validate the rest of the catalog before accepting this identity.
    pub fn computed_sha256(&self) -> Result<String, ProtocolError> {
        let bytes = canonical_agent_catalog_bytes(self)?;
        let digest = sha2::Sha256::digest(bytes);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}

/// Serialize the v1.4 catalog identity in its canonical, renderer-neutral form.
///
/// Canonicalization hashes the typed snapshot object with `catalog_sha256`
/// omitted. Object members are sorted by their UTF-8 byte keys recursively;
/// arrays retain their protocol-defined order (agents by `agent_id`, and
/// capabilities in their advertised order). JSON strings are UTF-8, numbers
/// are the typed integer values, and no whitespace is emitted. This function
/// does not normalize, sort, or otherwise repair malformed input: callers must
/// run [`AgentCatalog::validate`] first.
pub fn canonical_agent_catalog_bytes(catalog: &AgentCatalog) -> Result<Vec<u8>, ProtocolError> {
    let mut value = serde_json::to_value(catalog).map_err(|_| ProtocolError::InvalidResponse)?;
    let serde_json::Value::Object(object) = &mut value else {
        return Err(ProtocolError::InvalidResponse);
    };
    object.remove("catalog_sha256");
    object.remove("refreshed");
    let canonical = canonicalize_json(value);
    serde_json::to_vec(&canonical).map_err(|_| ProtocolError::InvalidResponse)
}

fn canonicalize_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let sorted: BTreeMap<String, serde_json::Value> = object
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect();
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(canonicalize_json).collect())
        }
        other => other,
    }
}

fn validate_target(target: &AgentTarget) -> Result<(), ProtocolError> {
    validate_token(&target.operating_system)?;
    validate_token(&target.architecture)?;
    validate_token(&target.libc)?;
    validate_token(&target.libc_version)?;
    Ok(())
}

fn validate_package(package: &AgentPackage) -> Result<(), ProtocolError> {
    validate_token(&package.package_id)?;
    validate_token(&package.version)?;
    validate_digest(&package.sha256)?;
    validate_digest(&package.signature_sha256)?;
    validate_signer(&package.signer)
}

fn validate_signer(signer: &AgentSigner) -> Result<(), ProtocolError> {
    validate_token(&signer.key_id)?;
    validate_token(&signer.principal)
}

fn validate_provenance(provenance: &AgentProvenance) -> Result<(), ProtocolError> {
    if provenance.source_revision.len() != 40
        || !provenance
            .source_revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ProtocolError::InvalidResponse);
    }
    validate_digest(&provenance.manifest_sha256)?;
    validate_digest(&provenance.sbom_sha256)?;
    validate_token(&provenance.license_ref)
}

fn validate_identifier(value: &str) -> Result<(), ProtocolError> {
    if value.is_empty()
        || value.len() > MAX_CATALOG_STRING_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(ProtocolError::InvalidResponse);
    }
    Ok(())
}

fn validate_token(value: &str) -> Result<(), ProtocolError> {
    if value.is_empty()
        || value.len() > MAX_CATALOG_STRING_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
    {
        return Err(ProtocolError::InvalidResponse);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(agent_id: &str) -> AgentCatalogEntry {
        AgentCatalogEntry {
            agent_id: agent_id.into(),
            target: AgentTarget {
                operating_system: "linux".into(),
                architecture: "x86_64".into(),
                libc: "glibc".into(),
                libc_version: "2.35".into(),
            },
            package: Some(AgentPackage {
                package_id: "agent-package".into(),
                version: "1.2.3".into(),
                sha256: "a".repeat(64),
                signature_sha256: "b".repeat(64),
                signer: AgentSigner {
                    key_id: "release-key-1".into(),
                    principal: "asb-release".into(),
                },
            }),
            provenance: Some(AgentProvenance {
                source_revision: "c".repeat(40),
                manifest_sha256: "d".repeat(64),
                sbom_sha256: "e".repeat(64),
                license_ref: "MIT".into(),
            }),
            capabilities: vec!["chat".into(), "tools".into()],
            availability: AgentAvailability::Available,
        }
    }

    fn catalog() -> AgentCatalog {
        let mut catalog = AgentCatalog {
            runner_instance_id: "runner-1".into(),
            generation: Revision(1),
            catalog_sha256: "e".repeat(64),
            target: entry("agent-a").target,
            agents: vec![entry("agent-a")],
            refreshed: false,
        };
        catalog.catalog_sha256 = catalog.computed_sha256().unwrap();
        catalog
    }

    #[test]
    fn valid_catalog_accepts_semver_and_binds_request_generation() {
        let value = catalog();
        validate_identity(&value.runner_instance_id).unwrap();
        validate_digest(&value.catalog_sha256).unwrap();
        validate_target(&value.target).unwrap();
        validate_package(value.agents[0].package.as_ref().unwrap()).unwrap();
        validate_provenance(value.agents[0].provenance.as_ref().unwrap()).unwrap();
        value.validate().unwrap();
        AgentCatalogRequest {
            action: AgentCatalogAction::Status,
            runner_instance_id: "runner-1".into(),
            known_generation: Some(Revision(1)),
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn malformed_catalogs_fail_closed() {
        let mut duplicate = catalog();
        duplicate.agents.push(entry("agent-a"));
        assert!(duplicate.validate().is_err());

        let mut wrong_target = catalog();
        wrong_target.agents[0].target.architecture = "aarch64".into();
        assert!(wrong_target.validate().is_err());

        let mut unsigned = catalog();
        unsigned.agents[0]
            .package
            .as_mut()
            .unwrap()
            .signature_sha256 = "0".into();
        assert!(unsigned.validate().is_err());

        let mut invalid_signer = catalog();
        invalid_signer.agents[0]
            .package
            .as_mut()
            .unwrap()
            .signer
            .key_id = "key id".into();
        assert!(invalid_signer.validate().is_err());

        let mut incomplete_provenance = catalog();
        incomplete_provenance.agents[0]
            .provenance
            .as_mut()
            .unwrap()
            .sbom_sha256 = "0".into();
        assert!(incomplete_provenance.validate().is_err());

        let mut unsorted_capabilities = catalog();
        unsorted_capabilities.agents[0].capabilities = vec!["tools".into(), "chat".into()];
        assert!(unsorted_capabilities.validate().is_err());
    }

    #[test]
    fn incomplete_unavailable_entry_is_valid_without_fabricated_metadata() {
        let mut catalog = catalog();
        catalog.agents[0].package = None;
        catalog.agents[0].provenance = None;
        catalog.agents[0].capabilities.clear();
        catalog.agents[0].availability =
            AgentAvailability::Unavailable(AgentUnavailableReason::IncompleteProvenance);
        catalog.catalog_sha256 = catalog.computed_sha256().unwrap();
        catalog.validate().unwrap();
    }

    #[test]
    fn canonical_digest_is_independent_of_object_member_order_and_excludes_itself() {
        let catalog = catalog();
        let bytes = canonical_agent_catalog_bytes(&catalog).unwrap();
        assert_eq!(
            catalog.computed_sha256().unwrap(),
            "79a915ed55e0484fa75130ec1f2275fe12fc99221d1b14f7e3b05ba9efb6bf90"
        );
        assert!(!String::from_utf8(bytes).unwrap().contains("catalog_sha256"));

        // Decode a deliberately hand-ordered representation.  The member
        // order differs at every object level from serde's representation;
        // canonicalization must make this produce the same identity.
        let reordered = r#"{
            "refreshed": false,
            "agents": [{
                "availability": {"status": "available"},
                "capabilities": ["chat", "tools"],
                "provenance": {
                    "license_ref": "MIT",
                    "manifest_sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                    "sbom_sha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                    "source_revision": "cccccccccccccccccccccccccccccccccccccccc"
                },
                "package": {
                    "version": "1.2.3",
                    "signature_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "package_id": "agent-package",
                    "signer": {
                        "principal": "asb-release",
                        "key_id": "release-key-1"
                    }
                },
                "target": {
                    "libc_version": "2.35",
                    "libc": "glibc",
                    "architecture": "x86_64",
                    "operating_system": "linux"
                },
                "agent_id": "agent-a"
            }],
            "target": {
                "libc_version": "2.35",
                "libc": "glibc",
                "architecture": "x86_64",
                "operating_system": "linux"
            },
            "catalog_sha256": "b7749d29dabbc7e8faf00d0f1c01b8c188fe5fef15f52d94f19145977b2b53b3",
            "generation": 1,
            "runner_instance_id": "runner-1"
        }"#;
        let decoded: AgentCatalog = serde_json::from_str(reordered).unwrap();
        assert_eq!(catalog.computed_sha256(), decoded.computed_sha256());
    }

    #[test]
    fn digest_mismatch_is_rejected_even_when_shape_is_valid() {
        let mut catalog = catalog();
        catalog.catalog_sha256 = "0".repeat(64);
        assert_eq!(catalog.validate(), Err(ProtocolError::InvalidResponse));
    }
}
