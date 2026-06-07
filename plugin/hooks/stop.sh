#!/usr/bin/env bash
# Anamnesis — Stop hook.
#
# A quiet conscience: if predictions are OVERDUE (past their resolve-by date and
# still open), drop a one-line reminder to resolve them. Deliberately scoped to
# *overdue* only — not every open prediction — so it stays silent on normal turns
# and never nags about predictions whose outcome isn't knowable yet.
# Never blocks. Fail-open and silent.
set -uo pipefail

LEDGER="${ANAMNESIS_AGENT_DATA:-$HOME/.anamnesis/agent.json}"
[ -f "$LEDGER" ] || exit 0
ANA="$(command -v ana 2>/dev/null || true)"
[ -z "$ANA" ] && [ -x "$HOME/.anamnesis/bin/ana" ] && ANA="$HOME/.anamnesis/bin/ana"
[ -x "$ANA" ] || exit 0
command -v jq >/dev/null 2>&1 || exit 0

due="$("$ANA" --json --data "$LEDGER" list --due 2>/dev/null | jq -r 'length' 2>/dev/null || echo 0)"
[ "${due:-0}" -gt 0 ] || exit 0

ids="$("$ANA" --json --data "$LEDGER" list --due 2>/dev/null | jq -r '[.[].id] | join(", ")' 2>/dev/null)"
jq -cn --arg n "$due" --arg ids "$ids" \
  '{hookSpecificOutput:{hookEventName:"Stop",additionalContext:("⟢ Anamnesis: \($n) prediction(s) are OVERDUE — resolve any whose outcome is now known (\($ids)) with /resolve <id>, before hindsight rewrites them.")}}'
exit 0
