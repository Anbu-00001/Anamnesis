#!/usr/bin/env bash
# Anamnesis — UserPromptSubmit hook: periodic self-introspection checkpoint.
#
# Every Nth user prompt in a session (default 7), re-inject the agent's standing
# calibration plus an explicit directive to run a full report and ADJUST. The
# SessionStart hook greets ONCE; this keeps the agent honest mid-session — because
# a soft "remember to introspect" instruction is exactly the thing an agent skips
# while heads-down on a task. The fix is mechanical, not motivational: a
# deterministic counter, not willpower.
#
# Contract: never block, never error out loud, stay COMPLETELY SILENT on the other
# 6 of every 7 prompts and whenever there is nothing worth saying. The counter is
# per-session (keyed by session_id) and self-cleaning. Fail-open throughout.
set -uo pipefail

EVERY="${ANAMNESIS_INTROSPECT_EVERY:-7}"
case "$EVERY" in ''|*[!0-9]*) EVERY=7 ;; esac
[ "$EVERY" -gt 0 ] 2>/dev/null || EVERY=7

LEDGER="${ANAMNESIS_AGENT_DATA:-$HOME/.anamnesis/agent.json}"
ANA="$(command -v ana 2>/dev/null || true)"
[ -z "$ANA" ] && [ -x "$HOME/.anamnesis/bin/ana" ] && ANA="$HOME/.anamnesis/bin/ana"
[ -x "$ANA" ] || exit 0
command -v jq >/dev/null 2>&1 || exit 0

input="$(cat 2>/dev/null || true)"
sid="$(jq -r '.session_id // "default"' <<<"$input" 2>/dev/null || echo default)"
[ -n "$sid" ] || sid="default"
sid="${sid//[^A-Za-z0-9._-]/_}"     # keep it a safe filename

# Per-session prompt counter; self-cleaning (drop counters older than a day).
cdir="$HOME/.anamnesis/counters"
mkdir -p "$cdir" 2>/dev/null || exit 0
find "$cdir" -type f -mtime +1 -delete 2>/dev/null || true
cf="$cdir/$sid"
n=0; [ -f "$cf" ] && n="$(cat "$cf" 2>/dev/null || echo 0)"
case "$n" in ''|*[!0-9]*) n=0 ;; esac
n=$((n + 1))
printf '%s' "$n" > "$cf"

# Fire only on every Nth prompt — silent otherwise.
[ $(( n % EVERY )) -eq 0 ] || exit 0

# Need a ledger with something graded to say anything useful.
[ -f "$LEDGER" ] || exit 0
report="$("$ANA" --json --data "$LEDGER" report --tag who:claude 2>/dev/null)" || exit 0
[ -n "$report" ] || exit 0
resolved="$(jq -r '.resolved // 0' <<<"$report" 2>/dev/null || echo 0)"

MIN_N="${ANAMNESIS_MIN_N:-6}"
if [ "${resolved:-0}" -ge "$MIN_N" ]; then
  gap="$(jq -r '
    (.confidence_gap // empty) as $gap | (.resolved) as $n
    | ($gap*100|round) as $g | ((.accuracy//0)*100|round) as $a | ((.mean_confidence//0)*100|round) as $c
    | if ($gap|fabs) < 0.05 then "well-calibrated overall (\($n) resolved)"
      elif $gap > 0 then "OVERCONFIDENT +\($g)pts — right \($a)% but claim ~\($c)% (\($n) resolved); widen estimates, add slack"
      else "UNDERCONFIDENT \($g)pts — right \($a)% vs claimed ~\($c)% (\($n) resolved); trust strong calls more"
      end' <<<"$report" 2>/dev/null)"
else
  gap="only ${resolved:-0} resolved so far — keep logging; calibration is still noisy"
fi
[ -n "$gap" ] || exit 0

ctx="⟢ Anamnesis — self-introspection checkpoint (prompt #$n this session)
Standing calibration: $gap
Pause and run /calibration (or \`ana report --tag who:claude\`), read it, and ADJUST your next estimates accordingly. Resolve any prediction whose outcome is now known with /resolve <id> before hindsight rewrites how sure you were."

jq -cn --arg c "$ctx" '{hookSpecificOutput:{hookEventName:"UserPromptSubmit",additionalContext:$c}}'
exit 0
