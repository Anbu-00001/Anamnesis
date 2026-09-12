#!/usr/bin/env python3
"""How often the two instruments disagree, and the null distribution of the ratio.

    python3 validation/ratio.py        # needs the built wheel: see bindings/python

The MCB noise floor is a 95th percentile, so `mcb > floor` is a fixed-n test at
alpha = 0.05 -- about one calibrated forecaster in twenty crosses it on any given
look, and a user running `ana report` weekly crosses it within months with near
certainty. That is the peeking problem in the one instrument with no anytime-valid
protection, which is why the report prints the RATIO and reserves prose for
`report::MCB_RATIO_NOTABLE`. This script is where that constant comes from.
"""
import numpy as np
import anamnesis as ana

rng_master = np.random.default_rng(20260912)
DRAWS, Q, SEED = 400, 0.95, 0xA11CE5EED0000001

def one(n, mode, rng):
    p = rng.uniform(0.05, 0.95, n)
    if mode == "calibrated": truth = p
    elif mode == "diffuse":  truth = 0.5 + 0.7*(p-0.5)      # <=13.5 pts off
    else:                    truth = np.where(p > 0.5, p-0.25, p+0.25)
    y = (rng.uniform(size=n) < truth).astype(float)
    pl, yl = p.tolist(), y.tolist()
    mcb = ana.corp_brier(pl, yl).mcb
    floor = ana.mcb_null_quantile(pl, yl, DRAWS, Q, SEED)
    e = ana.calibration_eprocess_seq(pl, yl)
    return mcb/floor if floor and floor > 0 else float("nan"), e

REPS, N = 400, 200
print(f"{'forecaster':<14} {'both quiet':>11} {'MCB only':>10} {'both fire':>10} {'e only':>8}")
ratios = {}
for mode in ("calibrated", "diffuse", "sharp"):
    rng = np.random.default_rng(7)
    rs, cells = [], [0,0,0,0]
    for _ in range(REPS):
        r, e = one(N, mode, rng)
        rs.append(r)
        hi_m, hi_e = r > 1.0, e >= 20.0
        cells[0 if not hi_m and not hi_e else 1 if hi_m and not hi_e else 2 if hi_m and hi_e else 3] += 1
    ratios[mode] = np.array(rs)
    print(f"{mode:<14} {cells[0]/REPS:>11.2f} {cells[1]/REPS:>10.2f} {cells[2]/REPS:>10.2f} {cells[3]/REPS:>8.2f}")

print()
print("null distribution of the RATIO mcb/floor (calibrated forecaster, n=200):")
c = ratios["calibrated"]
for q in (50, 75, 90, 95, 99):
    print(f"  p{q:<3} {np.percentile(c, q):.2f}")
print()
for cut in (1.0, 1.25, 1.5, 2.0, 2.5):
    print(f"  P(ratio >= {cut:.2f}) under the null = {np.mean(c >= cut):.3f}"
          f"   | diffuse = {np.mean(ratios['diffuse'] >= cut):.3f}"
          f"   | sharp = {np.mean(ratios['sharp'] >= cut):.3f}")


# ── how the 1.0-1.5 band behaves as the record grows ────────────────────────
#
# The band the prose threshold leaves silent is transient: it peaks around
# n=500 and drains upward as cases graduate into prose. It should stay silent —
# at moderate n it holds calibrated forecasters as well as drifting ones, so
# annotating it would assert more than the data supports.
print()
print("the silent band as n grows:")
def ratio(n, mode, rng):
    p = rng.uniform(0.05, 0.95, n)
    truth = p if mode == "calibrated" else 0.5 + 0.7*(p-0.5)
    y = (rng.uniform(size=n) < truth).astype(float)
    pl, yl = p.tolist(), y.tolist()
    fl = ana.mcb_null_quantile(pl, yl, DRAWS, Q, SEED)
    return ana.corp_brier(pl, yl).mcb / fl if fl and fl > 0 else np.nan
REPS = 120
print(f"{'n':>6} {'mode':<12} {'median':>7} {'P(>=1.5) prose':>15} {'P(1.0-1.5) silent':>18}")
for n in (100, 200, 500, 1000):
    for mode in ("diffuse", "calibrated"):
        rng = np.random.default_rng(31)
        r = np.array([ratio(n, mode, rng) for _ in range(REPS)])
        print(f"{n:>6} {mode:<12} {np.median(r):>7.2f} {np.mean(r>=1.5):>15.3f} {np.mean((r>=1.0)&(r<1.5)):>18.3f}")
