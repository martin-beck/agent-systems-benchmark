// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Runtime-owned, one-shot authority for strict-replay launch consumers.

use crate::loopback_sidecar::{LoopbackSidecar, SidecarHandoff};
use crate::sandbox::{ResourceLease, SandboxBackend, SandboxLaunchInput};
use crate::supervisor::PinnedCommand;
use std::io;
use std::time::Duration;

/// Opaque runtime-issued authority.  The contained handoff and sidecar can be
/// consumed exactly once; callers cannot clone or reconstruct readiness.
pub struct ReplayLaunchAuthority {
    handoff: Option<SidecarHandoff>,
    sidecar: Option<LoopbackSidecar>,
    launch: Option<LaunchParts>,
}

struct LaunchParts {
    backend: SandboxBackend,
    input: SandboxLaunchInput,
    lease: ResourceLease,
    supervisor: PinnedCommand,
    sidecar_command: PinnedCommand,
}

impl ReplayLaunchAuthority {
    /// Issue authority only after the runtime has independently attested the
    /// namespace and verified both executable identities.
    pub fn issue(
        sidecar: LoopbackSidecar,
        namespace_ready: bool,
        deadline: Duration,
        sidecar_command_digest: impl Into<String>,
        adapter_command_digest: impl Into<String>,
    ) -> io::Result<Self> {
        let handoff = sidecar.handoff(
            namespace_ready,
            deadline,
            sidecar_command_digest,
            adapter_command_digest,
        )?;
        Ok(Self {
            handoff: Some(handoff),
            sidecar: Some(sidecar),
            launch: None,
        })
    }

    /// Issue a complete runtime launch capability, including the denied
    /// sandbox context, resource lease, and verified supervisor commands.
    #[allow(clippy::too_many_arguments)]
    pub fn issue_launch(
        sidecar: LoopbackSidecar,
        namespace_ready: bool,
        deadline: Duration,
        sidecar_command_digest: impl Into<String>,
        adapter_command_digest: impl Into<String>,
        backend: SandboxBackend,
        input: SandboxLaunchInput,
        lease: ResourceLease,
        supervisor: PinnedCommand,
        sidecar_command: PinnedCommand,
    ) -> io::Result<Self> {
        let mut authority = Self::issue(
            sidecar,
            namespace_ready,
            deadline,
            sidecar_command_digest,
            adapter_command_digest,
        )?;
        authority.launch = Some(LaunchParts {
            backend,
            input,
            lease,
            supervisor,
            sidecar_command,
        });
        Ok(authority)
    }

    /// Consume the authority once for a supervised launch.
    pub fn take_once(&mut self) -> io::Result<(SidecarHandoff, LoopbackSidecar)> {
        match (self.handoff.take(), self.sidecar.take()) {
            (Some(handoff), Some(sidecar)) => Ok((handoff, sidecar)),
            _ => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "replay launch authority already consumed",
            )),
        }
    }

    /// Consume a complete launch capability exactly once.
    pub fn take_launch(
        &mut self,
    ) -> io::Result<(
        SidecarHandoff,
        LoopbackSidecar,
        SandboxBackend,
        SandboxLaunchInput,
        ResourceLease,
        PinnedCommand,
        PinnedCommand,
    )> {
        let (handoff, sidecar) = self.take_once()?;
        let launch = self.launch.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "launch capability is incomplete",
            )
        })?;
        Ok((
            handoff,
            sidecar,
            launch.backend,
            launch.input,
            launch.lease,
            launch.supervisor,
            launch.sidecar_command,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopback_sidecar::SidecarIdentity;

    #[test]
    fn authority_is_one_shot_and_requires_attested_namespace() {
        let root =
            std::env::temp_dir().join(format!("asb-replay-authority-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let identity = SidecarIdentity::new("generation-1", "a".repeat(64)).unwrap();
        let sidecar = LoopbackSidecar::bind(identity, root.join("relay.sock")).unwrap();
        assert!(
            ReplayLaunchAuthority::issue(
                sidecar,
                false,
                Duration::from_secs(1),
                "b".repeat(64),
                "c".repeat(64),
            )
            .is_err()
        );

        let identity = SidecarIdentity::new("generation-2", "a".repeat(64)).unwrap();
        let sidecar = LoopbackSidecar::bind(identity, root.join("relay-2.sock")).unwrap();
        let mut authority = ReplayLaunchAuthority::issue(
            sidecar,
            true,
            Duration::from_secs(1),
            "b".repeat(64),
            "c".repeat(64),
        )
        .unwrap();
        let (handoff, sidecar) = authority.take_once().unwrap();
        assert_eq!(handoff.generation(), "generation-2");
        drop(sidecar);
        assert!(authority.take_once().is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
