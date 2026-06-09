#!/usr/bin/env bash
# Anamnesis — one-time installer for true zero-touch calibration in EVERY project.
#
#   - installs `ana` + the hooks into ~/.anamnesis/
#   - registers SessionStart / Stop / PostToolUse hooks in ~/.claude/settings.json
#     (user scope — fires in every project, and is immune to the marketplace
#      SessionStart-hook bug). You never re-initialize per project again.
#
# Idempotent: re-running won't double-register. Reversible: remove the three
# entries from ~/.claude/settings.json (a .bak is left on first run).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ANA_HOME="$HOME/.anamnesis"
mkdir -p "$ANA_HOME/bin" "$ANA_HOME/hooks"
command -v jq >/dev/null 2>&1 || { echo "jq is required (brew/apt install jq)" >&2; exit 1; }

# 1) engine — reuse ana on PATH, else an already-vendored one, else fetch prebuilt.
if command -v ana >/dev/null 2>&1; then
  cp "$(command -v ana)" "$ANA_HOME/bin/ana"
elif [ -x "$ANA_HOME/bin/ana" ]; then
  :
else
  bash "$HERE/install-ana.sh" || { echo "Could not obtain ana — build it: cargo install --git https://github.com/Anbu-00001/Anamnesis ana" >&2; exit 1; }
fi

# 2) hooks → stable location independent of this repo.
cp "$HERE/hooks/"*.sh "$ANA_HOME/hooks/"
chmod +x "$ANA_HOME/hooks/"*.sh

# 3) register hooks in user settings.json (merge, idempotent, validated).
S="$HOME/.claude/settings.json"
mkdir -p "$(dirname "$S")"
[ -f "$S" ] || echo '{}' > "$S"
[ -f "$S.bak.anamnesis" ] || cp "$S" "$S.bak.anamnesis"
tmp="$(mktemp)"
jq --arg h "$ANA_HOME/hooks" '
  def addhook(ev; entry):
    .hooks[ev] = ((.hooks[ev] // [])
      + (if ((.hooks[ev] // []) | tostring | contains($h)) then [] else [entry] end));
  .hooks = (.hooks // {})
  | addhook("SessionStart";    {hooks:[{type:"command",command:("bash "+$h+"/session-start.sh"),timeout:5}]})
  | addhook("UserPromptSubmit"; {hooks:[{type:"command",command:("bash "+$h+"/user-prompt.sh"),timeout:5}]})
  | addhook("Stop";            {hooks:[{type:"command",command:("bash "+$h+"/stop.sh"),timeout:5}]})
  | addhook("PostToolUse";     {matcher:"Bash",hooks:[{type:"command",command:("bash "+$h+"/post-tool.sh"),timeout:5}]})
' "$S" > "$tmp"
jq -e . "$tmp" >/dev/null && mv "$tmp" "$S"

echo "✓ Anamnesis installed."
echo "  engine : $("$ANA_HOME/bin/ana" --version)"
echo "  ledger : ${ANAMNESIS_AGENT_DATA:-$ANA_HOME/agent.json}"
echo "  hooks  : SessionStart, UserPromptSubmit(every 7th), Stop, PostToolUse(Bash) registered in $S"
echo "  → run /hooks (or restart Claude Code) to activate in the current session."
