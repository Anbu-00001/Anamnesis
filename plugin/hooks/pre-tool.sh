#!/usr/bin/env bash
# Anamnesis — pre-tool hook (opt-in, and NOT registered by the plugin's hooks.json).
#
# Before a Bash call runs, refuses the first bare test run of a session once, with the exact
# `ana add --check` and `ana run` commands, so a pinned prediction exists BEFORE the outcome.
# Inert unless ANAMNESIS_PIN_NUDGE=1. Register it yourself in settings.json (PreToolUse,
# matcher "Bash") on a machine where you want that, and see docs/MEASUREMENT.md.
#
# The logic lives in `ana hook pre-tool`; this is only the launcher.
exec bash "$(dirname "${BASH_SOURCE[0]}")/_run.sh" pre-tool
