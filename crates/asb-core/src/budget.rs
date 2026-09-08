// SPDX-License-Identifier: MIT
//! Constructor-controlled execution budgets and uncertainty-aware accounting.

use std::collections::BTreeMap;

/// Largest accepted public evidence revision.
pub const MAX_BUDGET_REVISION_BYTES: usize = 4096;

/// One independently enforceable resource dimension.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum BudgetDimension {
    /// Absolute wall-clock duration.
    WallTime,
    /// Agent tool actions.
    Actions,
    /// Uncached provider input tokens.
    UncachedInputTokens,
    /// Cached provider input tokens.
    CachedInputTokens,
    /// Non-reasoning provider output tokens.
    OutputTokens,
    /// Provider reasoning output tokens.
    ReasoningTokens,
    /// Monetary cost in micros of the declared currency.
    Cost,
}

impl BudgetDimension {
    const ALL: [Self; 7] = [
        Self::WallTime,
        Self::Actions,
        Self::UncachedInputTokens,
        Self::CachedInputTokens,
        Self::OutputTokens,
        Self::ReasoningTokens,
        Self::Cost,
    ];
}

/// What an adapter proves about a dimension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Enforcement {
    /// The bound is reserved and enforced before effects.
    Enforced,
    /// Usage is observable only after effects.
    ObservedOnly,
    /// Trustworthy telemetry is unavailable.
    Unavailable,
}

/// Closed capability declaration for all dimensions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetCapabilities(BTreeMap<BudgetDimension, Enforcement>);

impl BudgetCapabilities {
    /// Declare every dimension exactly once.
    pub fn new(
        values: impl IntoIterator<Item = (BudgetDimension, Enforcement)>,
    ) -> Result<Self, BudgetError> {
        let values: BTreeMap<_, _> = values.into_iter().collect();
        if values.len() != BudgetDimension::ALL.len()
            || BudgetDimension::ALL
                .iter()
                .any(|item| !values.contains_key(item))
        {
            return Err(BudgetError::IncompleteCapabilities);
        }
        Ok(Self(values))
    }

    /// Conservative declaration for an opaque adapter.
    pub fn opaque() -> Self {
        Self(
            BudgetDimension::ALL
                .into_iter()
                .map(|d| (d, Enforcement::Unavailable))
                .collect(),
        )
    }

    /// Return one declared capability.
    pub fn get(&self, dimension: BudgetDimension) -> Enforcement {
        self.0[&dimension]
    }
}

/// Why trustworthy usage is absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnavailableReason {
    /// The provider or adapter does not expose the class.
    Unsupported,
    /// A supported telemetry stream omitted the value.
    MissingTelemetry,
    /// Conflicting or malformed telemetry was rejected.
    InvalidTelemetry,
}

/// Exact measurement or versioned bounded estimate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceKind {
    /// Directly measured or provider-reported.
    Measured,
    /// Estimated with an inclusive uncertainty interval.
    Estimated,
}

/// Constructor-controlled inclusive quantity interval.
///
/// External code cannot forge a measured or estimated classification:
///
/// ~~~compile_fail
/// use asb_core::{EvidenceKind, QuantityEvidence};
/// let _ = QuantityEvidence {
///     lower: 0,
///     upper: 1,
///     kind: EvidenceKind::Measured,
///     basis_revision: None,
/// };
/// ~~~
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuantityEvidence {
    lower: u64,
    upper: u64,
    kind: EvidenceKind,
    basis_revision: Option<String>,
}

impl QuantityEvidence {
    /// Construct exact measured evidence.
    pub const fn measured(value: u64) -> Self {
        Self {
            lower: value,
            upper: value,
            kind: EvidenceKind::Measured,
            basis_revision: None,
        }
    }

    /// Construct a versioned inclusive estimate.
    pub fn estimated(lower: u64, upper: u64, revision: &str) -> Result<Self, BudgetError> {
        if lower > upper || !token(revision) {
            return Err(BudgetError::InvalidEvidence);
        }
        Ok(Self {
            lower,
            upper,
            kind: EvidenceKind::Estimated,
            basis_revision: Some(revision.into()),
        })
    }

    /// Inclusive lower bound.
    pub const fn lower(&self) -> u64 {
        self.lower
    }
    /// Inclusive upper bound.
    pub const fn upper(&self) -> u64 {
        self.upper
    }
    /// Evidence source.
    pub const fn kind(&self) -> EvidenceKind {
        self.kind
    }
    /// Estimator revision, present only for estimates.
    pub fn basis_revision(&self) -> Option<&str> {
        self.basis_revision.as_deref()
    }
}

/// Available evidence or explicit absence; absence never means zero.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UsageEvidence {
    /// Measured or bounded estimated quantity.
    Available(QuantityEvidence),
    /// No trustworthy quantity is available.
    Unavailable(UnavailableReason),
}

impl UsageEvidence {
    /// Inclusive lower bound, absent when evidence is unavailable.
    pub fn lower(&self) -> Option<u64> {
        match self {
            Self::Available(value) => Some(value.lower()),
            Self::Unavailable(_) => None,
        }
    }

    /// Inclusive upper bound, absent when evidence is unavailable.
    pub fn upper(&self) -> Option<u64> {
        match self {
            Self::Available(value) => Some(value.upper()),
            Self::Unavailable(_) => None,
        }
    }

    /// Evidence source, absent when evidence is unavailable.
    pub fn kind(&self) -> Option<EvidenceKind> {
        match self {
            Self::Available(value) => Some(value.kind()),
            Self::Unavailable(_) => None,
        }
    }
}

/// Disjoint token classes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenUsage {
    /// Input tokens not served from cache.
    pub uncached_input: UsageEvidence,
    /// Input tokens served from cache.
    pub cached_input: UsageEvidence,
    /// Output tokens excluding reasoning.
    pub output: UsageEvidence,
    /// Provider-reported reasoning output tokens.
    pub reasoning: UsageEvidence,
}

/// Normalized settled usage for one call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedUsage {
    /// Disjoint token classes.
    pub tokens: TokenUsage,
    /// Tool actions.
    pub actions: UsageEvidence,
    /// Wall duration in nanoseconds.
    pub wall_time_ns: UsageEvidence,
    /// Cost in micros of currency.
    pub cost_micros: UsageEvidence,
    /// ISO 4217 currency if cost is available.
    pub currency: Option<String>,
    /// Immutable price table revision if cost is available.
    pub price_table_revision: Option<String>,
}

impl NormalizedUsage {
    /// Validate cost provenance without inventing absent values.
    pub fn validate(&self) -> Result<(), BudgetError> {
        let available = matches!(self.cost_micros, UsageEvidence::Available(_));
        if available != self.currency.as_deref().is_some_and(valid_currency)
            || available != self.price_table_revision.as_deref().is_some_and(token)
        {
            return Err(BudgetError::InvalidEvidence);
        }
        Ok(())
    }

    fn upper(&self, dimension: BudgetDimension) -> Option<u64> {
        match dimension {
            BudgetDimension::WallTime => self.wall_time_ns.upper(),
            BudgetDimension::Actions => self.actions.upper(),
            BudgetDimension::UncachedInputTokens => self.tokens.uncached_input.upper(),
            BudgetDimension::CachedInputTokens => self.tokens.cached_input.upper(),
            BudgetDimension::OutputTokens => self.tokens.output.upper(),
            BudgetDimension::ReasoningTokens => self.tokens.reasoning.upper(),
            BudgetDimension::Cost => self.cost_micros.upper(),
        }
    }
}

/// Closed nonzero maximums for a run or call reservation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetAmounts(BTreeMap<BudgetDimension, u64>);

impl BudgetAmounts {
    /// Construct a complete nonzero budget.
    pub fn new(
        values: impl IntoIterator<Item = (BudgetDimension, u64)>,
    ) -> Result<Self, BudgetError> {
        let values: BTreeMap<_, _> = values.into_iter().collect();
        if values.len() != BudgetDimension::ALL.len()
            || BudgetDimension::ALL
                .iter()
                .any(|d| values.get(d).is_none_or(|v| *v == 0))
        {
            return Err(BudgetError::InvalidBudget);
        }
        Ok(Self(values))
    }

    /// Return one maximum.
    pub fn get(&self, dimension: BudgetDimension) -> u64 {
        self.0[&dimension]
    }
}

/// Opaque pre-effect reservation.
///
/// Only a successful ledger admission can create this token:
///
/// ~~~compile_fail
/// let _ = asb_core::Reservation { id: 0 };
/// ~~~
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reservation {
    id: u64,
}

/// Remaining run budget and active pre-effect reservations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetLedger {
    remaining: BudgetAmounts,
    capabilities: BudgetCapabilities,
    active: BTreeMap<u64, BudgetAmounts>,
    next_id: u64,
}

impl BudgetLedger {
    /// Create a hard-budget ledger only if every dimension is enforceable.
    pub fn new(
        budget: BudgetAmounts,
        capabilities: BudgetCapabilities,
    ) -> Result<Self, BudgetError> {
        if let Some(dimension) = BudgetDimension::ALL
            .into_iter()
            .find(|dimension| capabilities.get(*dimension) != Enforcement::Enforced)
        {
            return Err(BudgetError::NotEnforceable(dimension));
        }
        Ok(Self {
            remaining: budget,
            capabilities,
            active: BTreeMap::new(),
            next_id: 0,
        })
    }

    /// Deduct a complete worst-case allowance before a provider call.
    pub fn reserve(&mut self, requested: BudgetAmounts) -> Result<Reservation, BudgetError> {
        if BudgetDimension::ALL
            .iter()
            .any(|d| requested.get(*d) > self.remaining.get(*d))
        {
            return Err(BudgetError::Exhausted);
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(BudgetError::Overflow)?;
        for dimension in BudgetDimension::ALL {
            let remaining = self.remaining.0.get_mut(&dimension).expect("closed budget");
            *remaining = remaining
                .checked_sub(requested.get(dimension))
                .ok_or(BudgetError::Overflow)?;
        }
        self.active.insert(id, requested);
        Ok(Reservation { id })
    }

    /// Settle and refund only a conservatively proven unused upper bound.
    pub fn settle(
        &mut self,
        reservation: Reservation,
        usage: &NormalizedUsage,
    ) -> Result<(), BudgetError> {
        usage.validate()?;
        let reserved = self
            .active
            .remove(&reservation.id)
            .ok_or(BudgetError::UnknownReservation)?;
        let consumed: BTreeMap<_, _> = BudgetDimension::ALL
            .into_iter()
            .map(|dimension| {
                (
                    dimension,
                    usage
                        .upper(dimension)
                        .unwrap_or_else(|| reserved.get(dimension)),
                )
            })
            .collect();
        if let Some(dimension) = BudgetDimension::ALL
            .into_iter()
            .find(|dimension| consumed[dimension] > reserved.get(*dimension))
        {
            self.active.insert(reservation.id, reserved);
            return Err(BudgetError::ReservationExceeded(dimension));
        }
        for dimension in BudgetDimension::ALL {
            let refund = reserved.get(dimension) - consumed[&dimension];
            let remaining = self.remaining.0.get_mut(&dimension).expect("closed budget");
            *remaining = remaining.checked_add(refund).ok_or(BudgetError::Overflow)?;
        }
        Ok(())
    }

    /// Return one remaining hard bound.
    pub fn remaining(&self, dimension: BudgetDimension) -> u64 {
        self.remaining.get(dimension)
    }
    /// Return the immutable capability declaration.
    pub fn capabilities(&self) -> &BudgetCapabilities {
        &self.capabilities
    }
}

/// Invalid budget, capability, reservation, or usage evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetError {
    /// A closed map omitted a dimension or used a zero bound.
    InvalidBudget,
    /// A capability map omitted a dimension.
    IncompleteCapabilities,
    /// A requested dimension cannot be enforced before effects.
    NotEnforceable(BudgetDimension),
    /// A call reservation exceeds remaining budget.
    Exhausted,
    /// A reservation was stale or already settled.
    UnknownReservation,
    /// Reported usage exceeds the reserved maximum.
    ReservationExceeded(BudgetDimension),
    /// Checked arithmetic overflowed.
    Overflow,
    /// Evidence bounds or provenance were invalid.
    InvalidEvidence,
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_BUDGET_REVISION_BYTES
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn valid_currency(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|b| b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn amounts(value: u64) -> BudgetAmounts {
        BudgetAmounts::new(BudgetDimension::ALL.map(|d| (d, value))).unwrap()
    }

    fn enforced() -> BudgetCapabilities {
        BudgetCapabilities::new(BudgetDimension::ALL.map(|d| (d, Enforcement::Enforced))).unwrap()
    }

    fn usage(value: UsageEvidence) -> NormalizedUsage {
        NormalizedUsage {
            tokens: TokenUsage {
                uncached_input: value.clone(),
                cached_input: value.clone(),
                output: value.clone(),
                reasoning: value.clone(),
            },
            actions: value.clone(),
            wall_time_ns: value.clone(),
            cost_micros: value,
            currency: Some("USD".into()),
            price_table_revision: Some("prices-2026-09-08".into()),
        }
    }

    #[test]
    fn reserves_before_effect_and_refunds_proven_unused_bounds() {
        let mut ledger = BudgetLedger::new(amounts(10), enforced()).unwrap();
        assert_eq!(
            ledger.capabilities().get(BudgetDimension::Actions),
            Enforcement::Enforced
        );
        let reservation = ledger.reserve(amounts(6)).unwrap();
        assert_eq!(ledger.remaining(BudgetDimension::Actions), 4);
        ledger
            .settle(
                reservation,
                &usage(UsageEvidence::Available(QuantityEvidence::measured(2))),
            )
            .unwrap();
        assert_eq!(ledger.remaining(BudgetDimension::Actions), 8);
    }

    #[test]
    fn missing_usage_is_not_zero_filled_or_refunded() {
        let mut ledger = BudgetLedger::new(amounts(10), enforced()).unwrap();
        let reservation = ledger.reserve(amounts(6)).unwrap();
        let missing = UsageEvidence::Unavailable(UnavailableReason::MissingTelemetry);
        let mut evidence = usage(missing);
        evidence.currency = None;
        evidence.price_table_revision = None;
        ledger.settle(reservation, &evidence).unwrap();
        assert_eq!(ledger.remaining(BudgetDimension::Cost), 4);
    }

    #[test]
    fn opaque_adapter_cannot_claim_enforcement() {
        assert_eq!(
            BudgetLedger::new(amounts(1), BudgetCapabilities::opaque()),
            Err(BudgetError::NotEnforceable(BudgetDimension::WallTime))
        );
    }

    #[test]
    fn estimates_require_ordered_bounds_and_public_revision() {
        assert_eq!(
            QuantityEvidence::estimated(2, 1, "estimator-v1"),
            Err(BudgetError::InvalidEvidence)
        );
        assert_eq!(
            QuantityEvidence::estimated(1, 2, "private path"),
            Err(BudgetError::InvalidEvidence)
        );
        let value = QuantityEvidence::estimated(1, 2, "estimator-v1").unwrap();
        assert_eq!(value.kind(), EvidenceKind::Estimated);
        assert_eq!(value.basis_revision(), Some("estimator-v1"));
    }

    #[test]
    fn usage_accessors_never_turn_absence_into_zero() {
        let measured = UsageEvidence::Available(QuantityEvidence::measured(7));
        assert_eq!(measured.lower(), Some(7));
        assert_eq!(measured.upper(), Some(7));
        assert_eq!(measured.kind(), Some(EvidenceKind::Measured));

        let missing = UsageEvidence::Unavailable(UnavailableReason::MissingTelemetry);
        assert_eq!(missing.lower(), None);
        assert_eq!(missing.upper(), None);
        assert_eq!(missing.kind(), None);
    }

    #[test]
    fn exhaustion_and_over_settlement_fail_closed() {
        let mut ledger = BudgetLedger::new(amounts(5), enforced()).unwrap();
        assert_eq!(ledger.reserve(amounts(6)), Err(BudgetError::Exhausted));
        let reservation = ledger.reserve(amounts(5)).unwrap();
        assert_eq!(
            ledger.settle(
                reservation,
                &usage(UsageEvidence::Available(QuantityEvidence::measured(6)))
            ),
            Err(BudgetError::ReservationExceeded(BudgetDimension::WallTime))
        );
        assert_eq!(ledger.remaining(BudgetDimension::WallTime), 0);
    }

    #[test]
    fn cost_requires_currency_and_price_revision() {
        let mut evidence = usage(UsageEvidence::Available(QuantityEvidence::measured(1)));
        evidence.price_table_revision = None;
        assert_eq!(evidence.validate(), Err(BudgetError::InvalidEvidence));
        evidence.cost_micros = UsageEvidence::Unavailable(UnavailableReason::Unsupported);
        evidence.currency = None;
        assert_eq!(evidence.validate(), Ok(()));
    }

    #[test]
    fn incomplete_closed_maps_are_rejected() {
        assert_eq!(
            BudgetCapabilities::new([(BudgetDimension::Actions, Enforcement::Enforced)]),
            Err(BudgetError::IncompleteCapabilities)
        );
        assert_eq!(
            BudgetAmounts::new([(BudgetDimension::Actions, 1)]),
            Err(BudgetError::InvalidBudget)
        );
    }
}
