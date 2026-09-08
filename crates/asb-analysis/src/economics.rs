// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Cost-per-success and uncertainty-aware Pareto analysis.

use asb_core::{EvidenceKind, QuantityEvidence, UsageEvidence};
use std::collections::BTreeSet;

/// Maximum points in one Pareto report.
pub const MAX_PARETO_POINTS: usize = 4096;

/// One immutable quality-latency-resource-cost observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeoffPoint {
    id: String,
    quality_millionths: u32,
    latency_ns: QuantityEvidence,
    resource_units: QuantityEvidence,
    cost_micros: QuantityEvidence,
}

impl TradeoffPoint {
    /// Validate a public point identity and bounded quality score.
    pub fn new(
        id: &str,
        quality_millionths: u32,
        latency_ns: QuantityEvidence,
        resource_units: QuantityEvidence,
        cost_micros: QuantityEvidence,
    ) -> Result<Self, EconomicsError> {
        if !token(id) || quality_millionths > 1_000_000 {
            return Err(EconomicsError::InvalidPoint);
        }
        Ok(Self {
            id: id.into(),
            quality_millionths,
            latency_ns,
            resource_units,
            cost_micros,
        })
    }

    /// Stable public point identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Quality in millionths.
    pub const fn quality_millionths(&self) -> u32 {
        self.quality_millionths
    }

    /// Latency interval in nanoseconds.
    pub const fn latency_ns(&self) -> &QuantityEvidence {
        &self.latency_ns
    }

    /// Versioned normalized resource interval.
    pub const fn resource_units(&self) -> &QuantityEvidence {
        &self.resource_units
    }

    /// Cost interval in micros.
    pub const fn cost_micros(&self) -> &QuantityEvidence {
        &self.cost_micros
    }
}

/// Invalid economic report input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EconomicsError {
    /// Point identity or quality was invalid.
    InvalidPoint,
    /// Point count exceeded the hard ceiling.
    TooManyPoints,
    /// Point identities were duplicated.
    DuplicatePoint,
    /// There were no successful attempts for a ratio.
    NoSuccesses,
    /// Checked fixed-point arithmetic overflowed.
    Overflow,
}

/// Compute conservative cost per success without treating missing cost as zero.
pub fn cost_per_success(
    costs: &[UsageEvidence],
    successes: u64,
) -> Result<UsageEvidence, EconomicsError> {
    if successes == 0 {
        return Err(EconomicsError::NoSuccesses);
    }
    let mut lower = 0_u64;
    let mut upper = 0_u64;
    let mut estimated = false;
    for cost in costs {
        let value = match cost {
            UsageEvidence::Available(value) => value,
            UsageEvidence::Unavailable(reason) => {
                return Ok(UsageEvidence::Unavailable(*reason));
            }
        };
        lower = lower
            .checked_add(value.lower())
            .ok_or(EconomicsError::Overflow)?;
        upper = upper
            .checked_add(value.upper())
            .ok_or(EconomicsError::Overflow)?;
        estimated |= value.kind() == EvidenceKind::Estimated;
    }
    let ratio_lower = lower / successes;
    let ratio_upper = upper
        .checked_add(successes - 1)
        .ok_or(EconomicsError::Overflow)?
        / successes;
    if !estimated && ratio_lower == ratio_upper {
        Ok(UsageEvidence::Available(QuantityEvidence::measured(
            ratio_lower,
        )))
    } else {
        Ok(UsageEvidence::Available(
            QuantityEvidence::estimated(ratio_lower, ratio_upper, "cost-per-success-v1")
                .map_err(|_| EconomicsError::Overflow)?,
        ))
    }
}

/// Return points not conservatively dominated across all four dimensions.
pub fn pareto_frontier(points: &[TradeoffPoint]) -> Result<Vec<&TradeoffPoint>, EconomicsError> {
    if points.len() > MAX_PARETO_POINTS {
        return Err(EconomicsError::TooManyPoints);
    }
    let mut ids = BTreeSet::new();
    if points.iter().any(|point| !ids.insert(point.id())) {
        return Err(EconomicsError::DuplicatePoint);
    }
    Ok(points
        .iter()
        .filter(|candidate| {
            !points
                .iter()
                .any(|other| other.id != candidate.id && conservatively_dominates(other, candidate))
        })
        .collect())
}

fn conservatively_dominates(left: &TradeoffPoint, right: &TradeoffPoint) -> bool {
    let quality = left.quality_millionths >= right.quality_millionths;
    let latency = left.latency_ns.upper() <= right.latency_ns.lower();
    let resources = left.resource_units.upper() <= right.resource_units.lower();
    let cost = left.cost_micros.upper() <= right.cost_micros.lower();
    let strict = left.quality_millionths > right.quality_millionths
        || left.latency_ns.upper() < right.latency_ns.lower()
        || left.resource_units.upper() < right.resource_units.lower()
        || left.cost_micros.upper() < right.cost_micros.lower();
    quality && latency && resources && cost && strict
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use asb_core::UnavailableReason;

    fn measured(id: &str, quality: u32, latency: u64, resource: u64, cost: u64) -> TradeoffPoint {
        TradeoffPoint::new(
            id,
            quality,
            QuantityEvidence::measured(latency),
            QuantityEvidence::measured(resource),
            QuantityEvidence::measured(cost),
        )
        .unwrap()
    }

    #[test]
    fn cost_ratio_retains_missing_and_estimated_evidence() {
        assert_eq!(
            cost_per_success(
                &[UsageEvidence::Unavailable(UnavailableReason::Unsupported)],
                1,
            ),
            Ok(UsageEvidence::Unavailable(UnavailableReason::Unsupported))
        );
        let estimate = QuantityEvidence::estimated(9, 11, "price-v1").unwrap();
        let result = cost_per_success(&[UsageEvidence::Available(estimate)], 2).unwrap();
        assert_eq!(result.lower(), Some(4));
        assert_eq!(result.upper(), Some(6));
        assert_eq!(result.kind(), Some(EvidenceKind::Estimated));
    }

    #[test]
    fn zero_success_and_overflow_fail_closed() {
        assert_eq!(cost_per_success(&[], 0), Err(EconomicsError::NoSuccesses));
        assert_eq!(
            cost_per_success(
                &[
                    UsageEvidence::Available(QuantityEvidence::measured(u64::MAX)),
                    UsageEvidence::Available(QuantityEvidence::measured(1)),
                ],
                1,
            ),
            Err(EconomicsError::Overflow)
        );
    }

    #[test]
    fn pareto_requires_conservative_interval_dominance() {
        let points = [
            measured("reference", 900_000, 10, 10, 10),
            measured("dominated", 800_000, 12, 12, 12),
            measured("quality", 950_000, 20, 20, 20),
        ];
        let ids: Vec<_> = pareto_frontier(&points)
            .unwrap()
            .into_iter()
            .map(TradeoffPoint::id)
            .collect();
        assert_eq!(ids, ["reference", "quality"]);

        let uncertain = TradeoffPoint::new(
            "uncertain",
            800_000,
            QuantityEvidence::estimated(8, 14, "measurement-v1").unwrap(),
            QuantityEvidence::measured(12),
            QuantityEvidence::measured(12),
        )
        .unwrap();
        assert_eq!(
            pareto_frontier(&[points[0].clone(), uncertain])
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn invalid_and_duplicate_points_are_rejected() {
        assert_eq!(
            TradeoffPoint::new(
                "private path",
                0,
                QuantityEvidence::measured(1),
                QuantityEvidence::measured(1),
                QuantityEvidence::measured(1),
            ),
            Err(EconomicsError::InvalidPoint)
        );
        let point = measured("same", 1, 1, 1, 1);
        assert_eq!(
            pareto_frontier(&[point.clone(), point]),
            Err(EconomicsError::DuplicatePoint)
        );
    }
}
