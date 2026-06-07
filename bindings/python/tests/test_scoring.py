"""Cross-validation tests for the Python binding.

Because the binding calls the *same* Rust code as the CLI, parity is guaranteed
by construction; these tests instead guard the wrapper itself — argument
marshalling, None/NaN handling, validation — against hand-computed values and
known identities that also appear in the Rust unit tests.
"""
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


def test_eprocess_distinguishes_calibrated_from_miscalibrated():
    # Alternating outcomes at p=0.5 → no evidence; "10% sure" but always true → lots.
    calibrated = ana.calibration_eprocess([0.5] * 60, [i % 2 for i in range(60)])
    assert calibrated < 5.0
    assert ana.eprocess_pvalue(calibrated) > 0.05
    gross = ana.calibration_eprocess([0.1] * 40, [1] * 40)
    assert gross >= 20.0
    assert ana.eprocess_pvalue(gross) < 0.05
    assert ana.calibration_eprocess([], []) is None


def test_recalibration_fit_and_apply():
    # Underconfident: stated 0.6 → true 0.9, stated 0.4 → true 0.1.
    probs = [0.6] * 100 + [0.4] * 100
    outs = [1] * 90 + [0] * 10 + [1] * 10 + [0] * 90
    r = ana.fit_recalibration(probs, outs)
    assert r.b > 1.0  # too timid
    assert r.apply(0.6) > 0.75
    assert r.apply(0.4) < 0.25
    assert "Recalibration" in repr(r)
    assert ana.fit_recalibration([], []) is None


def test_recalibration_identity_on_calibrated_data():
    probs = [0.7] * 100 + [0.3] * 100
    outs = [1] * 70 + [0] * 30 + [1] * 30 + [0] * 70
    r = ana.fit_recalibration(probs, outs, ridge=1.0)
    assert abs(r.apply(0.7) - 0.7) < 0.05
    assert abs(r.apply(0.3) - 0.3) < 0.05


def test_brier_ci_brackets_and_is_deterministic():
    probs = [0.7] * 40
    outs = [1 if i % 10 < 7 else 0 for i in range(40)]
    b = ana.brier(probs, outs)
    lo, hi = ana.brier_ci_bootstrap(probs, outs)
    assert lo <= b <= hi
    assert ana.brier_ci_bootstrap(probs, outs) == (lo, hi)  # reproducible
    assert ana.brier_ci_bootstrap([0.5], [1]) is None  # < 2 samples


def test_ewma_brier_rewards_recency():
    s = [(0.0, 1)] * 10 + [(1.0, 1)] * 10  # wrong early, perfect late
    probs = [p for p, _ in s]
    outs = [o for _, o in s]
    assert ana.ewma_brier(probs, outs, half_life=5.0) < ana.brier(probs, outs)
    assert ana.ewma_brier([], []) is None


def test_distinct_forecasts():
    assert ana.distinct_forecasts([0.6, 0.6, 0.3]) == 2
    assert ana.distinct_forecasts([]) == 0


def test_risk_coverage_rewards_good_ranking():
    # Confident calls right, unsure calls wrong → surest half near-perfect.
    probs = [0.95] * 10 + [0.55] * 10
    outs = [1] * 10 + [0] * 10
    rc = ana.risk_coverage(probs, outs)
    assert rc.risk_full == pytest.approx(0.5)
    assert rc.risk_half < 0.05
    assert rc.aurcc < rc.risk_full
    curve = ana.risk_coverage_curve(probs, outs)
    assert len(curve) == 20 and curve[-1] == (pytest.approx(1.0), pytest.approx(0.5))
    assert ana.risk_coverage([], []) is None


def test_brier_weighted():
    probs, outs = [1.0, 0.0], [1, 1]  # briers 0 and 1
    assert ana.brier_weighted(probs, outs, [1, 1]) == pytest.approx(0.5)
    assert ana.brier_weighted(probs, outs, [1, 9]) == pytest.approx(0.9)
    assert ana.brier_weighted(probs, outs, [2, 2]) == pytest.approx(ana.brier(probs, outs))
    assert ana.brier_weighted(probs, outs, [0, 0]) is None
    assert ana.brier_weighted(probs, outs, [1]) is None


def test_bools_and_numpy_inputs_work():
    # bools as outcomes
    assert ana.brier([0.9, 0.1], [True, False]) == pytest.approx(0.01)
    np = pytest.importorskip("numpy")
    p = np.array([0.9, 0.8, 0.3, 0.6, 0.5])
    o = np.array([1, 1, 0, 1, 0])
    assert ana.brier(p, o) == pytest.approx(0.11)
    assert ana.auc(p, o) == ana.auc(PROBS, OUT)
