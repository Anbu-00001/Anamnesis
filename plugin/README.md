# Anamnesis — the calibration plugin for coding agents

> claude-mem *remembers*. self-improving-agent *curates lessons*. **Anamnesis keeps score.**

Every agent-memory tool is qualitative — none measure whether your *confidence*
matched reality. This plugin is the missing quantitative layer: log a falsifiable
prediction before you act, get scored by a no-LLM engine when reality answers, and
have your standing over/under-confidence injected into **every project** at session
start so you actually plan differently.

## What it does

- **`SessionStart` hook** — injects your standing calibration (and any predictions
  *due* in this repo) as a system reminder, before the first prompt. This is the
  auto-engagement: your calibration follows you into every folder.
- **`/predict`** — log a prediction (probability or credible interval) with a
  `kind:` so you learn *per type of call* (estimates vs bug-hypotheses vs …).
- **`/resolve`** — score it the moment reality answers.
- **`/calibration`** — the full mirror on demand.
- **`Stop` hook** — a quiet one-liner only when predictions are *overdue*.

It is local-first, no-network, no-LLM. The ledger is a plain JSON file you own at
`~/.anamnesis/agent.json` (override with `ANAMNESIS_AGENT_DATA`).

## Install

Requires the `ana` engine on `PATH` (or vendored at `~/.anamnesis/bin/ana`) and `jq`.

```
/plugin marketplace add Anbu-00001/Anamnesis
/plugin install anamnesis
```

Get `ana` from the [Anamnesis releases](https://github.com/Anbu-00001/Anamnesis/releases)
(prebuilt binaries) or `cargo install --path .` from the repo root.

## Notes / known issues

- Claude Code has had bugs where **SessionStart hooks don't fire for marketplace
  plugins** ([#11509](https://github.com/anthropics/claude-code/issues/11509),
  [#10997](https://github.com/anthropics/claude-code/issues/10997)). If the
  calibration banner doesn't appear, register the hook directly in your
  `settings.json` (see `hooks/hooks.json`) as a fallback, or just run `/calibration`.
- The hooks are **fail-open and silent** when the ledger is empty, the engine is
  missing, or there are too few resolved predictions (`ANAMNESIS_MIN_N`, default 6)
  to say anything trustworthy — installed-but-unused is invisible.
