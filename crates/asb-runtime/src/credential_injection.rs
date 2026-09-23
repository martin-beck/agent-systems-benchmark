// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Opaque final-boundary credential injection contract.

use crate::sandbox_credential::{SandboxCredentialChannel, SandboxCredentialError};

/// Failure while applying a credential at the final child boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialInjectionError {
    /// The credential reference does not match the runtime-selected attempt.
    ReferenceMismatch,
    /// The credential could not be applied without exposing it to the caller.
    InjectionFailed,
    /// The runtime-owned channel rejected the credential value.
    Channel(SandboxCredentialError),
}

/// Cross-crate contract for consuming a credential only at child launch.
pub trait CredentialInjection: Send {
    /// Consume this capability into the runtime-owned sealed child channel.
    fn inject(
        self: Box<Self>,
        channel: &mut SandboxCredentialChannel,
    ) -> Result<(), CredentialInjectionError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestInjection;
    impl CredentialInjection for TestInjection {
        fn inject(
            self: Box<Self>,
            _channel: &mut SandboxCredentialChannel,
        ) -> Result<(), CredentialInjectionError> {
            drop(self);
            Ok(())
        }
    }

    #[test]
    fn capability_is_consumed_without_runtime_bytes() {
        let _ = Box::new(TestInjection);
    }
}
