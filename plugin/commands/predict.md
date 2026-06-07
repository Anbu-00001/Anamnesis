---
description: Log a falsifiable prediction (probability or interval) BEFORE acting, so it can be scored later.
argument-hint: "<claim>" --prob 0.6   |   "<claim>" --interval 2..6   [--kind estimate|tests-pass|bug-hypothesis|approach|compat] [--by YYYY-MM-DD]
allowed-tools: Bash(ana:*), Bash(git rev-parse:*)
---
Record this prediction in the **global agent ledger** *before* you act on it.

Ledger: `${ANAMNESIS_AGENT_DATA:-$HOME/.anamnesis/agent.json}`.
Engine: `ana` (on PATH, else `~/.anamnesis/bin/ana`).

Steps:
1. Parse `$ARGUMENTS` into the statement and either `--prob P` (binary) or `--interval LO..HIGH` (numeric, optionally `--level`).
2. Determine `project` = basename of `git rev-parse --show-toplevel` (fallback: current dir name).
3. Always stamp these tags: `who:claude`, `project:<project>`, `session:<today's date>`, and — if a `--kind K` was given — `kind:K`.
4. Run, e.g.:
   `ana --data "$LEDGER" add "<statement>" --prob 0.6 --tags who:claude,project:anamnesis,session:2026-06-07,kind:estimate`
5. Report the new id.

Log the **uncertain, falsifiable** calls — "tests pass first try", "this is the bug", "≤ N tool calls" — not the safe ones. One prediction per invocation. Be honest about the probability; this only helps if you don't fool yourself.
