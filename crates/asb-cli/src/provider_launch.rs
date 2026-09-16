// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! CLI-facing names for the provider-aware process boundary.

/// Environment marker understood by provider-aware adapter wrappers.
pub(crate) const LAUNCH_VERSION_ENV: &str = "ASB_PROVIDER_LAUNCH_V1";
/// Content address of the exact launch record.
pub(crate) const LAUNCH_DIGEST_ENV: &str = "ASB_PROVIDER_LAUNCH_SHA256";
/// Provider family selected by the launch contract.
pub(crate) const PROVIDER_ENV: &str = "ASB_PROVIDER";
/// Provider model selected by the launch contract.
pub(crate) const MODEL_ENV: &str = "ASB_PROVIDER_MODEL";
/// API route selected by the launch contract.
pub(crate) const API_MODE_ENV: &str = "ASB_PROVIDER_API_MODE";
/// Non-secret profile identity selected by the launch contract.
pub(crate) const PROFILE_DIGEST_ENV: &str = "ASB_PROVIDER_PROFILE_SHA256";
/// Exact settings identity selected by the launch contract.
pub(crate) const SETTINGS_DIGEST_ENV: &str = "ASB_PROVIDER_SETTINGS_SHA256";
/// Exact endpoint identity selected by the launch contract.
pub(crate) const ENDPOINT_DIGEST_ENV: &str = "ASB_PROVIDER_ENDPOINT_SHA256";
/// Exact adapter identity selected by the launch contract.
pub(crate) const ADAPTER_ENV: &str = "ASB_PROVIDER_ADAPTER";
/// Non-secret credential-reference identity selected by the launch contract.
pub(crate) const CREDENTIAL_REFERENCE_ENV: &str = "ASB_PROVIDER_CREDENTIAL_REFERENCE_SHA256";
/// Adapter-owned environment target selected for secret injection.
pub(crate) const CREDENTIAL_TARGET_ENV: &str = "ASB_PROVIDER_CREDENTIAL_TARGET";
