---
description: Should you act on a stated probability? Get a stake-aware PROCEED / VERIFY / ABSTAIN, corrected by your own track record.
argument-hint: --prob 0.8 [--stake 3] [--verify-cost 0.2] [--tag kind:estimate]
allowed-tools: Bash(ana:*)
---
Decide whether to act on a belief *before* you commit to a costly or irreversible step. This is the operational end of calibration — the moment a probability becomes an action.

Ledger: `${ANAMNESIS_AGENT_DATA:-$HOME/.anamnesis/agent.json}`.
Engine: `ana` (on PATH, else `~/.anamnesis/bin/ana`).

The gate does two things you cannot reliably do by introspection:
1. **Corrects your number.** Your stated confidence is pushed through your learned recalibration map — but only once your track record has *earned* a correction (real e-process evidence). Verbalized confidence is the least reliable signal; your recorded history is better.
2. **Thresholds by the stakes.** By Chow's reject rule it proceeds only when the expected cost of a wrong action, `(1 − p̂)·stake`, is below the cost of a verification step — i.e. when `p̂ ≥ 1 − verify_cost/stake`. The more consequential or irreversible the action, the higher `--stake`, and the closer to certain you must be to skip the check.

Steps:
1. Parse `$ARGUMENTS` into `--prob P` and optional `--stake N` (default 1; raise it for irreversible or expensive actions), `--verify-cost C` (default 0.2), and `--tag` (scope the correction to a kind, e.g. `kind:estimate`).
2. Run, e.g.: `ana --data "$LEDGER" decide --prob 0.8 --stake 3`
3. Report the verdict — **PROCEED** (confidence clears the bar), **VERIFY** (in the doubt zone — gather evidence first), or **ABSTAIN** (worse than even odds after correction — replan or escalate) — along with the corrected probability and the threshold. Then **do what it says.**

Use this at any decision point where being wrong is costly: shipping without a test, running a migration, deleting something, committing to an estimate you'll be held to. It is the difference between *knowing* how sure you are and *acting* on it — the gap the calibration research finds agents fall into.
