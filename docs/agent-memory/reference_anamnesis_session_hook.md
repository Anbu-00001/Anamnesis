---
name: anamnesis-session-hook
description: "How Anamnesis auto-loads my standing calibration at the start of every session, and what activates it."
metadata: 
  node_type: memory
  type: reference
  originSessionId: 434d3d50-1c5a-4981-acec-430b13856106
---

A global **SessionStart hook** in `~/.claude/settings.json` (`hooks.SessionStart`) runs
`bash "$HOME/.anamnesis/session-calibration.sh"`, which injects one tight paragraph of my
standing engineering calibration (confidence gap + over/under verdict, e-process evidence
level, overdue-prediction warning, a `ana decide` nudge) as `additionalContext` at the
start of *every* session and project. Set up 2026-06-08 (the user asked for it to trigger
automatically "without mentioning it").

The script reads the **JSON** report (`ana --data <agent ledger> --json report`) and is
token-cheap by design (a paragraph, not the whole report). It is a deliberate **NO-OP**
(exit 0, no output) until both conditions hold, so it's harmless when the tool isn't set up:
- `ana` is on PATH (or `ANAMNESIS_BIN` set, or `~/.anamnesis/bin/ana` exists), AND
- the agent ledger exists at `~/.anamnesis/agent.json` (or `ANAMNESIS_AGENT_DATA`).

**ACTIVATED 2026-06-08.** `ana` v0.3.0 is installed to `~/.local/bin/ana` (on PATH) and
`~/.anamnesis/bin/ana` (the hook's fallback); the agent ledger `~/.anamnesis/agent.json` has
9 resolved claims. End-to-end test passed: the hook emits valid SessionStart JSON injecting
e.g. *"…(9 resolved): confidence gap -0.17 — underconfident…"*. Fires on the **next** session
(not mid-turn). Edit/disable via `/hooks`.

**GOTCHA — the hook silently vanished once.** When re-checked on 2026-06-08 the
`hooks.SessionStart` key was *absent* from `~/.claude/settings.json` (a later settings write had
dropped it), so the auto-trigger was inert despite the script existing. Re-added + an
`env.ANAMNESIS_DATA` default in the same merge. **If calibration ever stops appearing at session
start, first check `jq '.hooks.SessionStart' ~/.claude/settings.json` is still present.**

**Canonical ledger = `~/.anamnesis/agent.json`** (what the hook + env read). It is a strict
*superset* of the legacy `~/.claude/projects/-home-anbu/memory/anamnesis_self_ledger.json` named
in CLAUDE.md's "For Claude" section — they began as identical copies; agent.json is now ahead.
**Footgun:** bare `ana` (no `--data`) otherwise defaults to `~/.anamnesis.json`, NOT the agent
ledger — fixed by `env.ANAMNESIS_DATA=/home/anbu/.anamnesis/agent.json` in `~/.claude/settings.json`
(takes effect next session), so a bare `ana add` now lands in the right ledger. Don't log to the
stale self_ledger.json path; use bare `ana` (or `--data ~/.anamnesis/agent.json`).
See [[anamnesis-research-roadmap]].
