#!/usr/bin/env bash
# Anamnesis — post-tool-failure hook.
#
# Claude Code reports a command that exits non-zero as PostToolUseFailure, never as
# PostToolUse, with the exit code only inside the text of `error`. A failing test run
# arrives here. Without this hook a prediction that "the tests pass" could only ever be
# graded when it was right.
#
# The logic lives in `ana hook post-tool-failure`; this is only the launcher.
exec bash "$(dirname "${BASH_SOURCE[0]}")/_run.sh" post-tool-failure
