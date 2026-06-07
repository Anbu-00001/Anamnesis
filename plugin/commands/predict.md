---
description: Log a falsifiable prediction (probability or interval) BEFORE acting, so it can be scored later.
argument-hint: "<claim>" --prob 0.6 [--second-prob 0.5] [--reference-class "..."]   |   "<claim>" --interval 2..6   [--kind ...] [--stake N] [--by YYYY-MM-DD]
allowed-tools: Bash(ana:*), Bash(git rev-parse:*)
---
Record this prediction in the **global agent ledger** *before* you act on it.

Ledger: `${ANAMNESIS_AGENT_DATA:-$HOME/.anamnesis/agent.json}`.
Engine: `ana` (on PATH, else `~/.anamnesis/bin/ana`).

**Before you pick a number** (research-backed elicitation — this is where calibration is won or lost):
- **Outside view first.** Name a *reference class* of similar past cases and its base rate ("refactors like this pass first try ~60%") and anchor on that, not on how this one *feels*. Pass it as `--reference-class "..."`.
- **Consider the opposite.** Make a first probability, then *assume it is wrong* and give yourself **two** concrete reasons why — that yields a second estimate. Pass it as `--second-prob P2` and the engine logs the **average** of the two (dialectical bootstrapping — the wisdom of your own crowd, ~half the gain of a second person). Two reasons, not ten.
- **Stake.** If the call is consequential, mark `--stake N` (default 1) so it weighs more in your scored calibration.

Steps:
1. Parse `$ARGUMENTS` into the statement and either `--prob P` (binary, optionally `--second-prob P2`) or `--interval LO..HIGH` (numeric, optionally `--level`).
2. Determine `project` = basename of `git rev-parse --show-toplevel` (fallback: current dir name).
3. Always stamp these tags: `who:claude`, `project:<project>`, `session:<today's date>`, and — if a `--kind K` was given — `kind:K`.
4. Run, e.g.:
   `ana --data "$LEDGER" add "<statement>" --prob 0.7 --second-prob 0.5 --reference-class "similar refactors pass ~60%" --tags who:claude,project:anamnesis,session:2026-06-07,kind:tests-pass`
5. Report the new id and the logged probability (the dialectical average, when `--second-prob` was given).

Log the **uncertain, falsifiable** calls — "tests pass first try", "this is the bug", "≤ N tool calls" — not the safe ones. One prediction per invocation. Be honest about the probability; this only helps if you don't fool yourself.
