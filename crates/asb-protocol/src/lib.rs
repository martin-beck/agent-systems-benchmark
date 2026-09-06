// SPDX-License-Identifier: MIT
//! Versioned contracts for ASB built-in and external extensions.
//!
//! External extensions are independent executables. They exchange one JSON-RPC
//! 2.0 object per newline on standard input/output; standard error is reserved
//! for logs. A caller must negotiate [`PROTOCOL_V1`] before any other method,
//! enforce a monotonic [`CallLimits::timeout_ms`] deadline, and reject a frame
//! larger than [`CallLimits::max_frame_bytes`]. This crate intentionally exposes
//! no Rust dynamic-library ABI.

mod experiment;

pub use experiment::*;

use std::collections::BTreeSet;
use std::io::{BufRead, Write};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// JSON-RPC version required on every envelope.
pub const JSONRPC_VERSION: &str = "2.0";
/// The only protocol version implemented by this crate.
pub const PROTOCOL_V1: ProtocolVersion = ProtocolVersion { major: 1, minor: 0 };
/// Largest limit a peer may negotiate, in bytes.
pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;
/// Longest per-call deadline a peer may negotiate, in milliseconds.
pub const MAX_TIMEOUT_MS: u64 = 60 * 60 * 1_000;
/// Largest number of requests a peer may allow concurrently.
pub const MAX_IN_FLIGHT: u16 = 1024;
/// Valid JSON-RPC envelope discriminator.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub enum JsonRpcVersion {
    /// JSON-RPC version 2.0.
    #[serde(rename = "2.0")]
    V2,
}

/// Stable JSON-RPC application error codes.
pub mod error_code {
    /// No supported protocol major overlaps the peer's offer.
    pub const INCOMPATIBLE_VERSION: i32 = -32_001;
    /// A newline-delimited frame exceeded its negotiated bound.
    pub const FRAME_TOO_LARGE: i32 = -32_002;
    /// The caller's monotonic deadline expired.
    pub const DEADLINE_EXCEEDED: i32 = -32_003;
    /// The peer did not negotiate the required capability.
    pub const CAPABILITY_UNAVAILABLE: i32 = -32_004;
    /// The attempt or session identity is stale.
    pub const STALE_ATTEMPT: i32 = -32_005;
    /// The request identifier is already outstanding.
    pub const DUPLICATE_REQUEST: i32 = -32_006;
    /// The negotiated in-flight request capacity is exhausted.
    pub const RESOURCE_EXHAUSTED: i32 = -32_007;
}

/// A major/minor protocol version.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(deny_unknown_fields)]
pub struct ProtocolVersion {
    /// Breaking protocol generation.
    pub major: u16,
    /// Backward-compatible feature generation.
    pub minor: u16,
}

impl ProtocolVersion {
    /// Negotiate the highest minor understood by both v1 peers.
    pub fn negotiate(self) -> Result<Self, NegotiationError> {
        if self.major != PROTOCOL_V1.major {
            return Err(NegotiationError::UnsupportedMajor(self.major));
        }
        Ok(Self {
            major: PROTOCOL_V1.major,
            minor: PROTOCOL_V1.minor,
        })
    }
}

/// Negotiation failure before extension methods may run.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum NegotiationError {
    /// The peer proposed an unknown breaking generation.
    #[error("unsupported protocol major {0}")]
    UnsupportedMajor(u16),
}

/// Negotiated resource limits for every request and response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CallLimits {
    /// Maximum UTF-8 JSON frame size including no newline.
    pub max_frame_bytes: u32,
    /// Monotonic request-to-response deadline in milliseconds.
    pub timeout_ms: u64,
    /// Maximum accepted requests whose terminal responses remain outstanding.
    pub max_in_flight: u16,
}

impl CallLimits {
    /// Validate nonzero limits against hard implementation ceilings.
    pub fn validate(self) -> Result<Self, LimitError> {
        if self.max_frame_bytes == 0 || self.max_frame_bytes > MAX_FRAME_BYTES {
            return Err(LimitError::InvalidFrameLimit(self.max_frame_bytes));
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_TIMEOUT_MS {
            return Err(LimitError::InvalidTimeout(self.timeout_ms));
        }
        if self.max_in_flight == 0 || self.max_in_flight > MAX_IN_FLIGHT {
            return Err(LimitError::InvalidInFlight(self.max_in_flight));
        }
        Ok(self)
    }

    /// Select limits no weaker than either peer offered.
    pub fn intersect(self, peer: Self) -> Result<Self, LimitError> {
        self.validate()?;
        peer.validate()?;
        Ok(Self {
            max_frame_bytes: self.max_frame_bytes.min(peer.max_frame_bytes),
            timeout_ms: self.timeout_ms.min(peer.timeout_ms),
            max_in_flight: self.max_in_flight.min(peer.max_in_flight),
        })
    }
}

/// Invalid protocol resource bounds.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum LimitError {
    /// The maximum frame is zero or exceeds [`MAX_FRAME_BYTES`].
    #[error("invalid maximum frame size {0}")]
    InvalidFrameLimit(u32),
    /// The timeout is zero or exceeds [`MAX_TIMEOUT_MS`].
    #[error("invalid timeout {0}ms")]
    InvalidTimeout(u64),
    /// The in-flight bound is zero or above the hard ceiling.
    #[error("invalid in-flight bound {0}")]
    InvalidInFlight(u16),
}

/// Outstanding request identities enforcing negotiated backpressure.
#[derive(Debug)]
pub struct InFlightRequests {
    maximum: usize,
    requests: BTreeSet<RequestId>,
}

impl InFlightRequests {
    /// Construct an empty tracker from validated limits.
    pub fn new(limits: CallLimits) -> Result<Self, LimitError> {
        let limits = limits.validate()?;
        Ok(Self {
            maximum: usize::from(limits.max_in_flight),
            requests: BTreeSet::new(),
        })
    }

    /// Reserve capacity before starting any external effect.
    pub fn start(&mut self, id: RequestId) -> Result<(), FlowControlError> {
        if self.requests.contains(&id) {
            return Err(FlowControlError::DuplicateRequest);
        }
        if self.requests.len() == self.maximum {
            return Err(FlowControlError::Backpressure);
        }
        self.requests.insert(id);
        Ok(())
    }

    /// Release capacity after the terminal response is committed.
    pub fn finish(&mut self, id: &RequestId) -> bool {
        self.requests.remove(id)
    }
}

/// Request admission failure before external effects.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum FlowControlError {
    /// The identifier is already outstanding.
    #[error("duplicate outstanding request")]
    DuplicateRequest,
    /// Negotiated request capacity is exhausted.
    #[error("request backpressure")]
    Backpressure,
}

/// Stable identifier transported without host-specific meaning.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Id(pub String);

/// Extension families with distinct lifecycle methods.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
    /// AI coding-agent adapter.
    Agent,
    /// Workload and independent grader.
    Workload,
    /// Measurement collector.
    Collector,
    /// Execution backend.
    ExecutionBackend,
}

/// Capabilities must be explicitly negotiated; an absent value is unavailable.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// The peer can cancel an active attempt.
    Cancellation,
    /// The peer emits incremental events.
    StreamingEvents,
    /// The peer reports token or monetary usage.
    Usage,
    /// The peer can execute without network access after preparation.
    Offline,
    /// An extension-defined capability that remains unavailable unless requested verbatim.
    Other(String),
}

/// A content-pinned external extension description.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionManifest {
    /// Stable extension identity.
    pub extension_id: Id,
    /// Extension contract family.
    pub kind: ExtensionKind,
    /// Extension implementation version.
    pub implementation_version: String,
    /// Protocol implemented by the executable.
    pub protocol: ProtocolVersion,
    /// Explicitly implemented capabilities.
    pub capabilities: BTreeSet<Capability>,
    /// Lowercase SHA-256 of the executable or package.
    pub executable_sha256: String,
}

/// Workload provenance and execution requirements.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadManifest {
    /// Stable workload identity.
    pub workload_id: Id,
    /// Workload content version.
    pub version: String,
    /// SPDX license expression.
    pub license: String,
    /// Immutable upstream source revision.
    pub source_revision: String,
    /// Lowercase SHA-256 of canonical workload content.
    pub content_sha256: String,
    /// Supported CPU architectures.
    pub architectures: BTreeSet<String>,
    /// Supported operating systems.
    pub operating_systems: BTreeSet<String>,
    /// Prepared fixture size in bytes.
    pub fixture_bytes: u64,
    /// Attempt timeout in milliseconds.
    pub timeout_ms: u64,
    /// Maximum logical CPUs requested.
    pub cpu_count: u32,
    /// Maximum memory requested in bytes.
    pub memory_bytes: u64,
    /// Independently versioned scoring implementation.
    pub scoring_version: String,
    /// Network destinations allowed during execution; empty means offline.
    pub allowed_network_destinations: BTreeSet<String>,
}

/// Methods in protocol v1.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Negotiate protocol version, bounds and capabilities.
    Negotiate,
    /// Describe an extension.
    Describe,
    /// Probe environmental support.
    Probe,
    /// Prepare resources.
    Prepare,
    /// Start an agent, collector or execution backend.
    Start,
    /// Cancel an active attempt.
    Cancel,
    /// Collect terminal agent/backend evidence.
    Collect,
    /// Clean up resources.
    Cleanup,
    /// Acquire workload content.
    Acquire,
    /// Produce the workload prompt.
    Prompt,
    /// Independently evaluate workload output.
    Evaluate,
    /// Sample a collector.
    Sample,
    /// Stop a collector.
    Stop,
}

/// JSON-RPC request identity; null is deliberately not accepted.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(untagged)]
pub enum RequestId {
    /// Decimal integer identity.
    Number(u64),
    /// String identity.
    String(String),
}

/// A strict JSON-RPC request envelope.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RpcRequest {
    /// Must equal [`JSONRPC_VERSION`].
    pub jsonrpc: JsonRpcVersion,
    /// Correlates exactly one response.
    pub id: RequestId,
    /// Typed protocol method.
    pub method: Method,
    /// Method parameters.
    pub params: Value,
}

/// A strict successful JSON-RPC response envelope.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RpcResponse {
    /// Must equal [`JSONRPC_VERSION`].
    pub jsonrpc: JsonRpcVersion,
    /// Identity copied from the request.
    pub id: RequestId,
    /// Method-specific result.
    pub result: Value,
}

/// A strict failed JSON-RPC response envelope.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RpcErrorResponse {
    /// Must equal [`JSONRPC_VERSION`].
    pub jsonrpc: JsonRpcVersion,
    /// Identity copied from the request when it was recoverable.
    pub id: Option<RequestId>,
    /// Typed error.
    pub error: RpcError,
}

/// Method names reserved for server-to-caller notifications.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationMethod {
    /// Ordered extension lifecycle event.
    Event,
}

/// A JSON-RPC notification has no request identity or response.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RpcNotification {
    /// Must equal [`JSONRPC_VERSION`].
    pub jsonrpc: JsonRpcVersion,
    /// Typed notification method.
    pub method: NotificationMethod,
    /// Event with session, attempt and sequence identity.
    pub params: ExtensionEvent,
}

/// JSON-RPC error with stable code and optional structured evidence.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RpcError {
    /// Standard JSON-RPC or ASB application code.
    pub code: i32,
    /// Safe diagnostic summary.
    pub message: String,
    /// Optional structured, privacy-reviewed data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Initial request parameters required before other methods.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NegotiateParams {
    /// Highest version understood by the caller.
    pub protocol: ProtocolVersion,
    /// Caller-enforced bounds.
    pub limits: CallLimits,
    /// Capabilities the caller may use.
    pub requested_capabilities: BTreeSet<Capability>,
}

/// Successful negotiated contract.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NegotiateResult {
    /// Selected common protocol.
    pub protocol: ProtocolVersion,
    /// Intersection of caller and extension bounds.
    pub limits: CallLimits,
    /// Requested capabilities that the extension explicitly supports.
    pub capabilities: BTreeSet<Capability>,
}

/// Identity and method-specific parameters for an attempt operation.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptParams {
    /// Stable session identity.
    pub session_id: Id,
    /// Stable attempt identity; stale attempts must be rejected.
    pub attempt_id: Id,
    /// Method-specific parameters.
    pub payload: Value,
}

/// Ordered event notification emitted by an extension.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionEvent {
    /// Stable session identity.
    pub session_id: Id,
    /// Stable attempt identity.
    pub attempt_id: Id,
    /// Strictly increasing sequence within the attempt.
    pub sequence: u64,
    /// Event-specific value.
    pub event: Event,
}

/// Agent and runtime lifecycle events.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Event {
    /// Extension is ready to accept work.
    Ready,
    /// A provider request started.
    RequestStarted,
    /// First response byte or event arrived.
    FirstResponse,
    /// A tool invocation started.
    ToolStarted {
        /// Stable causal tool-call identity.
        tool_call_id: Id,
        /// Tool name reported by the agent.
        name: String,
    },
    /// A tool invocation finished.
    ToolFinished {
        /// Stable causal tool-call identity.
        tool_call_id: Id,
        /// Whether the tool reported success.
        success: bool,
    },
    /// Usage evidence was emitted.
    Usage(Usage),
    /// Attempt completed normally.
    Completed,
    /// Attempt failed.
    Failed(RpcError),
}

/// Usage telemetry; unavailable values remain absent, never zero-filled.
#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    /// Provider input tokens when reported.
    pub input_tokens: Option<u64>,
    /// Provider output tokens when reported.
    pub output_tokens: Option<u64>,
    /// Monetary cost in micros of the manifest currency when known.
    pub cost_micros: Option<u64>,
    /// ISO 4217 currency code when cost is known.
    pub currency: Option<String>,
}

/// Terminal attempt status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalStatus {
    /// All required operations completed.
    Completed,
    /// The attempt failed.
    Failed,
    /// Cancellation reached a terminal state.
    Cancelled,
}

/// Metric aggregation semantics.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregation {
    /// Point-in-time gauge.
    Gauge,
    /// Monotonic cumulative counter.
    Counter,
    /// Sample distribution.
    Distribution,
}

/// Complete metric meaning independent of its numeric sample.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetricDescriptor {
    /// Stable metric identity.
    pub metric_id: Id,
    /// UCUM-style unit string.
    pub unit: String,
    /// Aggregation semantics.
    pub aggregation: Aggregation,
    /// Process, cgroup, host or other explicit measurement scope.
    pub scope: String,
    /// Kernel or userspace source.
    pub source: String,
    /// Nominal resolution in nanoseconds.
    pub resolution_ns: u64,
}

/// Metric observation preserving missing evidence.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "availability", rename_all = "snake_case")]
pub enum MetricValue {
    /// A finite numeric sample.
    Available {
        /// Finite value in the descriptor's unit.
        value: f64,
    },
    /// No sample exists and the reason is explicit.
    Unavailable {
        /// Evidence explaining why no value exists.
        reason: String,
    },
}

/// One metric sample.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetricSample {
    /// Metric definition.
    pub descriptor: MetricDescriptor,
    /// Monotonic offset from attempt start.
    pub offset_ns: u64,
    /// Present or explicitly unavailable value.
    pub value: MetricValue,
}

/// Content-addressed artifact emitted by an attempt.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    /// Relative logical artifact path.
    pub name: String,
    /// Byte length.
    pub size_bytes: u64,
    /// Lowercase SHA-256 of content.
    pub sha256: String,
}

/// Versioned terminal result contract.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionResult {
    /// Schema generation for durable readers.
    pub schema_version: ProtocolVersion,
    /// Stable session identity.
    pub session_id: Id,
    /// Stable attempt identity.
    pub attempt_id: Id,
    /// Terminal state.
    pub status: TerminalStatus,
    /// Failure evidence, required by policy when status is failed.
    pub error: Option<RpcError>,
    /// Content-addressed outputs.
    pub artifacts: Vec<ArtifactRef>,
    /// Measurements including explicit unavailable values.
    pub metrics: Vec<MetricSample>,
    /// Usage including explicit absence.
    pub usage: Usage,
}

impl ExtensionResult {
    /// Check cross-field invariants that JSON Schema cannot express.
    pub fn validate(&self) -> Result<(), ResultError> {
        if self.schema_version.major != PROTOCOL_V1.major {
            return Err(ResultError::UnsupportedSchemaMajor(
                self.schema_version.major,
            ));
        }
        match (self.status, self.error.is_some()) {
            (TerminalStatus::Failed, false) => Err(ResultError::MissingFailure),
            (TerminalStatus::Completed | TerminalStatus::Cancelled, true) => {
                Err(ResultError::UnexpectedFailure)
            }
            _ => Ok(()),
        }
    }
}

/// Invalid cross-field terminal result state.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ResultError {
    /// Durable result schema uses an unknown breaking generation.
    #[error("unsupported result schema major {0}")]
    UnsupportedSchemaMajor(u16),
    /// Failed terminal state lacks typed failure evidence.
    #[error("failed result lacks error evidence")]
    MissingFailure,
    /// A non-failed terminal state incorrectly carries failure evidence.
    #[error("non-failed result carries error evidence")]
    UnexpectedFailure,
}

/// Newline framing or JSON decoding failure.
#[derive(Debug, Error)]
pub enum FrameError {
    /// Underlying stream I/O failed.
    #[error("frame I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// End of stream occurred before a complete frame.
    #[error("unexpected end of stream")]
    EndOfStream,
    /// Frame exceeded the negotiated maximum.
    #[error("frame exceeds negotiated maximum of {max} bytes")]
    TooLarge {
        /// Negotiated maximum frame size.
        max: usize,
    },
    /// A frame was not valid UTF-8 JSON of the requested type.
    #[error("invalid JSON frame: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

/// Read and deserialize one bounded newline-delimited JSON frame.
pub fn read_frame<T: for<'de> Deserialize<'de>>(
    reader: &mut impl BufRead,
    max: usize,
) -> Result<T, FrameError> {
    let max = max.min(MAX_FRAME_BYTES as usize);
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Err(FrameError::EndOfStream);
        }
        if let Some(end) = available.iter().position(|byte| *byte == b'\n') {
            if bytes.len().saturating_add(end) > max {
                return Err(FrameError::TooLarge { max });
            }
            bytes.extend_from_slice(&available[..end]);
            reader.consume(end + 1);
            break;
        }
        if bytes.len().saturating_add(available.len()) > max {
            return Err(FrameError::TooLarge { max });
        }
        let consumed = available.len();
        bytes.extend_from_slice(available);
        reader.consume(consumed);
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    Ok(serde_json::from_slice(&bytes)?)
}

/// Serialize and write one bounded newline-delimited JSON frame.
pub fn write_frame<T: Serialize>(
    writer: &mut impl Write,
    value: &T,
    max: usize,
) -> Result<(), FrameError> {
    let max = max.min(MAX_FRAME_BYTES as usize);
    let mut buffer = BoundedBuffer::new(max);
    if let Err(error) = serde_json::to_writer(&mut buffer, value) {
        return Err(if buffer.exceeded {
            FrameError::TooLarge { max }
        } else {
            FrameError::InvalidJson(error)
        });
    }
    writer.write_all(&buffer.bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

struct BoundedBuffer {
    bytes: Vec<u8>,
    maximum: usize,
    exceeded: bool,
}

impl BoundedBuffer {
    fn new(maximum: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(maximum.min(4096)),
            maximum,
            exceeded: false,
        }
    }
}

impl Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.maximum.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(std::io::Error::other("JSON frame exceeds bound"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::*;

    #[test]
    fn unknown_major_is_rejected() {
        assert_eq!(
            ProtocolVersion { major: 2, minor: 0 }.negotiate(),
            Err(NegotiationError::UnsupportedMajor(2))
        );
    }

    #[test]
    fn limits_intersect_to_stricter_peer() {
        let local = CallLimits {
            max_frame_bytes: 4096,
            timeout_ms: 5_000,
            max_in_flight: 8,
        };
        let peer = CallLimits {
            max_frame_bytes: 1024,
            timeout_ms: 10_000,
            max_in_flight: 2,
        };
        assert_eq!(
            local.intersect(peer),
            Ok(CallLimits {
                max_frame_bytes: 1024,
                timeout_ms: 5_000,
                max_in_flight: 2,
            })
        );
    }

    #[test]
    fn zero_and_excessive_limits_fail() {
        assert!(matches!(
            CallLimits {
                max_frame_bytes: 0,
                timeout_ms: 1,
                max_in_flight: 1,
            }
            .validate(),
            Err(LimitError::InvalidFrameLimit(0))
        ));
        assert!(matches!(
            CallLimits {
                max_frame_bytes: 1,
                timeout_ms: MAX_TIMEOUT_MS + 1,
                max_in_flight: 1,
            }
            .validate(),
            Err(LimitError::InvalidTimeout(_))
        ));
        assert!(matches!(
            CallLimits {
                max_frame_bytes: 1,
                timeout_ms: 1,
                max_in_flight: 0,
            }
            .validate(),
            Err(LimitError::InvalidInFlight(0))
        ));
    }

    #[test]
    fn in_flight_tracker_rejects_duplicates_and_exhaustion() {
        let mut tracker = InFlightRequests::new(CallLimits {
            max_frame_bytes: 1,
            timeout_ms: 1,
            max_in_flight: 1,
        })
        .unwrap();
        let first = RequestId::Number(1);
        assert_eq!(tracker.start(first.clone()), Ok(()));
        assert_eq!(
            tracker.start(first.clone()),
            Err(FlowControlError::DuplicateRequest)
        );
        assert_eq!(
            tracker.start(RequestId::Number(2)),
            Err(FlowControlError::Backpressure)
        );
        assert!(tracker.finish(&first));
        assert!(!tracker.finish(&first));
        assert_eq!(tracker.start(RequestId::Number(2)), Ok(()));
    }

    #[test]
    fn oversized_input_is_rejected_before_json_decode() {
        let mut input = BufReader::new(Cursor::new(b"123456\n"));
        assert!(matches!(
            read_frame::<Value>(&mut input, 5),
            Err(FrameError::TooLarge { max: 5 })
        ));
    }

    #[test]
    fn unterminated_input_is_rejected() {
        let mut input = BufReader::new(Cursor::new(b"{}"));
        assert!(matches!(
            read_frame::<Value>(&mut input, 16),
            Err(FrameError::EndOfStream)
        ));
    }

    #[test]
    fn fragmented_oversized_input_is_rejected() {
        let input = Cursor::new(b"123456");
        let mut input = BufReader::with_capacity(2, input);
        assert!(matches!(
            read_frame::<Value>(&mut input, 5),
            Err(FrameError::TooLarge { max: 5 })
        ));
    }

    #[test]
    fn carriage_return_newline_is_accepted() {
        let mut input = BufReader::new(Cursor::new(b"{}\r\n"));
        assert_eq!(
            read_frame::<Value>(&mut input, 16).unwrap(),
            serde_json::json!({})
        );
    }

    #[test]
    fn oversized_output_is_not_partially_written() {
        let mut output = Vec::new();
        assert!(matches!(
            write_frame(&mut output, &"abcdef", 3),
            Err(FrameError::TooLarge { max: 3 })
        ));
        assert!(output.is_empty());
    }

    #[test]
    fn bounded_serialization_buffer_stops_before_growth() {
        let mut buffer = BoundedBuffer::new(2);
        assert_eq!(buffer.write(b"12").unwrap(), 2);
        assert!(buffer.write(b"3").is_err());
        assert!(buffer.exceeded);
        assert_eq!(buffer.bytes, b"12");
        buffer.flush().unwrap();
    }

    #[test]
    fn unknown_envelope_fields_fail_closed() {
        let json = br#"{"jsonrpc":"2.0","id":1,"method":"describe","params":{},"extra":true}"#;
        assert!(serde_json::from_slice::<RpcRequest>(json).is_err());
    }

    #[test]
    fn invalid_jsonrpc_discriminator_fails_closed() {
        let json = br#"{"jsonrpc":"1.0","id":1,"method":"describe","params":{}}"#;
        assert!(serde_json::from_slice::<RpcRequest>(json).is_err());
    }

    #[test]
    fn terminal_result_error_invariant_is_exhaustive() {
        fn result(status: TerminalStatus, error: Option<RpcError>) -> ExtensionResult {
            ExtensionResult {
                schema_version: PROTOCOL_V1,
                session_id: Id("session".into()),
                attempt_id: Id("attempt".into()),
                status,
                error,
                artifacts: Vec::new(),
                metrics: Vec::new(),
                usage: Usage::default(),
            }
        }
        let error = Some(RpcError {
            code: -32603,
            message: "failure".into(),
            data: None,
        });
        assert_eq!(result(TerminalStatus::Completed, None).validate(), Ok(()));
        assert_eq!(
            result(TerminalStatus::Completed, error.clone()).validate(),
            Err(ResultError::UnexpectedFailure)
        );
        assert_eq!(
            result(TerminalStatus::Failed, None).validate(),
            Err(ResultError::MissingFailure)
        );
        assert_eq!(
            result(TerminalStatus::Failed, error.clone()).validate(),
            Ok(())
        );
        assert_eq!(result(TerminalStatus::Cancelled, None).validate(), Ok(()));
        assert_eq!(
            result(TerminalStatus::Cancelled, error).validate(),
            Err(ResultError::UnexpectedFailure)
        );
        let mut wrong_major = result(TerminalStatus::Completed, None);
        wrong_major.schema_version.major = 2;
        assert_eq!(
            wrong_major.validate(),
            Err(ResultError::UnsupportedSchemaMajor(2))
        );
    }
}
