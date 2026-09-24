// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Offline runtime-bundle verifier command.

use asb_bundle::{ExpectedTarget, VerificationPolicy, VerifierConfig, verify_bundle_with_policy};
use std::env;
use std::path::{Path, PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("bundle verification failed: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = env::args().collect();
    if !(arguments.len() == 10 || arguments.len() == 12 && arguments[10] == "--profile") {
        return Err("usage: asb-bundle-verify BUNDLE ALLOWED_SIGNERS PRINCIPAL SSH_KEYGEN SSH_KEYGEN_SHA256 OS ARCH LIBC LIBC_VERSION [--profile signed|unsigned-development|unsigned-release]".into());
    }
    let policy = match arguments.get(11).map(String::as_str) {
        None | Some("signed") => VerificationPolicy::RequireSignature,
        Some("unsigned-development") => VerificationPolicy::AllowUnsignedDevelopment,
        Some("unsigned-release") => VerificationPolicy::AllowUnsignedRelease,
        Some(_) => {
            return Err(
                "invalid --profile; choose signed, unsigned-development, or unsigned-release"
                    .into(),
            );
        }
    };
    let verified = verify_bundle_with_policy(
        Path::new(&arguments[1]),
        &VerifierConfig {
            allowed_signers: PathBuf::from(&arguments[2]),
            principal: arguments[3].clone(),
            ssh_keygen: PathBuf::from(&arguments[4]),
            ssh_keygen_sha256: arguments[5].clone(),
        },
        &ExpectedTarget {
            operating_system: &arguments[6],
            architecture: &arguments[7],
            libc: &arguments[8],
            libc_version: &arguments[9],
        },
        policy,
    )?;
    println!(
        "verified {} {} profile={:?} signature_status={:?} manifest={} content={} artifacts={}",
        verified.bundle_id,
        verified.bundle_version,
        verified.profile,
        verified.signature_status,
        verified.manifest_sha256,
        verified.content_sha256,
        verified.artifact_count
    );
    Ok(())
}
