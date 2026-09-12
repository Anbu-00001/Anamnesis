#!/usr/bin/env bash
# Anamnesis — session-start hook.
#
# Injects the agent's standing calibration, plus anything due in this repo, before the first prompt.
#
# The logic lives in `ana hook session-start`; this is only the launcher.
exec bash "$(dirname "${BASH_SOURCE[0]}")/_run.sh" session-start
