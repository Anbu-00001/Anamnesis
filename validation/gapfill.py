#!/usr/bin/env python3
"""Stopping at gaps vs pricing them: the measurement behind `evidence.rs`.

    python3 validation/gapfill.py

Reproduces the table in docs/METHODS.md section 3b. Mirrors the Rust mixture
(3 betting strategies x 10 lambdas) exactly, so a disagreement here is a real
disagreement with the shipped code.
"""

import numpy as np
LAM = np.array([-0.9,-0.7,-0.5,-0.3,-0.1,0.1,0.3,0.5,0.7,0.9])
ALARM = np.log(20.0)

def H(p):
    return (1.0, 1.0 if p < 0.5 else (-1.0 if p > 0.5 else 0.0), 2.0*(0.5-p))

def mix(logw):
    a = logw.ravel(); m = a.max()
    return m + np.log(np.exp(a-m).sum()) - np.log(a.size)

def sim(seed, n, grade_rate, alt, check_every=5):
    rng = np.random.default_rng(seed)
    if alt == "null":
        ps = rng.uniform(0.05,0.95,n); truth = ps
    elif alt == "symmetric":
        ps = rng.choice([0.1,0.9],n); truth = np.where(ps>0.5,0.65,0.35)
    else:
        ps = rng.uniform(0.05,0.95,n); truth = 0.5+0.7*(ps-0.5)
    ys = (rng.uniform(size=n) < truth).astype(float)
    graded = rng.uniform(size=n) < grade_rate

    wb = np.zeros((3,len(LAM)));  blocked = False; nb = 0
    wg = np.zeros((3,len(LAM)));  ng = 0
    peak_b = peak_g = -np.inf
    for i in range(n):
        p, y, g = ps[i], ys[i], graded[i]
        hs = H(p)
        if not blocked:
            if g:
                for k,h in enumerate(hs): wb[k] += np.log1p(LAM*(h*(y-p)))
                nb += 1
            else:
                blocked = True
        for k,h in enumerate(hs):
            if g: wg[k] += np.log1p(LAM*(h*(y-p)))
            else: wg[k] += np.log(np.minimum(1.0+LAM*(h*(1.0-p)), 1.0+LAM*(h*(0.0-p))))
        if g: ng += 1
        if (i+1) % check_every == 0:
            peak_b = max(peak_b, mix(wb)); peak_g = max(peak_g, mix(wg))
    return (peak_b>=ALARM, nb, mix(wb)), (peak_g>=ALARM, ng, mix(wg))

REPS = 300
print(f"{'graded':>7} {'rule':>10} {'usable n':>9} {'detect sym':>11} {'detect gen':>11} {'false alarm':>12} {'median final e':>15}")
for gr in (1.00, 0.90, 0.65):
    acc = {r:{'ds':[], 'dg':[], 'fa':[], 'n':[], 'e':[]} for r in ('blocking','gapfilled')}
    for s in range(REPS):
        b,g = sim(s, 300, gr, "symmetric");    acc['blocking']['ds'].append(b[0]); acc['gapfilled']['ds'].append(g[0])
        acc['blocking']['n'].append(b[1]);     acc['gapfilled']['n'].append(g[1])
        acc['blocking']['e'].append(b[2]);     acc['gapfilled']['e'].append(g[2])
        b,g = sim(s+9000, 300, gr, "general"); acc['blocking']['dg'].append(b[0]); acc['gapfilled']['dg'].append(g[0])
        b,g = sim(s+5000, 300, gr, "null");    acc['blocking']['fa'].append(b[0]); acc['gapfilled']['fa'].append(g[0])
    for r in ('blocking','gapfilled'):
        a = acc[r]
        print(f"{gr:>7.0%} {r:>10} {np.mean(a['n']):>9.1f} {np.mean(a['ds']):>11.3f} {np.mean(a['dg']):>11.3f} {np.mean(a['fa']):>12.3f} {np.exp(np.median(a['e'])):>15.3g}")
