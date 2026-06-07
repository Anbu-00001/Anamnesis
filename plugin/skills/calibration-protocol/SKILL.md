---
name: calibration-protocol
description: How and when to log and resolve predictions with Anamnesis so your engineering judgement stays honest and improves across sessions. Engage at the start of any non-trivial coding task, when making effort/time estimates, when forming a bug hypothesis, or whenever you catch yourself saying something will "definitely" work.
---

# The calibration protocol

Anamnesis exists because an agent that forgets between sessions cannot otherwise
learn *how sure to be*. You are that agent. Use it on yourself.

## When to log a prediction (`/predict`)

At the start of a non-trivial task, and whenever you make a falsifiable claim you
are not certain of. The high-value ones:

- **`kind:tests-pass`** — "the tests/build will pass first try" (`--prob`).
- **`kind:estimate`** — effort: "≤ N tool calls", "under an hour" (`--interval`).
- **`kind:bug-hypothesis`** — "the bug is in X" (`--prob`).
- **`kind:approach`** — "this approach will work" (`--prob`).
- **`kind:compat`** — "this change is backward-compatible" (`--prob`).

Log the **uncertain** calls, not the safe ones — selection bias toward easy
predictions is self-deception with extra steps. State your *honest* probability.

## How to pick the number (elicitation)

The forecasting research is clear that *how* you arrive at the probability matters
as much as the scoring — a sharper input beats another decimal of metric. Two
cheap, evidence-backed moves, both supported by `/predict`:

1. **Outside view first** (reference-class forecasting). Before judging *this*
   case, name a class of similar past cases and its base rate — "refactors this
   size pass first-try maybe 60% of the time" — and start there. The inside view
   (how confident this one *feels*) is where overconfidence lives. Record it with
   `--reference-class "..."`.
2. **Consider the opposite** (dialectical bootstrapping / the "crowd within").
   Make a first estimate, then deliberately assume it is wrong and find **two**
   reasons why; that gives a second estimate. Pass both (`--prob`, `--second-prob`)
   and the engine logs their average — recovering about half the accuracy gain of
   consulting a second person. Two reasons, not ten (more backfires).

Mark consequential calls with `--stake N` so your scored calibration weights the
predictions that actually matter.

## When to resolve (`/resolve`)

The instant reality answers — a test runs, the bug is found, the task ships.
**Before** you rationalize. The urge to think "I basically knew that" is hindsight
bias; resolving immediately is how you beat it.

## How to use the result

At session start you'll see your standing calibration injected automatically.
Act on it:
- **Overconfident** (positive gap) → widen estimates, hedge claims, add slack to plans.
- **Underconfident** → trust your high-probability calls more.
- Watch the **By prediction kind** table: you may be calibrated on tests but wildly
  overconfident on bug hypotheses. Adjust *per type*.
- **Don't over-react to a handful of calls.** The report's *"Is it real?"* line is an
  anytime-valid test — only treat the miscalibration as established once it says
  **REAL**. When it does, the *"Recalibration"* line tells you the concrete
  correction to apply ("your 60%s should be 80%"); until then, keep logging.

## When to *act* on it (`/decide`)

Knowing how sure you are is worthless if you act anyway — and that is precisely the
trap the research finds agents fall into: stating uncertainty, then taking the
irreversible step regardless. Before any costly or hard-to-undo action — shipping
without a test, a migration, a deletion, committing to an estimate — run the call
through `/decide` instead of trusting the gut number:

- It **corrects** your stated probability through your earned recalibration map (so
  your documented overconfidence is discounted automatically), then **thresholds by
  the stakes** (Chow's rule: proceed only when `p̂ ≥ 1 − verify_cost/stake`).
- Raise `--stake` for consequential or irreversible actions; the bar to proceed
  climbs toward certainty. You get back **PROCEED**, **VERIFY** (gather evidence
  first), or **ABSTAIN** (replan/escalate — worse than even odds after correction).
- Then *do what it says*. This is the whole point of keeping score: not a prettier
  report, but a better next action.

The lesson only compounds if the ledger is honest. A no-LLM scorer can't flatter
you — don't do its job for it.
