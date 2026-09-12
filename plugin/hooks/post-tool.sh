#!/usr/bin/env bash
# Anamnesis — post-tool hook.
#
# After a test or build command, resolves any open kind:tests-pass prediction for this project FROM THE EXIT STATUS — not from the agent's account of what happened.
#
# The logic lives in `ana hook post-tool`; this is only the launcher.
exec bash "$(dirname "${BASH_SOURCE[0]}")/_run.sh" post-tool
