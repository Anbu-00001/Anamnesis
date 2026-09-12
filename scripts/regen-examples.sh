#!/usr/bin/env bash
# Regenerate the example output embedded in README.md from the real binary.
#
# Anything between a pair of
#     <!-- BEGIN:<name> -->  ...  <!-- END:<name> -->
# markers is replaced with the current output of the corresponding command. CI
# runs this and fails if the README moves, so a printed example can never drift
# from what the tool actually does — which is how `35 resolved · 6 open` sat in
# the docs above `88% graded (44 of 50)`.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release --quiet

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
ANA=./target/release/ana

"$ANA" demo --keep "$TMP/demo.json" >/dev/null

emit() {
  case "$1" in
    report) "$ANA" --data "$TMP/demo.json" report ;;
    plain)  "$ANA" --data "$TMP/demo.json" report --plain ;;
    help)   "$ANA" --help ;;
    *) echo "unknown example block: $1" >&2; exit 1 ;;
  esac
}

python3 - "$TMP" <<'PY'
import re, subprocess, sys, pathlib
tmp = sys.argv[1]
readme = pathlib.Path("README.md")
text = readme.read_text()

def run(name):
    cmd = {
        "report": ["./target/release/ana", "--data", f"{tmp}/demo.json", "report"],
        "plain":  ["./target/release/ana", "--data", f"{tmp}/demo.json", "report", "--plain"],
        "help":   ["./target/release/ana", "--help"],
    }[name]
    return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout.rstrip("\n")

def repl(m):
    name = m.group(1)
    return f"<!-- BEGIN:{name} -->\n```\n{run(name)}\n```\n<!-- END:{name} -->"

new = re.sub(
    r"<!-- BEGIN:(\w+) -->.*?<!-- END:\1 -->",
    repl,
    text,
    flags=re.S,
)
if new != text:
    readme.write_text(new)
    print("README examples regenerated")
else:
    print("README examples already current")
PY
