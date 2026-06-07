#!/usr/bin/env bash
# Anamnesis — PostToolUse(Bash) hook: the moment of truth.
#
# When a test/build command runs and you have an OPEN prediction about it *this
# session* (kind:tests-pass or kind:approach), nudge to resolve it NOW — before
# you explain the result away. The research is blunt: agents don't lower their
# confidence after failure unless something external makes them. This is that
# something. Fail-open, silent otherwise.
set -uo pipefail

LEDGER="${ANAMNESIS_AGENT_DATA:-$HOME/.anamnesis/agent.json}"
[ -f "$LEDGER" ] || exit 0
ANA="$(command -v ana 2>/dev/null || true)"
[ -z "$ANA" ] && [ -x "$HOME/.anamnesis/bin/ana" ] && ANA="$HOME/.anamnesis/bin/ana"
[ -x "$ANA" ] || exit 0
command -v jq >/dev/null 2>&1 || exit 0

input="$(cat)"
cmd="$(jq -r '.tool_input.command // ""' <<<"$input" 2>/dev/null || true)"
# React only to test/build commands — not every Bash call.
printf '%s' "$cmd" | grep -qiE '(cargo (test|build)|npm (test|run build)|pnpm (test|build)|yarn (test|build)|pytest|go test|gradle|mvn |flutter test|jest|vitest|ctest|make( |$))' || exit 0

SESSION="${ANAMNESIS_SESSION:-$(date +%Y-%m-%d)}"
ids="$("$ANA" --json --data "$LEDGER" list --open --tag "session:$SESSION" 2>/dev/null \
  | jq -r '[.[] | select((.tags|index("kind:tests-pass")) or (.tags|index("kind:approach"))) | .id] | join(", ")' 2>/dev/null || true)"
[ -n "$ids" ] || exit 0

jq -cn --arg ids "$ids" '{hookSpecificOutput:{hookEventName:"PostToolUse",additionalContext:("⟢ Anamnesis (moment of truth): a test/build just ran — resolve your open prediction(s) about it NOW (\($ids)) with /resolve <id> yes|no, before hindsight rewrites how sure you were.")}}'
exit 0
