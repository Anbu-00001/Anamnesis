#!/usr/bin/env bash
# Anamnesis — stop hook.
#
# Names any predictions that are past due and ungraded, because until they are graded the calibration numbers rest on a self-selected sample.
#
# The logic lives in `ana hook stop`; this is only the launcher.
exec bash "$(dirname "${BASH_SOURCE[0]}")/_run.sh" stop
