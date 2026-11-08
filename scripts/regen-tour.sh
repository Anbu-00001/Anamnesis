#!/usr/bin/env bash
# Regenerate the README animation: docs/assets/tour.gif and docs/assets/tour.mp4.
#
#   ./scripts/regen-tour.sh
#
# Needs Node 22+, Google Chrome (or $CHROME), ffmpeg and python3. Like regen-demo.sh, it
# is GENERATED, never hand-made: every line of `ana` output in it is captured here from
# the built binary, on a fictional ledger in a temporary directory that is deleted
# afterwards. The bet goes through the real `ana hook`, with a payload captured from
# Claude Code (tests/fixtures/). The motion design is docs/tour/tour.html; open it in a
# browser to watch it with placeholder text.
set -euo pipefail
cd "$(dirname "$0")/.."

for t in node ffmpeg python3; do
  command -v "$t" >/dev/null || { echo "missing $t" >&2; exit 1; }
done
command -v "${CHROME:-google-chrome}" >/dev/null || { echo "missing Chrome (set CHROME)" >&2; exit 1; }

cargo build --release --quiet
ANA="$PWD/target/release/ana"
FIXTURE="$PWD/tests/fixtures/claude-code-2.1.251/pass_cargo_test.json"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
D="$tmp/home"; C="$tmp/cap"
mkdir -p "$D/.anamnesis" "$C"

(
  export HOME="$D" USERPROFILE="$D" ANAMNESIS_DATA="$D/mine.json" ANAMNESIS_AGENT_DATA="$D/.anamnesis/agent.json"
  unset CLAUDE_CODE_SESSION_ID AI_AGENT ANAMNESIS_PIN_NUDGE ANAMNESIS_AUDIT_RATE || true
  tidy() { sed "s#$D#~#g"; }

  # Three earlier calls (backdated through import), then the one the animation shows.
  printf 'statement,prob,created,resolve_by,outcome,resolved_at,tags\n%s\n%s\n%s\n' \
    "the migration runs clean on staging,0.85,2026-10-20,2026-10-27,yes,2026-10-26,deploy" \
    "the p99 regression is in the cache layer,0.9,2026-10-22,2026-10-29,no,2026-10-28,debug" \
    "we ship the search rewrite this sprint,0.6,2026-10-28,2026-11-04,yes,2026-11-03,deploy" > "$tmp/h.csv"
  "$ANA" import "$tmp/h.csv" >/dev/null

  by="$(date -u -d '+7 days' +%F 2>/dev/null || date -u -v+7d +%F)"
  stmt="the flaky test is a race in the connection pool"
  echo "ana add \"$stmt\" --prob 0.7 --by $by" > "$C/add_cmd"
  "$ANA" add "$stmt" --prob 0.7 --by "$by" | tidy > "$C/add_out"
  id="$(sed -n 's/^added \[\([^]]*\)\].*/\1/p' "$C/add_out")"
  echo "ana resolve $id yes" > "$C/resolve_cmd"
  "$ANA" resolve "$id" yes | tidy > "$C/resolve_out"
  "$ANA" report | tidy > "$C/report"

  # The bet: typed as a whole prompt, blocked by the hook; then graded by the run after
  # an edit, on the agent's next turn.
  sess=00000000-0000-0000-0000-000000000000
  prompt() { printf '{"session_id":"%s","hook_event_name":"UserPromptSubmit","cwd":"/work/project","prompt":"%s"}' "$sess" "$1"; }
  prompt '?70' | "$ANA" hook user-prompt | tidy > "$C/bet.json"
  printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_EDIT","name":"Edit","input":{}}]}}' > "$tmp/t.jsonl"
  run="$(python3 -I -c 'import json,sys; v=json.load(open(sys.argv[1])); v["transcript_path"]=sys.argv[2]; print(json.dumps(v))' "$FIXTURE" "$tmp/t.jsonl")"
  printf '%s' "$run" | "$ANA" hook post-tool >/dev/null
  prompt 'now fix the parser' | "$ANA" hook user-prompt >/dev/null
  printf '%s' "$run" | "$ANA" hook post-tool | tidy > "$C/grade.json"
  "$ANA" --version | awk '{print $2}' > "$C/version"
)

python3 -I - "$C" "$tmp/data.json" <<'PY'
import json, os, sys
c, out = sys.argv[1], sys.argv[2]
r = lambda n: open(os.path.join(c, n)).read().rstrip("\n")
rep = r("report").splitlines()
def find(prefix):
    return next(i for i, l in enumerate(rep) if l.strip().startswith(prefix))
v = find("VERDICT")
why = rep[v + 1].strip()
why = why.split(". ")[0] + ". …" if ". " in why else why
g = find("Graded so far")
rows = []
for l in rep[g:]:
    if not l.strip():
        break
    rows.append(l)
excerpt = [rep[find("Resolution discipline") - 2].strip(), "", rep[v].strip(),
           " " * 17 + why, "", rep[find("Brier score")].strip(), ""] + [x[2:] for x in rows]
data = {
    "version": r("version"),
    "add_cmd": r("add_cmd"), "add_out": r("add_out"),
    "resolve_cmd": r("resolve_cmd"), "resolve_out": r("resolve_out"),
    "report": excerpt,
    "bet_reason": json.loads(r("bet.json"))["reason"],
    "grade_msg": json.loads(r("grade.json"))["systemMessage"],
    "install": "cargo install --git https://github.com/Anbu-00001/Anamnesis --locked",
}
json.dump(data, open(out, "w"), ensure_ascii=False, indent=1)
PY

if grep -iE '/home/|/Users/|/tmp/|PlayGround|who:claude' "$tmp/data.json"; then
  echo "the captured output contains a real path or tag — stopping" >&2
  exit 1
fi

node docs/tour/render.mjs docs/tour/tour.html "$tmp/data.json" "$tmp/frames" 30
mkdir -p docs/assets
ffmpeg -loglevel error -y -framerate 30 -i "$tmp/frames/f_%05d.png" \
  -c:v libx264 -pix_fmt yuv420p -crf 20 -preset slow -movflags +faststart docs/assets/tour.mp4
ffmpeg -loglevel error -y -framerate 30 -i "$tmp/frames/f_%05d.png" \
  -vf "fps=15,scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=96:stats_mode=diff[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" \
  docs/assets/tour.gif

for f in docs/assets/tour.gif docs/assets/tour.mp4; do
  echo "$f  $(( $(stat -c%s "$f" 2>/dev/null || stat -f%z "$f") / 1024 )) KiB"
done
size=$(stat -c%s docs/assets/tour.gif 2>/dev/null || stat -f%z docs/assets/tour.gif)
if [ "$size" -gt 4194304 ]; then
  echo "tour.gif is over the 4 MB budget" >&2
  exit 1
fi
