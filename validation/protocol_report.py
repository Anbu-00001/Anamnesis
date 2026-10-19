#!/usr/bin/env python3
"""Read a week (or more) of the "pin first" protocol and say what it does and does not show.

    python3 validation/protocol_report.py [--ledger PATH] [--log PATH] [--since YYYY-MM-DD]

Standard library only. It reads two files and writes nothing:

  * the agent ledger (default ~/.anamnesis/agent.json), and
  * protocol.jsonl beside it, written by `ana hook pre-tool` when ANAMNESIS_PIN_NUDGE is on.
    One line per test run seen: when, session, project slug, model, runner kind, and what
    happened. Never a command, a path or any output.

The questions, the baseline and the rules for what may be concluded are fixed in
docs/MEASUREMENT.md BEFORE the data exists. This script implements those rules; it does not
get to choose them. Where a sample is too small it says so and prints no comparison.
"""
import argparse
import collections
import datetime as dt
import json
import os
import random
import sys

MIN_N = 20  # below this, no comparison is printed (docs/MEASUREMENT.md, rule 2)
# Before this date the logger saw `ana run` only at the start of the last `&&` part, which left
# 382 of 471 real calls out, and it credited subagent runs to the parent's model. Compliance
# from those events is not comparable with what came after (docs/MEASUREMENT.md, addendum 1).
LOGGER_FIXED = dt.datetime(2026, 10, 20, tzinfo=dt.timezone.utc)


def load_json(path):
    with open(path) as f:
        return json.load(f)


def parse_ts(s):
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


def first_p(c):
    f = c.get("forecasts") or []
    return f[0].get("prob") if f else None


def settled(c):
    """(p, y) for a resolved, non-void binary claim, else None. First forecast, as always."""
    r = c.get("resolution")
    if not r or c.get("void") or r.get("outcome") is None:
        return None
    p = first_p(c)
    if p is None:
        return None
    return p, 1.0 if r["outcome"] in (True, "true") else 0.0


def stats(pairs):
    n = len(pairs)
    if n == 0:
        return None
    return {
        "n": n,
        "brier": sum((p - y) ** 2 for p, y in pairs) / n,
        "mean_p": sum(p for p, _ in pairs) / n,
        "hit": sum(y for _, y in pairs) / n,
    }


def boot_ci(fn, data, reps=4000, seed=20261005):
    """Percentile bootstrap, seeded so the same data gives the same interval."""
    rng = random.Random(seed)
    xs = []
    for _ in range(reps):
        xs.append(fn([data[rng.randrange(len(data))] for _ in data]))
    xs.sort()
    return xs[int(0.025 * reps)], xs[int(0.975 * reps)]


def fmt(s):
    return (f"n={s['n']:<4} Brier {s['brier']:.3f}   said {s['mean_p']:.0%}   "
            f"came true {s['hit']:.0%}   gap (said - true) {s['mean_p'] - s['hit']:+.0%}")


def main():
    ap = argparse.ArgumentParser()
    home = os.path.expanduser("~/.anamnesis")
    ap.add_argument("--ledger", default=os.path.join(home, "agent.json"))
    ap.add_argument("--log", default=None)
    ap.add_argument("--since", default=None, help="only look at things on or after this date")
    a = ap.parse_args()
    log_path = a.log or os.path.join(os.path.dirname(a.ledger), "protocol.jsonl")
    since = parse_ts(a.since + "T00:00:00Z") if a.since else None

    ledger = load_json(a.ledger)
    claims = ledger["claims"]
    print(f"ledger: {a.ledger}  ({len(claims)} claims)")
    print(f"log:    {log_path}")

    # ───────────────────────── 1. did the protocol run, and who followed it ─────────────────────────
    print("\n== 1. The protocol: how often tests ran with a pinned prediction first ==")
    events = []
    if os.path.exists(log_path):
        for line in open(log_path):
            try:
                e = json.loads(line)
                e["_t"] = parse_ts(e["at"])
            except Exception:
                continue
            if since is None or e["_t"] >= since:
                events.append(e)
    if not events:
        print("  no events. Either the reminder was never switched on (ANAMNESIS_PIN_NUDGE), the hook is not")
        print("  registered, or no test command ran. Check that before reading anything else here.")
    else:
        early = sum(e["_t"] < LOGGER_FIXED for e in events)
        if early:
            print(f"  ! {early} of these events predate the logger fix of {LOGGER_FIXED:%Y-%m-%d}: that logger missed most")
            print("    `ana run` calls and misnamed subagents' models, so its share is not a compliance figure.")
            print(f"    Read Q2 with --since {LOGGER_FIXED:%Y-%m-%d} (docs/MEASUREMENT.md, addendum 1).")
        first, last = min(e["_t"] for e in events), max(e["_t"] for e in events)
        print(f"  {len(events)} test-run events, {first:%Y-%m-%d} to {last:%Y-%m-%d}, "
              f"{len({e['session'] for e in events})} sessions, {len({e['project'] for e in events})} projects")
        acts = collections.Counter(e["action"] for e in events)
        for k, v in acts.most_common():
            print(f"    {k:<22} {v}")
        print("  by model (a pinned run = the agent used `ana run`; the rest are bare runs):")
        per = collections.defaultdict(collections.Counter)
        for e in events:
            per[e.get("model") or "(model not reported)"][e["action"]] += 1
        for m, c in sorted(per.items()):
            tot = sum(c.values())
            pinned = c["ana_run"]
            print(f"    {m:<26} {tot:>4} runs, {pinned:>4} through ana run ({pinned / tot:.0%}), "
                  f"{c['denied_no_pin'] + c['denied_unrun_pin']:>3} refused once, {c['allowed_after_nudge']:>3} ignored it")
        if any("subagent" in e for e in events):
            for who, flag in (("main agent", False), ("subagents", True)):
                c = collections.Counter(e["action"] for e in events if e.get("subagent", False) == flag)
                tot = sum(c.values())
                if tot:
                    print(f"    {who:<26} {tot:>4} runs, {c['ana_run']:>4} through ana run"
                          + (f" ({c['ana_run'] / tot:.0%})" if tot >= 10 else " (under 10 runs: no percentage)"))
        # did a refusal change behaviour? an `ana_run` later in the same session
        by_session = collections.defaultdict(list)
        for e in events:
            by_session[e["session"]].append(e)
        refused = followed = 0
        for evs in by_session.values():
            evs.sort(key=lambda e: e["_t"])
            for i, e in enumerate(evs):
                if e["action"].startswith("denied"):
                    refused += 1
                    followed += any(x["action"] == "ana_run" for x in evs[i + 1:])
                    break
        if refused:
            print(f"  after being refused once, {followed} of {refused} sessions went on to use `ana run` "
                  f"({followed / refused:.0%})")

    # ───────────────────────── 2. what the ledger holds ─────────────────────────
    print("\n== 2. Machine-graded predictions in the ledger ==")

    def when(c):
        return parse_ts(c["created_at"])

    window = [c for c in claims if since is None or when(c) >= since]
    pinned = [c for c in window if c.get("check")]
    auto = [c for c in window if c.get("resolution") and c["resolution"].get("resolved_by") == "auto"]
    auto_tp = [c for c in auto if "kind:tests-pass" in c.get("tags", [])]
    print(f"  claims in the window: {len(window)}   pinned: {len(pinned)}   graded by exit status: {len(auto)}"
          f"   (of which kind:tests-pass: {len(auto_tp)})")
    projects = collections.Counter(
        next((t.split(':', 1)[1] for t in c.get("tags", []) if t.startswith("project:")), "(no project: tag)")
        for c in window)
    if projects:
        top, n_top = projects.most_common(1)[0]
        print(f"  {len(projects)} projects; the largest, {top}, holds {n_top} of {len(window)} claims ({n_top / len(window):.0%}): "
              + ", ".join(f"{p} {n}" for p, n in projects.most_common(6)))
    voided = [c for c in pinned if c.get("void")]
    unrun = [c for c in pinned if not c.get("resolution") and not c.get("void")]
    print(f"  pinned and never run: {len(unrun)}   pinned and voided: {len(voided)}   "
          f"(a pinned claim that is never run is an ungraded claim, priced into the evidence test)")
    models = collections.Counter(
        next((t.split(':', 1)[1] for t in c.get("tags", []) if t.startswith("model:")), "(no model: tag)")
        for c in auto)
    if models:
        print("  machine-graded, by the model tag the agent wrote: " + ", ".join(f"{m} {n}" for m, n in models.most_common()))

    # ───────────────────────── 3. calibration: machine vs self ─────────────────────────
    print("\n== 3. Does self-grading flatter? (kind:tests-pass only) ==")
    tests_pass = [c for c in claims if "kind:tests-pass" in c.get("tags", [])]
    machine = [s for c in tests_pass if c.get("resolution") and c["resolution"].get("resolved_by") == "auto"
               and (since is None or when(c) >= since) for s in [settled(c)] if s]
    by_hand = [s for c in tests_pass if c.get("resolution") and c["resolution"].get("resolved_by") != "auto"
               for s in [settled(c)] if s]
    sm, sh = stats(machine), stats(by_hand)
    print("  graded by hand (baseline, all time): " + (fmt(sh) if sh else "none"))
    print("  graded by exit status (window):      " + (fmt(sm) if sm else "none yet"))
    if sm and sm["n"] >= MIN_N and sh and sh["n"] >= MIN_N:
        rng = random.Random(20261005)
        ds = []
        for _ in range(4000):
            m = [machine[rng.randrange(len(machine))] for _ in machine]
            h = [by_hand[rng.randrange(len(by_hand))] for _ in by_hand]
            ds.append(sum(y for _, y in h) / len(h) - sum(y for _, y in m) / len(m))
        ds.sort()
        lo, hi = ds[100], ds[3900]
        print(f"  hit rate by hand minus by exit status: {sh['hit'] - sm['hit']:+.0%}   95% bootstrap interval [{lo:+.0%}, {hi:+.0%}]")
        if lo > 0:
            print("  -> the interval excludes zero: hand-graded claims came true more often than machine-graded ones.")
            print("     That is what flattering self-grading would look like, but the two sets are different tasks")
            print("     in different projects; see the confounds in docs/MEASUREMENT.md before saying so.")
        elif hi < 0:
            print("  -> the interval excludes zero the other way: machine-graded claims came true MORE often.")
        else:
            print("  -> the interval includes zero: no difference can be claimed.")
        top = collections.Counter(
            next((t.split(':', 1)[1] for t in c.get("tags", []) if t.startswith("project:")), None)
            for c in tests_pass if c.get("resolution") and c["resolution"].get("resolved_by") == "auto"
            and (since is None or when(c) >= since)).most_common(1)
        if top and top[0][0]:
            rest = [s for c in tests_pass if c.get("resolution") and c["resolution"].get("resolved_by") == "auto"
                    and (since is None or when(c) >= since) and f"project:{top[0][0]}" not in c.get("tags", [])
                    for s in [settled(c)] if s]
            sr = stats(rest)
            print(f"  exploratory, not the pre-registered test: exit-graded without {top[0][0]} ({top[0][1]} claims): "
                  + (fmt(sr) if sr and sr["n"] >= MIN_N else f"n={sr['n'] if sr else 0}, under {MIN_N}: nothing to read"))
        lo2, hi2 = boot_ci(lambda ps: sum(p for p, _ in ps) / len(ps) - sum(y for _, y in ps) / len(ps), machine)
        print(f"  machine-graded gap (said - true): {sm['mean_p'] - sm['hit']:+.0%}   95% interval [{lo2:+.0%}, {hi2:+.0%}]")
    else:
        need = []
        if not sm or sm["n"] < MIN_N:
            need.append(f"machine-graded n={sm['n'] if sm else 0} (need {MIN_N})")
        if not sh or sh["n"] < MIN_N:
            need.append(f"hand-graded n={sh['n'] if sh else 0} (need {MIN_N})")
        print("  no comparison printed: " + "; ".join(need) + ".")
        print("  (rule 2 in docs/MEASUREMENT.md: below that, any difference is noise and saying otherwise is the error)")

    print("\n== What this cannot show ==")
    print("  Whether the tool makes the agent better at anything: there is no control group here, and the")
    print("  README says that effect is unmeasured. This measures whether the protocol runs, whether it is")
    print("  followed, and whether hand-graded and exit-graded records differ.")


if __name__ == "__main__":
    sys.exit(main())
