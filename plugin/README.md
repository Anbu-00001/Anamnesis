# Anamnesis — the calibration plugin for coding agents

> claude-mem *remembers*. self-improving-agent *curates lessons*. **Anamnesis keeps score.**

Most agent-memory tools record what happened. None that I know of measure whether your *confidence*
matched reality. This plugin is the missing quantitative layer: log a falsifiable
prediction before you act, get scored by a no-LLM engine when reality answers, and
have your standing over/under-confidence injected into **every project** at session
start so you actually plan differently.

## What it does

- **`SessionStart` hook** — injects your standing calibration (and any predictions
  *due* in this repo) as a system reminder, before the first prompt. This is the
  auto-engagement: your calibration follows you into every folder.
- **`UserPromptSubmit` hook** — a *self-introspection checkpoint* every **7th** user
  prompt (override with `ANAMNESIS_INTROSPECT_EVERY`): re-surfaces your standing
  calibration mid-session and tells you to run `/calibration` and adjust. The
  SessionStart banner greets once and is easy to forget twenty edits later; a
  deterministic counter is the mechanical fix for "agents don't introspect on their
  own." Per-session, fail-open, silent on the other 6 of every 7.
- **`/predict`** — log a prediction (probability or credible interval) with a
  `kind:` so you learn *per type of call* (estimates vs bug-hypotheses vs …).
- **`/resolve`** — score it the moment reality answers.
- **`/calibration`** — the full mirror on demand.
- **`Stop` hook** — one line to *you* (not the model), once per session, only when predictions are overdue. It never returns context: Claude Code reads context from a Stop hook as "keep going".

It is local-first, no-network, no-LLM. The ledger is a plain JSON file you own at
`~/.anamnesis/agent.json` (override with `ANAMNESIS_AGENT_DATA`).

## Install

Requires the `ana` engine on `PATH` (or vendored at `~/.anamnesis/bin/ana`). The hooks need
nothing else. Only `plugin/install.sh`, the manual installer for the `settings.json`
fallback below, also needs `jq`.

```
/plugin marketplace add Anbu-00001/Anamnesis
/plugin install anamnesis
```

Get `ana` from the [Anamnesis releases](https://github.com/Anbu-00001/Anamnesis/releases)
(prebuilt binaries, checksum-verified) or `cargo install --path . --locked` from the repo root.

## Optional: the pin-first reminder

`hooks/pre-tool.sh` is a `PreToolUse` hook that is **not** registered by this plugin. With
`ANAMNESIS_PIN_NUDGE=1` it refuses the first bare test run of a session once, with the commands
to log a pinned prediction first and run through `ana run`. See `docs/AGENTS.md` and
`docs/MEASUREMENT.md`.

## Updating

Claude Code decides a plugin has an update by comparing its `version`, and third-party
marketplaces do not auto-update by default, so an install stays on its cached copy until
you ask. Run `/plugin marketplace update`, then update the plugin, and replace the `ana`
binary too: the plugin's hooks run whichever `ana` is newest, and check `ana --version`.

## Notes / known issues

- Claude Code has had bugs where **SessionStart hooks don't fire for marketplace
  plugins** ([#11509](https://github.com/anthropics/claude-code/issues/11509),
  [#10997](https://github.com/anthropics/claude-code/issues/10997)). If the
  calibration banner doesn't appear, register the hook directly in your
  `settings.json` (see `hooks/hooks.json`) as a fallback, or just run `/calibration`.
- The hooks are **fail-open and silent** when the ledger is empty, the engine is
  missing, or there are too few graded predictions (the verdict needs 20) to say
  anything trustworthy — installed-but-unused is invisible.
