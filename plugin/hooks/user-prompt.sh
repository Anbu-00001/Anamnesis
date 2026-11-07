#!/usr/bin/env bash
# Anamnesis — user-prompt hook.
#
# On about one prompt in N (ANAMNESIS_INTROSPECT_EVERY, default 7), drawn at random and logged to checkpoints.jsonl, re-surfaces the standing calibration as a self-introspection checkpoint. The draw makes its effect measurable (docs/MEASUREMENT.md, addendum 3).
#
# The logic lives in `ana hook user-prompt`; this is only the launcher.
exec bash "$(dirname "${BASH_SOURCE[0]}")/_run.sh" user-prompt
