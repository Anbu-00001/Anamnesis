#!/usr/bin/env bash
# Regenerate the README recording. Requires `asciinema` and `agg`.
#
#   ./scripts/regen-demo.sh
#
# The recording is GENERATED, never hand-recorded, so it can be refreshed when
# the output changes instead of drifting away from it. The session it runs is
# docs/demo-session.sh; the ledger it runs against is the fictional demo year, in
# a temporary directory that is deleted afterwards. A real ledger is never used.
set -euo pipefail
cd "$(dirname "$0")/.."

for t in asciinema agg; do
  command -v "$t" >/dev/null || { echo "missing $t" >&2; exit 1; }
done

cargo build --release --quiet

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# A small fictional engineering history, committed as docs/demo-ledger.csv so the
# recording is reproducible. Deliberately smaller than `ana demo`'s year: no
# numeric claims and no revisions, which keeps the report short enough to read in
# a recording without cutting anything out of it.
./target/release/ana --data "$tmp/ledger.json" import docs/demo-ledger.csv >/dev/null
mkdir -p docs/assets

export ANAMNESIS_DATA="$tmp/ledger.json"
export PATH="$PWD/target/release:$PATH"
export PS1='$ '

asciinema rec "$tmp/demo.cast" \
  --overwrite --quiet --cols 100 --rows 46 \
  -c "bash docs/demo-session.sh"

agg --font-size 15 --theme asciinema --speed 1.0 \
  "$tmp/demo.cast" docs/assets/demo.gif

size=$(stat -c%s docs/assets/demo.gif)
echo "docs/assets/demo.gif  $((size / 1024)) KiB"
if [ "$size" -gt 2097152 ]; then
  echo "over the 2 MB budget — shorten the session or lower --font-size" >&2
  exit 1
fi

# The recording must never contain a real statement or a real path.
if grep -aiE '/home/|/Users/|PlayGround|who:claude' docs/assets/demo.gif >/dev/null; then
  echo "the recording contains a real path or tag — not committing it" >&2
  exit 1
fi
echo "clean: no real paths or tags in the recording"
