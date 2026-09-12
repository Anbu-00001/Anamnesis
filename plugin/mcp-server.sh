#!/usr/bin/env bash
# Launch the Anamnesis MCP server for the plugin.
#
# Locates the `ana` engine the same way the hooks do — PATH first, then the
# vendored install at ~/.anamnesis/bin — and execs it. A wrapper rather than a
# direct `command` entry in plugin.json, because the binary's location depends on
# how the user installed it.
#
# The ledger is deliberately NOT put under ${CLAUDE_PLUGIN_DATA}: the agent ledger
# is global on purpose, so calibration follows the agent into every project
# rather than restarting per install. ANAMNESIS_AGENT_DATA overrides it.
set -uo pipefail

ANA="$(command -v ana 2>/dev/null || true)"
[ -z "$ANA" ] && [ -x "$HOME/.anamnesis/bin/ana" ] && ANA="$HOME/.anamnesis/bin/ana"

if [ ! -x "${ANA:-}" ]; then
  # stderr only: stdout is the JSON-RPC channel and must carry nothing else.
  echo "anamnesis: the 'ana' engine is not installed, so the MCP server cannot start." >&2
  echo "  install it:  bash \"\${CLAUDE_PLUGIN_ROOT}/install-ana.sh\"" >&2
  exit 1
fi

exec "$ANA" mcp
