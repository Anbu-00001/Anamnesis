---
description: Show your standing calibration — the mirror — from the global agent ledger.
argument-hint: [--tag kind:estimate]   [--bins 10]
allowed-tools: Bash(ana:*)
---
Run:
`ana --data "${ANAMNESIS_AGENT_DATA:-$HOME/.anamnesis/agent.json}" report --tag who:claude $ARGUMENTS`

Predictions you logged through the MCP tools carry `who:claude-code` (Claude Code's own
name for itself), not `who:claude`. If you have used those tools, run the same command
with `--tag who:claude-code` too. The standing line the hooks show you already counts both.

Read it honestly. Pay attention to:
- the **confidence gap** (positive = overconfident → plan with more slack),
- the **By prediction kind** table (which *type* of call you misjudge — estimates? bug hypotheses?),
- the **base-rate 95% CI** (if it's wide, you simply don't have enough resolved predictions yet — keep logging).

If `ana` is not installed, tell the user to install it from the Anamnesis releases (or `cargo install`), then retry.
