#!/usr/bin/env bash
# Anamnesis — install the calibration hooks for EVERY project.
#
#   bash install.sh --dry-run     # show exactly what would change (default)
#   bash install.sh --yes         # actually change it
#   bash install.sh --uninstall   # take it back out
#
# This edits ~/.claude/settings.json, which is YOUR configuration file, so it
# prints the exact JSON it will add and does nothing at all without --yes. A tool
# that asks you to be honest with yourself should not quietly rewrite your
# settings and tell you afterwards.
#
# Idempotent, and reversible with --uninstall. A .bak is kept on first change.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ANA_HOME="$HOME/.anamnesis"
S="$HOME/.claude/settings.json"
MODE="dry-run"

for arg in "$@"; do
  case "$arg" in
    --yes|-y)    MODE="install" ;;
    --dry-run|-n) MODE="dry-run" ;;
    --uninstall) MODE="uninstall" ;;
    -h|--help)   sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown option: $arg (try --help)" >&2; exit 2 ;;
  esac
done

command -v jq >/dev/null 2>&1 || { echo "jq is required (brew/apt install jq)" >&2; exit 1; }
mkdir -p "$(dirname "$S")"
[ -f "$S" ] || echo '{}' > "$S"

HOOKDIR="$ANA_HOME/hooks"

hook_json() {
  jq -n --arg h "$HOOKDIR" '{
    SessionStart:     [{hooks:[{type:"command",command:("bash "+$h+"/session-start.sh"),timeout:5}]}],
    UserPromptSubmit: [{hooks:[{type:"command",command:("bash "+$h+"/user-prompt.sh"),timeout:5}]}],
    Stop:             [{hooks:[{type:"command",command:("bash "+$h+"/stop.sh"),timeout:5}]}],
    PostToolUse:      [{matcher:"Bash",hooks:[{type:"command",command:("bash "+$h+"/post-tool.sh"),timeout:5}]}]
  }'
}

# If the marketplace plugin is ALSO installed, its own hooks/hooks.json registers
# the same four events and everything would fire twice — two session-start blocks,
# two checkpoints, two auto-resolves.
plugin_also_installed() {
  local d
  for d in "$HOME/.claude/plugins"/*/anamnesis "$HOME/.claude/plugins/anamnesis"; do
    [ -d "$d" ] && return 0
  done
  return 1
}

if [ "$MODE" = "uninstall" ]; then
  tmp="$(mktemp)"
  jq --arg h "$HOOKDIR" '
    .hooks |= with_entries(
      .value |= map(select((tostring | contains($h)) | not))
    )
    | .hooks |= with_entries(select(.value | length > 0))
  ' "$S" > "$tmp"
  jq -e . "$tmp" >/dev/null || { echo "refusing to write invalid JSON" >&2; exit 1; }
  cp "$S" "$S.bak.anamnesis-uninstall"
  mv "$tmp" "$S"
  echo "✓ Anamnesis hooks removed from $S"
  echo "  a backup of the previous file is at $S.bak.anamnesis-uninstall"
  echo "  your ledger at ${ANAMNESIS_AGENT_DATA:-$ANA_HOME/agent.json} was NOT touched."
  exit 0
fi

already="$(jq --arg h "$HOOKDIR" '[.hooks // {} | to_entries[] | .value[] | select(tostring | contains($h))] | length' "$S")"

echo "This will add the following to $S:"
echo
hook_json | sed 's/^/    /'
echo
if [ "$already" -gt 0 ]; then
  echo "  ($already Anamnesis hook entr(y|ies) are already registered; re-running will not duplicate them.)"
fi
if plugin_also_installed; then
  echo
  echo "  ⚠ The Anamnesis PLUGIN also appears to be installed, and it registers the"
  echo "    same four events. Installing both means every hook fires TWICE."
  echo "    Pick one: either this script, or the plugin — not both."
fi
echo
echo "It will also copy the engine and hook scripts into $ANA_HOME/."
echo "Your ledger is never modified by this script."
echo

if [ "$MODE" != "install" ]; then
  echo "Nothing was changed. Re-run with --yes to apply, or --uninstall to remove."
  exit 0
fi

mkdir -p "$ANA_HOME/bin" "$ANA_HOME/hooks"

# 1) engine — reuse ana on PATH, else an already-vendored one, else fetch prebuilt.
if command -v ana >/dev/null 2>&1; then
  cp "$(command -v ana)" "$ANA_HOME/bin/ana"
elif [ -x "$ANA_HOME/bin/ana" ]; then
  :
else
  bash "$HERE/install-ana.sh" || {
    echo "Could not obtain ana — build it: cargo install --git https://github.com/Anbu-00001/Anamnesis --locked" >&2
    exit 1
  }
fi

# 2) hooks → a stable location, independent of this repo's checkout.
cp "$HERE/hooks/"*.sh "$ANA_HOME/hooks/"
chmod +x "$ANA_HOME/hooks/"*.sh

# 3) merge into user settings (idempotent, validated before it replaces anything).
[ -f "$S.bak.anamnesis" ] || cp "$S" "$S.bak.anamnesis"
tmp="$(mktemp)"
jq --arg h "$HOOKDIR" '
  def addhook(ev; entry):
    .hooks[ev] = ((.hooks[ev] // [])
      + (if ((.hooks[ev] // []) | tostring | contains($h)) then [] else [entry] end));
  .hooks = (.hooks // {})
  | addhook("SessionStart";     {hooks:[{type:"command",command:("bash "+$h+"/session-start.sh"),timeout:5}]})
  | addhook("UserPromptSubmit"; {hooks:[{type:"command",command:("bash "+$h+"/user-prompt.sh"),timeout:5}]})
  | addhook("Stop";             {hooks:[{type:"command",command:("bash "+$h+"/stop.sh"),timeout:5}]})
  | addhook("PostToolUse";      {matcher:"Bash",hooks:[{type:"command",command:("bash "+$h+"/post-tool.sh"),timeout:5}]})
' "$S" > "$tmp"
jq -e . "$tmp" >/dev/null || { echo "refusing to write invalid JSON" >&2; exit 1; }
mv "$tmp" "$S"

echo "✓ Anamnesis installed."
echo "  engine : $("$ANA_HOME/bin/ana" --version)"
echo "  ledger : ${ANAMNESIS_AGENT_DATA:-$ANA_HOME/agent.json}"
echo "  hooks  : SessionStart, UserPromptSubmit (every 7th), Stop, PostToolUse(Bash) → $S"
echo "  backup : $S.bak.anamnesis"
echo "  → run /hooks (or restart Claude Code) to activate in the current session."
echo "  → undo with: bash $HERE/install.sh --uninstall"
