"""Simulations backing P0-3 (e-process power and null validity) and P0-4 (CORP).

Pure numpy, no binary needed. These are the numbers quoted in docs/METHODS.md.
Run: python3 validation/sims.py
"""
import numpy as np
rng = np.random.default_rng(11)
LAMS = np.array([-0.9,-0.7,-0.5,-0.3,-0.1,0.1,0.3,0.5,0.7,0.9])

def path_current(p, y):
    return np.exp(np.cumsum(np.log1p(np.outer(y - p, LAMS)), axis=0)).mean(axis=1)

def path_proposed(p, y):
    z = y - p
    hs = [np.ones_like(p), np.where(p < .5, 1., np.where(p > .5, -1., 0.)), 2*(.5 - p)]
    ws = [np.exp(np.cumsum(np.log1p(np.outer(h*z, LAMS)), axis=0)).sum(axis=1) for h in hs]
    return sum(ws) / (len(hs) * len(LAMS))

def alarm_rate(gen, path, n, reps=600, thr=20., every=5):
    return sum((path(*gen(n))[every-1::every] >= thr).any() for _ in range(reps)) / reps

sym  = lambda n: (lambda p: (p, (rng.uniform(size=n) < np.where(p > .5, .65, .35)).astype(float)))(
                  np.where(rng.uniform(size=n) < .5, .9, .1))
cal  = lambda n: (lambda p: (p, (rng.uniform(size=n) < p).astype(float)))(
                  np.round(rng.uniform(.02, .98, n), 2))

print("== P0-3: power against symmetric overconfidence, and the null false-alarm rate ==")
print("   (Ville's inequality caps the null rate at 1/20 = 0.05 at ANY stopping time)")
for n in (50, 100, 300):
    print(f"n={n:4d}  symmetric overconfidence: current {alarm_rate(sym, path_current, n):.3f}"
          f"  proposed {alarm_rate(sym, path_proposed, n):.3f}"
          f"   |  null: current {alarm_rate(cal, path_current, n):.3f}"
          f"  proposed {alarm_rate(cal, path_proposed, n):.3f}")

def pav(x, y):
    o = np.argsort(x, kind="mergesort"); xs, ys = x[o], y[o]
    _, start = np.unique(xs, return_index=True)
    counts = np.diff(np.append(start, len(xs))); sums = np.add.reduceat(ys, start)
    blocks = []
    for s, c in zip(sums, counts):
        blocks.append([s, c, 1])
        while len(blocks) > 1 and blocks[-2][0]/blocks[-2][1] > blocks[-1][0]/blocks[-1][1]:
            s2, c2, g2 = blocks.pop()
            blocks[-1][0] += s2; blocks[-1][1] += c2; blocks[-1][2] += g2
    fit = np.repeat(np.concatenate([[b[0]/b[1]]*b[2] for b in blocks]), counts)
    out = np.empty_like(fit); out[o] = fit
    return out

brier = lambda p, y: np.mean((p - y)**2)
def mcb(p, y): return brier(p, y) - brier(pav(p, y), y)
def exact_rel(p, y):
    v, inv = np.unique(p, return_inverse=True)
    return sum((inv==k).sum() * (x - y[inv==k].mean())**2 for k, x in enumerate(v)) / len(p)

print()
print("== P0-4: calibration error reported for a PERFECTLY calibrated 2-dp forecaster ==")
print("   (the true value is 0.000; anything above that is the metric's own noise)")
for n in (20, 50, 200, 1000):
    e, m = [], []
    for _ in range(400):
        p = np.round(rng.uniform(.02, .98, n), 2); y = (rng.uniform(size=n) < p).astype(float)
        e.append(exact_rel(p, y)); m.append(mcb(p, y))
    print(f"n={n:5d} calibrated, 2dp:  exact-group REL {np.mean(e):.3f}   CORP MCB {np.mean(m):.3f}")
