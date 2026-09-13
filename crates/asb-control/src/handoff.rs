// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Closed binary framing and generation state for authenticated control handoff.

use std::os::unix::net::UnixStream;
use std::time::Duration;

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{MAX_CONTROL_ID_BYTES, PeerIdentity, validate_identity};

/// Exact byte length of one router/frontend broker packet.
pub const BROKER_PACKET_BYTES: usize = 72;
/// Exact byte length of one service provisioning packet.
pub const PROVISIONING_PACKET_BYTES: usize = 64;
/// Maximum replacement acquisitions during one frontend lifetime.
pub const MAX_REPLACEMENTS: u8 = 8;
/// Minimum interval between replacement acquisitions.
pub const MIN_REPLACEMENT_INTERVAL: Duration = Duration::from_millis(100);

const BROKER_MAGIC: [u8; 8] = *b"ASBHND01";
const PROVISIONING_MAGIC: [u8; 8] = *b"ASBPRV01";
const WIRE_VERSION: u16 = 1;
const RUNNER_IDENTITY_DOMAIN: &[u8] = b"asb-control-runner-instance-v1";

/// Operation encoded by a broker packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrokerOperation {
    /// First acquisition for a new frontend lifetime.
    Initial,
    /// Replacement after an authenticated service disconnect.
    Replacement,
}

impl BrokerOperation {
    const fn wire(self) -> u8 {
        match self {
            Self::Initial => 1,
            Self::Replacement => 2,
        }
    }

    fn parse(value: u8) -> Result<Self, HandoffError> {
        match value {
            1 => Ok(Self::Initial),
            2 => Ok(Self::Replacement),
            _ => Err(HandoffError::InvalidPacket),
        }
    }
}

/// Closed status encoded by broker and provisioning replies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffStatus {
    /// Request packet; never a reply.
    Request,
    /// One authenticated descriptor accompanies the reply.
    Success,
    /// The authoritative service is temporarily unavailable.
    Unavailable,
    /// The request is invalid or unauthorized.
    Rejected,
    /// The frontend lifetime exhausted its replacement ceiling.
    Exhausted,
}

impl HandoffStatus {
    const fn wire(self) -> u8 {
        match self {
            Self::Request => 0,
            Self::Success => 1,
            Self::Unavailable => 2,
            Self::Rejected => 3,
            Self::Exhausted => 4,
        }
    }

    fn parse_broker(value: u8) -> Result<Self, HandoffError> {
        match value {
            0 => Ok(Self::Request),
            1 => Ok(Self::Success),
            2 => Ok(Self::Unavailable),
            3 => Ok(Self::Rejected),
            4 => Ok(Self::Exhausted),
            _ => Err(HandoffError::InvalidPacket),
        }
    }

    fn parse_provisioning(value: u8) -> Result<Self, HandoffError> {
        match value {
            0 => Ok(Self::Request),
            1 => Ok(Self::Success),
            2 => Ok(Self::Unavailable),
            3 => Ok(Self::Rejected),
            _ => Err(HandoffError::InvalidPacket),
        }
    }

    const fn is_closed_failure(self) -> bool {
        matches!(self, Self::Unavailable | Self::Rejected | Self::Exhausted)
    }
}

/// Monotonic generation accepted during one live broker lifetime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrokerGeneration {
    epoch: [u8; 16],
    sequence: u64,
}

impl BrokerGeneration {
    /// Fresh nonzero broker epoch.
    #[must_use]
    pub const fn epoch(self) -> [u8; 16] {
        self.epoch
    }

    /// Nonzero sequence within this live epoch.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }
}

/// Exact 72-byte closed broker request or reply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrokerPacket {
    /// Packet operation.
    pub operation: BrokerOperation,
    /// Request or closed reply status.
    pub status: HandoffStatus,
    /// Broker epoch; zero only for an initial request or pre-generation failure.
    pub epoch: [u8; 16],
    /// Sequence; zero only for an initial request or pre-generation failure.
    pub sequence: u64,
    /// Expected runner identity; nonzero only in a successful reply.
    pub expected_runner_identity: [u8; 32],
}

impl BrokerPacket {
    /// Encode the frozen v1 representation.
    #[must_use]
    pub fn encode(self) -> [u8; BROKER_PACKET_BYTES] {
        let mut bytes = [0_u8; BROKER_PACKET_BYTES];
        bytes[..8].copy_from_slice(&BROKER_MAGIC);
        bytes[8..10].copy_from_slice(&WIRE_VERSION.to_be_bytes());
        bytes[10] = self.operation.wire();
        bytes[11] = self.status.wire();
        bytes[16..32].copy_from_slice(&self.epoch);
        bytes[32..40].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[40..].copy_from_slice(&self.expected_runner_identity);
        bytes
    }

    /// Decode the exact frozen v1 representation and reject reserved bits.
    pub fn decode(bytes: &[u8]) -> Result<Self, HandoffError> {
        if bytes.len() != BROKER_PACKET_BYTES
            || bytes[..8] != BROKER_MAGIC
            || bytes[8..10] != WIRE_VERSION.to_be_bytes()
            || bytes[12..16] != [0; 4]
        {
            return Err(HandoffError::InvalidPacket);
        }
        let mut epoch = [0_u8; 16];
        epoch.copy_from_slice(&bytes[16..32]);
        let mut sequence = [0_u8; 8];
        sequence.copy_from_slice(&bytes[32..40]);
        let mut identity = [0_u8; 32];
        identity.copy_from_slice(&bytes[40..72]);
        let packet = Self {
            operation: BrokerOperation::parse(bytes[10])?,
            status: HandoffStatus::parse_broker(bytes[11])?,
            epoch,
            sequence: u64::from_be_bytes(sequence),
            expected_runner_identity: identity,
        };
        packet.validate_shape()?;
        Ok(packet)
    }

    fn validate_shape(self) -> Result<(), HandoffError> {
        let zero_tuple = self.epoch == [0; 16] && self.sequence == 0;
        let zero_identity = self.expected_runner_identity == [0; 32];
        let valid = match (self.status, self.operation) {
            (HandoffStatus::Request, BrokerOperation::Initial) => zero_tuple && zero_identity,
            (HandoffStatus::Request, BrokerOperation::Replacement) => {
                !zero_tuple && self.epoch != [0; 16] && self.sequence != 0 && zero_identity
            }
            (HandoffStatus::Success, _) => {
                self.epoch != [0; 16] && self.sequence != 0 && !zero_identity
            }
            (status, BrokerOperation::Initial) if status.is_closed_failure() => {
                zero_tuple && zero_identity
            }
            (status, BrokerOperation::Replacement) if status.is_closed_failure() => {
                self.epoch != [0; 16] && self.sequence != 0 && zero_identity
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(HandoffError::InvalidPacket)
        }
    }
}

/// Exact 64-byte request or response on the private provisioning endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProvisioningPacket {
    /// Request or closed reply status.
    pub status: HandoffStatus,
    /// Caller-generated nonzero request nonce.
    pub nonce: [u8; 16],
    /// Service runner identity digest; nonzero only on success.
    pub runner_identity: [u8; 32],
}

impl ProvisioningPacket {
    /// Encode the frozen v1 representation.
    #[must_use]
    pub fn encode(self) -> [u8; PROVISIONING_PACKET_BYTES] {
        let mut bytes = [0_u8; PROVISIONING_PACKET_BYTES];
        bytes[..8].copy_from_slice(&PROVISIONING_MAGIC);
        bytes[8..10].copy_from_slice(&WIRE_VERSION.to_be_bytes());
        bytes[10] = 1;
        bytes[11] = self.status.wire();
        bytes[16..32].copy_from_slice(&self.nonce);
        bytes[32..].copy_from_slice(&self.runner_identity);
        bytes
    }

    /// Decode the exact frozen v1 representation and reject reserved bits.
    pub fn decode(bytes: &[u8]) -> Result<Self, HandoffError> {
        if bytes.len() != PROVISIONING_PACKET_BYTES
            || bytes[..8] != PROVISIONING_MAGIC
            || bytes[8..10] != WIRE_VERSION.to_be_bytes()
            || bytes[10] != 1
            || bytes[12..16] != [0; 4]
        {
            return Err(HandoffError::InvalidPacket);
        }
        let mut nonce = [0_u8; 16];
        nonce.copy_from_slice(&bytes[16..32]);
        let mut identity = [0_u8; 32];
        identity.copy_from_slice(&bytes[32..64]);
        let packet = Self {
            status: HandoffStatus::parse_provisioning(bytes[11])?,
            nonce,
            runner_identity: identity,
        };
        let valid = packet.nonce != [0; 16]
            && match packet.status {
                HandoffStatus::Request => packet.runner_identity == [0; 32],
                HandoffStatus::Success => packet.runner_identity != [0; 32],
                HandoffStatus::Unavailable | HandoffStatus::Rejected => {
                    packet.runner_identity == [0; 32]
                }
                HandoffStatus::Exhausted => false,
            };
        if valid {
            Ok(packet)
        } else {
            Err(HandoffError::InvalidPacket)
        }
    }
}

/// Compute the frozen, domain-separated runner instance identity.
pub fn runner_identity_digest(runner_instance_id: &str) -> Result<[u8; 32], HandoffError> {
    validate_identity(runner_instance_id).map_err(|_| HandoffError::InvalidRunnerIdentity)?;
    if runner_instance_id.len() > MAX_CONTROL_ID_BYTES {
        return Err(HandoffError::InvalidRunnerIdentity);
    }
    let length =
        u16::try_from(runner_instance_id.len()).map_err(|_| HandoffError::InvalidRunnerIdentity)?;
    let mut digest = Sha256::new();
    digest.update(RUNNER_IDENTITY_DOMAIN);
    digest.update([0]);
    digest.update(length.to_be_bytes());
    digest.update(runner_instance_id.as_bytes());
    Ok(digest.finalize().into())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BrokerPhase {
    New,
    Pending {
        operation: BrokerOperation,
        response_sequence: u64,
    },
    Established(BrokerGeneration),
    Terminal,
}

/// Fail-closed producer state for one private router/frontend broker.
pub struct BrokerState {
    epoch: [u8; 16],
    phase: BrokerPhase,
    replacement_attempts: u8,
    last_replacement: Option<Duration>,
}

impl BrokerState {
    /// Create state for one frontend lifetime from a fresh kernel-random epoch.
    pub fn new(epoch: [u8; 16]) -> Result<Self, HandoffError> {
        if epoch == [0; 16] {
            return Err(HandoffError::InvalidEpoch);
        }
        Ok(Self {
            epoch,
            phase: BrokerPhase::New,
            replacement_attempts: 0,
            last_replacement: None,
        })
    }

    /// Admit exactly one ordered request and reserve its response generation.
    pub fn begin_request(
        &mut self,
        packet: BrokerPacket,
        now: Duration,
    ) -> Result<(), HandoffError> {
        if packet.status != HandoffStatus::Request {
            return self.terminate(HandoffError::UnexpectedState);
        }
        match (self.phase, packet.operation) {
            (BrokerPhase::New, BrokerOperation::Initial) => {
                if packet.validate_shape().is_err() {
                    return self.terminate(HandoffError::InvalidPacket);
                }
                self.phase = BrokerPhase::Pending {
                    operation: BrokerOperation::Initial,
                    response_sequence: 1,
                };
                Ok(())
            }
            (BrokerPhase::Established(current), BrokerOperation::Replacement) => {
                if packet.validate_shape().is_err() {
                    return self.terminate(HandoffError::InvalidPacket);
                }
                if packet.epoch != current.epoch || packet.sequence != current.sequence {
                    return self.terminate(HandoffError::GenerationMismatch);
                }
                if self.replacement_attempts >= MAX_REPLACEMENTS {
                    return self.terminate(HandoffError::ReplacementExhausted);
                }
                if self.last_replacement.is_some_and(|last| {
                    now.checked_sub(last)
                        .is_none_or(|elapsed| elapsed < MIN_REPLACEMENT_INTERVAL)
                }) {
                    return self.terminate(HandoffError::RateLimited);
                }
                let Some(response_sequence) = current.sequence.checked_add(1) else {
                    return self.terminate(HandoffError::SequenceExhausted);
                };
                self.replacement_attempts += 1;
                self.last_replacement = Some(now);
                self.phase = BrokerPhase::Pending {
                    operation: BrokerOperation::Replacement,
                    response_sequence,
                };
                Ok(())
            }
            _ => self.terminate(HandoffError::UnexpectedState),
        }
    }

    /// Commit a successful descriptor send and advance the generation exactly once.
    pub fn complete_success(
        &mut self,
        expected_runner_identity: [u8; 32],
    ) -> Result<BrokerPacket, HandoffError> {
        if expected_runner_identity == [0; 32] {
            return self.terminate(HandoffError::InvalidRunnerIdentity);
        }
        let BrokerPhase::Pending {
            operation,
            response_sequence,
        } = self.phase
        else {
            return self.terminate(HandoffError::UnexpectedState);
        };
        let generation = BrokerGeneration {
            epoch: self.epoch,
            sequence: response_sequence,
        };
        self.phase = BrokerPhase::Established(generation);
        Ok(BrokerPacket {
            operation,
            status: HandoffStatus::Success,
            epoch: generation.epoch,
            sequence: generation.sequence,
            expected_runner_identity,
        })
    }

    /// Complete a descriptor-free closed failure without advancing a generation.
    pub fn complete_failure(
        &mut self,
        status: HandoffStatus,
    ) -> Result<BrokerPacket, HandoffError> {
        if !status.is_closed_failure() {
            return self.terminate(HandoffError::UnexpectedState);
        }
        let BrokerPhase::Pending { operation, .. } = self.phase else {
            return self.terminate(HandoffError::UnexpectedState);
        };
        let generation = match operation {
            BrokerOperation::Initial => {
                self.phase = BrokerPhase::Terminal;
                BrokerGeneration {
                    epoch: [0; 16],
                    sequence: 0,
                }
            }
            BrokerOperation::Replacement => {
                let BrokerPhase::Pending { .. } = self.phase else {
                    unreachable!();
                };
                let current = BrokerGeneration {
                    epoch: self.epoch,
                    sequence: self.pending_previous_sequence(),
                };
                self.phase = BrokerPhase::Established(current);
                current
            }
        };
        Ok(BrokerPacket {
            operation,
            status,
            epoch: generation.epoch,
            sequence: generation.sequence,
            expected_runner_identity: [0; 32],
        })
    }

    fn pending_previous_sequence(&self) -> u64 {
        match self.phase {
            BrokerPhase::Pending {
                response_sequence, ..
            } => response_sequence - 1,
            _ => 0,
        }
    }

    fn terminate<T>(&mut self, error: HandoffError) -> Result<T, HandoffError> {
        self.phase = BrokerPhase::Terminal;
        Err(error)
    }
}

/// Authenticated anonymous control stream produced by ASB.
pub struct AuthenticatedGeneration {
    stream: UnixStream,
    broker_generation: BrokerGeneration,
    kernel_peer: PeerIdentity,
    expected_runner_identity: [u8; 32],
}

impl AuthenticatedGeneration {
    /// Create a generation only from kernel-authenticated ASB-side evidence.
    pub fn new(
        stream: UnixStream,
        broker_generation: BrokerGeneration,
        kernel_peer: PeerIdentity,
        expected_runner_identity: [u8; 32],
    ) -> Result<Self, HandoffError> {
        if broker_generation.epoch == [0; 16]
            || broker_generation.sequence == 0
            || expected_runner_identity == [0; 32]
        {
            return Err(HandoffError::UnexpectedState);
        }
        Ok(Self {
            stream,
            broker_generation,
            kernel_peer,
            expected_runner_identity,
        })
    }

    /// Accepted broker generation.
    #[must_use]
    pub const fn broker_generation(&self) -> BrokerGeneration {
        self.broker_generation
    }

    /// Short-lived kernel peer evidence captured during acquisition.
    #[must_use]
    pub const fn kernel_peer(&self) -> PeerIdentity {
        self.kernel_peer
    }

    /// Independently recomputable runner identity digest.
    #[must_use]
    pub const fn expected_runner_identity(&self) -> [u8; 32] {
        self.expected_runner_identity
    }

    /// Consume the evidence wrapper and return the anonymous control stream.
    #[must_use]
    pub fn into_stream(self) -> UnixStream {
        self.stream
    }
}

/// Closed handoff failure without endpoint, credential, or descriptor details.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum HandoffError {
    /// Exact packet shape or frozen field is invalid.
    #[error("invalid handoff packet")]
    InvalidPacket,
    /// Runner identity is not a bounded protocol identity.
    #[error("invalid runner identity")]
    InvalidRunnerIdentity,
    /// Kernel-random broker epoch is invalid.
    #[error("invalid broker epoch")]
    InvalidEpoch,
    /// Packet is stale, reordered, or from another broker lifetime.
    #[error("handoff generation mismatch")]
    GenerationMismatch,
    /// State transition is not legal for this broker lifetime.
    #[error("unexpected handoff state")]
    UnexpectedState,
    /// Replacement ceiling was reached.
    #[error("handoff replacement limit exhausted")]
    ReplacementExhausted,
    /// Replacement arrived before the fixed minimum interval.
    #[error("handoff replacement rate exceeded")]
    RateLimited,
    /// Generation sequence cannot advance without wrapping.
    #[error("handoff sequence exhausted")]
    SequenceExhausted,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn bytes_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn initial_request() -> BrokerPacket {
        BrokerPacket {
            operation: BrokerOperation::Initial,
            status: HandoffStatus::Request,
            epoch: [0; 16],
            sequence: 0,
            expected_runner_identity: [0; 32],
        }
    }

    #[test]
    fn frozen_packets_match_canonical_hashes() {
        let initial = initial_request().encode();
        assert_eq!(
            digest_hex(&initial),
            "e1dc9317afbc4bce748f0ce65424ad185c8a29e5ab40eb3e724d8930e77f216a"
        );
        assert_eq!(BrokerPacket::decode(&initial).unwrap(), initial_request());

        let success = BrokerPacket {
            operation: BrokerOperation::Initial,
            status: HandoffStatus::Success,
            epoch: std::array::from_fn(|index| u8::try_from(index).unwrap()),
            sequence: 1,
            expected_runner_identity: std::array::from_fn(|index| {
                0xa0 + u8::try_from(index).unwrap()
            }),
        };
        assert_eq!(
            digest_hex(&success.encode()),
            "0936d182cde09230bcb3fd3f40c9f0aaa886e23fdb42ac68cfb1e098b8cf98f5"
        );

        let request = ProvisioningPacket {
            status: HandoffStatus::Request,
            nonce: std::array::from_fn(|index| u8::try_from(index).unwrap()),
            runner_identity: [0; 32],
        };
        assert_eq!(
            digest_hex(&request.encode()),
            "b3d2dc48c3c4afffbf2be9081974d72be4ed4ec3f737c51ad07af4aeab68ef4d"
        );
        let response = ProvisioningPacket {
            status: HandoffStatus::Success,
            nonce: request.nonce,
            runner_identity: std::array::from_fn(|index| 0xa0 + u8::try_from(index).unwrap()),
        };
        assert_eq!(
            digest_hex(&response.encode()),
            "72c2bd598a591bf033db321e9e2acccea451a69ccabfc27498a9d8cdb2c032e5"
        );
        assert_eq!(
            ProvisioningPacket::decode(&request.encode()).unwrap(),
            request
        );
        assert_eq!(
            ProvisioningPacket::decode(&response.encode()).unwrap(),
            response
        );
    }

    #[test]
    fn runner_identity_digest_matches_frozen_golden() {
        assert_eq!(
            bytes_hex(&runner_identity_digest("runner-handoff-test").unwrap()),
            "9035cbae7ce01f88036f2b24b524268b91e2e2d3833e1607b29ce37a9e0cc872"
        );
    }

    #[test]
    fn broker_advances_only_after_success_and_preserves_failed_generation() {
        let epoch = [7; 16];
        let identity = [9; 32];
        let mut broker = BrokerState::new(epoch).unwrap();
        broker
            .begin_request(initial_request(), Duration::ZERO)
            .unwrap();
        let first = broker.complete_success(identity).unwrap();
        assert_eq!((first.epoch, first.sequence), (epoch, 1));

        let replacement = BrokerPacket {
            operation: BrokerOperation::Replacement,
            status: HandoffStatus::Request,
            epoch,
            sequence: 1,
            expected_runner_identity: [0; 32],
        };
        broker
            .begin_request(replacement, MIN_REPLACEMENT_INTERVAL)
            .unwrap();
        let failed = broker.complete_failure(HandoffStatus::Unavailable).unwrap();
        assert_eq!((failed.epoch, failed.sequence), (epoch, 1));
        broker
            .begin_request(replacement, MIN_REPLACEMENT_INTERVAL * 2)
            .unwrap();
        let second = broker.complete_success(identity).unwrap();
        assert_eq!((second.epoch, second.sequence), (epoch, 2));
    }

    #[test]
    fn malformed_and_out_of_order_packets_fail_closed() {
        let mut bytes = initial_request().encode();
        for index in 0..BROKER_PACKET_BYTES {
            let saved = bytes[index];
            bytes[index] ^= 0x80;
            assert!(BrokerPacket::decode(&bytes).is_err(), "byte {index}");
            bytes[index] = saved;
        }

        let mut broker = BrokerState::new([1; 16]).unwrap();
        let stale = BrokerPacket {
            operation: BrokerOperation::Replacement,
            status: HandoffStatus::Request,
            epoch: [1; 16],
            sequence: 1,
            expected_runner_identity: [0; 32],
        };
        assert_eq!(
            broker.begin_request(stale, Duration::ZERO),
            Err(HandoffError::UnexpectedState)
        );
        assert_eq!(
            broker.begin_request(initial_request(), Duration::ZERO),
            Err(HandoffError::UnexpectedState)
        );
    }
}
