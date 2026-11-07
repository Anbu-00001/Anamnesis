#!/usr/bin/env python3
"""Pool the weekly trial across volunteers (card D), with no network and nothing shared but one line each.

    python3 validation/pool_evalues.py line1.json line2.json ...   (or lines on stdin)
    python3 validation/pool_evalues.py --selftest

Each line is what `crossover_report.py --export` prints. Only lines that ran the same rules
(the same `protocol_sha256`) are pooled. Two answers, each valid on its own terms:

  * the mean of the e-values, valid whatever the dependence between people (a mean of
    e-values is an e-value), and their product, valid only if the people are independent;
  * a sign-flip randomization test over every pair from every person, exact by enumeration
    up to 20 pairs and Monte Carlo (seeded) above.

Nobody can check a line. A pooled result is as honest as the people who sent it, and the
people who send one are the ones who chose to: a selected sample, read as such.
"""
import argparse
import itertools
import json
import math
import random
import statistics
import sys

ALPHA = 0.05


def sign_flip(ds, draws=100000, seed=20261107):
    k = len(ds)
    obs = abs(sum(ds))
    if k <= 20:
        hits = sum(1 for s in itertools.product((1, -1), repeat=k)
                   if abs(sum(a * d for a, d in zip(s, ds))) >= obs - 1e-12)
        return hits / 2 ** k, "exact"
    rng = random.Random(seed)
    hits = sum(1 for _ in range(draws)
               if abs(sum(d if rng.random() < 0.5 else -d for d in ds)) >= obs - 1e-12)
    return (hits + 1) / (draws + 1), f"Monte Carlo, {draws} draws"


def pool(lines, out=sys.stdout):
    p = lambda *a: print(*a, file=out)
    groups = {}
    for l in lines:
        groups.setdefault(l.get("protocol_sha256"), []).append(l)
    if len(groups) > 1:
        p(f"{len(groups)} different protocol texts; only the largest group is pooled.")
    sha, group = max(groups.items(), key=lambda kv: len(kv[1]))
    es = [float(l["e_value"]) for l in group]
    ds = [d for l in group for d in l["differences"]]
    p(f"{len(group)} volunteer(s) on protocol {str(sha)[:12]}, {len(ds)} pair(s) in all, "
      f"{sum(1 for l in group if l.get('complete'))} trial(s) complete")
    p(f"mean e-value {statistics.fmean(es):.2f} (valid under any dependence; 20 or more is evidence at 0.05)")
    p(f"product of e-values {math.prod(es):.2f} (valid only if the volunteers are independent)")
    if ds:
        pv, how = sign_flip(ds)
        p(f"pooled sign-flip p = {pv:.4f} ({how}); mean difference OFF minus ON {statistics.fmean(ds):+.3f}")
    p("Every line is self-reported and self-selected; read the result as that.")
    return statistics.fmean(es), math.prod(es)


def selftest():
    import io
    a = {"protocol_sha256": "x", "e_value": 4.0, "differences": [0.1, 0.2], "complete": True}
    b = {"protocol_sha256": "x", "e_value": 9.0, "differences": [0.3], "complete": False}
    c = {"protocol_sha256": "other", "e_value": 1000.0, "differences": [1.0], "complete": True}
    buf = io.StringIO()
    mean, prod = pool([a, b, c], out=buf)
    assert (mean, prod) == (6.5, 36.0), (mean, prod, buf.getvalue())
    assert "only the largest group" in buf.getvalue()
    assert sign_flip([0.1, 0.2, 0.3]) == (0.25, "exact")
    print("selftest: ok")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("files", nargs="*")
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        selftest()
        return 0
    texts = [open(f).read() for f in a.files] if a.files else [sys.stdin.read()]
    lines = [json.loads(l) for t in texts for l in t.splitlines() if l.strip()]
    if not lines:
        print("no lines to pool")
        return 0
    pool(lines)
    return 0


if __name__ == "__main__":
    sys.exit(main())
