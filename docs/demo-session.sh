#!/usr/bin/env bash
# The commands shown in the README recording, in order.
#
# This file is what the GIF shows. Regenerate with `scripts/regen-demo.sh`, which
# builds a throwaway fictional ledger first and points $ANAMNESIS_DATA at it, so
# no real record is ever recorded.
set -euo pipefail

# No leading pause: the first frame of the GIF is its poster, so it has to
# carry the first command rather than an empty terminal.
prompt() { printf '\033[1;32m$\033[0m %s\n' "$*"; sleep 0.7; }

prompt 'ana add "the flaky test is a race in the connection pool" --prob 0.7 --by 2026-10-01'
out="$(ana add "the flaky test is a race in the connection pool" --prob 0.7 --by 2026-10-01)"
echo "$out"
id="$(sed -n 's/^added \[\([^]]*\)\].*/\1/p' <<<"$out")"
echo
sleep 1.5

prompt "ana resolve $id yes"
ana resolve "$id" yes
echo
sleep 1.8

prompt 'ana report'
ana report
sleep 2.5
