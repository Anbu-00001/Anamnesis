#!/usr/bin/env python3
"""Does refusing a bare test run after an edit change what the agent hands over?

    python3 validation/audit_report.py [--ledger PATH] [--today YYYY-MM-DD]
    python3 validation/audit_report.py --selftest

Standard library only. It reads outcomes.jsonl (and trial.json, if one is enrolled) beside
the agent ledger and writes nothing. The rules are fixed in docs/MEASUREMENT.md, addendum 5,
part B, before the data exist:

  1. From 2026-12-20 (UTC) this machine runs ANAMNESIS_AUDIT_RATE=0.5. Every prompt turn is
     drawn with chance 0.5 by `pin::audit_drawn`, a hash of the session id and the prompt
     number, ported below bit for bit (src/pin.rs asserts the same 64 draws). In a drawn
     turn every bare test run after an edit is refused until it goes through `ana run`.
  2. Unit: the edit-turn, built exactly as addendum 4 builds it (crossover_report.edit_turns).
     Every edit-turn in the window is assigned by its draw, whether or not it ran a test
     (intent to treat). Days in an OFF week of the weekly trial are left out: the refusal is
     silent there.
  3. Primary outcome: the handoff failure rate, the share of edit-turns whose first graded run
     after the last edit failed. Test: re-draw every turn's assignment from Bernoulli(0.5),
     10,000 times with a fixed seed, outcomes held fixed; two-sided at 0.05.
  4. One confirmatory reading, at the end of addendum 4's last pair, or on 2027-07-05 when no
     trial is enrolled. No comparison with fewer than 30 graded edit-turns in either arm.
  5. Secondary outcomes, all exploratory: the share of edit-turns with no later graded run
     (does a refusal make the agent stop testing?), the share with a weakening test edit
     (does pressure raise test gaming?), test runs per edit-turn, the share of runs through
     `ana run`, and minutes to green.
"""
import argparse
import datetime as dt
import os
import random
import statistics
import sys
import tempfile

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from crossover_report import arm_of, edit_turns, load_trial, parse_ts, read_lines, weakening  # noqa: E402

RATE = 0.5
START = dt.date(2026, 12, 20)
FALLBACK_END = dt.date(2027, 7, 5)
MIN_PER_ARM = 30
REDRAWS = 10_000
SEED = 20261220
ALPHA = 0.05
M64 = (1 << 64) - 1


def audit_drawn(session, prompt, p):
    """src/pin.rs `audit_drawn`: FNV-1a with its own basis over the session's bytes, the
    prompt number as 8 little-endian bytes and b"audit", then a SplitMix64 finaliser."""
    h = 0x6A09E667F3BCC908
    for b in session.encode() + int(prompt).to_bytes(8, "little") + b"audit":
        h ^= b
        h = (h * 0x100000001B3) & M64
    z = (h + 0x9E3779B97F4A7C15) & M64
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M64
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M64
    z ^= z >> 31
    return ((z >> 11) / float(1 << 53)) < p


def turns_in_window(events, trial, today):
    for e in events:
        e["_t"] = parse_ts(e["at"])
    runs = [e for e in events if e.get("kind") == "run"]
    out = []
    for t in edit_turns(events):
        day = t["at"].date()
        if day < START or day > today:
            continue
        if trial is not None:
            a = arm_of(trial, day)
            if a is not None and a[0] == "off":
                continue
        key = (t["session"], t["prompt"], t["repo"])
        in_turn = [r for r in runs if (r.get("session"), r.get("prompt"), r.get("repo")) == key]
        out.append({
            **t,
            "drawn": audit_drawn(t["session"] or "default", t["prompt"] or 0, RATE),
            "ana_runs": sum(1 for r in in_turn if r.get("via") == "ana_run"),
            "all_runs": len(in_turn),
        })
    return out


def rate(turns):
    g = [t for t in turns if t["failed"] is not None]
    return (sum(t["failed"] for t in g) / len(g)) if g else None, len(g)


def randomization_p(turns, seed=SEED, redraws=REDRAWS):
    """Two-sided p for (not drawn minus drawn) failure rate, re-drawing assignments."""
    g = [t["failed"] for t in turns if t["failed"] is not None]
    obs_assign = [t["drawn"] for t in turns if t["failed"] is not None]

    def diff(assign):
        on = [f for f, a in zip(g, assign) if a]
        off = [f for f, a in zip(g, assign) if not a]
        if not on or not off:
            return None
        return sum(off) / len(off) - sum(on) / len(on)

    obs = diff(obs_assign)
    if obs is None:
        return None, None
    rng = random.Random(seed)
    hits = total = 0
    for _ in range(redraws):
        d = diff([rng.random() < RATE for _ in g])
        if d is None:
            continue
        total += 1
        hits += abs(d) >= abs(obs) - 1e-12
    return obs, (hits + 1) / (total + 1)


def share(xs):
    return (sum(xs) / len(xs)) if xs else None


def fmt(x, spec="{:.3f}"):
    return "—" if x is None else spec.format(x)


def report(events, trial, today, out=sys.stdout):
    p = lambda *a: print(*a, file=out)
    turns = turns_in_window(events, trial, today)
    p(f"Audit experiment (docs/MEASUREMENT.md, addendum 5, part B): rate {RATE} from {START}")
    if not turns:
        p("No edit-turn in the window yet.")
        return
    p("")
    p(" arm        edit-turns  graded  fail-rate  no-later-run  weakened  runs/turn  via-ana-run  min-to-green")
    for name, arm in (("drawn", [t for t in turns if t["drawn"]]), ("not drawn", [t for t in turns if not t["drawn"]])):
        r, n = rate(arm)
        mins = [t["minutes_to_green"] for t in arm if t["minutes_to_green"] is not None]
        runs = sum(t["all_runs"] for t in arm)
        p(f" {name:<9}  {len(arm):>10}  {n:>6}  {fmt(r):>9}  "
          f"{fmt(share([t['failed'] is None for t in arm])):>12}  {fmt(share([t['weakened'] for t in arm])):>8}  "
          f"{fmt(share([t['runs_in_turn'] for t in arm]), '{:.2f}'):>9}  "
          f"{fmt(sum(t['ana_runs'] for t in arm) / runs if runs else None):>11}  "
          f"{fmt(statistics.median(mins) if mins else None, '{:.1f}'):>12}")
    p("")
    end = FALLBACK_END
    if trial is not None:
        end = dt.date.fromisoformat(trial["start"]) + dt.timedelta(days=14 * len(trial["on_first"]))
    _, n_on = rate([t for t in turns if t["drawn"]])
    _, n_off = rate([t for t in turns if not t["drawn"]])
    if min(n_on, n_off) < MIN_PER_ARM:
        p(f"Fewer than {MIN_PER_ARM} graded edit-turns in an arm: no comparison (rule 4).")
        return
    if today < end:
        p(f"Exploratory until {end}: the p-value is read once, then (rule 4).")
        return
    d, pv = randomization_p(turns)
    p(f"Confirmatory reading: not drawn minus drawn = {d:+.3f}, randomization p = {pv:.4f} "
      "(positive means more handoff failures without the refusal)")
    if pv > ALPHA:
        p("A null: no difference this test can see, which is not proof of none.")


def selftest():
    bits = "".join("1" if audit_drawn(f"s{i % 5}", i, 0.5) else "0" for i in range(64))
    assert bits == "0100000011100001010011101001011000010110100011011110111110000110", bits
    assert all(audit_drawn("s", n, 1.0) for n in range(50))
    assert weakening({"class": "test", "delta": [0, -1, 1]})
    assert not weakening({"class": "test", "delta": [1, 2, 0]})
    assert not weakening({"class": "code", "delta": [-1, 0, 0]})
    assert not weakening({"class": "test", "delta": None})
    # 200 turns where drawn turns never fail and undrawn ones always do.
    events = []
    for i in range(200):
        s, n = f"s{i % 7}", i
        t = dt.datetime(2027, 1, 2, 9) + dt.timedelta(minutes=2 * i)
        fail = not audit_drawn(s, n, RATE)
        events.append({"kind": "edit", "at": t.isoformat() + "Z", "session": s, "prompt": n, "repo": "r",
                       "class": "test", "delta": [0, -1, 0] if i % 10 == 0 else [0, 0, 0]})
        events.append({"kind": "run", "at": (t + dt.timedelta(seconds=30)).isoformat() + "Z", "session": s,
                       "prompt": n, "repo": "r", "graded": "fail" if fail else "pass", "via": "bare"})
    turns = turns_in_window(events, None, dt.date(2027, 7, 6))
    assert len(turns) == 200 and sum(t["weakened"] for t in turns) == 20
    d, pv = randomization_p(turns, redraws=2000)
    assert d == 1.0 and pv < 0.01, (d, pv)
    with tempfile.TemporaryFile("w+") as f:
        report(events, None, dt.date(2027, 7, 6), out=f)
        f.seek(0)
        text = f.read()
    assert "Confirmatory reading" in text and "+1.000" in text, text
    # Before the window nothing counts.
    early = [dict(e, at=e["at"].replace("2027-01-02", "2026-12-01")) for e in events]
    assert turns_in_window(early, None, dt.date(2027, 7, 6)) == []
    print("selftest: ok")


def main():
    home = os.environ.get("ANAMNESIS_HOME") or os.path.join(os.path.expanduser("~"), ".anamnesis")
    ap = argparse.ArgumentParser()
    ap.add_argument("--ledger", default=os.environ.get("ANAMNESIS_AGENT_DATA") or os.path.join(home, "agent.json"))
    ap.add_argument("--today", default=None)
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        selftest()
        return 0
    base = os.path.dirname(os.path.abspath(a.ledger))
    tpath = os.path.join(base, "trial.json")
    trial = load_trial(tpath) if os.path.exists(tpath) else None
    today = dt.date.fromisoformat(a.today) if a.today else dt.datetime.now(dt.timezone.utc).date()
    report(read_lines(os.path.join(base, "outcomes.jsonl")), trial, today)
    return 0


if __name__ == "__main__":
    sys.exit(main())
