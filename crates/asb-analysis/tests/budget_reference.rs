// Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT
//! Checked reference vectors for quality-latency-resource-cost Pareto reports.

use asb_analysis::{TradeoffPoint, pareto_frontier};
use asb_core::QuantityEvidence;

#[test]
fn checked_in_budget_reference_vectors_match_frontier() {
    let mut points = Vec::new();
    let mut expected = Vec::new();
    for line in include_str!("fixtures/budget-reference-vectors.tsv")
        .lines()
        .skip(1)
    {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 6);
        points.push(
            TradeoffPoint::new(
                fields[0],
                fields[1].parse().unwrap(),
                QuantityEvidence::measured(fields[2].parse().unwrap()),
                QuantityEvidence::measured(fields[3].parse().unwrap()),
                QuantityEvidence::measured(fields[4].parse().unwrap()),
            )
            .unwrap(),
        );
        if fields[5] == "true" {
            expected.push(fields[0]);
        }
    }
    let actual: Vec<_> = pareto_frontier(&points)
        .unwrap()
        .into_iter()
        .map(TradeoffPoint::id)
        .collect();
    assert_eq!(actual, expected);
}
