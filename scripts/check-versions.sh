#!/usr/bin/env bash
# One version, three files. They were 0.3.0, 0.2.0 and 0.1.0 at the time of the
# pre-launch audit, which is the kind of thing nobody notices until a user does.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo_v=$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)
plugin_v=$(grep -m1 '"version"' plugin/.claude-plugin/plugin.json | cut -d'"' -f4)
market_v=$(grep -m1 '"version"' .claude-plugin/marketplace.json | cut -d'"' -f4)

fail=0
for pair in "plugin/.claude-plugin/plugin.json:$plugin_v" ".claude-plugin/marketplace.json:$market_v"; do
  file="${pair%%:*}"; got="${pair##*:}"
  if [ "$got" != "$cargo_v" ]; then
    echo "version mismatch: Cargo.toml is $cargo_v but $file is $got" >&2
    fail=1
  fi
done

# Every plugin entry in the marketplace, not just the first.
if command -v python3 >/dev/null 2>&1; then
  python3 - "$cargo_v" <<'PY' || fail=1
import json, sys
want = sys.argv[1]
d = json.load(open(".claude-plugin/marketplace.json"))
bad = [p["name"] for p in d.get("plugins", []) if p.get("version") != want]
if bad:
    print(f"version mismatch: marketplace plugins {bad} are not {want}", file=sys.stderr)
    raise SystemExit(1)
PY
fi

if [ "$fail" -eq 0 ]; then
  echo "versions agree: $cargo_v"
fi
exit "$fail"
