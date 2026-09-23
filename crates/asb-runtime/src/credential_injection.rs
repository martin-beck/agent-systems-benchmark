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
    use crate::sandbox_credential::SandboxCredentialBinding;

    struct TestInjection(Result<(), CredentialInjectionError>);
    impl CredentialInjection for TestInjection {
        fn inject(
            self: Box<Self>,
            _channel: &mut SandboxCredentialChannel,
        ) -> Result<(), CredentialInjectionError> {
            self.0
        }
    }

    #[test]
    fn capability_is_consumed_without_runtime_bytes() {
        let binding = SandboxCredentialBinding::new("a".repeat(64), "TARGET").unwrap();
        let mut channel = SandboxCredentialChannel::new(&binding).unwrap();
        assert!(Box::new(TestInjection(Ok(()))).inject(&mut channel).is_ok());
        assert_eq!(
            Box::new(TestInjection(Err(
                CredentialInjectionError::ReferenceMismatch
            )))
            .inject(&mut channel),
            Err(CredentialInjectionError::ReferenceMismatch)
        );
        assert_eq!(
            Box::new(TestInjection(Err(
                CredentialInjectionError::InjectionFailed
            )))
            .inject(&mut channel),
            Err(CredentialInjectionError::InjectionFailed)
        );
        assert_eq!(
            Box::new(TestInjection(Err(CredentialInjectionError::Channel(
                SandboxCredentialError::InvalidValue,
            ))))
            .inject(&mut channel),
            Err(CredentialInjectionError::Channel(
                SandboxCredentialError::InvalidValue,
            ))
        );
    }
}
