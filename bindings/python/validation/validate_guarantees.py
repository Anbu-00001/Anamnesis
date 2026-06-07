"""Empirically validate the statistical guarantees Anamnesis *claims*.

This is the project's own ethos turned on itself: don't trust the math because the
docstring says so — prove it by simulation, against the real compiled engine
(`import anamnesis`, the same Rust core the CLI runs). Three Monte-Carlo studies:

  1. The calibration **e-process is anytime-valid** — under the null (perfectly
     calibrated forecasts) the false-alarm rate stays ≤ α *even though you peek
     after every single outcome*. A naive repeated z-test, checked the same way,
     blows past α — the whole reason the e-process exists.
  2. The **bootstrap Brier band** has roughly its nominal coverage (and is a touch
     narrow at small n, exactly as the docstring warns).
  3. The **recalibration map** improves Brier *out-of-sample* on a miscalibrated
     forecaster (train/test split) — it genuinely corrects, not just overfits.

Run locally:   python validation/validate_guarantees.py
Run in Colab:  upload the wheel, then in a cell:
                   !pip install anamnesis-0.3.0-*.whl numpy
                   # paste this file, or: !python validate_guarantees.py
Only deps: numpy + the anamnesis wheel. No network, no GPU.
"""
from __future__ import annotations

import numpy as np

import anamnesis as ana

RNG = np.random.default_rng(20260607)


def _sigmoid(x):
    return 1.0 / (1.0 + np.exp(-x))


def _logit(p):
    p = np.clip(p, 1e-9, 1 - 1e-9)
    return np.log(p / (1 - p))


def _ok(cond: bool) -> str:
    return "PASS ✅" if cond else "FAIL ❌"


# ─────────────────────────────────────────────────────────────────────────────
def study_1_eprocess_anytime_validity(reps=1000, n=60, alpha=0.05):
    """Under the null, P(e-value ever ≥ 1/α while peeking every step) ≤ α."""
    thresh = 1.0 / alpha  # e ≥ 20 ⇔ α = 0.05
    e_false_alarms = 0
    z_false_alarms = 0  # the naive fixed-n test, checked the same (invalid) way
    for _ in range(reps):
        p = RNG.uniform(0.05, 0.95, n)
        y = (RNG.uniform(size=n) < p).astype(float)  # calibrated by construction

        # e-process: peek after every outcome, record if it ever crosses.
        crossed = False
        for k in range(2, n + 1):
            e = ana.calibration_eprocess(p[:k].tolist(), y[:k].tolist())
            if e is not None and e >= thresh:
                crossed = True
                break
        e_false_alarms += crossed

        # naive repeated z-test on calibration-in-the-large, same peeking.
        num = np.cumsum(y - p)
        den = np.sqrt(np.cumsum(p * (1 - p)))
        z = np.divide(num, den, out=np.zeros_like(num), where=den > 0)
        z_false_alarms += bool(np.any(np.abs(z[1:]) > 1.96))

    e_rate = e_false_alarms / reps
    z_rate = z_false_alarms / reps
    print("\n[1] e-process anytime-validity  (null = perfectly calibrated)")
    print(f"    reps={reps}, n={n}, peeking after EVERY outcome, target α={alpha}")
    print(f"    e-process false-alarm rate : {e_rate:.3f}   (should be ≤ {alpha})")
    print(f"    naive z-test false-alarm   : {z_rate:.3f}   (fixed-n test, invalidated by peeking)")
    # Allow a little Monte-Carlo slack on the e-process; require the contrast to hold.
    valid = e_rate <= alpha + 0.02
    contrast = z_rate > e_rate + 0.03
    print(f"    {_ok(valid)} e-process stays valid under continuous monitoring")
    print(f"    {_ok(contrast)} naive test inflates — the e-process earns its keep")
    return valid and contrast


# ─────────────────────────────────────────────────────────────────────────────
def study_2_bootstrap_coverage(reps=1500):
    """The 95% bootstrap band should contain the true expected Brier ~95% of the time."""
    print("\n[2] bootstrap Brier band coverage  (nominal 95%)")
    all_ok = True
    for n in (30, 150):
        p = RNG.uniform(0.05, 0.95, n)  # fixed, calibrated forecaster (q = p)
        true_brier = float(np.mean(p * (1 - p)))  # E[(p−y)^2] with q=p
        hits = 0
        for _ in range(reps):
            y = (RNG.uniform(size=n) < p).astype(float)
            ci = ana.brier_ci_bootstrap(p.tolist(), y.tolist())
            if ci is not None and ci[0] <= true_brier <= ci[1]:
                hits += 1
        cov = hits / reps
        # Percentile bootstrap is known to be a touch narrow at small n.
        floor = 0.88 if n < 50 else 0.92
        ok = cov >= floor
        all_ok &= ok
        print(f"    n={n:>3}: coverage {cov:.3f}  (≥ {floor} expected) {_ok(ok)}")
    print("    (slightly-low coverage at n=30 is the documented small-n narrowness)")
    return all_ok


# ─────────────────────────────────────────────────────────────────────────────
def study_3_recalibration_helps_out_of_sample(reps=300, n=400, b_true=2.0):
    """A too-timid forecaster, recalibrated on train, should score better on test."""
    print("\n[3] recalibration improves out-of-sample Brier  (underconfident forecaster)")
    wins = 0
    raw_acc, cal_acc = [], []
    for _ in range(reps):
        p_tr = RNG.uniform(0.05, 0.95, n)
        q_tr = _sigmoid(b_true * _logit(p_tr))  # true prob is more extreme → timid stated p
        y_tr = (RNG.uniform(size=n) < q_tr).astype(float)

        rec = ana.fit_recalibration(p_tr.tolist(), y_tr.tolist())

        p_te = RNG.uniform(0.05, 0.95, n)
        q_te = _sigmoid(b_true * _logit(p_te))
        y_te = (RNG.uniform(size=n) < q_te).astype(float)

        raw = ana.brier(p_te.tolist(), y_te.tolist())
        corrected = [rec.apply(p) for p in p_te.tolist()]
        cal = ana.brier(corrected, y_te.tolist())
        wins += cal < raw
        raw_acc.append(raw)
        cal_acc.append(cal)

    win_rate = wins / reps
    print(f"    reps={reps}, n={n}, true slope b={b_true}")
    print(f"    mean test Brier  raw {np.mean(raw_acc):.4f}  →  recalibrated {np.mean(cal_acc):.4f}")
    print(f"    recalibrated wins {win_rate:.1%} of the time")
    ok = win_rate > 0.9 and np.mean(cal_acc) < np.mean(raw_acc)
    print(f"    {_ok(ok)} the learned map genuinely corrects out-of-sample")
    return ok


if __name__ == "__main__":
    print(f"anamnesis {ana.__version__} — validating statistical guarantees by simulation")
    results = [
        study_1_eprocess_anytime_validity(),
        study_2_bootstrap_coverage(),
        study_3_recalibration_helps_out_of_sample(),
    ]
    print("\n" + "=" * 60)
    print(f"OVERALL: {_ok(all(results))}  ({sum(results)}/{len(results)} studies passed)")
