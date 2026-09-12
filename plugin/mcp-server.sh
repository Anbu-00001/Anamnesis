#!/usr/bin/env bash
# Launch the Anamnesis MCP server for the plugin.
#
# Runs the NEWEST `ana` engine it can find — on PATH or the vendored install at
# ~/.anamnesis/bin — the same way hooks/_run.sh does, and execs it. A wrapper rather than a
# direct `command` entry in plugin.json, because the binary's location depends on
# how the user installed it.
#
# The ledger is deliberately NOT put under ${CLAUDE_PLUGIN_DATA}: the agent ledger
# is global on purpose, so calibration follows the agent into every project
# rather than restarting per install. ANAMNESIS_AGENT_DATA overrides it.
set -uo pipefail

# Newest, not first-on-PATH: an older `ana` earlier on PATH used to shadow the
# vendored engine, so the agent's tools ran it — without `update`, in wording the
# release had removed. Kept in step with hooks/_run.sh, which resolves the same way.
version_of() { "$1" --version 2>/dev/null | awk 'NR==1{print $2}'; }
newer() {   # is dotted version $1 strictly newer than $2?
  awk -v a="$1" -v b="$2" 'BEGIN {
    n = split(a, x, "."); m = split(b, y, "."); k = (n > m) ? n : m
    for (i = 1; i <= k; i++) {
      if ((x[i] + 0) > (y[i] + 0)) exit 0
      if ((x[i] + 0) < (y[i] + 0)) exit 1
    }
    exit 1
  }'
}
ANA=""
best=""
for cand in "$HOME/.anamnesis/bin/ana" "$(command -v ana 2>/dev/null || true)"; do
  { [ -n "$cand" ] && [ -x "$cand" ]; } || continue
  v="$(version_of "$cand")"
  [ -n "$v" ] || continue
  if [ -z "$ANA" ] || newer "$v" "$best"; then
    ANA="$cand"
    best="$v"
  fi
done

if [ ! -x "${ANA:-}" ]; then
  # stderr only: stdout is the JSON-RPC channel and must carry nothing else.
  echo "anamnesis: the 'ana' engine is not installed, so the MCP server cannot start." >&2
  echo "  install it:  bash \"\${CLAUDE_PLUGIN_ROOT}/install-ana.sh\"" >&2
  exit 1
fi

exec "$ANA" mcp
