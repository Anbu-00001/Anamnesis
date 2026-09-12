#!/usr/bin/env bash
# Shared launcher for every Anamnesis hook.
#
# All these scripts do now is find the engine and hand it the hook's JSON:
#     bash _run.sh <session-start|user-prompt|post-tool|stop>
#
# They used to each re-derive the calibration verdict with their own `jq`
# expression — three more copies of logic that has to agree with the report, and
# a hard dependency on `jq`. `ana hook` does the work; this file only locates it.
#
# Failure policy: a hook must never block a tool call, so a missing engine is not
# an error — but it is also no longer SILENT. Every script here used to `exit 0`
# without a word when `ana` or `jq` was absent, so a user whose hooks never fired
# had no way at all to find out why. Now it says so once per day, to the user
# only, and carries on.
set -uo pipefail

event="${1:?usage: _run.sh <event>}"

ANA="$(command -v ana 2>/dev/null || true)"
[ -z "$ANA" ] && [ -x "$HOME/.anamnesis/bin/ana" ] && ANA="$HOME/.anamnesis/bin/ana"

if [ ! -x "${ANA:-}" ]; then
  stamp="$HOME/.anamnesis/.missing-engine-warned"
  today="$(date +%Y-%m-%d)"
  if [ "$(cat "$stamp" 2>/dev/null || true)" != "$today" ]; then
    mkdir -p "$(dirname "$stamp")" 2>/dev/null || true
    echo "$today" > "$stamp" 2>/dev/null || true
    printf '%s\n' '{"systemMessage":"anamnesis: the `ana` engine is not on PATH, so calibration hooks are doing nothing. Install it, or remove the plugin. See: https://github.com/Anbu-00001/Anamnesis#install"}'
  fi
  exit 0
fi

exec "$ANA" hook "$event"
