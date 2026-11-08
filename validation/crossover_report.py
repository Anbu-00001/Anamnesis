#!/usr/bin/env python3
"""Does Anamnesis, as a whole, change how often the agent hands over code that fails its next test run?

    python3 validation/crossover_report.py [--ledger PATH] [--today YYYY-MM-DD]
    python3 validation/crossover_report.py --export     # one line to share (card D)
    python3 validation/crossover_report.py --selftest

Standard library only. It reads two files beside the agent ledger and writes nothing:

  * trial.json: the start Monday and, per pair of weeks, whether the pair opens ON
    (`ana trial start` writes it once);
  * outcomes.jsonl: one line per file edit and per test run the hooks saw, in both arms.
    Never a command, a file name or any content.

The rules are fixed in docs/MEASUREMENT.md, addendum 4, before the data exist:

  1. Primary outcome, per week: of the prompt turns that edited a file, the share whose first
     graded test run after the turn's last edit (same session and repo, that turn or a later
     one) failed. Turns with no later graded run are counted per arm and left out of the rate.
  2. The exact sign-flip test over the within-pair differences (OFF minus ON), two-sided at
     0.05, with the 95% interval found by inverting it.
  3. One confirmatory reading, at the end of the last pair. The e-value may be read at any time.
  4. Adherence: a week is in its arm if at least 90% of its graded runs carry that arm. Weeks
     are analysed as assigned either way (intent to treat), and the count is reported.
  5. Secondary outcomes are exploratory.

Weeks are Monday to Sunday by UTC date, the clock the hooks write.
"""
import argparse
import datetime as dt
import hashlib
import itertools
import json
import math
import os
import statistics
import sys
import tempfile

LAMBDAS = (1, 2, 3, 4, 6, 8)
ADHERENCE = 0.9
ALPHA = 0.05


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


def arm_of(trial, day):
    """('on'|'off', week index) for a date, or None outside the trial."""
    start = dt.date.fromisoformat(trial["start"])
    if day < start:
        return None
    week = (day - start).days // 7
    pair = week // 2
    if pair >= len(trial["on_first"]):
        return None
    on = (week % 2 == 0) == trial["on_first"][pair]
    return ("on" if on else "off"), week


def weakening(edit):
    """An edit line (v2, addendum 5) to an existing test or test-config file whose counts
    fell: fewer test definitions or assertions, or more skips. A candidate, never a verdict."""
    d = edit.get("delta")
    return (
        edit.get("created") is not True  # a new file can only add tests
        and edit.get("class") in ("test", "test-config")
        and isinstance(d, list)
        and len(d) == 3
        and (d[0] < 0 or d[1] < 0 or d[2] > 0)
    )


def edit_turns(events, kinds=("edit",)):
    """One record per (session, prompt, repo) that edited a file: its last edit, and what
    the graded runs after that edit said. Addendum 4 counts tool edits only; addendum 5's
    sensitivity reading adds `shell_edit` lines."""
    edits = [e for e in events if e.get("kind") in kinds]
    runs = sorted(
        (e for e in events if e.get("kind") == "run" and e.get("graded") in ("pass", "fail")),
        key=lambda e: e["_t"],
    )
    weak = {(e.get("session"), e.get("prompt"), e.get("repo")) for e in edits if weakening(e)}
    tested = {(e.get("session"), e.get("prompt"), e.get("repo")) for e in edits
              if e.get("class") in ("test", "test-config")}
    last = {}
    for e in edits:
        key = (e.get("session"), e.get("prompt"), e.get("repo"))
        if key not in last or e["_t"] > last[key]:
            last[key] = e["_t"]
    turns = []
    for (session, prompt, repo), t in last.items():
        after = [r for r in runs if r.get("session") == session and r.get("repo") == repo and r["_t"] >= t]
        first = after[0] if after else None
        in_turn = [r for r in after if r.get("prompt") == prompt]
        green = next((r for r in after if r["graded"] == "pass"), None)
        turns.append({
            "session": session,
            "prompt": prompt,
            "repo": repo,
            "at": t,
            "failed": None if first is None else first["graded"] == "fail",
            "ended_green": bool(in_turn) and in_turn[-1]["graded"] == "pass",
            "runs_in_turn": len(in_turn),
            "minutes_to_green": None if green is None else (green["_t"] - t).total_seconds() / 60,
            "weakened": (session, prompt, repo) in weak,
            "test_edit": (session, prompt, repo) in tested,
        })
    return turns


def sign_flip(ds):
    """Exact two-sided sign-flip p-value for the mean of ds."""
    k = len(ds)
    obs = abs(sum(ds)) / k
    hits = 0
    for signs in itertools.product((1, -1), repeat=k):
        if abs(sum(s * d for s, d in zip(signs, ds))) / k >= obs - 1e-12:
            hits += 1
    return hits / 2 ** k


def interval(ds, step=0.002):
    """The shifts the test does not reject at ALPHA: the 95% interval for the mean."""
    kept = [
        x * step
        for x in range(int(-1 / step), int(1 / step) + 1)
        if sign_flip([d - x * step for d in ds]) > ALPHA
    ]
    return (min(kept), max(kept)) if kept else (None, None)


def e_value(ds):
    """Mean over LAMBDAS of prod exp(l*d - l^2 d^2 / 2). Under the sharp null every d is
    equally likely to have either sign, and cosh(x) <= exp(x^2/2), so each product has
    expectation at most 1: valid at any time. It grows when OFF weeks fail more."""
    return statistics.fmean(
        math.exp(sum(l * d - l * l * d * d / 2 for d in ds)) for l in LAMBDAS
    )


def analyse(trial, events, today):
    for e in events:
        e["_t"] = parse_ts(e["at"])
    weeks = {}
    for t in edit_turns(events):
        a = arm_of(trial, t["at"].date())
        if a is None:
            continue
        w = weeks.setdefault(a[1], {"arm": a[0], "turns": []})
        w["turns"].append(t)
    shell_rate = {}
    for t in edit_turns(events, kinds=("edit", "shell_edit")):
        a = arm_of(trial, t["at"].date())
        if a is not None and t["failed"] is not None:
            shell_rate.setdefault(a[1], []).append(t["failed"])
    runs_by_week = {}
    for e in events:
        if e.get("kind") == "run" and e.get("graded") in ("pass", "fail"):
            a = arm_of(trial, e["_t"].date())
            if a is not None:
                runs_by_week.setdefault(a[1], []).append(e)
    n_weeks = 2 * len(trial["on_first"])
    rows = []
    for i in range(n_weeks):
        start = dt.date.fromisoformat(trial["start"]) + dt.timedelta(days=7 * i)
        here = arm_of(trial, start)
        assert here is not None  # every week of the trial is inside it
        arm = here[0]
        turns = weeks.get(i, {"turns": []})["turns"]
        graded = [t for t in turns if t["failed"] is not None]
        runs = runs_by_week.get(i, [])
        carried = sum(1 for r in runs if r.get("arm") == arm)
        ana_runs = sum(1 for r in runs if r.get("via") == "ana_run")
        mins = [t["minutes_to_green"] for t in turns if t["minutes_to_green"] is not None]
        rows.append({
            "week": i + 1,
            "arm": arm,
            "edit_turns": len(turns),
            "no_later_run": len(turns) - len(graded),
            "rate": (sum(t["failed"] for t in graded) / len(graded)) if graded else None,
            "ended_green": (sum(t["ended_green"] for t in turns) / len(turns)) if turns else None,
            "runs_per_turn": (sum(t["runs_in_turn"] for t in turns) / len(turns)) if turns else None,
            "median_minutes_to_green": statistics.median(mins) if mins else None,
            "adherent": (carried / len(runs) >= ADHERENCE) if runs else None,
            "ana_runs": ana_runs,
            # Addendum 5, exploratory: test-surface edits and the rate without them.
            "test_edit_share": (sum(t["test_edit"] for t in turns) / len(turns)) if turns else None,
            "weakened_share": (sum(t["weakened"] for t in turns) / len(turns)) if turns else None,
            "rate_unweakened": (lambda g: (sum(t["failed"] for t in g) / len(g)) if g else None)(
                [t for t in graded if not t["weakened"]]),
            "rate_with_shell": (lambda g: sum(g) / len(g) if g else None)(shell_rate.get(i, [])),
        })
    ds, dropped = [], []
    for p in range(len(trial["on_first"])):
        a, b = rows[2 * p], rows[2 * p + 1]
        on = a if a["arm"] == "on" else b
        off = b if on is a else a
        if on["rate"] is None or off["rate"] is None:
            dropped.append(p + 1)
        else:
            ds.append(off["rate"] - on["rate"])
    end = dt.date.fromisoformat(trial["start"]) + dt.timedelta(days=7 * n_weeks)
    return rows, ds, dropped, today >= end


def fmt(x, spec="{:.3f}"):
    return "—" if x is None else spec.format(x)


def report(trial, events, today, out=sys.stdout):
    rows, ds, dropped, complete = analyse(trial, events, today)
    p = lambda *a: print(*a, file=out)
    p(f"Weekly trial [{trial['id']}]: {len(trial['on_first'])} pairs from {trial['start']} "
      "(docs/MEASUREMENT.md, addendum 4)")
    p("")
    p(" week  arm  edit-turns  no-later-run  fail-rate  ended-green  runs/turn  min-to-green  adherent  ana-runs")
    for r in rows:
        p(f" {r['week']:>4}  {r['arm']:<3}  {r['edit_turns']:>10}  {r['no_later_run']:>12}  "
          f"{fmt(r['rate']):>9}  {fmt(r['ended_green']):>11}  {fmt(r['runs_per_turn'], '{:.2f}'):>9}  "
          f"{fmt(r['median_minutes_to_green'], '{:.1f}'):>12}  {fmt(r['adherent'], '{}'):>8}  {r['ana_runs']:>8}")
    p("")
    p("Test surface (addendum 5, exploratory; edit lines from v2 on):")
    p(" week  arm  test-edit-turns  weakened-turns  fail-rate-without-weakened  fail-rate-with-shell-edits")
    for r in rows:
        p(f" {r['week']:>4}  {r['arm']:<3}  {fmt(r['test_edit_share']):>15}  {fmt(r['weakened_share']):>14}  "
          f"{fmt(r['rate_unweakened']):>26}  {fmt(r['rate_with_shell']):>26}")
    p("")
    off_adh = sum(1 for r in rows if r["adherent"] is False)
    p(f"Weeks not in their arm (rule 4): {off_adh}. They are analysed as assigned.")
    if dropped:
        p(f"Pairs with a week that has no graded edit-turn, left out of the test: {dropped}")
    if not ds:
        p("No pair has a rate in both weeks yet: nothing to test.")
        return
    mean = statistics.fmean(ds)
    e = e_value(ds)
    p(f"Mean difference, OFF minus ON, over {len(ds)} pair(s): {mean:+.3f} "
      "(positive means more handoff failures without the tool)")
    p(f"Anytime-valid e-value: {e:.2f} (20 or more is evidence at the 0.05 level; readable at any time)")
    if complete:
        pv = sign_flip(ds)
        lo, hi = interval(ds)
        p(f"Confirmatory reading (rule 3; read once): exact sign-flip p = {pv:.4f}, "
          f"95% interval {fmt(lo, '{:+.3f}')} to {fmt(hi, '{:+.3f}')}")
        if pv > ALPHA:
            p("A null: no change this test can see. Under the addendum's simulation it sees changes of "
              "about ten points; a smaller one would read like this.")
    else:
        p("The trial is still running: the p-value is withheld until the last pair ends (rule 3).")


def protocol_sha256():
    """SHA-256 of addendum 4's text in docs/MEASUREMENT.md, so pooled lines can show they
    ran the same rules. None when the file is not beside this script."""
    doc = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "docs", "MEASUREMENT.md")
    try:
        text = open(doc, encoding="utf-8").read()
    except OSError:
        return None
    start = text.find("## Addendum 4,")
    if start < 0:
        return None
    end = text.find("\n## ", start + 1)
    return hashlib.sha256(text[start:end if end > 0 else None].encode()).hexdigest()


def export_line(trial, events, today):
    """The one line a volunteer shares: no ids, no times, no repos, nothing of the work.
    The differences are what a pooled sign-flip test needs; the e-value is what a pooled
    e-value needs."""
    rows, ds, dropped, complete = analyse(trial, events, today)
    return json.dumps({
        "v": 1,
        "protocol": "anamnesis-addendum-4",
        "protocol_sha256": protocol_sha256(),
        "engine": trial.get("engine"),
        "pairs_planned": len(trial["on_first"]),
        "pairs_used": len(ds),
        "weeks_not_in_arm": sum(1 for r in rows if r["adherent"] is False),
        "complete": complete,
        "differences": [round(d, 4) for d in ds],
        "e_value": round(e_value(ds), 6) if ds else 1.0,
    }, sort_keys=True)


def load_trial(path):
    with open(path) as f:
        return json.load(f)


def selftest():
    # A known sign-flip value: three positive differences, 1 of 8 assignments as extreme
    # each way.
    assert abs(sign_flip([0.1, 0.2, 0.3]) - 0.25) < 1e-12
    assert sign_flip([0.0, 0.0]) == 1.0
    # The e-value under no effect stays small; under a large effect it grows.
    assert e_value([0.0] * 13) == 1.0
    assert e_value([0.3] * 13) > 20
    # An end-to-end trial of 2 pairs: OFF weeks fail every handoff, ON weeks never do.
    trial = {"id": "self", "start": "2027-01-04", "on_first": [True, False]}
    events = []
    for week in range(4):
        day = dt.date(2027, 1, 4) + dt.timedelta(days=7 * week + 1)
        here = arm_of(trial, day)
        assert here is not None
        arm = here[0]
        for turn in range(3):
            t = f"{day}T10:{turn:02d}:00Z"
            r = f"{day}T10:{turn:02d}:30Z"
            events.append({"kind": "edit", "at": t, "session": f"s{week}", "prompt": turn, "repo": "r"})
            events.append({"kind": "run", "at": r, "session": f"s{week}", "prompt": turn, "repo": "r",
                           "graded": "fail" if arm == "off" else "pass", "arm": arm, "via": "bare"})
    rows, ds, dropped, complete = analyse(trial, events, dt.date(2027, 2, 1))
    assert ds == [1.0, 1.0] and not dropped and complete, (ds, dropped, complete)
    assert [r["arm"] for r in rows] == ["on", "off", "off", "on"]
    assert all(r["adherent"] for r in rows)
    assert all(r["weakened_share"] == 0 for r in rows)
    weak = events + [{"kind": "edit", "at": "2027-01-05T10:00:10Z", "session": "s0", "prompt": 0,
                      "repo": "r", "class": "test", "delta": [0, -1, 0]}]
    rows_w, _, _, _ = analyse(trial, weak, dt.date(2027, 2, 1))
    assert abs(rows_w[0]["weakened_share"] - 1 / 3) < 1e-12, rows_w[0]
    assert rows_w[0]["rate_unweakened"] == 0.0
    new_file = events + [{"kind": "edit", "at": "2027-01-05T10:00:10Z", "session": "s0", "prompt": 0,
                          "repo": "r", "class": "test", "created": True, "delta": [1, 1, 1]}]
    assert analyse(trial, new_file, dt.date(2027, 2, 1))[0][0]["weakened_share"] == 0
    shell = events + [{"kind": "shell_edit", "at": "2027-01-12T11:00:00Z", "session": "x", "prompt": 9, "repo": "r"},
                      {"kind": "run", "at": "2027-01-12T11:00:30Z", "session": "x", "prompt": 9, "repo": "r",
                       "graded": "pass", "arm": "off", "via": "bare"}]
    rows_s = analyse(trial, shell, dt.date(2027, 2, 1))[0]
    assert rows_s[1]["rate"] == 1.0 and rows_s[1]["rate_with_shell"] == 0.75, rows_s[1]
    with tempfile.TemporaryFile("w+") as f:
        report(trial, events, dt.date(2027, 2, 1), out=f)
        f.seek(0)
        text = f.read()
    assert "Confirmatory reading" in text and "+1.000" in text, text
    line = json.loads(export_line(trial, events, dt.date(2027, 2, 1)))
    assert line["differences"] == [1.0, 1.0] and line["complete"], line
    assert not any(k in json.dumps(line) for k in ("s0", "2027-01")), line
    print("selftest: ok")


def main():
    home = os.environ.get("ANAMNESIS_HOME") or os.path.join(os.path.expanduser("~"), ".anamnesis")
    ap = argparse.ArgumentParser()
    ap.add_argument("--ledger", default=os.environ.get("ANAMNESIS_AGENT_DATA") or os.path.join(home, "agent.json"))
    ap.add_argument("--today", default=None, help="read as of this UTC date (default: today)")
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--export", action="store_true", help="print the one shareable line (card D)")
    a = ap.parse_args()
    if a.selftest:
        selftest()
        return 0
    base = os.path.dirname(os.path.abspath(a.ledger))
    tpath = os.path.join(base, "trial.json")
    if not os.path.exists(tpath):
        print(f"no trial enrolled ({tpath} does not exist). `ana trial start` enrols one.")
        return 0
    today = dt.date.fromisoformat(a.today) if a.today else dt.datetime.now(dt.timezone.utc).date()
    trial, events = load_trial(tpath), read_lines(os.path.join(base, "outcomes.jsonl"))
    if a.export:
        print(export_line(trial, events, today))
    else:
        report(trial, events, today)
    return 0


if __name__ == "__main__":
    sys.exit(main())
