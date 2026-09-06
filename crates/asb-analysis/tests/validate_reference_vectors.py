# SPDX-License-Identifier: MIT
"""Independently reproduce the checked-in statistical reference vectors."""

import math
from pathlib import Path

import numpy as np
import scipy
from scipy.stats import binomtest, sem, t

assert np.__version__ == "2.5.3"
assert scipy.__version__ == "1.16.1"

fixtures = Path(__file__).parent / "fixtures"
for line in (fixtures / "reference-vectors.tsv").read_text(encoding="utf-8").splitlines():
    if line.startswith("#"):
        continue
    kind, raw, parameter, estimate, lower, upper = line.split("\t")
    expected = tuple(map(float, (estimate, lower, upper)))
    if kind == "quantile":
        value = float(
            np.quantile(np.array(list(map(int, raw.split(",")))), float(parameter), method="linear")
        )
        actual = (value, value, value)
    elif kind == "wilson":
        successes, total = map(int, raw.split(","))
        interval = binomtest(successes, total).proportion_ci(
            confidence_level=float(parameter), method="wilson"
        )
        actual = (successes / total, interval.low, interval.high)
    elif kind == "throughput":
        values = np.array(list(map(float, raw.split(","))))
        average = float(np.mean(values))
        radius = float(t.ppf(0.975, len(values) - 1) * sem(values))
        actual = (average, max(0.0, average - radius), average + radius)
    else:
        raise AssertionError(f"unknown vector kind: {kind}")
    assert np.allclose(actual, expected, rtol=0.0, atol=1e-8), (kind, actual, expected)

for line in (fixtures / "dkw-reference-vectors.tsv").read_text(encoding="utf-8").splitlines():
    if line.startswith("#"):
        continue
    raw, quantile, confidence, estimate, lower, upper = line.split("\t")
    start, stop = map(int, raw.removeprefix("range:").split(":"))
    values = np.arange(start, stop + 1, dtype=float)
    probability = float(quantile)
    alpha = 1.0 - float(confidence)
    epsilon = math.sqrt(math.log(2.0 / alpha) / (2.0 * len(values)))
    actual_estimate = float(np.quantile(values, probability, method="linear"))
    actual_lower = float(np.quantile(values, max(0.0, probability - epsilon), method="linear"))
    actual_upper = (
        None
        if probability + epsilon >= 1.0
        else float(np.quantile(values, probability + epsilon, method="linear"))
    )
    assert math.isclose(actual_estimate, float(estimate), rel_tol=0.0, abs_tol=1e-8)
    assert math.isclose(actual_lower, float(lower), rel_tol=0.0, abs_tol=1e-8)
    if upper == "none":
        assert actual_upper is None
    else:
        assert actual_upper is not None
        assert math.isclose(actual_upper, float(upper), rel_tol=0.0, abs_tol=1e-8)

print(f"validated reference vectors with scipy {scipy.__version__} and numpy {np.__version__}")
