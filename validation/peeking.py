"""Finding F: the e-process ordering must not depend on the outcome.

Re-runs `ana report` the way a user actually experiences it — once more each time
a few more claims resolve — on a forecaster who is PERFECTLY CALIBRATED. Any
alarm is a false alarm by construction.

Run: cargo build --release && python3 validation/peeking.py
"""
import json, datetime, random, subprocess, sys

ANA = "./target/release/ana"
T0 = datetime.datetime(2025, 1, 1, tzinfo=datetime.timezone.utc)
TMP = "_peek.json"

def build(n=60, seed=0):
    """One batch of 'will X happen by <same deadline>?' questions: YES resolves
    the day it happens, NO waits for the deadline. That asymmetry is ordinary,
    and it is what makes resolution-time ordering outcome-dependent."""
    rng, cl = random.Random(seed), []
    for i in range(n):
        p = round(rng.uniform(0.05, 0.95), 2); y = rng.random() < p
        created = T0 + datetime.timedelta(seconds=i)
        deadline = T0 + datetime.timedelta(days=30)
        res = (created + datetime.timedelta(days=rng.uniform(0, 29))) if y else \
              (deadline + datetime.timedelta(seconds=i))
        iso = lambda d: d.isoformat().replace("+00:00", "Z")
        cl.append(dict(id=f"c{i:04d}", statement=f"c{i}", created_at=iso(created),
                       resolve_by=deadline.date().isoformat(), tags=[], kind="binary",
                       forecasts=[dict(at=iso(created), prob=p)],
                       resolution=dict(at=iso(res), outcome="true" if y else "false"),
                       _res=res, _created=created))
    return cl

def evalue(claims, path=TMP):
    json.dump({"claims": [{k: v for k, v in c.items() if not k.startswith("_")}
                          for c in claims]}, open(path, "w"))
    out = subprocess.run([ANA, "--json", "--data", path, "report"],
                         capture_output=True, text=True).stdout
    d = json.loads(out)
    return d.get("eprocess") or 0.0

def peak(claims, key, every=5):
    o = sorted(claims, key=key)
    return max(evalue(o[:k]) for k in range(every, len(o) + 1, every))

seeds = range(int(sys.argv[1]) if len(sys.argv) > 1 else 40)
res = [peak(build(seed=s), key=lambda c: c["_res"])     for s in seeds]
dl  = [peak(build(seed=s), key=lambda c: c["_created"]) for s in seeds]
alarm = lambda v: sum(x >= 20 for x in v) / len(v)
med = lambda v: sorted(v)[len(v)//2]
print("resolution order (outcome-dependent):", f"{alarm(res):.0%}", "median peak e", f"{med(res):.2f}")
print("deadline order   (outcome-independent):", f"{alarm(dl):.0%}", "median peak e", f"{med(dl):.2f}")
