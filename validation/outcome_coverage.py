#!/usr/bin/env python3
"""Does the outcome log see the test runs? The check before any trial reads it.

    python3 validation/outcome_coverage.py [--ledger PATH]
    python3 validation/outcome_coverage.py --selftest

Standard library only; reads two files beside the agent ledger and writes nothing.

`protocol.jsonl` is written by the pre-tool hook before a test run, `outcomes.jsonl` by the
post-tool hooks after it. Two hooks, two events, so each can miss what the other sees. Every
pre-tool line whose run went ahead (`ana_run`, `allowed_after_nudge`, `audit_not_drawn`,
`audit_not_eligible`) should have an outcome line from the same session and runner, run the
same way, within the hour after it. The rule (docs/MEASUREMENT.md, addendum 4, and the plan
it came from): at least 95% of them are matched over at least 14 days of both logs, or the
logger is fixed before any trial reads it.

A miss in the other direction (an outcome with no pre-tool line) is normal: the pre-tool log
only exists while the pin reminder or audit sampling is on.
"""
import argparse
import datetime as dt
import json
import os
import sys
import tempfile

RAN = {"ana_run", "allowed_after_nudge", "audit_not_drawn", "audit_not_eligible"}
WINDOW = dt.timedelta(hours=1)
SLACK = dt.timedelta(seconds=2)
TARGET = 0.95
MIN_DAYS = 14


def parse_ts(s):
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00"))


def read_lines(path):
    out = []
    if os.path.exists(path):
        with open(path) as f:
            for line in f:
                try:
                    out.append(json.loads(line))
                except ValueError:
                    continue
    return out


def coverage(protocol, outcomes):
    runs = sorted((o for o in outcomes if o.get("kind") == "run"), key=lambda o: o["at"])
    if not runs:
        return None
    start = parse_ts(runs[0]["at"])
    for o in runs:
        o["_t"] = parse_ts(o["at"])
    used = set()
    eligible, matched, missed = 0, 0, {}
    last = start
    for p in sorted(protocol, key=lambda p: p.get("at", "")):
        if p.get("action") not in RAN or "at" not in p:
            continue
        t = parse_ts(p["at"])
        # A pre-tool line from before the outcome log began counts only if its outcome
        # made it in: the first outcome line follows its own pre-tool line.
        early = t < start
        if early and t < start - WINDOW:
            continue
        via = "ana_run" if p["action"] == "ana_run" else "bare"
        hit = next(
            (i for i, o in enumerate(runs)
             if i not in used and o.get("session") == p.get("session")
             and o.get("runner") == p.get("runner") and o.get("via") == via
             and t - SLACK <= o["_t"] <= t + WINDOW),
            None,
        )
        if early and hit is None:
            continue
        last = max(last, t)
        eligible += 1
        if hit is None:
            key = f"{p['action']} / {p.get('runner')}"
            missed[key] = missed.get(key, 0) + 1
        else:
            used.add(hit)
            matched += 1
    last = max(last, runs[-1]["_t"])
    return {
        "since": start.date().isoformat(),
        "days": (last - start).total_seconds() / 86400,
        "eligible": eligible,
        "matched": matched,
        "share": matched / eligible if eligible else None,
        "missed": missed,
        "outcome_lines": len(runs),
    }


def report(c, out=sys.stdout):
    p = lambda *a: print(*a, file=out)
    if c is None:
        p("outcomes.jsonl has no test-run lines yet: nothing to check.")
        return
    p(f"Both logs since {c['since']} ({c['days']:.1f} days); {c['outcome_lines']} outcome line(s).")
    if not c["eligible"]:
        p("No pre-tool line since then for a run that went ahead: the pin reminder or audit sampling must be on for this check.")
        return
    p(f"Matched {c['matched']} of {c['eligible']} runs the pre-tool hook saw go ahead: {c['share']:.1%} (target {TARGET:.0%}).")
    for k, n in sorted(c["missed"].items(), key=lambda kv: -kv[1]):
        p(f"  unmatched: {n} × {k}")
    if c["days"] < MIN_DAYS:
        p(f"Too early to rule: the check is read after {MIN_DAYS} days.")
    elif c["share"] >= TARGET:
        p("The outcome log sees the runs: a trial may read it.")
    else:
        p("Below target: fix the logger before any trial reads it.")


def selftest():
    def line(at, **k):
        return dict(at=at, **k)
    outcomes = [
        line("2026-11-08T10:00:05Z", kind="run", session="s", runner="cargo", via="bare"),
        line("2026-11-08T10:30:00Z", kind="run", session="s", runner="cargo", via="ana_run"),
        line("2026-11-08T10:40:00Z", kind="edit", session="s"),
    ]
    protocol = [
        line("2026-11-08T10:00:00Z", session="s", runner="cargo", action="allowed_after_nudge"),
        line("2026-11-08T10:29:00Z", session="s", runner="cargo", action="ana_run"),
        line("2026-11-08T11:00:00Z", session="s", runner="pytest", action="ana_run"),  # missed
        line("2026-11-08T11:05:00Z", session="s", runner="cargo", action="denied_no_pin"),  # never ran
        line("2026-11-07T09:00:00Z", session="s", runner="cargo", action="ana_run"),  # before the log
    ]
    c = coverage(protocol, outcomes)
    assert c is not None
    assert (c["eligible"], c["matched"]) == (3, 2), c
    assert c["missed"] == {"ana_run / pytest": 1}, c
    with tempfile.TemporaryFile("w+") as f:
        report(c, out=f)
        f.seek(0)
        assert "Too early to rule" in f.read()
    assert coverage(protocol, []) is None
    print("selftest: ok")


def main():
    home = os.path.join(os.path.expanduser("~"), ".anamnesis")
    ap = argparse.ArgumentParser()
    ap.add_argument("--ledger", default=os.environ.get("ANAMNESIS_AGENT_DATA") or os.path.join(home, "agent.json"))
    ap.add_argument("--selftest", action="store_true")
    a = ap.parse_args()
    if a.selftest:
        selftest()
        return 0
    base = os.path.dirname(os.path.abspath(a.ledger))
    report(coverage(read_lines(os.path.join(base, "protocol.jsonl")),
                    read_lines(os.path.join(base, "outcomes.jsonl"))))
    return 0


if __name__ == "__main__":
    sys.exit(main())
