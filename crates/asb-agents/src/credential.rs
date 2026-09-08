// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Bounded credential-reference resolution and isolated process injection.

use asb_protocol::{CredentialSource, ProviderProfileV1};
use asb_runtime::{ProcessError, ProcessLimits, RunningProcess};
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::process::{Command, Stdio};

/// Largest credential value accepted at the provider process boundary.
pub const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;
/// Largest explicit environment-reference or target name.
pub const MAX_ENVIRONMENT_NAME_BYTES: usize = 128;

/// Resolver for one explicit logical environment credential reference.
#[derive(Clone, Eq, PartialEq)]
pub struct EnvironmentCredentialResolver {
    variable: OsString,
    reference_sha256: String,
}

impl fmt::Debug for EnvironmentCredentialResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvironmentCredentialResolver")
            .field("reference_sha256", &self.reference_sha256)
            .finish_non_exhaustive()
    }
}

impl EnvironmentCredentialResolver {
    /// Bind a resolver to one explicit environment variable name.
    pub fn new(variable: impl Into<OsString>) -> Result<Self, CredentialResolutionError> {
        let variable = variable.into();
        validate_environment_name(&variable)?;
        let reference_sha256 = environment_reference_sha256(&variable)?;
        Ok(Self {
            variable,
            reference_sha256,
        })
    }

    /// Credential-free digest to place in [`ProviderProfileV1`].
    pub fn reference_sha256(&self) -> &str {
        &self.reference_sha256
    }

    /// Resolve with an injected lookup boundary, useful for isolated callers and tests.
    pub fn resolve_with<F>(
        &self,
        profile: &ProviderProfileV1,
        mut lookup: F,
    ) -> Result<ResolvedCredential, CredentialResolutionError>
    where
        F: FnMut(&OsStr) -> Option<OsString>,
    {
        profile
            .validate()
            .map_err(|_| CredentialResolutionError::InvalidProfile)?;
        if profile.credential.source != CredentialSource::Environment
            || profile.credential.reference_sha256.as_deref() != Some(&self.reference_sha256)
        {
            return Err(CredentialResolutionError::ReferenceMismatch);
        }
        let value = lookup(&self.variable).ok_or(CredentialResolutionError::Unavailable)?;
        ResolvedCredential::new(value)
    }

    /// Resolve only the explicitly bound variable from the current process environment.
    pub fn resolve_environment(
        &self,
        profile: &ProviderProfileV1,
    ) -> Result<ResolvedCredential, CredentialResolutionError> {
        self.resolve_with(profile, |name| std::env::var_os(name))
    }
}

/// Compute the canonical credential-free identity of one environment reference.
pub fn environment_reference_sha256(variable: &OsStr) -> Result<String, CredentialResolutionError> {
    validate_environment_name(variable)?;
    let mut digest = Sha256::new();
    digest.update(b"asb-credential-reference-v1\0environment\0");
    digest.update(variable.as_bytes());
    Ok(format!("{:x}", digest.finalize()))
}

/// Opaque resolved credential. It is neither cloneable, printable, nor serializable.
pub struct ResolvedCredential {
    bytes: Vec<u8>,
}

impl ResolvedCredential {
    fn new(value: OsString) -> Result<Self, CredentialResolutionError> {
        let bytes = value.into_vec();
        if bytes.is_empty()
            || bytes.len() > MAX_CREDENTIAL_BYTES
            || bytes.iter().any(|byte| !matches!(*byte, 0x21..=0x7e))
        {
            return Err(CredentialResolutionError::InvalidValue);
        }
        Ok(Self { bytes })
    }

    /// Spawn a child with an otherwise empty environment and one credential target.
    ///
    /// The credential is never placed in arguments, output, or an ASB evidence type.
    pub fn spawn<I, S>(
        mut self,
        program: &Path,
        arguments: I,
        credential_target: &OsStr,
        limits: ProcessLimits,
    ) -> Result<RunningProcess, CredentialProcessError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        validate_environment_name(credential_target).map_err(CredentialProcessError::Resolution)?;
        let mut command = Command::new(program);
        command
            .args(arguments)
            .env_clear()
            .env(credential_target, OsStr::from_bytes(&self.bytes))
            .stdin(Stdio::null());
        let result = RunningProcess::spawn(command, limits).map_err(CredentialProcessError::Spawn);
        self.erase();
        result
    }

    fn erase(&mut self) {
        self.bytes.fill(0);
    }
}

impl Drop for ResolvedCredential {
    fn drop(&mut self) {
        self.erase();
    }
}

/// Credential resolution failure without source names or values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialResolutionError {
    /// Explicit environment variable name is malformed or unbounded.
    InvalidEnvironmentName,
    /// Provider profile failed its common validation.
    InvalidProfile,
    /// Profile source or logical reference does not match this resolver.
    ReferenceMismatch,
    /// The explicitly bound source did not return a value.
    Unavailable,
    /// Credential value is empty, unbounded, non-ASCII, or contains whitespace/control bytes.
    InvalidValue,
}

impl fmt::Display for CredentialResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEnvironmentName => {
                formatter.write_str("credential environment reference is invalid")
            }
            Self::InvalidProfile => formatter.write_str("provider credential profile is invalid"),
            Self::ReferenceMismatch => {
                formatter.write_str("provider credential reference does not match resolver")
            }
            Self::Unavailable => formatter.write_str("provider credential is unavailable"),
            Self::InvalidValue => formatter.write_str("provider credential value is invalid"),
        }
    }
}

impl std::error::Error for CredentialResolutionError {}

/// Credential-bearing process boundary failure.
#[derive(Debug)]
pub enum CredentialProcessError {
    /// Credential target name was invalid.
    Resolution(CredentialResolutionError),
    /// Isolated process creation failed.
    Spawn(ProcessError),
}

impl fmt::Display for CredentialProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resolution(error) => error.fmt(formatter),
            Self::Spawn(_) => {
                formatter.write_str("credential-isolated provider process failed to start")
            }
        }
    }
}

impl std::error::Error for CredentialProcessError {}

fn validate_environment_name(value: &OsStr) -> Result<(), CredentialResolutionError> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_ENVIRONMENT_NAME_BYTES
        || !bytes[0].is_ascii_uppercase()
        || bytes
            .iter()
            .any(|byte| !(byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_'))
    {
        Err(CredentialResolutionError::InvalidEnvironmentName)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_protocol::{
        CredentialProvenance, EndpointClass, EndpointProvenance, PROVIDER_PROFILE_V1, ProviderKind,
        ProviderSettings, ProviderTransportLimits,
    };
    use std::fs;
    use std::time::Duration;

    fn profile(reference: String) -> ProviderProfileV1 {
        let mut profile = ProviderProfileV1 {
            version: PROVIDER_PROFILE_V1,
            settings_sha256: String::new(),
            provider: ProviderKind::OpenAi,
            endpoint: EndpointProvenance {
                class: EndpointClass::PublicService,
                identity_sha256: "a".repeat(64),
            },
            model: "pinned-model".into(),
            settings: ProviderSettings {
                temperature_milli: None,
                top_p_millionth: None,
                seed: None,
                max_output_tokens: None,
                reasoning_effort: None,
                additional_settings_sha256: None,
            },
            transport: ProviderTransportLimits {
                max_request_bytes: 1024,
                max_response_bytes: 1024,
                connect_timeout_ms: 1000,
                request_timeout_ms: 1000,
                max_concurrent_requests: 1,
            },
            credential: CredentialProvenance {
                source: CredentialSource::Environment,
                reference_sha256: Some(reference),
            },
        };
        profile.refresh_settings_sha256().unwrap();
        profile
    }

    #[test]
    fn explicit_reference_resolves_and_is_stable() {
        let resolver = EnvironmentCredentialResolver::new("ASB_TEST_KEY").unwrap();
        assert_eq!(
            resolver.reference_sha256(),
            environment_reference_sha256(OsStr::new("ASB_TEST_KEY")).unwrap()
        );
        let resolved = resolver
            .resolve_with(&profile(resolver.reference_sha256().into()), |name| {
                assert_eq!(name, OsStr::new("ASB_TEST_KEY"));
                Some(OsString::from("synthetic-value"))
            })
            .unwrap();
        assert_eq!(resolved.bytes.len(), 15);
    }

    #[test]
    fn mismatched_absent_and_unsupported_sources_fail_without_lookup() {
        let resolver = EnvironmentCredentialResolver::new("ASB_TEST_KEY").unwrap();
        let mut calls = 0;
        assert!(matches!(
            resolver.resolve_with(&profile("b".repeat(64)), |_| {
                calls += 1;
                None
            }),
            Err(CredentialResolutionError::ReferenceMismatch)
        ));
        assert_eq!(calls, 0);
        assert!(matches!(
            resolver.resolve_with(&profile(resolver.reference_sha256().into()), |_| None),
            Err(CredentialResolutionError::Unavailable)
        ));
        for source in [CredentialSource::FileDescriptor, CredentialSource::Helper] {
            let mut unsupported = profile(resolver.reference_sha256().into());
            unsupported.credential.source = source;
            unsupported.refresh_settings_sha256().unwrap();
            assert!(matches!(
                resolver.resolve_with(&unsupported, |_| {
                    calls += 1;
                    None
                }),
                Err(CredentialResolutionError::ReferenceMismatch)
            ));
        }
        assert_eq!(calls, 0);
    }

    #[test]
    fn names_and_values_are_strictly_bounded() {
        for name in ["", "lower", "1START", "HAS-DASH"] {
            assert_eq!(
                EnvironmentCredentialResolver::new(name),
                Err(CredentialResolutionError::InvalidEnvironmentName)
            );
        }
        let resolver = EnvironmentCredentialResolver::new("ASB_TEST_KEY").unwrap();
        let profile = profile(resolver.reference_sha256().into());
        for value in [
            OsString::new(),
            OsString::from("has space"),
            OsString::from("line\nbreak"),
        ] {
            assert!(matches!(
                resolver.resolve_with(&profile, |_| Some(value.clone())),
                Err(CredentialResolutionError::InvalidValue)
            ));
        }
        assert!(matches!(
            resolver.resolve_with(&profile, |_| Some(OsString::from(
                "x".repeat(MAX_CREDENTIAL_BYTES + 1)
            ))),
            Err(CredentialResolutionError::InvalidValue)
        ));
    }

    #[test]
    fn child_gets_only_target_credential_and_public_output_is_clean() {
        let resolver = EnvironmentCredentialResolver::new("ASB_TEST_KEY").unwrap();
        let profile = profile(resolver.reference_sha256().into());
        let resolved = resolver
            .resolve_with(&profile, |_| Some(OsString::from("synthetic-value")))
            .unwrap();
        let scratch = std::env::temp_dir().join(format!("asb-credential-{}", std::process::id()));
        fs::create_dir_all(&scratch).unwrap();
        let script = scratch.join("check.sh");
        fs::write(
            &script,
            "test \"$TARGET_KEY\" = synthetic-value\ntest \"$(env | wc -l)\" -eq 1\nprintf isolated\n",
        )
        .unwrap();
        let limits = ProcessLimits::new(
            1024,
            1024,
            Duration::from_secs(2),
            Duration::from_millis(50),
            Duration::from_millis(5),
        )
        .unwrap();
        let mut child = resolved
            .spawn(
                Path::new("/bin/sh"),
                [&script],
                OsStr::new("TARGET_KEY"),
                limits,
            )
            .unwrap();
        let output = child.wait().unwrap();
        assert_eq!(output.exit_code, Some(0));
        assert_eq!(output.stdout.bytes, b"isolated");
        assert!(
            !output
                .stdout
                .bytes
                .windows(15)
                .any(|window| window == b"synthetic-value")
        );
        fs::remove_dir_all(scratch).unwrap();
    }
}
