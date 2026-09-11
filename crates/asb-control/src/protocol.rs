// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Wire-visible control requests, responses, and events.

use std::collections::BTreeSet;

use asb_protocol::{
    MAX_MEASUREMENT_CATALOG_WIRE_BYTES, MeasurementCatalogV1, MeasurementSelectionReason,
    baseline_measurement_catalog,
};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::{Value, value::RawValue};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Required JSON-RPC discriminator.
pub const JSONRPC_VERSION: &str = "2.0";
/// Current control protocol generation.
pub const CONTROL_V1: ControlVersion = ControlVersion { major: 1, minor: 0 };
/// Version of the additive history and analysis evidence extension.
pub const CONTROL_HISTORY_ANALYSIS_V1: ControlVersion = ControlVersion { major: 1, minor: 1 };
/// Version of the additive measurement-catalog operation.
pub const CONTROL_MEASUREMENT_CATALOG_V1: ControlVersion = ControlVersion { major: 1, minor: 2 };
/// Version of additive structured measurement-selection validation diagnostics.
pub const CONTROL_MEASUREMENT_SELECTION_V1: ControlVersion = ControlVersion { major: 1, minor: 3 };
/// Exact wire versions implemented by the endpoint, in negotiation order.
pub const SUPPORTED_CONTROL_VERSIONS: [ControlVersion; 3] = [
    CONTROL_V1,
    CONTROL_MEASUREMENT_CATALOG_V1,
    CONTROL_MEASUREMENT_SELECTION_V1,
];
/// Absolute maximum frame accepted by the local control boundary.
pub const MAX_CONTROL_FRAME_BYTES: u32 = 1024 * 1024;
/// Absolute maximum request deadline.
pub const MAX_CONTROL_TIMEOUT_MS: u64 = 5 * 60 * 1000;
/// Absolute maximum page size.
pub const MAX_PAGE_ITEMS: u16 = 256;
/// Absolute maximum outstanding requests per connection.
pub const MAX_CONTROL_IN_FLIGHT: u16 = 64;
/// Maximum size of one opaque idempotency key.
pub const MAX_IDEMPOTENCY_KEY_BYTES: usize = 128;
/// Maximum size of one transport-neutral causal identity.
pub const MAX_CONTROL_ID_BYTES: usize = 128;
/// Maximum runs accepted by one analysis request.
pub const MAX_ANALYSIS_RUNS: usize = 256;
/// Maximum string exposed through the ordinary public control API.
pub const MAX_PUBLIC_STRING_BYTES: usize = 4096;
/// Maximum recursive JSON nodes exposed in one public result.
pub const MAX_PUBLIC_JSON_NODES: usize = 4096;
/// Maximum recursive JSON nesting exposed in one public result.
pub const MAX_PUBLIC_JSON_DEPTH: usize = 16;
/// Maximum encoded measurement-catalog publication returned through control v1.
///
/// This leaves at least 64 KiB for the JSON-RPC envelope under the default frame limit.
pub const MAX_MEASUREMENT_CATALOG_PUBLICATION_BYTES: usize = 192 * 1024;

/// Stable control-protocol application errors.
pub mod error_code {
    /// The client must negotiate before other calls.
    pub const NEGOTIATION_REQUIRED: i32 = -33_001;
    /// No compatible protocol version exists.
    pub const INCOMPATIBLE_VERSION: i32 = -33_002;
    /// A request or response exceeds a negotiated resource bound.
    pub const RESOURCE_EXHAUSTED: i32 = -33_003;
    /// The request deadline expired.
    pub const DEADLINE_EXCEEDED: i32 = -33_004;
    /// A causal run or attempt identity is stale.
    pub const STALE_IDENTITY: i32 = -33_005;
    /// A reconnect cursor is outside retained runner history.
    pub const STALE_CURSOR: i32 = -33_006;
    /// The peer is not authorized.
    pub const UNAUTHORIZED: i32 = -33_007;
    /// An idempotency key was reused for a different mutation.
    pub const IDEMPOTENCY_CONFLICT: i32 = -33_008;
    /// The requested capability is unavailable.
    pub const CAPABILITY_UNAVAILABLE: i32 = -33_009;
}

/// A major/minor protocol version.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(deny_unknown_fields)]
pub struct ControlVersion {
    /// Breaking protocol generation.
    pub major: u16,
    /// Backward-compatible feature generation.
    pub minor: u16,
}

/// Negotiated per-connection bounds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlLimits {
    /// Largest four-byte-length-prefixed JSON body.
    pub max_frame_bytes: u32,
    /// Longest request processing deadline measured from receipt.
    pub max_timeout_ms: u64,
    /// Largest page a client may request.
    pub max_page_items: u16,
    /// Largest number of admitted requests awaiting terminal responses.
    pub max_in_flight: u16,
}

impl ControlLimits {
    /// Validate all fields against implementation ceilings.
    pub fn validate(self) -> Result<Self, ProtocolError> {
        if self.max_frame_bytes == 0 || self.max_frame_bytes > MAX_CONTROL_FRAME_BYTES {
            return Err(ProtocolError::InvalidLimit("max_frame_bytes"));
        }
        if self.max_timeout_ms == 0 || self.max_timeout_ms > MAX_CONTROL_TIMEOUT_MS {
            return Err(ProtocolError::InvalidLimit("max_timeout_ms"));
        }
        if self.max_page_items == 0 || self.max_page_items > MAX_PAGE_ITEMS {
            return Err(ProtocolError::InvalidLimit("max_page_items"));
        }
        if self.max_in_flight == 0 || self.max_in_flight > MAX_CONTROL_IN_FLIGHT {
            return Err(ProtocolError::InvalidLimit("max_in_flight"));
        }
        Ok(self)
    }

    /// Select the stricter bound offered by either peer.
    pub fn intersect(self, peer: Self) -> Result<Self, ProtocolError> {
        self.validate()?;
        peer.validate()?;
        Ok(Self {
            max_frame_bytes: self.max_frame_bytes.min(peer.max_frame_bytes),
            max_timeout_ms: self.max_timeout_ms.min(peer.max_timeout_ms),
            max_page_items: self.max_page_items.min(peer.max_page_items),
            max_in_flight: self.max_in_flight.min(peer.max_in_flight),
        })
    }
}

impl Default for ControlLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 256 * 1024,
            max_timeout_ms: 30_000,
            max_page_items: 100,
            max_in_flight: 16,
        }
    }
}

/// Numeric JSON-RPC request identity.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct RequestId(pub u64);

/// Opaque durable revision used to resume event streams.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct Revision(pub u64);

/// Opaque stable run identity.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RunId(pub String);

/// Opaque stable execution-attempt identity.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct AttemptId(pub String);

/// JSON-RPC request accepted from a frontend.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    /// Must equal `2.0`.
    pub jsonrpc: String,
    /// Unique outstanding request identity on this connection.
    pub id: RequestId,
    /// Deadline relative to server receipt.
    pub timeout_ms: u64,
    /// Typed operation and parameters.
    #[serde(flatten)]
    pub call: ControlCall,
}

/// Typed frontend operations.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum ControlCall {
    /// Negotiate protocol version and resource bounds.
    Negotiate(NegotiateParams),
    /// Obtain runner and transport capabilities.
    Capabilities,
    /// Obtain the immutable catalog of selectable measurements.
    MeasurementCatalog,
    /// Validate settings without creating durable run state.
    ValidateSettings {
        /// Candidate settings document.
        settings: Value,
    },
    /// Validate and durably create a content-pinned plan.
    CreatePlan(MutationParams),
    /// Launch one previously created plan.
    Launch(LaunchParams),
    /// Read authoritative current run state.
    Status {
        /// Durable run to inspect.
        run_id: RunId,
    },
    /// Request cancellation of an exact attempt.
    Cancel(CancelParams),
    /// Page through recent durable runs.
    History(PageParams),
    /// Create a new plan from a completed run.
    Repeat(RepeatParams),
    /// Analyze a bounded set of durable runs.
    Analyze {
        /// Bounded set of durable runs to compare.
        run_ids: Vec<RunId>,
    },
    /// Resume public runner events after a durable revision.
    Events(PageParams),
    /// Obtain metadata for one sensitive artifact; content needs separate authorization.
    ArtifactMetadata {
        /// Durable run owning the artifact reference.
        run_id: RunId,
        /// Exact content digest from the runner journal.
        digest: String,
    },
}

impl ControlCall {
    /// Earliest exact wire version that defines this operation.
    #[must_use]
    pub const fn minimum_version(&self) -> ControlVersion {
        match self {
            Self::MeasurementCatalog => CONTROL_MEASUREMENT_CATALOG_V1,
            _ => CONTROL_V1,
        }
    }
}

/// Initial negotiation offer.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NegotiateParams {
    /// Versions supported by the client.
    pub versions: BTreeSet<ControlVersion>,
    /// Client-side bounds.
    pub limits: ControlLimits,
}

/// Generic idempotent mutation input.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MutationParams {
    /// Opaque key retained by the runner to make retries safe.
    pub idempotency_key: String,
    /// Content-pinned validated definition.
    pub definition: Value,
}

/// Idempotent launch input.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchParams {
    /// Opaque key retained by the runner to make retries safe.
    pub idempotency_key: String,
    /// Previously created durable plan identity.
    pub plan_id: String,
}

/// Causally fenced cancellation input.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CancelParams {
    /// Run to cancel.
    pub run_id: RunId,
    /// Exact attempt expected by the frontend.
    pub attempt_id: AttemptId,
    /// Opaque key retained by the runner to make retries safe.
    pub idempotency_key: String,
}

/// Repetition input bound to an immutable source run.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepeatParams {
    /// Immutable source run.
    pub run_id: RunId,
    /// Opaque key retained by the runner to make retries safe.
    pub idempotency_key: String,
}

/// Bounded cursor page request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PageParams {
    /// Last revision already durably observed, or none for the retained beginning.
    pub after: Option<Revision>,
    /// Maximum requested records.
    pub limit: u16,
}

/// JSON-RPC terminal response.
#[derive(Clone, Debug, JsonSchema, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ControlResponse {
    /// Successful typed response.
    Success(ControlSuccessResponse),
    /// Fixed privacy-safe failure response.
    Failure(ControlFailureResponse),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawResponseEnvelope {
    jsonrpc: String,
    id: RequestId,
    #[serde(default)]
    result: RawField,
    #[serde(default)]
    error: RawField,
}

#[derive(Default)]
enum RawField {
    #[default]
    Missing,
    Present(Box<RawValue>),
}

impl<'de> Deserialize<'de> for RawField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Box::<RawValue>::deserialize(deserializer).map(Self::Present)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTaggedValue {
    kind: String,
    value: Box<RawValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBoundResult {
    request_sha256: String,
    result: Box<RawValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalogPublication {
    version: ControlVersion,
    freshness: MeasurementCatalogFreshness,
    source: MeasurementCatalogPublicationSource,
    catalog: Box<RawValue>,
}

fn decode_raw_response(raw: &str) -> Result<ControlResponse, serde_json::Error> {
    let envelope: RawResponseEnvelope = serde_json::from_str(raw)?;
    let result = match (envelope.result, envelope.error) {
        (RawField::Present(result), RawField::Missing) => result,
        (RawField::Missing, RawField::Present(_)) => {
            return serde_json::from_str::<ControlFailureResponse>(raw)
                .map(ControlResponse::Failure);
        }
        _ => {
            return Err(<serde_json::Error as de::Error>::custom(
                "response must contain exactly one terminal outcome",
            ));
        }
    };
    let success: RawTaggedValue = serde_json::from_str(result.get())?;
    if success.kind != "operation" {
        return serde_json::from_str::<ControlSuccessResponse>(raw).map(ControlResponse::Success);
    }
    let bound: RawBoundResult = serde_json::from_str(success.value.get())?;
    let result: RawTaggedValue = serde_json::from_str(bound.result.get())?;
    if result.kind != "measurement_catalog" {
        return serde_json::from_str::<ControlSuccessResponse>(raw).map(ControlResponse::Success);
    }
    if result.value.get().len() > MAX_MEASUREMENT_CATALOG_PUBLICATION_BYTES {
        return Err(<serde_json::Error as de::Error>::custom(
            "measurement catalog publication exceeds encoded size bound",
        ));
    }
    let publication: RawCatalogPublication = serde_json::from_str(result.value.get())?;
    let catalog = MeasurementCatalogV1::from_slice_bounded(publication.catalog.get().as_bytes())
        .map_err(|error| <serde_json::Error as de::Error>::custom(error.to_string()))?;
    Ok(ControlResponse::Success(ControlSuccessResponse {
        jsonrpc: envelope.jsonrpc,
        id: envelope.id,
        result: ControlSuccess::Operation(BoundControlResult {
            request_sha256: bound.request_sha256,
            result: ControlResult::MeasurementCatalog(MeasurementCatalogPublication {
                version: publication.version,
                freshness: publication.freshness,
                source: publication.source,
                catalog: ControlMeasurementCatalog(catalog),
            }),
        }),
    }))
}

impl<'de> Deserialize<'de> for ControlResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = <&'de RawValue>::deserialize(deserializer)?;
        decode_raw_response(raw.get()).map_err(de::Error::custom)
    }
}

/// Closed successful response envelope.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlSuccessResponse {
    /// Must equal `2.0`.
    jsonrpc: String,
    /// Matching request identity.
    id: RequestId,
    /// Closed successful result.
    result: ControlSuccess,
}

/// Closed failed response envelope.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlFailureResponse {
    /// Must equal `2.0`.
    jsonrpc: String,
    /// Matching request identity.
    id: RequestId,
    /// Fixed privacy-safe error.
    error: ControlRpcError,
}

impl ControlResponse {
    /// Construct a successful terminal response.
    #[must_use]
    pub fn success(id: RequestId, result: ControlSuccess) -> Self {
        Self::Success(ControlSuccessResponse {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            result,
        })
    }

    /// Construct a failed terminal response without exposing private details.
    #[must_use]
    pub fn failure(id: RequestId, code: i32, message: impl Into<String>) -> Self {
        Self::Failure(ControlFailureResponse {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            error: ControlRpcError {
                code,
                message: message.into(),
            },
        })
    }

    /// Matching request identity.
    #[must_use]
    pub fn id(&self) -> RequestId {
        match self {
            Self::Success(value) => value.id,
            Self::Failure(value) => value.id,
        }
    }

    /// Borrow a successful result.
    #[must_use]
    pub fn result(&self) -> Option<&ControlSuccess> {
        match self {
            Self::Success(value) => Some(&value.result),
            Self::Failure(_) => None,
        }
    }

    /// Borrow a fixed failure.
    #[must_use]
    pub fn error(&self) -> Option<&ControlRpcError> {
        match self {
            Self::Success(_) => None,
            Self::Failure(value) => Some(&value.error),
        }
    }

    /// Consume a successful result.
    #[must_use]
    pub fn into_result(self) -> Option<ControlSuccess> {
        match self {
            Self::Success(value) => Some(value.result),
            Self::Failure(_) => None,
        }
    }

    /// Validate the discriminator and closed result/error body.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        match self {
            Self::Success(value) => {
                validate_jsonrpc(&value.jsonrpc)?;
                value.result.validate(absolute_limits())
            }
            Self::Failure(value) => {
                validate_jsonrpc(&value.jsonrpc)?;
                value.error.validate()
            }
        }
    }
}

/// Privacy-safe JSON-RPC error.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRpcError {
    /// Stable machine-readable code.
    pub code: i32,
    /// Bounded public diagnostic.
    pub message: String,
}

impl ControlRpcError {
    fn validate(&self) -> Result<(), ProtocolError> {
        validate_public_string(&self.message)?;
        let valid = matches!(
            (self.code, self.message.as_str()),
            (
                error_code::NEGOTIATION_REQUIRED,
                "protocol negotiation is required"
            ) | (
                error_code::INCOMPATIBLE_VERSION,
                "no compatible protocol version"
            ) | (
                error_code::RESOURCE_EXHAUSTED,
                "request rejected by runner policy"
            ) | (
                error_code::DEADLINE_EXCEEDED,
                "control request deadline exceeded"
            ) | (error_code::STALE_IDENTITY, "durable object not found")
                | (error_code::STALE_IDENTITY, "causal identity is stale")
                | (error_code::STALE_CURSOR, "reconnect cursor is stale")
                | (error_code::UNAUTHORIZED, "control request is unauthorized")
                | (
                    error_code::IDEMPOTENCY_CONFLICT,
                    "runner reconciliation is required"
                )
                | (
                    error_code::CAPABILITY_UNAVAILABLE,
                    "requested capability is unavailable"
                )
        );
        if valid {
            Ok(())
        } else {
            Err(ProtocolError::InvalidResponse)
        }
    }
}

/// Negotiated connection result.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Negotiated {
    /// Selected protocol version.
    pub version: ControlVersion,
    /// Effective bounds.
    pub limits: ControlLimits,
    /// Stable runner instance identity.
    pub runner_instance_id: String,
    /// Oldest retained public event revision.
    pub oldest_revision: Revision,
    /// Latest retained public event revision.
    pub latest_revision: Revision,
}

/// Closed wire-visible success envelope.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ControlSuccess {
    /// Initial protocol negotiation.
    Negotiated(Negotiated),
    /// Result of one post-negotiation operation.
    Operation(BoundControlResult),
}

impl ControlSuccess {
    /// Validate the selected bounds or typed operation result.
    pub fn validate(&self, limits: ControlLimits) -> Result<(), ProtocolError> {
        match self {
            Self::Negotiated(value) => {
                value.limits.validate()?;
                validate_identity(&value.runner_instance_id)?;
                if value.oldest_revision > value.latest_revision {
                    return Err(ProtocolError::InvalidResponse);
                }
                Ok(())
            }
            Self::Operation(value) => {
                validate_digest(&value.request_sha256)?;
                value.result.validate(limits)
            }
        }
    }
}

/// Public run lifecycle; absence is never encoded as a successful state.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PublicRunState {
    /// Definition is durable but execution has not started.
    Planned,
    /// Inputs are prepared.
    Prepared,
    /// Execution is active.
    Running,
    /// Evidence collection is active.
    Collecting,
    /// All required evidence is durable.
    Completed,
    /// Execution or collection failed.
    Failed,
    /// Cancellation reached a durable terminal state.
    Cancelled,
    /// External effects require runner-side reconciliation.
    NeedsReconciliation,
}

/// Privacy-reviewed run summary.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunSummary {
    /// Stable run identity.
    pub run_id: RunId,
    /// Exact attempt identity.
    pub attempt_id: AttemptId,
    /// State derived from the authoritative journal.
    pub state: PublicRunState,
    /// Immutable revision at which this run first entered the journal.
    pub created_revision: Revision,
    /// Durable revision from which this summary was derived.
    pub revision: Revision,
    /// Content digest of the immutable plan.
    pub plan_sha256: String,
}

/// Explicit availability of a public history field.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceAvailability {
    /// The field is present and independently verified.
    Available,
    /// The field could not be obtained; the reason is recorded separately.
    Unavailable,
}

/// Result-integrity state exposed by the history extension.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultIntegrity {
    /// All required public result records are verified.
    Verified,
    /// Some required evidence is absent.
    Incomplete,
    /// Evidence was present but failed integrity checks.
    Invalid,
}

/// Durable public outcome used by the history extension.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableOutcome {
    /// Required evidence was collected successfully.
    Completed,
    /// Execution or evidence collection failed durably.
    Failed,
    /// Cancellation reached a durable terminal state.
    Cancelled,
    /// Outcome is unavailable and must not be rendered as success or zero.
    Unavailable,
}

/// Bounded provenance and outcome evidence for one history item.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEvidence {
    /// RFC3339 UTC creation time, never inferred from list order.
    pub created_at: String,
    /// Public agent identity, or an explicit unavailable marker.
    pub agent_id: String,
    /// Public provider source/profile identity, never a credential.
    pub provider_source: String,
    /// Immutable workload identity and revision.
    pub workload_id: String,
    /// Workload content/scorer revision.
    pub workload_revision: String,
    /// Public platform identity, never a host path or hostname.
    pub platform_id: String,
    /// Whether all required result evidence is intact.
    pub result_integrity: ResultIntegrity,
    /// Durable terminal outcome.
    pub outcome: DurableOutcome,
    /// Explicit reason codes for unavailable fields/evidence.
    pub unavailable_reasons: Vec<SettingsIssue>,
}

/// A typed analysis confounder; values are identities, not private details.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisConfounder {
    /// Stable confounder category.
    pub kind: String,
    /// Public bounded explanation.
    pub description: String,
}

/// Compatibility decision supplied by the experiment-comparability contract.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisCompatibility {
    /// All required identities and controls match.
    Comparable,
    /// A typed confounder or missing evidence prevents comparison.
    NotComparable,
    /// Compatibility evidence is unavailable.
    Unavailable,
}

/// Machine-checked analysis evidence bound to exact runs and revisions.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisEvidence {
    /// Exact run identities included in the analysis.
    pub run_ids: Vec<RunId>,
    /// Exact durable revisions paired with `run_ids`.
    pub revisions: Vec<Revision>,
    /// AR-1001 compatibility decision.
    pub compatibility: AnalysisCompatibility,
    /// Every exclusion/confounder retained deterministically.
    pub confounders: Vec<AnalysisConfounder>,
    /// Metric identity, or `unavailable` when no metric exists.
    pub metric: String,
    /// Scoring identity, or `unavailable` when no scorer exists.
    pub scoring: String,
    /// Whether uncertainty was computed.
    pub uncertainty_available: bool,
    /// Aggregate result-integrity state.
    pub result_integrity: ResultIntegrity,
    /// Digest of sensitive detailed analysis, if one exists.
    pub detail_sha256: Option<String>,
}

impl HistoryEvidence {
    /// Validate bounded public fields and explicit unavailable semantics.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.created_at.len() < 20
            || self.created_at.len() > MAX_PUBLIC_STRING_BYTES
            || !self.created_at.ends_with('Z')
        {
            return Err(ProtocolError::UnsafePublicValue);
        }
        for value in [
            &self.agent_id,
            &self.provider_source,
            &self.workload_id,
            &self.workload_revision,
            &self.platform_id,
        ] {
            validate_public_string(value)?;
        }
        if self.unavailable_reasons.len() > MAX_PAGE_ITEMS as usize
            || (self.outcome == DurableOutcome::Unavailable && self.unavailable_reasons.is_empty())
            || (self.result_integrity != ResultIntegrity::Verified
                && self.unavailable_reasons.is_empty())
        {
            return Err(ProtocolError::InvalidResponse);
        }
        Ok(())
    }
}

impl AnalysisEvidence {
    /// Validate exact-run binding, confounder completeness, and digest bounds.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.run_ids.is_empty()
            || self.run_ids.len() > MAX_ANALYSIS_RUNS
            || self.run_ids.len() != self.revisions.len()
            || self.run_ids.iter().collect::<BTreeSet<_>>().len() != self.run_ids.len()
            || self.confounders.len() > MAX_ANALYSIS_RUNS
        {
            return Err(ProtocolError::InvalidAnalysisSet);
        }
        for run_id in &self.run_ids {
            validate_identity(&run_id.0)?;
        }
        validate_public_string(&self.metric)?;
        validate_public_string(&self.scoring)?;
        let mut kinds = BTreeSet::new();
        for confounder in &self.confounders {
            validate_identity(&confounder.kind)?;
            validate_public_string(&confounder.description)?;
            if !kinds.insert(&confounder.kind) {
                return Err(ProtocolError::InvalidResponse);
            }
        }
        if self.compatibility == AnalysisCompatibility::Comparable
            && (!self.confounders.is_empty()
                || self.result_integrity != ResultIntegrity::Verified
                || !self.uncertainty_available)
        {
            return Err(ProtocolError::InvalidResponse);
        }
        if self.compatibility != AnalysisCompatibility::Comparable && self.confounders.is_empty() {
            return Err(ProtocolError::InvalidResponse);
        }
        if let Some(digest) = &self.detail_sha256 {
            validate_digest(digest)?;
        }
        Ok(())
    }
}

/// Privacy-reviewed public runner event.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlEvent {
    /// Durable contiguous revision.
    pub revision: Revision,
    /// Event kind from the stable public vocabulary.
    pub kind: ControlEventKind,
    /// Related run when applicable.
    pub run_id: Option<RunId>,
    /// Related exact attempt when applicable.
    pub attempt_id: Option<AttemptId>,
}

/// Stable privacy-safe runner event vocabulary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlEventKind {
    /// Runner instance is ready after startup or recovery.
    RunnerReady,
    /// A validated plan became durable.
    PlanCreated,
    /// An exact execution attempt started.
    RunStarted,
    /// Durable public run state advanced.
    RunUpdated,
    /// Required run evidence became durable.
    RunCompleted,
    /// The run reached a durable failure.
    RunFailed,
    /// Cancellation reached a durable terminal state.
    RunCancelled,
    /// The runner requires reconciliation before another effect.
    ReconciliationRequired,
}

/// Bounded event or history page.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Page<T> {
    /// Ordered page records.
    pub items: Vec<T>,
    /// Cursor representing the final returned record.
    pub next: Option<Revision>,
    /// Whether a later retained record exists.
    pub has_more: bool,
}

/// Artifact sensitivity is explicit and content is never embedded in summaries.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactSensitivity {
    /// Safe to expose through the ordinary control API.
    Public,
    /// Requires a separate least-privilege content authorization.
    Sensitive,
}

/// Privacy-safe artifact metadata.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMetadata {
    /// Content digest, never a host path.
    pub sha256: String,
    /// Byte length.
    pub size_bytes: u64,
    /// Explicit sensitivity class.
    pub sensitivity: ArtifactSensitivity,
}

/// Runner features exposed without embedding configuration or host details.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    /// Settings can be validated without creating a plan.
    pub validate_settings: bool,
    /// Plans can be created and launched.
    pub run_control: bool,
    /// Completed runs can be repeated.
    pub repeat: bool,
    /// Multiple runs can be analysed.
    pub analysis: bool,
    /// Public event cursors are retained.
    pub events: bool,
}

/// Explicit freshness semantics for a published measurement catalog.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementCatalogFreshness {
    /// The publication is immutable and freshness is established by its content digest.
    ContentAddressed,
}

/// Closed provenance for the catalog exposed by the runner.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementCatalogPublicationSource {
    /// The catalog is compiled from ASB's qualified built-in collector inventory.
    BuiltInCollectors,
}

/// A catalog decoded only through the bounded AR-1013 constructor.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ControlMeasurementCatalog(pub MeasurementCatalogV1);

impl<'de> Deserialize<'de> for ControlMeasurementCatalog {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = <&'de RawValue>::deserialize(deserializer)?;
        let bytes = raw.get().as_bytes();
        if bytes.len() > MAX_MEASUREMENT_CATALOG_WIRE_BYTES {
            return Err(de::Error::custom(
                "measurement catalog exceeds its wire bound",
            ));
        }
        MeasurementCatalogV1::from_slice_bounded(bytes)
            .map(Self)
            .map_err(de::Error::custom)
    }
}

/// Versioned, bounded publication returned by `measurement_catalog`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(transform = measurement_catalog_publication_schema)]
#[serde(deny_unknown_fields)]
pub struct MeasurementCatalogPublication {
    /// Additive control extension version defining this result.
    pub version: ControlVersion,
    /// How consumers determine whether this immutable catalog changed.
    pub freshness: MeasurementCatalogFreshness,
    /// Closed authority that produced the catalog.
    pub source: MeasurementCatalogPublicationSource,
    /// Canonical AR-1013 catalog and its content digest.
    pub catalog: ControlMeasurementCatalog,
}

fn measurement_catalog_publication_schema(schema: &mut schemars::Schema) {
    schema.ensure_object().insert(
        "allOf".into(),
        serde_json::json!([{
            "properties": {
                "version": {
                    "properties": {
                        "major": {"const": 1},
                        "minor": {"const": 2}
                    }
                }
            }
        }]),
    );
}

impl MeasurementCatalogPublication {
    /// Construct the authoritative built-in publication.
    #[must_use]
    pub fn built_in(catalog: MeasurementCatalogV1) -> Self {
        Self {
            version: CONTROL_MEASUREMENT_CATALOG_V1,
            freshness: MeasurementCatalogFreshness::ContentAddressed,
            source: MeasurementCatalogPublicationSource::BuiltInCollectors,
            catalog: ControlMeasurementCatalog(catalog),
        }
    }

    /// Validate version, catalog semantics, digest, privacy, and result size.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.version != CONTROL_MEASUREMENT_CATALOG_V1 {
            return Err(ProtocolError::InvalidResponse);
        }
        self.catalog
            .0
            .validate()
            .map_err(|_| ProtocolError::InvalidResponse)?;
        let bytes = serde_json::to_vec(self).map_err(|_| ProtocolError::InvalidResponse)?;
        if bytes.len() > MAX_MEASUREMENT_CATALOG_PUBLICATION_BYTES {
            return Err(ProtocolError::UnsafePublicValue);
        }
        Ok(())
    }
}

/// Stable, non-sensitive settings diagnostic.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SettingsIssue {
    /// The settings shape or syntax is invalid.
    InvalidFormat,
    /// A requested capability is unavailable.
    UnsupportedCapability,
    /// A referenced immutable component cannot be verified.
    UnverifiedComponent,
    /// A configured resource bound is invalid.
    InvalidResourceBound,
    /// The measurement selection schema generation is unsupported.
    MeasurementUnsupportedSchemaVersion,
    /// The measurement catalog generation does not match.
    MeasurementCatalogGenerationMismatch,
    /// The measurement catalog content address is stale or malformed.
    MeasurementCatalogDigestMismatch,
    /// The measurement selection content address is stale or malformed.
    MeasurementSelectionDigestMismatch,
    /// The measurement selection exceeds its identity bound.
    MeasurementTooMany,
    /// Measurement identities are not in canonical order.
    MeasurementNonCanonicalOrder,
    /// A measurement identity occurs more than once.
    MeasurementDuplicateId,
    /// A measurement identity is unknown.
    MeasurementUnknownId,
    /// A selected measurement source is not qualified.
    MeasurementSourceUnqualified,
    /// A selected measurement does not support the execution mode.
    MeasurementModeUnsupported,
    /// A selected measurement is not supported on the target platform.
    MeasurementPlatformUnsupported,
    /// A selected measurement requires unavailable permission.
    MeasurementPermissionRequired,
    /// The runner has no authoritative target for a selected measurement.
    MeasurementTargetScopeUnavailable,
    /// Measurement cadence presence is inconsistent with the selected set.
    MeasurementInvalidCadence,
    /// A selected measurement does not support the requested cadence.
    MeasurementCadenceTooFast,
    /// The requested measurement schedule exceeds its fixed capacity.
    MeasurementCadenceCapacityExceeded,
}

impl SettingsIssue {
    /// Exact stable settings category for a measurement-selection validation reason.
    #[must_use]
    pub const fn from_measurement_reason(reason: MeasurementSelectionReason) -> Self {
        match reason {
            MeasurementSelectionReason::UnsupportedSchemaVersion => {
                Self::MeasurementUnsupportedSchemaVersion
            }
            MeasurementSelectionReason::CatalogGenerationMismatch => {
                Self::MeasurementCatalogGenerationMismatch
            }
            MeasurementSelectionReason::CatalogDigestMismatch => {
                Self::MeasurementCatalogDigestMismatch
            }
            MeasurementSelectionReason::SelectionDigestMismatch => {
                Self::MeasurementSelectionDigestMismatch
            }
            MeasurementSelectionReason::TooManyMeasurements => Self::MeasurementTooMany,
            MeasurementSelectionReason::NonCanonicalOrder => Self::MeasurementNonCanonicalOrder,
            MeasurementSelectionReason::DuplicateId => Self::MeasurementDuplicateId,
            MeasurementSelectionReason::UnknownId => Self::MeasurementUnknownId,
            MeasurementSelectionReason::SourceUnqualified => Self::MeasurementSourceUnqualified,
            MeasurementSelectionReason::ModeUnsupported => Self::MeasurementModeUnsupported,
            MeasurementSelectionReason::PlatformUnsupported => Self::MeasurementPlatformUnsupported,
            MeasurementSelectionReason::PermissionRequired => Self::MeasurementPermissionRequired,
            MeasurementSelectionReason::TargetScopeUnavailable => {
                Self::MeasurementTargetScopeUnavailable
            }
            MeasurementSelectionReason::InvalidCadence => Self::MeasurementInvalidCadence,
            MeasurementSelectionReason::CadenceTooFast => Self::MeasurementCadenceTooFast,
            MeasurementSelectionReason::CadenceCapacityExceeded => {
                Self::MeasurementCadenceCapacityExceeded
            }
            MeasurementSelectionReason::WireTooLarge
            | MeasurementSelectionReason::InvalidWire
            | MeasurementSelectionReason::Serialization => Self::InvalidFormat,
        }
    }

    /// Backward-compatible category used when projecting a v1.3 issue to v1.0 or v1.2.
    #[must_use]
    pub const fn legacy_projection(self) -> Self {
        match self {
            Self::MeasurementUnsupportedSchemaVersion
            | Self::MeasurementCatalogGenerationMismatch
            | Self::MeasurementCatalogDigestMismatch
            | Self::MeasurementSelectionDigestMismatch
            | Self::MeasurementTooMany
            | Self::MeasurementNonCanonicalOrder
            | Self::MeasurementDuplicateId
            | Self::MeasurementUnknownId
            | Self::MeasurementSourceUnqualified
            | Self::MeasurementModeUnsupported
            | Self::MeasurementPlatformUnsupported
            | Self::MeasurementPermissionRequired
            | Self::MeasurementTargetScopeUnavailable
            | Self::MeasurementInvalidCadence
            | Self::MeasurementCadenceTooFast
            | Self::MeasurementCadenceCapacityExceeded => Self::InvalidFormat,
            legacy => legacy,
        }
    }

    const fn requires_measurement_catalog_v1(self) -> bool {
        !matches!(
            self,
            Self::InvalidFormat
                | Self::UnsupportedCapability
                | Self::UnverifiedComponent
                | Self::InvalidResourceBound
        )
    }
}

/// Exact privacy-safe measurement selection diagnostic returned only by control v1.3.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementSettingsIssue {
    /// Stable semantic failure reason.
    pub reason: MeasurementSelectionReason,
    /// Catalog-owned identity for an ID-specific failure; never copied for an unknown ID.
    #[schemars(length(max = 256))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl MeasurementSettingsIssue {
    fn validate(&self) -> Result<(), ProtocolError> {
        let requires_catalog_id = matches!(
            self.reason,
            MeasurementSelectionReason::SourceUnqualified
                | MeasurementSelectionReason::ModeUnsupported
                | MeasurementSelectionReason::PlatformUnsupported
                | MeasurementSelectionReason::PermissionRequired
                | MeasurementSelectionReason::TargetScopeUnavailable
                | MeasurementSelectionReason::CadenceTooFast
        );
        if requires_catalog_id != self.id.is_some() {
            return Err(ProtocolError::InvalidResponse);
        }
        let Some(id) = self.id.as_deref() else {
            return Ok(());
        };
        validate_identity(id)?;
        if baseline_measurement_catalog()
            .measurements
            .binary_search_by(|candidate| candidate.id.as_str().cmp(id))
            .is_err()
        {
            return Err(ProtocolError::UnsafePublicValue);
        }
        Ok(())
    }
}

/// Privacy-safe settings validation outcome.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsValidation {
    /// Whether plan creation may proceed.
    pub valid: bool,
    /// Bounded stable issue vocabulary, without copied input values.
    pub issues: Vec<SettingsIssue>,
    /// Exact first measurement failure when validation reached the bound selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement_issue: Option<MeasurementSettingsIssue>,
}

/// Durable immutable plan identity.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanReference {
    /// Opaque stable plan identity.
    pub plan_id: String,
    /// SHA-256 of the canonical plan.
    pub plan_sha256: String,
}

/// Stable acknowledgement for an accepted durable mutation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MutationAcknowledgement {
    /// The mutation is durably accepted.
    pub accepted: bool,
}

/// Privacy-safe aggregate analysis projection.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisSummary {
    /// Number of runs included.
    pub run_count: u16,
    /// Digest of the canonical analysis result stored as a sensitive artifact.
    pub analysis_sha256: String,
}

/// Closed allowlist of successful results returned by runner backends.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ControlResult {
    /// Runner feature availability.
    Capabilities(Capabilities),
    /// Immutable selectable-measurement catalog.
    MeasurementCatalog(MeasurementCatalogPublication),
    /// Settings validation outcome.
    SettingsValidation(SettingsValidation),
    /// Created or repeated immutable plan.
    Plan(PlanReference),
    /// Launch returned its durable run identity.
    Launch(RunSummary),
    /// Current durable run state.
    Status(RunSummary),
    /// Cancellation or another mutation was durably accepted.
    Acknowledged(MutationAcknowledgement),
    /// Recent run page.
    History(Page<RunSummary>),
    /// Public event page.
    Events(Page<ControlEvent>),
    /// Aggregate analysis metadata.
    Analysis(AnalysisSummary),
    /// Artifact metadata without content or host paths.
    ArtifactMetadata(ArtifactMetadata),
}

/// Successful result bound to the complete canonical request.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BoundControlResult {
    /// SHA-256 of the canonical typed call.
    pub request_sha256: String,
    /// Closed result body.
    pub result: ControlResult,
}

impl BoundControlResult {
    /// Bind one result to exactly one request.
    pub fn new(call: &ControlCall, result: ControlResult) -> Result<Self, ProtocolError> {
        Ok(Self {
            request_sha256: canonical_call_sha256(call)?,
            result,
        })
    }

    /// Validate shape, causal parameters, and the complete request digest.
    pub fn validate_for_call(
        &self,
        call: &ControlCall,
        limits: ControlLimits,
    ) -> Result<(), ProtocolError> {
        validate_digest(&self.request_sha256)?;
        if self.request_sha256 != canonical_call_sha256(call)? {
            return Err(ProtocolError::InvalidResponse);
        }
        self.result.validate_for_call(call, limits)
    }

    /// Validate a result and reject shapes not defined by the negotiated version.
    pub fn validate_for_call_and_version(
        &self,
        call: &ControlCall,
        limits: ControlLimits,
        version: ControlVersion,
    ) -> Result<(), ProtocolError> {
        if !SUPPORTED_CONTROL_VERSIONS.contains(&version) || version < call.minimum_version() {
            return Err(ProtocolError::InvalidResponse);
        }
        self.validate_for_call(call, limits)?;
        match &self.result {
            ControlResult::MeasurementCatalog(_) if version < CONTROL_MEASUREMENT_CATALOG_V1 => {
                return Err(ProtocolError::InvalidResponse);
            }
            ControlResult::SettingsValidation(value)
                if version < CONTROL_MEASUREMENT_SELECTION_V1
                    && (value
                        .issues
                        .iter()
                        .any(|issue| issue.requires_measurement_catalog_v1())
                        || value.measurement_issue.is_some()) =>
            {
                return Err(ProtocolError::InvalidResponse);
            }
            _ => {}
        }
        Ok(())
    }
}

impl ControlResult {
    /// Validate identities, digests, pages, and bounded public collections.
    pub fn validate(&self, limits: ControlLimits) -> Result<(), ProtocolError> {
        fn run(value: &RunSummary) -> Result<(), ProtocolError> {
            validate_identity(&value.run_id.0)?;
            validate_identity(&value.attempt_id.0)?;
            validate_digest(&value.plan_sha256)
        }
        match self {
            Self::Capabilities(_) => Ok(()),
            Self::MeasurementCatalog(value) => value.validate(),
            Self::Acknowledged(value) => {
                if value.accepted {
                    Ok(())
                } else {
                    Err(ProtocolError::InvalidResponse)
                }
            }
            Self::SettingsValidation(value) => {
                if value.issues.len() > usize::from(limits.validate()?.max_page_items) {
                    Err(ProtocolError::UnsafePublicValue)
                } else if value.valid != value.issues.is_empty()
                    || value.issues.iter().collect::<BTreeSet<_>>().len() != value.issues.len()
                {
                    Err(ProtocolError::InvalidResponse)
                } else if let Some(detail) = &value.measurement_issue {
                    if value.valid
                        || value.issues != [SettingsIssue::from_measurement_reason(detail.reason)]
                    {
                        Err(ProtocolError::InvalidResponse)
                    } else {
                        detail.validate()
                    }
                } else {
                    Ok(())
                }
            }
            Self::Plan(value) => {
                validate_identity(&value.plan_id)?;
                validate_digest(&value.plan_sha256)
            }
            Self::Launch(value) | Self::Status(value) => {
                run(value)?;
                if value.created_revision > value.revision {
                    Err(ProtocolError::InvalidResponse)
                } else {
                    Ok(())
                }
            }
            Self::History(value) => {
                if value.items.len() > usize::from(limits.validate()?.max_page_items) {
                    return Err(ProtocolError::UnsafePublicValue);
                }
                value.items.iter().try_for_each(run)?;
                if value
                    .items
                    .iter()
                    .any(|item| item.created_revision > item.revision)
                {
                    return Err(ProtocolError::InvalidResponse);
                }
                validate_page_result(
                    value.items.iter().map(|item| item.created_revision),
                    value.next,
                    value.has_more,
                    false,
                )
            }
            Self::Events(value) => {
                if value.items.len() > usize::from(limits.validate()?.max_page_items) {
                    return Err(ProtocolError::UnsafePublicValue);
                }
                for item in &value.items {
                    if let Some(run_id) = &item.run_id {
                        validate_identity(&run_id.0)?;
                    }
                    if let Some(attempt_id) = &item.attempt_id {
                        validate_identity(&attempt_id.0)?;
                    }
                    let causal = item.run_id.is_some() && item.attempt_id.is_some();
                    let associated = match item.kind {
                        ControlEventKind::RunnerReady | ControlEventKind::PlanCreated => {
                            item.run_id.is_none() && item.attempt_id.is_none()
                        }
                        ControlEventKind::RunStarted
                        | ControlEventKind::RunUpdated
                        | ControlEventKind::RunCompleted
                        | ControlEventKind::RunFailed
                        | ControlEventKind::RunCancelled
                        | ControlEventKind::ReconciliationRequired => causal,
                    };
                    if !associated {
                        return Err(ProtocolError::InvalidResponse);
                    }
                }
                validate_page_result(
                    value.items.iter().map(|item| item.revision),
                    value.next,
                    value.has_more,
                    true,
                )
            }
            Self::Analysis(value) => {
                if value.run_count == 0 || usize::from(value.run_count) > MAX_ANALYSIS_RUNS {
                    return Err(ProtocolError::InvalidAnalysisSet);
                }
                validate_digest(&value.analysis_sha256)
            }
            Self::ArtifactMetadata(value) => validate_digest(&value.sha256),
        }
    }

    /// Require the result variant defined for a particular operation.
    #[must_use]
    pub fn matches_call(&self, call: &ControlCall) -> bool {
        matches!(
            (call, self),
            (ControlCall::Capabilities, Self::Capabilities(_))
                | (ControlCall::MeasurementCatalog, Self::MeasurementCatalog(_))
                | (
                    ControlCall::ValidateSettings { .. },
                    Self::SettingsValidation(_)
                )
                | (ControlCall::CreatePlan(_), Self::Plan(_))
                | (ControlCall::Launch(_), Self::Launch(_))
                | (ControlCall::Status { .. }, Self::Status(_))
                | (ControlCall::Cancel(_), Self::Acknowledged(_))
                | (ControlCall::History(_), Self::History(_))
                | (ControlCall::Repeat(_), Self::Plan(_))
                | (ControlCall::Analyze { .. }, Self::Analysis(_))
                | (ControlCall::Events(_), Self::Events(_))
                | (
                    ControlCall::ArtifactMetadata { .. },
                    Self::ArtifactMetadata(_)
                )
        )
    }

    /// Validate both the result shape and its causal relationship to a request.
    pub fn validate_for_call(
        &self,
        call: &ControlCall,
        limits: ControlLimits,
    ) -> Result<(), ProtocolError> {
        self.validate(limits)?;
        if !self.matches_call(call) {
            return Err(ProtocolError::InvalidResponse);
        }
        let page_matches =
            |items: usize, revisions: &[Revision], page: &PageParams, contiguous: bool| {
                items <= usize::from(page.limit)
                    && revisions
                        .iter()
                        .all(|revision| page.after.is_none_or(|after| *revision > after))
                    && (!contiguous
                        || revisions.first().is_none_or(|first| {
                            page.after.is_none_or(|after| {
                                after
                                    .0
                                    .checked_add(1)
                                    .is_some_and(|expected| first.0 == expected)
                            })
                        }))
            };
        let causally_matches = match (call, self) {
            (ControlCall::Status { run_id }, Self::Status(summary)) => summary.run_id == *run_id,
            (ControlCall::ArtifactMetadata { digest, .. }, Self::ArtifactMetadata(metadata)) => {
                metadata.sha256 == *digest
            }
            (ControlCall::History(page), Self::History(result)) => page_matches(
                result.items.len(),
                &result
                    .items
                    .iter()
                    .map(|item| item.created_revision)
                    .collect::<Vec<_>>(),
                page,
                false,
            ),
            (ControlCall::Events(page), Self::Events(result)) => page_matches(
                result.items.len(),
                &result
                    .items
                    .iter()
                    .map(|item| item.revision)
                    .collect::<Vec<_>>(),
                page,
                true,
            ),
            (ControlCall::Analyze { run_ids }, Self::Analysis(result)) => {
                usize::from(result.run_count) == run_ids.len()
            }
            _ => true,
        };
        if causally_matches {
            Ok(())
        } else {
            Err(ProtocolError::InvalidResponse)
        }
    }
}

fn canonical_call_sha256(call: &ControlCall) -> Result<String, ProtocolError> {
    let value = serde_json::to_value(call).map_err(|_| ProtocolError::InvalidResponse)?;
    let mut encoded = Vec::new();
    write_canonical_json(&mut encoded, &value)?;
    if encoded.len() > MAX_CONTROL_FRAME_BYTES as usize {
        return Err(ProtocolError::UnsafePublicValue);
    }
    let digest = Sha256::digest(encoded);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Emit the v1 call-digest representation: recursively UTF-8-byte-sorted object
/// members, preserved array order, no insignificant whitespace, and scalar
/// spelling from the pinned serde_json serializer.
fn write_canonical_json(output: &mut Vec<u8>, value: &Value) -> Result<(), ProtocolError> {
    match value {
        Value::Null => output.extend_from_slice(b"null"),
        Value::Bool(true) => output.extend_from_slice(b"true"),
        Value::Bool(false) => output.extend_from_slice(b"false"),
        Value::Number(number) => {
            serde_json::to_writer(output, number).map_err(|_| ProtocolError::InvalidResponse)?
        }
        Value::String(string) => {
            serde_json::to_writer(output, string).map_err(|_| ProtocolError::InvalidResponse)?
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, item) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_canonical_json(output, item)?;
            }
            output.push(b']');
        }
        Value::Object(object) => {
            output.push(b'{');
            let mut members = object.iter().collect::<Vec<_>>();
            members.sort_unstable_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
            for (index, (name, item)) in members.into_iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, name)
                    .map_err(|_| ProtocolError::InvalidResponse)?;
                output.push(b':');
                write_canonical_json(output, item)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

fn absolute_limits() -> ControlLimits {
    ControlLimits {
        max_frame_bytes: MAX_CONTROL_FRAME_BYTES,
        max_timeout_ms: MAX_CONTROL_TIMEOUT_MS,
        max_page_items: MAX_PAGE_ITEMS,
        max_in_flight: MAX_CONTROL_IN_FLIGHT,
    }
}

fn validate_page_result(
    revisions: impl Iterator<Item = Revision>,
    next: Option<Revision>,
    has_more: bool,
    contiguous: bool,
) -> Result<(), ProtocolError> {
    let mut previous = None;
    let mut last = None;
    for revision in revisions {
        if previous.is_some_and(|value| {
            revision <= value || (contiguous && revision.0 != value.0.saturating_add(1))
        }) {
            return Err(ProtocolError::InvalidResponse);
        }
        previous = Some(revision);
        last = Some(revision);
    }
    if next != last || (last.is_none() && has_more) {
        return Err(ProtocolError::InvalidResponse);
    }
    Ok(())
}

/// Protocol-shape validation failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ProtocolError {
    /// JSON-RPC discriminator is not supported.
    #[error("invalid JSON-RPC version")]
    InvalidJsonRpc,
    /// A negotiated bound is zero or above its ceiling.
    #[error("invalid control limit: {0}")]
    InvalidLimit(&'static str),
    /// Request timeout exceeds the negotiated bound.
    #[error("request timeout is outside the negotiated bound")]
    InvalidTimeout,
    /// Page is empty or exceeds the negotiated bound.
    #[error("page limit is outside the negotiated bound")]
    InvalidPage,
    /// Idempotency key is empty or too large.
    #[error("invalid idempotency key")]
    InvalidIdempotencyKey,
    /// A transport-neutral identity is empty, oversized, or unsafe.
    #[error("invalid control identity")]
    InvalidIdentity,
    /// A content digest is not lowercase SHA-256.
    #[error("invalid content digest")]
    InvalidDigest,
    /// Analysis input is empty or exceeds its run bound.
    #[error("invalid analysis run set")]
    InvalidAnalysisSet,
    /// Response does not contain exactly one terminal outcome.
    #[error("response must contain exactly one of result and error")]
    InvalidResponse,
    /// A public response is oversized or contains a sensitive field/value.
    #[error("response is not safe for the public control API")]
    UnsafePublicValue,
}

/// Validate a complete negotiation envelope against server limits.
pub fn validate_negotiation_request(
    request: &ControlRequest,
    server_limits: ControlLimits,
) -> Result<(), ProtocolError> {
    validate_jsonrpc(&request.jsonrpc)?;
    let server_limits = server_limits.validate()?;
    if request.timeout_ms == 0 || request.timeout_ms > server_limits.max_timeout_ms {
        return Err(ProtocolError::InvalidTimeout);
    }
    let ControlCall::Negotiate(offer) = &request.call else {
        return Err(ProtocolError::InvalidResponse);
    };
    if offer.versions.is_empty() {
        return Err(ProtocolError::InvalidLimit("versions"));
    }
    offer.limits.validate()?;
    Ok(())
}

/// Validate common envelope and negotiated request bounds.
pub fn validate_request(
    request: &ControlRequest,
    limits: ControlLimits,
) -> Result<(), ProtocolError> {
    validate_jsonrpc(&request.jsonrpc)?;
    let limits = limits.validate()?;
    if request.timeout_ms == 0 || request.timeout_ms > limits.max_timeout_ms {
        return Err(ProtocolError::InvalidTimeout);
    }
    match &request.call {
        ControlCall::History(page) | ControlCall::Events(page) => validate_page(*page, limits)?,
        ControlCall::CreatePlan(params) => validate_idempotency_key(&params.idempotency_key)?,
        ControlCall::Launch(params) => {
            validate_idempotency_key(&params.idempotency_key)?;
            validate_identity(&params.plan_id)?;
        }
        ControlCall::Status { run_id } => validate_identity(&run_id.0)?,
        ControlCall::Cancel(params) => {
            validate_idempotency_key(&params.idempotency_key)?;
            validate_identity(&params.run_id.0)?;
            validate_identity(&params.attempt_id.0)?;
        }
        ControlCall::Repeat(params) => {
            validate_idempotency_key(&params.idempotency_key)?;
            validate_identity(&params.run_id.0)?;
        }
        ControlCall::Analyze { run_ids } => {
            if run_ids.is_empty() || run_ids.len() > MAX_ANALYSIS_RUNS {
                return Err(ProtocolError::InvalidAnalysisSet);
            }
            for run_id in run_ids {
                validate_identity(&run_id.0)?;
            }
            if run_ids.iter().collect::<BTreeSet<_>>().len() != run_ids.len() {
                return Err(ProtocolError::InvalidAnalysisSet);
            }
        }
        ControlCall::ArtifactMetadata { run_id, digest } => {
            validate_identity(&run_id.0)?;
            validate_digest(digest)?;
        }
        _ => {}
    }
    Ok(())
}

/// Validate a page against effective bounds.
pub fn validate_page(page: PageParams, limits: ControlLimits) -> Result<(), ProtocolError> {
    if page.limit == 0 || page.limit > limits.validate()?.max_page_items {
        return Err(ProtocolError::InvalidPage);
    }
    Ok(())
}

/// Validate an opaque idempotency key without interpreting its contents.
pub fn validate_idempotency_key(key: &str) -> Result<(), ProtocolError> {
    if key.is_empty() || key.len() > MAX_IDEMPOTENCY_KEY_BYTES || !key.bytes().all(is_identity_byte)
    {
        return Err(ProtocolError::InvalidIdempotencyKey);
    }
    Ok(())
}

/// Validate a transport-neutral identity.
pub fn validate_identity(value: &str) -> Result<(), ProtocolError> {
    if value.is_empty()
        || value.len() > MAX_CONTROL_ID_BYTES
        || !value.bytes().all(is_identity_byte)
    {
        return Err(ProtocolError::InvalidIdentity);
    }
    Ok(())
}

/// Validate a lowercase SHA-256 digest.
pub fn validate_digest(value: &str) -> Result<(), ProtocolError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ProtocolError::InvalidDigest);
    }
    Ok(())
}

fn is_identity_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
}

fn validate_jsonrpc(value: &str) -> Result<(), ProtocolError> {
    if value == JSONRPC_VERSION {
        Ok(())
    } else {
        Err(ProtocolError::InvalidJsonRpc)
    }
}

/// Enforce the ordinary API's bounded, fail-closed privacy projection.
pub fn validate_public_value(value: &Value) -> Result<(), ProtocolError> {
    fn walk(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), ProtocolError> {
        *nodes = nodes
            .checked_add(1)
            .ok_or(ProtocolError::UnsafePublicValue)?;
        if depth > MAX_PUBLIC_JSON_DEPTH || *nodes > MAX_PUBLIC_JSON_NODES {
            return Err(ProtocolError::UnsafePublicValue);
        }
        match value {
            Value::String(value) => validate_public_string(value),
            Value::Array(values) => {
                for value in values {
                    walk(value, depth + 1, nodes)?;
                }
                Ok(())
            }
            Value::Object(values) => {
                for (key, value) in values {
                    if sensitive_public_key(key) {
                        return Err(ProtocolError::UnsafePublicValue);
                    }
                    walk(value, depth + 1, nodes)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    walk(value, 0, &mut 0)
}

fn validate_public_string(value: &str) -> Result<(), ProtocolError> {
    let lower = value.to_ascii_lowercase();
    if value.len() > MAX_PUBLIC_STRING_BYTES
        || value.starts_with('/')
        || value.contains("/home/")
        || value.contains("://")
        || value.contains(":\\Users\\")
        || value.contains("-----BEGIN OPENSSH PRIVATE KEY-----")
        || value.contains("-----BEGIN PRIVATE KEY-----")
        || [
            "password=",
            "passwd=",
            "token=",
            "secret=",
            "api_key=",
            "authorization: bearer ",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        Err(ProtocolError::UnsafePublicValue)
    } else {
        Ok(())
    }
}

fn sensitive_public_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    [
        "password",
        "passwd",
        "token",
        "secret",
        "credential",
        "apikey",
        "prompt",
        "response",
        "responsebody",
        "transcript",
        "body",
        "content",
        "stdout",
        "stderr",
        "log",
        "path",
        "hostpath",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}
