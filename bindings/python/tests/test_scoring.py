"""Cross-validation tests for the Python binding.

Because the binding calls the *same* Rust code as the CLI, parity is guaranteed
by construction; these tests instead guard the wrapper itself — argument
marshalling, None/NaN handling, validation — against hand-computed values and
known identities that also appear in the Rust unit tests.
"""
import math

import pytest

import anamnesis as ana

# A small, fully hand-checkable record.
PROBS = [0.9, 0.8, 0.3, 0.6, 0.5]
OUT = [1, 1, 0, 1, 0]


def test_brier_hand_computed():
    # (.01 + .04 + .09 + .16 + .25) / 5 = 0.11
    assert ana.brier(PROBS, OUT) == pytest.approx(0.11)


def test_empty_is_none():
    assert ana.brier([], []) is None
    assert ana.auc([], []) is None
    assert ana.decompose([], []) is None


def test_decomposition_identity_is_exact():
    d = ana.decompose(PROBS, OUT)
    assert d is not None
    assert (d.reliability - d.resolution + d.uncertainty) == pytest.approx(d.brier, abs=1e-12)
    # uncertainty = base_rate*(1-base_rate); base rate here is 3/5 = 0.6
    assert d.uncertainty == pytest.approx(0.6 * 0.4)


def test_auc_perfect_and_reversed():
    p = [0.2, 0.4, 0.6, 0.8]
    assert ana.auc(p, [0, 0, 1, 1]) == pytest.approx(1.0)  # perfect separation
    assert ana.auc(p, [1, 1, 0, 0]) == pytest.approx(0.0)  # perfectly wrong order
    assert ana.auc(p, [1, 1, 1, 1]) is None  # one class → undefined


def test_auc_is_blind_to_calibration():
    # A forecaster who always says the base rate is perfectly calibrated but has
    # no discrimination: AUC should be 0.5 even though ranking is degenerate.
    assert ana.auc([0.5, 0.5, 0.5, 0.5], [1, 0, 1, 0]) == pytest.approx(0.5)


def test_overconfidence_sign():
    # Bold and wrong → overconfident (gap > 0).
    bold_wrong = ana.overconfidence([0.95, 0.95, 0.95], [0, 0, 0])
    assert bold_wrong.gap > 0
    # Timid and right → underconfident (gap < 0).
    timid_right = ana.overconfidence([0.55, 0.55, 0.55], [1, 1, 1])
    assert timid_right.gap < 0


def test_log_score_punishes_confident_miss():
    # A confident miss should score worse (higher) than a hedged one.
    assert ana.log_score([0.99], [0]) > ana.log_score([0.6], [0])


def test_shrinkage_pulls_small_n():
    assert ana.shrink_toward(1, 1, prior_mean=0.5, strength=4) == pytest.approx(0.6)
    # strength 0 recovers the raw rate
    assert ana.shrink_toward(3, 4, prior_mean=0.5, strength=0) == pytest.approx(0.75)
    # large n barely moves
    assert ana.shrink_toward(80, 100, 0.5, 4) == pytest.approx(0.8, abs=0.02)


def test_wilson_interval_bounds():
    lo, hi = ana.wilson_interval(8, 10)
    assert 0.0 <= lo <= hi <= 1.0
    # 0 successes → lower bound pinned at 0, upper strictly below 1
    lo0, hi0 = ana.wilson_interval(0, 5)
    assert lo0 == pytest.approx(0.0)
    assert 0.0 < hi0 < 1.0
    assert ana.wilson_interval(0, 0) is None


def test_winkler_inside_is_width_outside_is_penalized():
    # Inside the interval, the score is exactly the width.
    assert ana.winkler(0.0, 10.0, 0.8, 5.0) == pytest.approx(10.0)
    # Outside, width + penalty > width.
    assert ana.winkler(0.0, 10.0, 0.8, 20.0) > 10.0


def test_coverage_fraction():
    lows = [0, 0, 0]
    highs = [10, 10, 10]
    levels = [0.8, 0.8, 0.8]
    values = [5, 50, 5]  # 2 of 3 inside
    assert ana.coverage(lows, highs, levels, values) == pytest.approx(2 / 3)


def test_calibration_curve_empty_bin_is_none():
    rows = ana.calibration_curve([0.95, 0.95], [1, 1], n_bins=10)
    assert len(rows) == 10
    populated = [b for b in rows if b.count > 0]
    assert len(populated) == 1 and populated[0].observed == pytest.approx(1.0)
    # an empty bin reports observed=None, never NaN
    empties = [b for b in rows if b.count == 0]
    assert empties and all(b.observed is None for b in empties)


def test_validation_errors():
    with pytest.raises(ValueError):
        ana.brier([0.5, 0.5], [1])  # length mismatch
    with pytest.raises(ValueError):
        ana.brier([1.5], [1])  # probability out of [0, 1]
    with pytest.raises(ValueError):
        ana.brier([float("nan")], [1])  # NaN probability


def test_report_dict_shape():
    r = ana.report(PROBS, OUT)
    assert r["n"] == 5
    assert r["brier"] == pytest.approx(0.11)
    assert r["base_rate"] == pytest.approx(0.6)
    assert len(r["base_rate_ci95"]) == 2


def test_calibration_repr_classifies():
    c = ana.Calibration([0.95, 0.95, 0.95], [0, 0, 0])
    assert "overconfident" in repr(c)


def test_bools_and_numpy_inputs_work():
    # bools as outcomes
    assert ana.brier([0.9, 0.1], [True, False]) == pytest.approx(0.01)
    np = pytest.importorskip("numpy")
    p = np.array([0.9, 0.8, 0.3, 0.6, 0.5])
    o = np.array([1, 1, 0, 1, 0])
    assert ana.brier(p, o) == pytest.approx(0.11)
    assert ana.auc(p, o) == ana.auc(PROBS, OUT)
