#!/usr/bin/env bash
# Reproduces findings A, B, E against a built binary.
set -euo pipefail
ANA="${ANA:-./target/release/ana}"
W="$(mktemp -d)"; trap 'rm -rf "$W"' EXIT
id() { python3 -c "import sys,json;print(json.load(sys.stdin)['id'])"; }

echo "=== A: hindsight — log 0.5, update to 0.99, resolve YES ==="
L="$W/hindsight.json"
for i in $(seq 1 10); do
  c=$($ANA --json --data "$L" add "claim $i" --prob 0.5 --by 2026-08-01 | id)
  $ANA --data "$L" update "$c" --prob 0.99 >/dev/null
  $ANA --data "$L" resolve "$c" yes >/dev/null
done
$ANA --data "$L" report | grep -iE "brier score|TOWARD" || echo "(no match)"

echo "=== B: 90% on coin flips + 60% on sure things ==="
L="$W/verdict.json"
for i in $(seq 1 50); do
  c=$($ANA --json --data "$L" add "coinflip $i" --prob 0.9 --by 2026-08-01 | id)
  if [ $((i % 2)) -eq 0 ]; then $ANA --data "$L" resolve "$c" yes >/dev/null
  else $ANA --data "$L" resolve "$c" no >/dev/null; fi
done
for i in $(seq 1 50); do
  c=$($ANA --json --data "$L" add "surething $i" --prob 0.6 --by 2026-08-01 | id)
  $ANA --data "$L" resolve "$c" yes >/dev/null
done
$ANA --data "$L" report | grep -iE "brier skill|reliability" || echo "(no match)"
$ANA --data "$L" report --plain | grep -iE "DIALED|honest|Keep doing" || echo "(no match)"
$ANA --data "$L" report --badge | grep -oE ">[^<]{3,40}<" | head -3
cp "$L" /tmp/claude-1000/verdict_B.json 2>/dev/null || true

echo "=== E: 40 concurrent adds ==="
for run in 1 2 3; do
  L="$W/conc$run.json"
  for i in $(seq 1 40); do $ANA --data "$L" add "claim $i" --prob 0.5 >/dev/null 2>&1 & done
  wait
  python3 -c "
import json
try: print('run $run surviving:', len(json.load(open('$L'))['claims']), '/ 40')
except Exception as e: print('run $run CORRUPT:', type(e).__name__)"
done
