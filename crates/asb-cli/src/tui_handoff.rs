// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! ASB-side lifecycle handoff seam for the independently installed frontend.
//!
//! This module deliberately owns no frontend transport or terminal code.  The
//! control crate owns the wire format and authentication checks; the router
//! only admits an already-created broker descriptor and advances a bounded
//! broker state machine.  Endpoint discovery and the actual frontend consumer
//! remain separate integration points.

use asb_control::{BrokerConnection, BrokerState, HandoffError, PendingBrokerSuccess};
use std::os::fd::OwnedFd;
use std::time::Duration;

/// A validated request admitted for one frontend broker lifetime.
pub struct PendingHandoff {
    broker: BrokerConnection,
    state: BrokerState,
    pending: PendingBrokerSuccess,
}

impl PendingHandoff {
    /// Receive and admit the initial request on an inherited broker endpoint.
    ///
    /// The descriptor is validated by `asb-control` before it is retained;
    /// ancillary data, malformed packets, peer mismatches, and expired
    /// requests fail closed.  No benchmark operation is started here.
    pub fn receive_initial(socket: OwnedFd) -> Result<Self, HandoffError> {
        let broker = BrokerConnection::new(socket)?;
        let mut state = BrokerState::fresh()?;
        let request = broker.receive_request()?;
        let pending = state.begin_request(request, Duration::ZERO)?;
        Ok(Self {
            broker,
            state,
            pending,
        })
    }

    /// Return the reserved generation without exposing the broker descriptor.
    #[must_use]
    pub fn generation(&self) -> asb_control::BrokerGeneration {
        self.pending.generation()
    }

    /// Access the broker endpoint for the subsequent authenticated producer.
    ///
    /// The caller must either complete the transfer through the control
    /// contract or drop this value; no partial response is emitted by this
    /// seam.
    pub fn broker(&self) -> &BrokerConnection {
        &self.broker
    }

    /// Access the state only to the coordinator integration layer.
    pub fn state(&mut self) -> &mut BrokerState {
        &mut self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_control::{BrokerOperation, BrokerPacket, HandoffStatus};
    use rustix::net::{AddressFamily, SendFlags, SocketFlags, SocketType, socketpair};
    use std::os::fd::OwnedFd;

    fn pair() -> (OwnedFd, OwnedFd) {
        socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC,
            None,
        )
        .expect("socketpair")
    }

    #[test]
    fn malformed_inherited_endpoint_fails_closed() {
        let (router, peer) = pair();
        let packet = BrokerPacket {
            operation: BrokerOperation::Initial,
            status: HandoffStatus::Success,
            epoch: [1; 16],
            sequence: 1,
            expected_runner_identity: [2; 32],
        };
        rustix::net::send(&peer, &packet.encode(), SendFlags::NOSIGNAL).expect("send");
        match PendingHandoff::receive_initial(router) {
            Err(error) => assert_eq!(error, HandoffError::InvalidPacket),
            Ok(_) => panic!("malformed broker request was accepted"),
        }
    }

    #[test]
    fn valid_request_reserves_a_nonzero_generation() {
        let (router, peer) = pair();
        let packet = BrokerPacket {
            operation: BrokerOperation::Initial,
            status: HandoffStatus::Request,
            epoch: [0; 16],
            sequence: 0,
            expected_runner_identity: [0; 32],
        };
        rustix::net::send(&peer, &packet.encode(), SendFlags::NOSIGNAL).expect("send");
        let pending = PendingHandoff::receive_initial(router).expect("request");
        assert_ne!(pending.generation().epoch(), [0; 16]);
        assert_ne!(pending.generation().sequence(), 0);
    }
}
