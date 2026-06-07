#!/usr/bin/env bash
# Anamnesis — SessionStart hook.
#
# Injects the agent's STANDING CALIBRATION (from the global ledger) plus any
# predictions DUE in this repo, as a system reminder before the first prompt.
# This is the whole "auto-engages in every project" mechanism: your calibration
# follows you into every folder.
#
# Contract: never block, never error out loud, and stay COMPLETELY SILENT when
# there is nothing worth saying (empty ledger, too few samples, no engine).
# Idempotent and fast (<~50ms): a tiny JSON read against a static binary.
set -uo pipefail

LEDGER="${ANAMNESIS_AGENT_DATA:-$HOME/.anamnesis/agent.json}"
[ -f "$LEDGER" ] || exit 0                       # nothing logged yet → silent

# Locate the engine: PATH first, then the vendored install. Missing → silent.
ANA="$(command -v ana 2>/dev/null || true)"
[ -z "$ANA" ] && [ -x "$HOME/.anamnesis/bin/ana" ] && ANA="$HOME/.anamnesis/bin/ana"
[ -x "$ANA" ] || exit 0
command -v jq >/dev/null 2>&1 || exit 0          # need jq to compose → silent

MIN_N="${ANAMNESIS_MIN_N:-6}"                     # below this, calibration is noise
slug="$(basename "$(git rev-parse --show-toplevel 2>/dev/null || pwd)")"

report="$("$ANA" --json --data "$LEDGER" report --tag who:claude 2>/dev/null)" || exit 0
[ -n "$report" ] || exit 0
resolved="$(jq -r '.resolved // 0' <<<"$report" 2>/dev/null || echo 0)"

lines=""
add() { lines="${lines:+$lines$'\n'}$1"; }

# 1) Standing over/under-confidence — only once we have enough resolved to trust.
if [ "${resolved:-0}" -ge "$MIN_N" ]; then
  gap="$(jq -r '
    (.confidence_gap // empty) as $gap | (.resolved) as $n
    | ($gap*100|round) as $g | ((.accuracy//0)*100|round) as $a | ((.mean_confidence//0)*100|round) as $c
    | if ($gap|fabs) < 0.05 then "well-calibrated overall (\($n) resolved)"
      elif $gap > 0 then "OVERCONFIDENT +\($g)pts — right \($a)% but claim ~\($c)% (\($n) resolved). Add slack."
      else "UNDERCONFIDENT \($g)pts — right \($a)% vs claimed ~\($c)% (\($n) resolved). Trust yourself more."
      end' <<<"$report" 2>/dev/null)"
  [ -n "$gap" ] && add "$gap"

  kind="$(jq -r '
    ([.by_kind[]? | (.confidence_gap_shrunk // .confidence_gap) as $g | . + {g:$g} | select(.n >= 2 and ($g|fabs) >= 0.10)] | sort_by(-(.g|fabs)))[0]
    | if . == null then empty else "  worst type: kind:\(.tag) (gap \((.g*100|round))pts shrunk, n=\(.n))" end' \
    <<<"$report" 2>/dev/null)"
  [ -n "$kind" ] && add "$kind"
fi

# 2) Predictions DUE in this repo (always actionable, regardless of sample size).
due="$("$ANA" --json --data "$LEDGER" list --due --tag "project:$slug" 2>/dev/null \
       | jq -r '.[]? | "  DUE [\(.id)] \(.statement) — resolve with /resolve \(.id)"' 2>/dev/null || true)"
[ -n "$due" ] && add "$due"

[ -z "$lines" ] && exit 0                          # nothing to say → silent

ctx="⟢ Anamnesis — your standing calibration (global ledger)
$lines
Log non-trivial predictions with /predict before acting; resolve them the moment reality answers."

jq -cn --arg c "$ctx" '{hookSpecificOutput:{hookEventName:"SessionStart",additionalContext:$c}}'
exit 0
