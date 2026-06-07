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

The lesson only compounds if the ledger is honest. A no-LLM scorer can't flatter
you — don't do its job for it.
