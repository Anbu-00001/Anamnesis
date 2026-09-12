#!/usr/bin/env bash
# Fail if the program can tell a user they are "well calibrated".
#
#   ./scripts/check-banned-phrases.sh
#
# Finding B of the pre-launch audit has now arrived three times by three
# different routes: the confidence gap (where over- and under-confidence cancel),
# the verdict state table via a -1e-15 direction, and `label()` keying on `n`
# alone while the calibration error sat at 1.9x its noise floor. Each time the
# output was the same two words on a forecaster who was not.
#
# The structural answer is that the phrase does not exist in the program. This
# keeps it that way: a future refactor that reintroduces it fails CI instead of
# shipping. Absence of evidence is not evidence of calibration, and the two
# instruments have opposite blind spots (docs/METHODS.md section 3b).
#
# Scope: string literals in non-test code under src/. Comments and doc comments
# may discuss the phrase freely — that is how the reasoning survives — and test
# code asserts on it deliberately.
set -euo pipefail
cd "$(dirname "$0")/.."

BANNED=(
  "well calibrated"
  "well-calibrated"
  "confidence is honest"
)

fail=0
for f in $(find src -name '*.rs'); do
  # Everything from `#[cfg(test)]` on is test code, which is allowed to name it.
  body="$(awk '/^#\[cfg\(test\)\]/{exit} {print}' "$f")"
  # Strip comment lines: the history is documented, and documenting it is fine.
  code="$(grep -vE '^[[:space:]]*(//|/\*|\*)' <<<"$body" || true)"
  for phrase in "${BANNED[@]}"; do
    if hits="$(grep -inF "$phrase" <<<"$code")"; then
      echo "check-banned-phrases: \"$phrase\" reachable in $f" >&2
      sed 's/^/    /' <<<"$hits" >&2
      fail=1
    fi
  done
done

if [ "$fail" -ne 0 ]; then
  echo "" >&2
  echo "  A quiet e-process is absence of evidence, not evidence of calibration." >&2
  echo "  See docs/METHODS.md 'Why there are two numbers, and what each one cannot see'." >&2
  exit 1
fi
echo "check-banned-phrases: clean"
