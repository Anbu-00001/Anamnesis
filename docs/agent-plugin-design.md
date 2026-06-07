# Design — Anamnesis for Agents: a calibration layer that auto-engages in every project

- **Status:** F1–F6 implemented & verified — 2026-06-07 (supersedes the original draft below; the plan held)
- **Authors:** Claude (Opus 4.8) + Anbu
- **Scope:** Phase F of [project-anamnesis-playground]. Plan only; no code until the open decisions in §13 are settled.
- **Context:** competitive research in `reference_anamnesis_competitive_landscape` — the agent-calibration niche is empty; every Claude-Code memory tool (claude-mem, self-improving-agent, Reflexion) is *qualitative*. This design is the *quantitative* layer.

---

## 1. Problem & goal

Today Anamnesis is an inert CLI: it only runs when an agent *remembers* to invoke it, and its ledger sits in one folder. The goal is to make calibration **follow the agent into every project automatically**, with near-zero friction, so that:

1. At the start of work in **any** repo, the agent sees its **standing calibration** ("you are +0.18 overconfident on effort estimates") *before* it plans.
2. Predictions made in one session are **surfaced for resolution** in later sessions/repos, so the score actually updates instead of rotting as "open".
3. The whole thing is **installable by any Claude Code user** in one command, local-first, no network.

**One-line thesis:** claude-mem *remembers*, self-improving-agent *curates lessons*, **Anamnesis keeps score** — the measurement layer that turns "I learned X" into "you're measurably overconfident on Y; plan with slack."

## 2. Non-goals

- Not a model-internals confidence calibrator (that's World 2 — ECE/logits). We score *task-level* predictions an agent states in natural language.
- Not a replacement for claude-mem; **complementary** (it could later read claude-mem summaries to *suggest* predictions).
- Not cloud/social. No accounts, no sync in v1. The ledger is a local JSON file the user owns.
- Not auto-magical resolution (see §10 — resolution is hook-*surfaced*, agent-*performed*).

## 3. Architecture overview

```
            ┌──────────────── any project repo ────────────────┐
 SessionStart hook ──reads──▶  ana  ──reads──▶  ~/.anamnesis/agent.json  (GLOBAL ledger)
        │  injects additionalContext (standing calibration + due predictions for THIS repo)
        ▼
   [ agent plans with calibration in view ]
        │  /predict "<claim>" --prob 0.6  --kind estimate   (logs BEFORE acting)
        ▼
   [ agent works ]
        │  PostToolUse(Failure) / Stop hooks ──surface──▶ "you have open predictions; resolve?"
        ▼
   /resolve <id> yes|no|--value N         (scores against the no-LLM engine)
```

Three moving parts: **(a)** a global ledger, **(b)** a thin set of `ana` enhancements, **(c)** a Claude Code *plugin* (hooks + commands + skill) that wires `ana` into the session lifecycle.

## 4. Ledger — location, scope, schema

- **Global agent ledger:** `~/.anamnesis/agent.json`, selected by a new env var `ANAMNESIS_AGENT_DATA` (falls back to that path). Kept **separate from the human default** `~/.anamnesis.json` so a person's personal forecasts and an agent's task predictions don't pollute each other.
- **Per-project ledgers:** optional, opt-in via the existing `--data <repo>/.anamnesis.json`. Not the default; most value comes from the cross-project global ledger.
- **No schema change to `Claim`.** We use the existing free-form **tags** (already used for `who:claude`, `session:<date>`) as the metadata channel. This keeps backward compatibility (the `legacy_binary_json_still_loads` guarantee holds) and reuses the existing `report --tag` / proposed `list --tag` filters.

### Tag taxonomy (convention, not schema)

| Tag | Meaning | Example |
|---|---|---|
| `who:claude` | author (vs human) | always set by the plugin |
| `session:<id>` | the session that logged it | `session:2026-06-07-a3f` |
| `project:<slug>` | repo it was made in | `project:anamnesis` (git toplevel basename, or path hash) |
| `kind:estimate` | effort/time/tool-count (usually a **numeric interval**) | "≤ 6 tool calls" → `--interval 2..6` |
| `kind:tests-pass` | "tests pass first try" | binary |
| `kind:bug-hypothesis` | "the bug is in X" | binary |
| `kind:approach` | "this approach will work" | binary |
| `kind:compat` | "this change is backward-compatible" | binary |

The `kind:*` tags are the payoff: the report can then say *"+0.30 overconfident on `bug-hypothesis`, well-calibrated on `tests-pass`"* — per-failure-mode calibration, which is far more actionable than a single global number.

## 5. `ana` enhancements required (small, additive, testable)

1. **`list --tag <t>`** + ensure `list` honors global `--json` (machine-readable open/due predictions). Needed by the SessionStart hook to fetch "due predictions for this project".
2. **`report --json` per-tag block**: expose per-`kind:*` confidence-gap and n in `ReportData` (already render twice; just add a `by_kind` slice keyed by the `kind:` tag family).
3. **`predict` / `resolve` ergonomics**: `predict` = thin alias of `add` that auto-stamps `who:claude`, `session:`, `project:` tags from env. (The plugin command can also just call `add` with tags; an alias is nicer but optional.)
4. **`report --min-n <k>`** (or hook-side gating): so calibration assertions are suppressed below a sample threshold — see §9 & Phase G.

All four are additive, each gets a unit/integration test, and none touch the pure `scoring` core except adding `by_kind` aggregation (pure, testable against a hand example).

## 6. Hook contracts

All hooks: **idempotent**, invoked via `bash hooks/<name>.sh` (don't rely on the exec bit — known marketplace bug strips it), **fail-open** (any error → emit nothing, never block), **<100 ms** (a Rust binary reading a small JSON file is sub-ms; shell overhead dominates). If `ana` is not on PATH and no vendored binary exists → emit nothing, silently.

### 6.1 `SessionStart` (cannot block; injects context)
- **stdin:** session JSON (cwd, source). **Action:** compute `project:<slug>` from `git rev-parse --show-toplevel` (fallback cwd basename); run `ana --json report` (global) and `ana --json list --due --tag project:<slug>`.
- **stdout (JSON):**
```json
{ "hookSpecificOutput": {
    "hookEventName": "SessionStart",
    "additionalContext": "⟢ Anamnesis (global, 142 resolved)\n  +0.18 OVERCONFIDENT on kind:estimate — right 61%, not the ~85% implied → add slack.\n  ▸ DUE here: [a3f] \"tests pass first try\" (80%) → /resolve a3f yes|no"
} }
```
- **Silence rule (critical):** if ledger empty, or nothing relevant AND no calibration line clears the §9 thresholds → print nothing. Installed-but-unused must be invisible.

### 6.2 `Stop` (can block; we will **not** block)
- After the agent finishes responding, if predictions were logged this `session:` and remain open, inject a one-line nudge: *"2 predictions still open this session — resolve any whose outcome is now known."* Never blocks; `decision:"block"` is **not** used (annoying + risky).

### 6.3 `PostToolUseFailure` / `PostToolUse` (resolve-at-moment-of-truth) — **deferred to Phase G**
- *(Not in v1 — see §12.4.)* When a **test/build command** fails or passes (detected by matching the tool command against a small regex of `cargo test|npm test|pytest|...`), and there is an open `kind:tests-pass` prediction in this session, inject: *"A test just FAILED — you predicted 'tests pass' at 80%. Resolve it now (before rationalizing): /resolve <id> no."* This catches the datapoint at the exact instant reality answers — the literature's key insight is that agents *rewrite* their confidence after the fact, so capture must be immediate.

### 6.4 `UserPromptSubmit` (optional, later)
- Could detect a new non-trivial task and remind "log a prediction first." High annoyance risk → defer to Phase G behind a setting.

## 7. Commands & skills

- **`/predict "<claim>" --prob 0.6 [--interval a..b] [--kind estimate] [--by DATE]`** → `ana add` on the global ledger with auto tags. The before-acting log.
- **`/resolve <id> yes|no | --value N [--note ...]`** → `ana resolve`.
- **`/calibration [--tag kind:estimate]`** → `ana report` (text) — the mirror, on demand.
- **Skill `calibration-protocol`** (the behavioral glue, shipped in the plugin's own CLAUDE.md fragment): "At the start of a non-trivial task, log 1–3 honest predictions (`/predict`) — especially the *uncertain* ones, not the safe ones. Resolve the moment reality answers, before you rationalize. Read the SessionStart calibration and **widen your plans where you are historically overconfident.**"

## 8. What gets injected — and the silence/noise budget

The single biggest adoption risk is **nagging**. Rules:
- **≤ 3 lines** of `additionalContext`, ever.
- A calibration line appears **only if** `n ≥ min_n` (default 10) for that slice **and** `|confidence_gap| ≥ 0.08`. Otherwise omitted (small-n is noise — see the n=8 AUC=0.14 artifact from this very project).
- Due/open predictions for the current project are always worth showing (they're actionable, not nagging).
- Empty/quiet ledger → **zero output**. Silence is the default.

## 9. Resolution strategy (the genuinely hard problem)

Open predictions that never resolve = a dead ledger. Full auto-resolution is impossible (mapping a real outcome to a claim needs judgment). So the contract is **hook-surfaced, agent-resolved**, with three nets:
1. **Catch-up** (`SessionStart`) — *v1*: surface `--due` predictions (past `resolve_by`) and prior-session opens for the current project every time the agent returns.
2. **End-of-turn** (`Stop`) — *v1*: nudge on still-open same-session predictions.
3. **Moment-of-truth** (`PostToolUse*`) — *Phase G*: catch test/build outcomes against open predictions the instant they resolve (before rationalization).
- Track **unresolved-rate** as a health metric in the report; if it climbs, the loop is failing.

## 10. Distribution & packaging

- **Layout:** ship the plugin inside this repo at `/plugin` (one repo to maintain), promote to its own repo only if it grows.
```
plugin/
├── .claude-plugin/plugin.json     # name, version, hooks, commands, skills
├── hooks/{session-start,stop,post-tool}.sh
├── commands/{predict,resolve,calibration}.md
├── skills/calibration-protocol/SKILL.md
└── README.md
marketplace.json                   # so `/plugin marketplace add <repo>` works
```
- **`ana` availability:** publish prebuilt `ana` binaries on GitHub Releases (linux/mac/win × arch); the plugin's install step fetches the right one to `~/.anamnesis/bin/ana`. Hooks look for `ana` on PATH then that vendored path. **Graceful fallback:** if absent, hooks emit nothing and `/calibration` prints an install hint. (Pure-prompt-only fallback — no scoring — is possible but degraded; decide in §13.)
- **Install UX:** `/plugin marketplace add Anbu-00001/anamnesis` → `/plugin install anamnesis`.

## 11. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Hooks fire twice / lose exec bit (filed CC bugs) | idempotent; invoke via `bash`; no +x reliance |
| Nagging kills adoption | ≤3 lines, n & gap thresholds, silent-when-empty |
| Small-n stats mislead | gate on `min_n`; Phase G adds Wilson/Bayesian CIs |
| Latency in every session | fail-open, <100ms, binary reads tiny JSON |
| **Gaming / selection bias** (agent logs only easy calls) | protocol stresses logging *uncertain* calls; report surfaces base-rate & n; honest limitation, not fully solvable |
| Privacy | 100% local, no network — core ethos preserved |
| Ledger corruption | already atomic write (temp+rename) in `store.rs` |

## 12. Resolved decisions (2026-06-07)

1. **Ledger separation:** ✅ separate — agent `~/.anamnesis/agent.json` (env `ANAMNESIS_AGENT_DATA`) distinct from the human `~/.anamnesis.json`.
2. **`ana` dependency:** ✅ **prebuilt binaries on GitHub Releases + silent fallback** — hooks emit nothing if `ana` is absent; `/calibration` prints an install hint. No pure-prompt mode in v1.
3. **Plugin home:** ✅ **subdir `/plugin` of this repo.**
4. **Resolve aggressiveness:** ✅ **v1 = SessionStart + Stop only.** The `PostToolUse` moment-of-truth nudge (§6.3) is **deferred to Phase G** behind a setting.
5. **Naming:** ✅ plugin id `anamnesis`; global commands `/predict` `/resolve` `/calibration` (namespace to `/ana:*` only if collisions arise).

## 13. Milestones (each independently verifiable)

- **F0 (this doc)** — design agreed.
- **F1** — `ana` enhancements (`list --tag/--json`, `report` `by_kind`, `--min-n`) + tests green.
- **F2** — global ledger + `predict/resolve/calibration` commands; manual end-to-end on a throwaway ledger.
- **F3 — the proof:** `SessionStart` hook injects calibration into a *fresh* session in a *different* repo. **Verification:** open a scratch project, confirm the system-reminder shows standing calibration + due predictions. This single milestone validates the entire thesis.
- **F4** — `Stop` resolution nudge; measure unresolved-rate. (`PostToolUse` moment-of-truth → Phase G.)
- **F5** — packaging: `plugin.json`, `marketplace.json`, prebuilt binaries, README + demo.

## 14. Success metrics

- The agent **reads and acts on** the SessionStart calibration (plans wider where overconfident).
- **Unresolved-rate** stays low (the loop closes).
- Calibration **confidence-gap → 0** over many sessions (the agent actually improves).
- A second person can `/plugin install` it and get value in <5 minutes.
