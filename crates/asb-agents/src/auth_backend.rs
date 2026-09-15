// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Concrete adapters over the qualified credential resolvers.

use crate::credential::{
    CredentialResolutionError, EnvironmentCredentialResolver, FileDescriptorCredentialResolver,
    HelperCredentialError, HelperCredentialResolver, ResolvedCredential,
};
use asb_protocol::ProviderProfileV1;
use std::ffi::OsString;
use std::os::fd::OwnedFd;
use std::time::Duration;

/// Concrete provider-secret source selected before a launch.
pub enum CredentialBackend {
    /// One explicitly named environment variable, resolved only when requested.
    Environment(EnvironmentCredentialResolver),
    /// One already-open, owner-checked, one-shot descriptor.
    FileDescriptor(FileDescriptorCredentialResolver),
    /// One staged, content-pinned helper executable.
    Helper(HelperCredentialResolver),
}

impl std::fmt::Debug for CredentialBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("CredentialBackend")
            .field(&self.reference_sha256())
            .finish()
    }
}

impl CredentialBackend {
    /// Bind an explicit environment variable without reading it during setup.
    pub fn environment(variable: impl Into<OsString>) -> Result<Self, CredentialResolutionError> {
        Ok(Self::Environment(EnvironmentCredentialResolver::new(
            variable,
        )?))
    }

    /// Bind an already-open descriptor; no path is opened by this constructor.
    pub fn file_descriptor(
        descriptor: OwnedFd,
        logical_locator: &str,
    ) -> Result<Self, CredentialResolutionError> {
        Ok(Self::FileDescriptor(FileDescriptorCredentialResolver::new(
            descriptor,
            logical_locator,
        )?))
    }

    /// Bind a content-pinned helper executable.
    pub fn helper(
        executable: OwnedFd,
        logical_locator: &str,
        expected_executable_sha256: &str,
    ) -> Result<Self, CredentialResolutionError> {
        Ok(Self::Helper(HelperCredentialResolver::new(
            executable,
            logical_locator,
            expected_executable_sha256,
        )?))
    }

    /// Credential-free resolver identity.
    pub fn reference_sha256(&self) -> &str {
        match self {
            Self::Environment(value) => value.reference_sha256(),
            Self::FileDescriptor(value) => value.reference_sha256(),
            Self::Helper(value) => value.reference_sha256(),
        }
    }

    /// Resolve one secret through its explicit source and profile binding.
    pub fn resolve(
        self,
        profile: &ProviderProfileV1,
        helper_timeout: Duration,
    ) -> Result<ResolvedCredential, BackendError> {
        match self {
            Self::Environment(value) => value
                .resolve_environment(profile)
                .map_err(BackendError::Resolution),
            Self::FileDescriptor(value) => value.resolve(profile).map_err(BackendError::Resolution),
            Self::Helper(value) => value
                .start(profile, helper_timeout)
                .map_err(BackendError::Helper)?
                .wait()
                .map_err(BackendError::Helper),
        }
    }
}

/// Failure from a concrete backend, with no secret or source disclosure.
#[derive(Debug)]
pub enum BackendError {
    /// Explicit source or profile binding failed.
    Resolution(CredentialResolutionError),
    /// Helper process failed, timed out, or returned malformed output.
    Helper(HelperCredentialError),
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Resolution(error) => error.fmt(f),
            Self::Helper(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for BackendError {}
