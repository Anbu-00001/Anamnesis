#!/usr/bin/env python3
"""Does the calibration checkpoint change anything? The one comparison Anamnesis can run on itself.

    python3 validation/effect_report.py [--ledger PATH] [--since YYYY-MM-DD] [--draws N]

Standard library only. It reads three files and writes nothing:

  * the agent ledger (default ~/.anamnesis/agent.json): claims carry `origin.session` when they
    were logged inside Claude Code (0.5.0 and later);
  * checkpoints.jsonl beside it: one line per prompt, with the chance the checkpoint had and
    whether it was drawn. Never the prompt;
  * protocol.jsonl beside it: the test runs, for the secondary outcome on `ana run`.

The design is a micro-randomized trial (Klasnja et al. 2015): every prompt draws the checkpoint
with a known probability, so within one session the prompts that got it and the prompts that
did not differ only by chance. The test is a randomization test of the sharp null "the
checkpoint changes nothing": the logged assignments are re-drawn from their logged
probabilities, holding outcomes fixed. That is exact in finite samples, with no assumption about
how many sessions there are or how alike they are, which is the problem every other figure in
this repository has.

The questions and the rules are fixed in docs/MEASUREMENT.md, addendum 3, before the data exist.
"""
import argparse
import bisect
import collections
import datetime as dt
import json
import os
import random
import sys

MIN_DRAWN = 10   # below this many drawn prompts with an outcome, no comparison is printed
CONFIRM_DRAWN = 40  # the one confirmatory reading (addendum 3, rule 2)


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


def first_p(c):
    f = c.get("forecasts") or []
    return f[0].get("prob") if f else None


def outcome(c):
    r = c.get("resolution")
    if not r or c.get("void") or r.get("outcome") is None:
        return None
    return 1.0 if str(r["outcome"]).strip().lower() in ("true", "yes", "y", "1") else 0.0


def prompts_of(checkpoints):
    """Per session, the prompts in time order: (at, prompt number, p, drawn)."""
    by = collections.defaultdict(list)
    for e in checkpoints:
        try:
            by[e["session"]].append((parse_ts(e["at"]), e["prompt"], float(e["p"]), bool(e["drawn"])))
        except (KeyError, TypeError, ValueError):
            continue
    for v in by.values():
        v.sort()
    return by


def assign(by, session, at):
    """The prompt a thing that happened at `at` in `session` belongs to: the latest before it."""
    rows = by.get(session)
    if not rows:
        return None
    i = bisect.bisect_right([r[0] for r in rows], at) - 1
    return (session, rows[i][1]) if i >= 0 else None


def diff(units):
    """Mean outcome where drawn minus where not. units: list of (drawn, value)."""
    a = [v for d, v in units if d]
    b = [v for d, v in units if not d]
    if not a or not b:
        return None
    return sum(a) / len(a) - sum(b) / len(b)


def randomization_p(units, probs, draws, seed):
    """Two-sided p-value of the observed difference under the sharp null, re-drawing every
    unit's assignment from its own logged probability."""
    observed = diff(units)
    if observed is None:
        return None, None
    rng = random.Random(seed)
    values = [v for _, v in units]
    extreme = used = 0
    for _ in range(draws):
        sim = [(rng.random() < p, v) for p, v in zip(probs, values)]
        d = diff(sim)
        if d is None:
            continue
        used += 1
        extreme += abs(d) >= abs(observed) - 1e-12
    return observed, (1 + extreme) / (1 + used)


def compare(title, per_unit, prompt_info, draws, seed, fmt, lower_is_better=None):
    units = [(prompt_info[k][1], v) for k, v in per_unit.items() if k in prompt_info]
    probs = [prompt_info[k][0] for k in per_unit if k in prompt_info]
    drawn = sum(d for d, _ in units)
    print(f"\n  {title}")
    print(f"    {len(units)} prompts with an outcome: {drawn} drawn, {len(units) - drawn} not")
    if drawn < MIN_DRAWN or len(units) - drawn < MIN_DRAWN:
        print(f"    no comparison: fewer than {MIN_DRAWN} prompts in an arm. (Below that it is noise.)")
        return
    a = [v for d, v in units if d]
    b = [v for d, v in units if not d]
    observed, p = randomization_p(units, probs, draws, seed)
    if observed is None or p is None:
        print("    no comparison: one arm is empty.")
        return
    print(f"    drawn {fmt(sum(a) / len(a))}   not drawn {fmt(sum(b) / len(b))}   "
          f"difference {observed:+.3f}   randomization p {'< 0.001' if p < 0.001 else f'= {p:.3f}'}")
    if p < 0.05:
        better = None if lower_is_better is None else (observed < 0) == lower_is_better
        word = {True: "better", False: "worse", None: "different"}[better]
        print(f"    -> under 0.05: prompts with the checkpoint did {word} on this, beyond what the draw alone explains.")
    else:
        print("    -> no difference can be claimed. (A null, not proof of none.)")


def main():
    ap = argparse.ArgumentParser()
    home = os.path.expanduser("~/.anamnesis")
    ap.add_argument("--ledger", default=os.path.join(home, "agent.json"))
    ap.add_argument("--since", default=None)
    ap.add_argument("--draws", type=int, default=10000)
    a = ap.parse_args()
    here = os.path.dirname(a.ledger)
    since = parse_ts(a.since + "T00:00:00Z") if a.since else None

    with open(a.ledger) as f:
        claims = json.load(f)["claims"]
    checkpoints = [e for e in read_lines(os.path.join(here, "checkpoints.jsonl"))
                   if since is None or parse_ts(e["at"]) >= since]
    runs = [e for e in read_lines(os.path.join(here, "protocol.jsonl"))
            if since is None or parse_ts(e["at"]) >= since]
    by = prompts_of(checkpoints)
    prompt_info = {(s, n): (p, d) for s, rows in by.items() for _, n, p, d in rows}

    print(f"ledger: {a.ledger}")
    if not checkpoints:
        print("\nno checkpoint draws yet. They are written by `ana hook user-prompt` from 0.5.0 on; check")
        print("that the UserPromptSubmit hook runs this engine (`ana --version`) before reading anything else.")
        return 0
    drawn = sum(d for _, d in prompt_info.values())
    ps = collections.Counter(round(p, 4) for p, _ in prompt_info.values())
    print(f"\n== The draws ==\n  {len(prompt_info)} prompts in {len(by)} sessions, {drawn} drawn "
          f"(probabilities used: {', '.join(f'{p:g}' for p in sorted(ps))})")
    expected = sum(p for p, _ in prompt_info.values())
    print(f"  expected about {expected:.0f} drawn; a large gap means the draw is not what the log says")

    # claims, matched to the prompt they were logged under
    per_brier, per_gap, per_true = collections.defaultdict(list), collections.defaultdict(list), collections.defaultdict(list)
    logged = collections.Counter()
    stamped = 0
    for c in claims:
        session = (c.get("origin") or {}).get("session")
        if not session:
            continue
        at = parse_ts(c["created_at"])
        if since is not None and at < since:
            continue
        stamped += 1
        k = assign(by, session, at)
        if k is None:
            continue
        logged[k] += 1
        p, y = first_p(c), outcome(c)
        if p is None or y is None:
            continue
        per_brier[k].append((p - y) ** 2)
        per_gap[k].append(p - y)
        if "kind:tests-pass" in c.get("tags", []) and (c.get("resolution") or {}).get("resolved_by") == "auto":
            per_true[k].append(y)
    print(f"  {stamped} claims carry a session; {sum(logged.values())} matched to a prompt")

    mean = lambda v: sum(v) / len(v)
    print("\n== Primary (addendum 3): Brier of the claims logged under a prompt, lower is better ==")
    compare("mean Brier per prompt", {k: mean(v) for k, v in per_brier.items()}, prompt_info,
            a.draws, 20261107, lambda x: f"{x:.3f}", lower_is_better=True)

    print("\n== Secondary (exploratory, each labelled so) ==")
    compare("confidence gap per prompt (said - true; nearer 0 is better, sign shown)",
            {k: mean(v) for k, v in per_gap.items()}, prompt_info, a.draws, 20261108, lambda x: f"{x:+.3f}")
    compare("exit-graded tests that passed, per prompt (higher is better)",
            {k: mean(v) for k, v in per_true.items()}, prompt_info, a.draws, 20261109, lambda x: f"{x:.0%}",
            lower_is_better=False)
    compare("claims logged per prompt (every prompt counts, 0 included)",
            {k: logged.get(k, 0) for k in prompt_info}, prompt_info, a.draws, 20261110, lambda x: f"{x:.2f}",
            lower_is_better=False)
    pinned = collections.defaultdict(list)
    for e in runs:
        if e.get("v", 0) < 3:
            continue
        k = assign(by, e.get("session"), parse_ts(e["at"]))
        if k is not None:
            pinned[k].append(1.0 if e["action"] == "ana_run" else 0.0)
    compare("test runs through `ana run`, per prompt (higher is better)",
            {k: mean(v) for k, v in pinned.items()}, prompt_info, a.draws, 20261111, lambda x: f"{x:.0%}",
            lower_is_better=False)

    with_outcome = sum(1 for k in per_brier if prompt_info.get(k, (0, False))[1])
    print("\n== Where this stands ==")
    if with_outcome >= CONFIRM_DRAWN:
        print(f"  {with_outcome} drawn prompts have a graded claim: the confirmatory reading is due (rule 2).")
        print("  The first run at or past this point is that reading; record it in docs/MEASUREMENT.md.")
    else:
        print(f"  {with_outcome} of the {CONFIRM_DRAWN} drawn prompts with a graded claim the confirmatory reading")
        print("  waits for (rule 2). Anything above is exploratory until then.")
    print("  What this can show: whether seeing the standing calibration mid-session changes the calls made")
    print("  right after it. What it cannot: the effect of the whole tool, which is always on, or of the")
    print("  session-start line, which every session gets.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
