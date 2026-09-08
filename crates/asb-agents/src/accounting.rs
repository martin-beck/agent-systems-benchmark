// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Provider usage normalization without zero-filling absent token classes.

use asb_core::{
    BudgetAmounts, BudgetCapabilities, BudgetDimension, BudgetError, BudgetLedger, Enforcement,
    NormalizedUsage, QuantityEvidence, TokenUsage, UnavailableReason, UsageEvidence,
};
use asb_protocol::Usage as ProtocolUsage;

/// Raw provider totals retained outside private model content.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProviderUsage {
    /// Total input tokens, including cached input, when reported.
    pub input_total: Option<u64>,
    /// Cached subset of input tokens, when reported.
    pub cached_input: Option<u64>,
    /// Total output tokens, including reasoning, when reported.
    pub output_total: Option<u64>,
    /// Reasoning subset of output tokens, when reported.
    pub reasoning_output: Option<u64>,
    /// Provider-reported monetary charge in micros.
    pub cost_micros: Option<u64>,
    /// Provider-reported ISO 4217 currency.
    pub currency: Option<String>,
    /// Immutable provider billing or price revision for a reported charge.
    pub cost_basis_revision: Option<String>,
    /// Measured tool actions.
    pub actions: Option<u64>,
    /// Measured wall duration in nanoseconds.
    pub wall_time_ns: Option<u64>,
}

/// Immutable per-million-token price table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceTable {
    revision: String,
    currency: String,
    uncached_input_per_million: u64,
    cached_input_per_million: u64,
    output_per_million: u64,
    reasoning_per_million: u64,
}

impl PriceTable {
    /// Validate one public price table.
    pub fn new(
        revision: &str,
        currency: &str,
        uncached_input_per_million: u64,
        cached_input_per_million: u64,
        output_per_million: u64,
        reasoning_per_million: u64,
    ) -> Result<Self, AccountingError> {
        if revision.is_empty()
            || revision.len() > 4096
            || !revision
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            || currency.len() != 3
            || !currency.bytes().all(|b| b.is_ascii_uppercase())
        {
            return Err(AccountingError::InvalidPriceTable);
        }
        Ok(Self {
            revision: revision.into(),
            currency: currency.into(),
            uncached_input_per_million,
            cached_input_per_million,
            output_per_million,
            reasoning_per_million,
        })
    }

    /// Immutable price-table revision.
    pub fn revision(&self) -> &str {
        &self.revision
    }
}

/// Provider accounting failed closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountingError {
    /// A reported subset exceeded its total.
    ContradictoryTokens,
    /// Cost and currency were not reported together.
    ContradictoryCost,
    /// A price table identity was invalid.
    InvalidPriceTable,
    /// Fixed-point cost arithmetic overflowed.
    Overflow,
    /// Normalized core evidence was invalid.
    InvalidEvidence,
}

/// A budgeted provider effect failed at admission, execution, or settlement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BudgetedCallError<E> {
    /// The complete worst-case allowance could not be reserved before the effect.
    Admission(BudgetError),
    /// The provider effect failed; its reservation remains consumed conservatively.
    Effect(E),
    /// Returned usage was invalid or exceeded the reservation.
    Settlement(BudgetError),
}

/// Reserve before invoking an effect, then settle from its normalized usage.
///
/// Failed effects deliberately retain their complete reservation: absent usage is
/// never interpreted as zero and cannot replenish the caller's budget.
pub fn execute_budgeted<T, E>(
    ledger: &mut BudgetLedger,
    requested: BudgetAmounts,
    effect: impl FnOnce() -> Result<(T, NormalizedUsage), E>,
) -> Result<T, BudgetedCallError<E>> {
    let reservation = ledger
        .reserve(requested)
        .map_err(BudgetedCallError::Admission)?;
    let (output, usage) = effect().map_err(BudgetedCallError::Effect)?;
    ledger
        .settle(reservation, &usage)
        .map_err(BudgetedCallError::Settlement)?;
    Ok(output)
}

/// Normalize totals into disjoint measured classes and versioned estimated cost.
pub fn normalize_usage(
    raw: ProviderUsage,
    prices: Option<&PriceTable>,
) -> Result<NormalizedUsage, AccountingError> {
    let (uncached_input, cached_input) = disjoint(
        raw.input_total,
        raw.cached_input,
        UnavailableReason::MissingTelemetry,
    )?;
    let (output, reasoning) = disjoint(
        raw.output_total,
        raw.reasoning_output,
        UnavailableReason::MissingTelemetry,
    )?;
    let tokens = TokenUsage {
        uncached_input,
        cached_input,
        output,
        reasoning,
    };
    let actions = measured_or_missing(raw.actions);
    let wall_time_ns = measured_or_missing(raw.wall_time_ns);
    let (cost_micros, currency, price_table_revision) = normalize_cost(
        raw.cost_micros,
        raw.currency.as_deref(),
        raw.cost_basis_revision.as_deref(),
        &tokens,
        prices,
    )?;
    let usage = NormalizedUsage {
        tokens,
        actions,
        wall_time_ns,
        cost_micros,
        currency,
        price_table_revision,
    };
    usage
        .validate()
        .map_err(|_| AccountingError::InvalidEvidence)?;
    Ok(usage)
}

/// Declare the generic protocol usage boundary without overstating enforcement.
pub fn protocol_usage_capabilities() -> BudgetCapabilities {
    BudgetCapabilities::new([
        (BudgetDimension::WallTime, Enforcement::Enforced),
        (BudgetDimension::Actions, Enforcement::ObservedOnly),
        (
            BudgetDimension::UncachedInputTokens,
            Enforcement::ObservedOnly,
        ),
        (BudgetDimension::CachedInputTokens, Enforcement::Unavailable),
        (BudgetDimension::OutputTokens, Enforcement::ObservedOnly),
        (BudgetDimension::ReasoningTokens, Enforcement::Unavailable),
        (BudgetDimension::Cost, Enforcement::ObservedOnly),
    ])
    .expect("complete static capability declaration")
}

/// Normalize the public extension protocol without misclassifying aggregate totals.
///
/// Protocol v1 does not distinguish cached or reasoning subsets and carries no
/// immutable cost-basis revision. Its aggregate token and cost fields therefore
/// cannot be projected into the stricter disjoint accounting model.
pub fn normalize_protocol_usage(
    raw: &ProtocolUsage,
    actions: Option<u64>,
    wall_time_ns: Option<u64>,
) -> Result<NormalizedUsage, AccountingError> {
    match (&raw.cost_micros, &raw.currency) {
        (Some(_), Some(currency))
            if currency.len() == 3 && currency.bytes().all(|b| b.is_ascii_uppercase()) => {}
        (None, None) => {}
        _ => return Err(AccountingError::ContradictoryCost),
    }
    let unavailable = || UsageEvidence::Unavailable(UnavailableReason::Unsupported);
    let usage = NormalizedUsage {
        tokens: TokenUsage {
            uncached_input: unavailable(),
            cached_input: unavailable(),
            output: unavailable(),
            reasoning: unavailable(),
        },
        actions: measured_or_missing(actions),
        wall_time_ns: measured_or_missing(wall_time_ns),
        cost_micros: unavailable(),
        currency: None,
        price_table_revision: None,
    };
    usage
        .validate()
        .map_err(|_| AccountingError::InvalidEvidence)?;
    Ok(usage)
}

fn disjoint(
    total: Option<u64>,
    subset: Option<u64>,
    missing: UnavailableReason,
) -> Result<(UsageEvidence, UsageEvidence), AccountingError> {
    match (total, subset) {
        (Some(total), Some(subset)) if subset <= total => Ok((
            UsageEvidence::Available(QuantityEvidence::measured(total - subset)),
            UsageEvidence::Available(QuantityEvidence::measured(subset)),
        )),
        (Some(_), Some(_)) => Err(AccountingError::ContradictoryTokens),
        (None, Some(_)) => Err(AccountingError::ContradictoryTokens),
        _ => Ok((
            UsageEvidence::Unavailable(missing),
            UsageEvidence::Unavailable(missing),
        )),
    }
}

fn measured_or_missing(value: Option<u64>) -> UsageEvidence {
    value.map_or(
        UsageEvidence::Unavailable(UnavailableReason::MissingTelemetry),
        |value| UsageEvidence::Available(QuantityEvidence::measured(value)),
    )
}

fn normalize_cost(
    reported: Option<u64>,
    reported_currency: Option<&str>,
    reported_revision: Option<&str>,
    tokens: &TokenUsage,
    prices: Option<&PriceTable>,
) -> Result<(UsageEvidence, Option<String>, Option<String>), AccountingError> {
    match (reported, reported_currency, reported_revision) {
        (Some(value), Some(currency), Some(revision))
            if currency.len() == 3
                && currency.bytes().all(|b| b.is_ascii_uppercase())
                && valid_revision(revision) =>
        {
            Ok((
                UsageEvidence::Available(QuantityEvidence::measured(value)),
                Some(currency.into()),
                Some(revision.into()),
            ))
        }
        (Some(_), _, _) | (None, Some(_), _) | (None, None, Some(_)) => {
            Err(AccountingError::ContradictoryCost)
        }
        (None, None, None) => {
            let Some(prices) = prices else {
                return Ok((
                    UsageEvidence::Unavailable(UnavailableReason::Unsupported),
                    None,
                    None,
                ));
            };
            let values = [
                (&tokens.uncached_input, prices.uncached_input_per_million),
                (&tokens.cached_input, prices.cached_input_per_million),
                (&tokens.output, prices.output_per_million),
                (&tokens.reasoning, prices.reasoning_per_million),
            ];
            let mut lower_numerator = 0_u128;
            let mut upper_numerator = 0_u128;
            for (evidence, rate) in values {
                let (Some(lower), Some(upper)) = (evidence.lower(), evidence.upper()) else {
                    return Ok((
                        UsageEvidence::Unavailable(UnavailableReason::MissingTelemetry),
                        None,
                        None,
                    ));
                };
                lower_numerator = lower_numerator
                    .checked_add(u128::from(lower) * u128::from(rate))
                    .ok_or(AccountingError::Overflow)?;
                upper_numerator = upper_numerator
                    .checked_add(u128::from(upper) * u128::from(rate))
                    .ok_or(AccountingError::Overflow)?;
            }
            let lower = u64::try_from(lower_numerator / 1_000_000)
                .map_err(|_| AccountingError::Overflow)?;
            let upper = u64::try_from(
                upper_numerator
                    .checked_add(999_999)
                    .ok_or(AccountingError::Overflow)?
                    / 1_000_000,
            )
            .map_err(|_| AccountingError::Overflow)?;
            let evidence = QuantityEvidence::estimated(lower, upper, prices.revision())
                .map_err(|_: BudgetError| AccountingError::InvalidEvidence)?;
            Ok((
                UsageEvidence::Available(evidence),
                Some(prices.currency.clone()),
                Some(prices.revision.clone()),
            ))
        }
    }
}

fn valid_revision(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_core::EvidenceKind;

    const DIMENSIONS: [BudgetDimension; 7] = [
        BudgetDimension::WallTime,
        BudgetDimension::Actions,
        BudgetDimension::UncachedInputTokens,
        BudgetDimension::CachedInputTokens,
        BudgetDimension::OutputTokens,
        BudgetDimension::ReasoningTokens,
        BudgetDimension::Cost,
    ];

    fn prices() -> PriceTable {
        PriceTable::new("prices-v1", "USD", 1_000_000, 500_000, 2_000_000, 3_000_000).unwrap()
    }

    fn amounts(value: u64) -> BudgetAmounts {
        BudgetAmounts::new(DIMENSIONS.map(|dimension| (dimension, value))).unwrap()
    }

    fn ledger(value: u64) -> BudgetLedger {
        let capabilities =
            BudgetCapabilities::new(DIMENSIONS.map(|d| (d, Enforcement::Enforced))).unwrap();
        BudgetLedger::new(amounts(value), capabilities).unwrap()
    }

    fn complete_usage(value: u64) -> NormalizedUsage {
        let available = || UsageEvidence::Available(QuantityEvidence::measured(value));
        NormalizedUsage {
            tokens: TokenUsage {
                uncached_input: available(),
                cached_input: available(),
                output: available(),
                reasoning: available(),
            },
            actions: available(),
            wall_time_ns: available(),
            cost_micros: available(),
            currency: Some("USD".into()),
            price_table_revision: Some("billing-v1".into()),
        }
    }

    #[test]
    fn measured_totals_become_disjoint_classes_and_estimated_cost() {
        let usage = normalize_usage(
            ProviderUsage {
                input_total: Some(10),
                cached_input: Some(4),
                output_total: Some(7),
                reasoning_output: Some(2),
                actions: Some(3),
                wall_time_ns: Some(9),
                ..ProviderUsage::default()
            },
            Some(&prices()),
        )
        .unwrap();
        assert_eq!(usage.tokens.uncached_input.upper(), Some(6));
        assert_eq!(usage.tokens.cached_input.upper(), Some(4));
        assert_eq!(usage.tokens.output.upper(), Some(5));
        assert_eq!(usage.tokens.reasoning.upper(), Some(2));
        assert!(matches!(
            usage.cost_micros,
            UsageEvidence::Available(ref value) if value.kind() == EvidenceKind::Estimated
        ));
        assert_eq!(usage.price_table_revision.as_deref(), Some("prices-v1"));
    }

    #[test]
    fn absent_subclasses_remain_unavailable() {
        let usage = normalize_usage(
            ProviderUsage {
                input_total: Some(10),
                output_total: Some(7),
                ..ProviderUsage::default()
            },
            Some(&prices()),
        )
        .unwrap();
        assert!(matches!(
            usage.tokens.uncached_input,
            UsageEvidence::Unavailable(_)
        ));
        assert!(matches!(
            usage.tokens.reasoning,
            UsageEvidence::Unavailable(_)
        ));
        assert!(matches!(usage.cost_micros, UsageEvidence::Unavailable(_)));
    }

    #[test]
    fn contradictions_fail_closed() {
        assert_eq!(
            normalize_usage(
                ProviderUsage {
                    input_total: Some(1),
                    cached_input: Some(2),
                    ..ProviderUsage::default()
                },
                None
            ),
            Err(AccountingError::ContradictoryTokens)
        );
        assert_eq!(
            normalize_usage(
                ProviderUsage {
                    cost_micros: Some(1),
                    ..ProviderUsage::default()
                },
                None
            ),
            Err(AccountingError::ContradictoryCost)
        );
        assert_eq!(
            normalize_usage(
                ProviderUsage {
                    cost_micros: Some(1),
                    currency: Some("USD".into()),
                    ..ProviderUsage::default()
                },
                None
            ),
            Err(AccountingError::ContradictoryCost)
        );
        assert_eq!(
            normalize_usage(
                ProviderUsage {
                    cost_micros: Some(1),
                    currency: Some("USD".into()),
                    cost_basis_revision: Some("../private".into()),
                    ..ProviderUsage::default()
                },
                None
            ),
            Err(AccountingError::ContradictoryCost)
        );
    }

    #[test]
    fn measured_cost_requires_immutable_provider_revision() {
        let usage = normalize_usage(
            ProviderUsage {
                cost_micros: Some(7),
                currency: Some("USD".into()),
                cost_basis_revision: Some("provider-billing-v2".into()),
                ..ProviderUsage::default()
            },
            None,
        )
        .unwrap();
        assert_eq!(usage.cost_micros.upper(), Some(7));
        assert_eq!(
            usage.price_table_revision.as_deref(),
            Some("provider-billing-v2")
        );
    }

    #[test]
    fn budgeted_boundary_reserves_before_effect_and_settles_afterward() {
        let mut ledger = ledger(10);
        let output = execute_budgeted(&mut ledger, amounts(5), || {
            Ok::<_, ()>(("done", complete_usage(1)))
        })
        .unwrap();
        assert_eq!(output, "done");
        assert_eq!(ledger.remaining(BudgetDimension::Actions), 9);

        let mut called = false;
        let error = execute_budgeted(&mut ledger, amounts(10), || {
            called = true;
            Ok::<_, ()>(("impossible", complete_usage(1)))
        });
        assert!(matches!(error, Err(BudgetedCallError::Admission(_))));
        assert!(!called);
    }

    #[test]
    fn failed_effect_never_refunds_absent_usage() {
        let mut ledger = ledger(10);
        assert_eq!(
            execute_budgeted::<(), _>(&mut ledger, amounts(4), || Err("provider failed")),
            Err(BudgetedCallError::Effect("provider failed"))
        );
        assert_eq!(ledger.remaining(BudgetDimension::Actions), 6);
    }

    #[test]
    fn opaque_protocol_gaps_are_explicit() {
        let capabilities = protocol_usage_capabilities();
        assert_eq!(
            capabilities.get(BudgetDimension::WallTime),
            Enforcement::Enforced
        );
        assert_eq!(
            capabilities.get(BudgetDimension::CachedInputTokens),
            Enforcement::Unavailable
        );
        assert_eq!(
            capabilities.get(BudgetDimension::Actions),
            Enforcement::ObservedOnly
        );

        let usage = normalize_protocol_usage(
            &ProtocolUsage {
                input_tokens: Some(9),
                output_tokens: Some(4),
                cost_micros: Some(3),
                currency: Some("USD".into()),
            },
            Some(2),
            Some(7),
        )
        .unwrap();
        assert_eq!(usage.actions.upper(), Some(2));
        assert_eq!(usage.wall_time_ns.upper(), Some(7));
        assert_eq!(usage.tokens.uncached_input.upper(), None);
        assert!(matches!(
            usage.cost_micros,
            UsageEvidence::Unavailable(UnavailableReason::Unsupported)
        ));
        assert_eq!(
            normalize_protocol_usage(
                &ProtocolUsage {
                    cost_micros: Some(3),
                    ..ProtocolUsage::default()
                },
                None,
                None
            ),
            Err(AccountingError::ContradictoryCost)
        );
    }
}
