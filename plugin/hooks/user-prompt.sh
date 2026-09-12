#!/usr/bin/env bash
# Anamnesis — user-prompt hook.
#
# Every Nth prompt (ANAMNESIS_INTROSPECT_EVERY, default 7), re-surfaces the standing calibration as a self-introspection checkpoint. A counter, not willpower.
#
# The logic lives in `ana hook user-prompt`; this is only the launcher.
exec bash "$(dirname "${BASH_SOURCE[0]}")/_run.sh" user-prompt
