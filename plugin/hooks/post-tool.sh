#!/usr/bin/env bash
# Anamnesis — post-tool hook.
#
# After a plain test command that succeeded, resolves any open kind:tests-pass prediction for this project FROM THE EXIT STATUS — not from the agent's account of what happened. (A command that FAILED arrives as PostToolUseFailure; see post-tool-failure.sh.)
#
# The logic lives in `ana hook post-tool`; this is only the launcher.
exec bash "$(dirname "${BASH_SOURCE[0]}")/_run.sh" post-tool
