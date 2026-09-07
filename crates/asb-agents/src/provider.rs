// SPDX-License-Identifier: MIT
//! Fail-closed binding of provider profiles to built-in adapter configuration.

use asb_protocol::{
    ProviderProfileCapabilities, ProviderProfileError, ProviderProfileV1, VerifiedProviderProfile,
};

/// Side-effect-free provider translation boundary implemented by concrete adapters.
///
/// Capability negotiation occurs before [`Self::prepare_provider_profile`]. The
/// prepared value must not start an agent, and [`Self::effective_provider_profile`]
/// must describe the exact credential-free configuration that will reach the provider.
pub trait ProviderProfileAdapter {
    /// Concrete adapter configuration produced before process start.
    type Prepared;

    /// Return the complete bounded capability matrix for this exact adapter revision.
    fn provider_profile_capabilities(&self) -> ProviderProfileCapabilities;

    /// Translate a previously negotiated profile without starting an agent.
    fn prepare_provider_profile(
        &self,
        profile: &ProviderProfileV1,
    ) -> Result<Self::Prepared, ProviderProfileError>;

    /// Inspect the exact effective credential-free configuration before process start.
    fn effective_provider_profile(
        &self,
        prepared: &Self::Prepared,
    ) -> Result<ProviderProfileV1, ProviderProfileError>;
}

/// Constructor-controlled adapter configuration paired with exact effective proof.
///
/// External callers cannot attach a forged proof to prepared configuration:
///
/// ```compile_fail
/// use asb_agents::provider::BoundProviderProfile;
/// fn forge<T>(prepared: T, verified: asb_protocol::VerifiedProviderProfile) {
///     let _ = BoundProviderProfile { prepared, verified };
/// }
/// ```
///
/// Existing bindings cannot have their proof replaced either:
///
/// ```compile_fail
/// fn replace<T>(mut bound: asb_agents::provider::BoundProviderProfile<T>) {
///     bound.verified = panic!();
/// }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundProviderProfile<T> {
    prepared: T,
    verified: VerifiedProviderProfile,
}

impl<T> BoundProviderProfile<T> {
    /// Borrow the adapter-specific pre-start configuration.
    pub fn prepared(&self) -> &T {
        &self.prepared
    }

    /// Borrow proof of the exact effective credential-free profile.
    pub fn verified(&self) -> &VerifiedProviderProfile {
        &self.verified
    }

    /// Consume the binding into adapter configuration and proof.
    pub fn into_parts(self) -> (T, VerifiedProviderProfile) {
        (self.prepared, self.verified)
    }
}

/// Negotiate, translate, and verify one profile before any agent start.
pub fn bind_provider_profile<A: ProviderProfileAdapter>(
    adapter: &A,
    profile: &ProviderProfileV1,
) -> Result<BoundProviderProfile<A::Prepared>, ProviderProfileError> {
    let negotiated = profile.negotiate(&adapter.provider_profile_capabilities())?;
    let prepared = adapter.prepare_provider_profile(profile)?;
    let effective = adapter.effective_provider_profile(&prepared)?;
    let verified = negotiated.verify_effective(&effective)?;
    Ok(BoundProviderProfile { prepared, verified })
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_protocol::{
        CredentialProvenance, CredentialSource, EndpointClass, EndpointProvenance,
        OptionalSettingSupport, PROVIDER_PROFILE_V1, ProviderKind, ProviderProfileVersion,
        ProviderSettingField, ProviderSettings, ProviderTransportLimits,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct SyntheticAdapter {
        capabilities: ProviderProfileCapabilities,
        translations: AtomicUsize,
        substitute_model: bool,
    }

    impl ProviderProfileAdapter for SyntheticAdapter {
        type Prepared = ProviderProfileV1;

        fn provider_profile_capabilities(&self) -> ProviderProfileCapabilities {
            self.capabilities.clone()
        }

        fn prepare_provider_profile(
            &self,
            profile: &ProviderProfileV1,
        ) -> Result<Self::Prepared, ProviderProfileError> {
            self.translations.fetch_add(1, Ordering::SeqCst);
            let mut effective = profile.clone();
            if self.substitute_model {
                effective.model = "substituted-model".into();
                effective.refresh_settings_sha256()?;
            }
            Ok(effective)
        }

        fn effective_provider_profile(
            &self,
            prepared: &Self::Prepared,
        ) -> Result<ProviderProfileV1, ProviderProfileError> {
            Ok(prepared.clone())
        }
    }

    fn profile() -> ProviderProfileV1 {
        let digest = "a".repeat(64);
        let mut profile = ProviderProfileV1 {
            version: PROVIDER_PROFILE_V1,
            settings_sha256: String::new(),
            provider: ProviderKind::OpenAiCompatible,
            endpoint: EndpointProvenance {
                class: EndpointClass::Loopback,
                identity_sha256: digest,
            },
            model: "test-model".into(),
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
                max_response_bytes: 2048,
                connect_timeout_ms: 1000,
                request_timeout_ms: 2000,
                max_concurrent_requests: 1,
            },
            credential: CredentialProvenance {
                source: CredentialSource::None,
                reference_sha256: None,
            },
        };
        profile.refresh_settings_sha256().unwrap();
        profile
    }

    fn adapter() -> SyntheticAdapter {
        let support = [
            ProviderSettingField::Temperature,
            ProviderSettingField::TopP,
            ProviderSettingField::Seed,
            ProviderSettingField::MaxOutputTokens,
            ProviderSettingField::ReasoningEffort,
            ProviderSettingField::AdditionalSettings,
        ]
        .into_iter()
        .map(|field| {
            (
                field,
                OptionalSettingSupport {
                    exact_value: true,
                    explicit_omission: true,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
        SyntheticAdapter {
            capabilities: ProviderProfileCapabilities {
                minimum_version: ProviderProfileVersion { major: 1, minor: 0 },
                maximum_version: ProviderProfileVersion { major: 1, minor: 0 },
                providers: BTreeSet::from([ProviderKind::OpenAiCompatible]),
                endpoint_classes: BTreeSet::from([EndpointClass::Loopback]),
                credential_sources: BTreeSet::from([CredentialSource::None]),
                settings: support,
                transport_ceiling: ProviderTransportLimits {
                    max_request_bytes: 4096,
                    max_response_bytes: 4096,
                    connect_timeout_ms: 5000,
                    request_timeout_ms: 5000,
                    max_concurrent_requests: 2,
                },
            },
            translations: AtomicUsize::new(0),
            substitute_model: false,
        }
    }

    #[test]
    fn exact_binding_returns_constructor_controlled_proof() {
        let adapter = adapter();
        let profile = profile();
        let bound = bind_provider_profile(&adapter, &profile).unwrap();
        assert_eq!(adapter.translations.load(Ordering::SeqCst), 1);
        assert_eq!(bound.prepared().model, profile.model);
        assert_eq!(bound.verified().settings_sha256(), profile.settings_sha256);
        let (prepared, verified) = bound.into_parts();
        assert_eq!(prepared, profile);
        assert_eq!(verified.settings_sha256(), prepared.settings_sha256);
    }

    #[test]
    fn unsupported_profiles_fail_before_translation_and_loss_is_rejected() {
        let mut unsupported = adapter();
        unsupported.capabilities.providers.clear();
        assert!(matches!(
            bind_provider_profile(&unsupported, &profile()),
            Err(ProviderProfileError::UnsupportedProvider(_))
        ));
        assert_eq!(unsupported.translations.load(Ordering::SeqCst), 0);

        let mut lossy = adapter();
        lossy.substitute_model = true;
        assert_eq!(
            bind_provider_profile(&lossy, &profile()),
            Err(ProviderProfileError::LossyTranslation)
        );
        assert_eq!(lossy.translations.load(Ordering::SeqCst), 1);
    }
}
