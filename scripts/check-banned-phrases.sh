#!/usr/bin/env bash
# Fail if the program, or anything it ships, can tell a user they are
# "well calibrated".
#
#   ./scripts/check-banned-phrases.sh
#
# Finding B of the pre-launch audit arrived three times by three different routes:
# the confidence gap (where over- and under-confidence cancel), the verdict state
# table via a -1e-15 direction, and `label()` keying on `n` alone while the
# calibration error sat at 1.9x its noise floor. Each time the output was the same
# two words on a forecaster who was not.
#
# It also shipped a fourth way, which is why the scope reaches past src/. The
# 0.3.0 plugin's hook scripts computed their own verdict from the confidence gap
# with jq and printed "well-calibrated overall" into every session. Those scripts
# kept working against 0.4.0's JSON, so upgrading the binary did not stop them.
#
# Scope: string literals in non-test Rust under src/, non-comment lines of the
# shell scripts under plugin/, and all of the markdown and JSON under plugin/,
# since every word there reaches a user or the model. Rust comments and tests may
# name the phrase; that is how the reasoning survives.
set -euo pipefail
cd "$(dirname "$0")/.."

BANNED=(
  "well calibrated"
  "well-calibrated"
  "confidence is honest"
)

fail=0
while IFS= read -r f; do
  case "$f" in
    *.rs)
      # Everything from `#[cfg(test)]` on is test code, which may name it.
      body="$(awk '/^#\[cfg\(test\)\]/{exit} {print}' "$f")"
      scanned="$(grep -vE '^[[:space:]]*(//|/\*|\*)' <<<"$body" || true)"
      ;;
    *.sh)
      scanned="$(grep -vE '^[[:space:]]*#' "$f" || true)"
      ;;
    *)
      scanned="$(cat "$f")"
      ;;
  esac
  for phrase in "${BANNED[@]}"; do
    if hits="$(grep -inF "$phrase" <<<"$scanned")"; then
      echo "check-banned-phrases: \"$phrase\" reachable in $f" >&2
      sed 's/^/    /' <<<"$hits" >&2
      fail=1
    fi
  done
done < <(find src -name '*.rs'; find plugin -type f \( -name '*.sh' -o -name '*.md' -o -name '*.json' \))

if [ "$fail" -ne 0 ]; then
  echo "" >&2
  echo "  A quiet e-process is absence of evidence, not evidence of calibration." >&2
  echo "  See docs/METHODS.md 'Why there are two numbers, and what each one cannot see'." >&2
  exit 1
fi
echo "check-banned-phrases: clean"
