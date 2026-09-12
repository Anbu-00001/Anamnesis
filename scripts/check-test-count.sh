#!/usr/bin/env bash
# Run the test suite and fail if the number of tests that RAN has dropped.
#
#   ./scripts/check-test-count.sh
#
# Why this exists, specifically:
#
# A test in this repo lost its `#[test]` attribute during an edit and kept
# "passing" for several runs by not existing. That is the software version of the
# ungraded claim this whole tool is about — missing data read as success — and a
# project whose subject is not fooling yourself has an obligation to notice it.
#
# `cargo test` reports a green suite whether it ran 118 tests or 3. Only the
# count distinguishes them, so the count is pinned. Raise MIN when you add tests;
# lowering it is a deliberate act that shows up in review.
set -euo pipefail
cd "$(dirname "$0")/.."

MIN=119

out="$(cargo test --all --no-fail-fast 2>&1)"
echo "$out"

lines="$(grep -c '^test result:' <<<"$out" || true)"
if [ "$lines" -eq 0 ]; then
  echo "check-test-count: no 'test result:' lines — the suite did not run" >&2
  exit 1
fi

total="$(grep '^test result:' <<<"$out" \
  | sed -E 's/^test result: [a-zA-Z]+\. ([0-9]+) passed.*/\1/' \
  | awk '{n += $1} END {print n + 0}')"

if grep -qE '^test result: FAILED' <<<"$out"; then
  echo "check-test-count: the suite is red" >&2
  exit 1
fi

if [ "$total" -lt "$MIN" ]; then
  echo "check-test-count: only $total tests ran, expected at least $MIN." >&2
  echo "  A test that stops running is indistinguishable from one that passes." >&2
  echo "  If you removed tests on purpose, lower MIN in $0 in the same commit." >&2
  exit 1
fi

echo "check-test-count: $total tests ran (floor $MIN)"
