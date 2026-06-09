# CLAUDE.md — orientation for an agent working on Anamnesis

This file exists so a future Claude (or any agent) can re-grasp this repo in one
read instead of re-deriving it from source every session. Keep it true; update it
when the shape of the code changes.

## What this is

`ana` is a local-first, offline, no-LLM CLI for fighting self-deception. You log a
falsifiable belief with a probability (binary) or a credible interval (numeric)
*before* the outcome is known; you resolve it later; the engine scores your
calibration. The thesis: **being able to tell true from false (discrimination) is
not the same as knowing how sure to be (calibration).** The report shows both.

## Architecture (one line each)

- [src/scoring.rs](src/scoring.rs) — **pure `std` math, no I/O.** Brier, log score,
  Murphy decomposition (exact, grouped by unique forecast value), rank-based AUC,
  Lichtenstein–Fischhoff overconfidence, Winkler interval score, coverage, Wilson
  interval, empirical-Bayes shrinkage, an **anytime-valid calibration e-process**
  (`calibration_eprocess`: a mixture of betting martingales, valid under
  continuous peeking — answers "is the miscalibration real, or n-too-small noise?")
  and a **ridge-shrunk logistic recalibration map** (`fit_recalibration` →
  `Recalibration::apply`: `p ↦ σ(a+b·logit p)`, the mechanical self-correction),
  a **bootstrap Brier band** (`brier_ci_bootstrap`, seeded SplitMix64 → reproducible),
  a **recency-weighted EWMA Brier** (`ewma_brier`, a descriptive "lately" trend — *not*
  a control-chart alarm, which false-alarms at an agent's n), a
  **confidence-vocabulary** count (`distinct_forecasts`), a **selective-prediction
  risk–coverage curve** (`risk_coverage`: error among your surest calls vs all — when
  to trust your own judgement), a **stake-weighted Brier** (`brier_weighted`: are you
  miscalibrated on the calls that *matter*?) and a **dialectical-bootstrapping**
  aggregator (`dialectical_mean`: average a first estimate with a "consider the
  opposite" second — an elicitation aid, not a score) and a **conformal interval
  recalibration** (`conformal_width_factor`: the multiplier on your credible-interval
  half-widths that makes them hit nominal coverage — the numeric analogue of the
  recalibration map, the split-conformal quantile of standardized residuals). This is
  the load-bearing core; everything else is plumbing. Types: `Sample` (binary),
  `NumericSample` (interval). Tier 3 reuses the e-process two more ways: per-`kind:`
  (multicalibration — which prediction *type* is really miscalibrated, anytime-valid
  so tiny subgroups can't false-alarm) and on interval coverage (`prob=level`,
  `outcome=contained`) to gate the width correction; `fit_recalibration`'s `(a,b)`
  doubles as the Cox calibration slope/intercept the report reads aloud. The
  **decision gate** `decide` (`Act::{Proceed,Verify,Abstain}` + `Decision`) is the
  operational end: recalibrate the stated `p`, then apply Chow's reject threshold
  `τ = 1 − verify_cost/stake` (proceed iff `p̂ ≥ τ`; abstain below even odds) — a
  number becomes an action, and the bar climbs with the stakes. `mean_boldness`
  (outcome-free `mean(max(p,1−p))`) and `asmd` (absolute standardized mean
  difference, the covariate-balance / missing-not-at-random effect size) feed the
  report's **resolution-discipline** check — is the calibration computed on a fair
  sample of your calls, or a self-selected one?
- [src/model.rs](src/model.rs) — domain types + serde. `Claim` is a palimpsest
  (forecasts appended, never overwritten). `ClaimKind::{Binary,Numeric}`. `Forecast`
  holds `Option<prob>` xor `Option<interval>`; `Resolution` holds `Option<outcome>`
  xor `Option<value>`. A `stake` field (serde-default 1.0, not serialised when
  default → old ledgers stay byte-identical) weights consequential calls;
  `compose_reasoning` weaves the outside-view reference class + dialectical
  estimates into the stored `because`. `Ledger::index_of` resolves id prefixes.
- [src/store.rs](src/store.rs) — one JSON file, atomic write (temp + rename),
  missing file = empty ledger.
- [src/report.rs](src/report.rs) — **compute once into `ReportData`, render five ways**
  (`render` = rich text, `render_json` = JSON, `render_plain` = plain-English bridge
  with a four-mood **calibration cat** `mood`, `render_html` = self-contained offline
  card, `render_badge_svg` = embeddable README badge). The HTML/SVG are pixel-faithful
  to a Claude Design handoff (`CARD_CSS` is verbatim; gauge `pos = 50 + gap·100`).
  Never compute metrics in a renderer; `plain_summary` builds the plain prose once so
  the text, HTML, and badge views can't drift. Also
  home of `earned_recalibration` (the shared evidence gate: fitted map + whether the
  e-process has earned it, next to the `RECAL_*` constants) reused by the CLI and the
  MCP `recalibrate`/`decide` tools so the gate is defined exactly once. `compute`
  takes `today: NaiveDate` (threaded through `render`/`render_json`; real clock in
  the CLI/MCP, a fixed date in tests) for the **`ResolutionDiscipline`** selection-bias
  check — resolution rate, overdue count, and the boldness `asmd` of graded vs
  ungraded calls — rendered up top as the honesty caveat on every number below.
- [src/main.rs](src/main.rs) — clap CLI: `add/update/resolve/list/show/report/decide/mcp`,
  global `--data` and `--json`. `decide --prob --stake` is the decision gate (below);
  `report --plain` (plain-English + cat), `--html` (offline card), `--badge` (README
  SVG) pick the renderer; `list --tag` filters by tag; the agent ledger is `~/.anamnesis/agent.json`
  (`ANAMNESIS_AGENT_DATA`).
- [src/mcp.rs](src/mcp.rs) — `ana mcp`: a hand-rolled Model Context Protocol
  server over newline-delimited JSON-RPC stdio (no new deps), exposing
  predict/resolve/calibration/**recalibrate**/**decide**/list as tools for any MCP
  agent. `recalibrate` returns a stated probability unchanged until the e-process
  finds real evidence; `decide` corrects it through that map then applies a
  stake-aware threshold (proceed/verify/abstain) — the operational end of
  calibration, the documented fix for "agents verbalize uncertainty but act anyway".
  Both share `report::earned_recalibration` via the thin `fit_and_gate` helper. The
  cross-agent reach surface; reuses scoring/report as the single source of truth.
- [bindings/python/](bindings/python/) — **PyO3 + maturin** binding exposing the
  pure `scoring` core to Python as an `abi3` wheel (`import anamnesis`). It is a
  *standalone* crate (its own empty `[workspace]`) depending on the core lib by
  path, so a bare `cargo build`/`clippy`/`test` of the core never touches pyo3.
  Thin delegates only — **one implementation, two languages, zero drift**. Build:
  `cd bindings/python && maturin develop && pytest`. Don't reimplement metrics
  here; wrap `anamnesis::scoring`.
- [examples/seed.rs](examples/seed.rs) — backdated demo ledger generator.
- [tests/cli.rs](tests/cli.rs) — drives the compiled binary end-to-end.

## Invariants you must not break

1. **`scoring` stays pure** — no I/O, no serde-only logic, no clap. It is the part
   that must be trustworthy and is the easiest to unit-test. Keep it that way.
2. **No NaN in serialized output.** serde_json silently turns `NaN`/`Inf` into
   `null` and cannot read it back. Model any undefinable metric as `Option<f64>`
   (→ `null`). `report::finite()` enforces this for floats that might be NaN.
3. **Exact Brier decomposition** depends on grouping by the *exact* forecast value
   (`f64::to_bits`). Don't switch it to range-binning — the
   `decomposition_identity_holds_exactly` test asserts `REL−RES+UNC == Brier`.
4. **Backward compatibility**: old ledgers have no `kind`, bare `prob` numbers, and
   string `outcome`s. The serde `default` + `Option` fields keep them loading. The
   `legacy_binary_json_still_loads` test guards this. Don't make those fields
   required.
5. **The integration test depends on exact output substrings**: `added [id]`,
   `30% → 60%`, `resolved TRUE`, `already resolved`, `between 0 and 1`,
   `no claim matches`. If you change these strings, update [tests/cli.rs](tests/cli.rs).
6. **Tests as oracles**: the fast `auc` is validated against a self-evidently
   correct `O(n²)` `auc_pairwise` in tests. When optimising a metric, keep the slow
   version as a test oracle rather than deleting it.

## Build / test / lint

```bash
cargo build --release            # binary: target/release/ana
cargo test                       # unit + integration
cargo clippy --all-targets -- -D warnings
cargo run --example seed -- seed.json && ./target/release/ana --data seed.json report
```

No dependency may be added without a real reason; current deps are clap, serde,
serde_json, chrono only, and `scoring` touches none of them.

## How to extend (recipes)

- **A new metric**: add a pure fn to `scoring.rs` with a unit test (ideally against
  a hand-computed value or a slow oracle). Surface it as a field on
  `report::ReportData`, then render it in both `render` and (free) JSON.
- **A new command**: add a `Cmd` variant in `main.rs`, handle `cli.json`, keep
  error messages friendly (they go to stderr, exit code 1).

## For Claude: use this tool ON YOURSELF

Anamnesis is built for a mind that forgets — which is exactly what you are. Use it
to calibrate your own engineering judgement across sessions. This is not a gimmick;
it is the point.

- Your self-ledger (the agent ledger) is the canonical `~/.anamnesis/agent.json`;
  bare `ana` already defaults there via the `ANAMNESIS_DATA` env, so a stray
  `ana add` lands in the right place. (A legacy copy at
  `~/.claude/projects/.../memory/anamnesis_self_ledger.json` is a claim or two
  behind — prefer the agent ledger.) The SessionStart hook reads it to greet every
  session with your standing over/under-confidence; a `UserPromptSubmit` hook
  (`plugin/hooks/user-prompt.sh`, mirrored live at `~/.anamnesis/hooks/user-prompt.sh`)
  then re-injects it as a **self-introspection checkpoint every 7th prompt**
  (`ANAMNESIS_INTROSPECT_EVERY`) — a mechanical counter, because you will not
  remember to self-audit on your own. When that checkpoint fires, actually run
  `ana report --tag who:claude` and adjust; don't just acknowledge it.
- **Protocol**: at the start of a non-trivial task, log honest predictions *before*
  acting — "tests pass first try", "this needs N tool-calls" (`--interval`), "the
  migration is backward compatible". Tag them `who:claude,session:<date>`. Resolve
  each the moment reality answers, *before* you rationalise. Then
  `ana --data <self> report --tag who:claude` to see your standing miscalibration.
- The lesson compounds: if your `personal`/self predictions are overconfident (they
  were, in the demo seed), plan with more slack next time.

## Design rationale & decisions (folded from the agent's working memory)

*Distilled from the session memory that lived only on one machine
(`~/.claude/.../memory/`) so the **why** survives in the repo — across accounts,
devices, and re-clones — not just the **what**. Captured 2026-06-09.*

### The corrected thesis: mechanical, not motivational

The calibration win is **mechanical, not motivational**. Lean on the recorded track
record + a deterministically-applied recalibration map ("I feel 60% → log 80%"), not
on nudging a model to "introspect harder" — introspection is unreliable. **Inputs
dominate downstream math:** elicitation quality and logging compliance matter more
than any new metric — which is why the scoring core is deliberately small and now
*saturated*. Everything in `scoring.rs` stays pure `std` (no deps, no LLM, no
network); elicitation lives in the workflow/`predict` layer.

### Why each pillar exists (the external evidence, not recalled intuition)

A 2023–2026 literature review drives the design:
- **Models have little self-knowledge** (Generalized Correctness Models, arXiv
  2509.24988): a model predicting its own correctness does no better than an unrelated
  one — reliable confidence is learned from *correctness history*, not introspection.
- **Recorded feedback works without weight updates** (Reflexion, arXiv 2303.11366):
  episodic track record materially improves agents — the mechanism the hook relies on.
- **Training rewards confident guessing** (OpenAI 2025; code-calibration lit): models
  are *structurally* pushed toward overconfidence, so an external instrument is the
  counter-pressure. In code, token-probability confidence beats *verbalized* — the
  numbers we log are the weakest signal, which is precisely *why* a mechanical
  recalibration layer is needed.
- **Anytime-valid e-processes** (Henzi–Ziegel arXiv 2103.08402; Ramdas): fixed-n tests
  (Spiegelhalter Z) are **invalid under per-session peeking** (false-positive
  0.05→0.15); the e-process (running product, valid under optional stopping) is why
  "Is it real?" stays honest when you check it every session — it **supersedes
  Spiegelhalter** as the gate.
- **Crowd-within** (Herzog–Hertwig 2009): one deliberate "consider the opposite"
  estimate recovers ~half the gain of a second person (consider **2** counter-reasons,
  not 10) → `dialectical_mean` + the `predict` protocol.
- **Multicalibration** (Hébert-Johnson 2018): per-`kind:` calibration, made peek-proof
  by running the e-process *within* each subgroup so a tiny fluky group can't false-alarm.

### The 2026 frontier — why the decision gate is the centerpiece

A fresh pass found the frontier has **moved off scoring math onto decision-coupling**:
agents *verbalize* uncertainty accurately yet **fail to act on it** — taking
irreversible actions while saying they're unsure; self-improvement loops *raise*
overconfidence. The named fix (ReDAct arXiv 2604.07036 — uncertainty-aware deferral
for LLM agents) is **confidence-gating against a calibrated threshold**, grounded in the
decision-theoretic evaluation of probabilities (Ferrer & Ramos, arXiv 2408.02841) — exactly
`scoring::decide`: recalibrate the stated `p` (evidence-gated), then Chow's reject rule
`τ = 1 − verify_cost/stake` → **Proceed / Verify / Abstain**, the bar climbing with the
stakes. A Monte-Carlo study (`bindings/python/validation/validate_guarantees.py`)
confirms it lowers expected decision cost (100% win-rate, ~8% cheaper). This is the
load-bearing operational payoff — the literature's #1 agent open-problem made concrete.

### Deliberately NOT built (don't re-litigate)

- **CRPS** — for the interval format we log, the Winkler score already *is* its
  specialization (WIS → CRPS as #intervals → ∞); CRPS needs a distribution shape we
  never recorded = more math, no new truth.
- **CUSUM / control-chart alarms & Adaptive Conformal Inference** — they false-alarm at
  an agent's small n; the EWMA "lately" line is descriptive only, and the static pooled
  `conformal_width_factor` is more stable than ACI's learning-rate knob.
- **A bespoke LangChain binding** — `ana mcp` is already consumed by LangChain/LangGraph
  via `langchain-mcp-adapters` with zero custom code. The real gap was the *scoring core
  as a numpy-friendly Python lib* (the PyO3 binding). Extend it by wrapping
  `anamnesis::scoring`, **never** by reimplementing math in Python.
- **An incomplete-beta exact coverage interval** — Wilson ≈ Jeffreys at small n, and the
  e-process is the better peek-proof gate.

### Status, surfaces & the working agreement

- **At saturation:** Tiers 1–3 + the decision gate + the resolution-discipline /
  selection-bias check are shipped; further work is polish, usage, or genuinely new
  research — not backlog.
- **Five renderers, one computation:** `report` (rich) · `--plain` (plain English + the
  calibration cat) · `--html` (offline card) · `--badge` (README SVG) · `--json`. The
  HTML/SVG are pixel-faithful to a Claude Design handoff; `plain_summary` builds the
  prose once so the views can't drift.
- **Python:** one PyO3/maturin `abi3` wheel wrapping the pure core — one implementation,
  two languages, zero drift.
- **Working agreement:** the **user handles ALL git** (commit, branch, push) — never
  commit or push; stage at most, and only when asked. Never commit secrets/`.env`, the
  local `.claude/` config, or large models/datasets. The tool is built first for the
  agent itself: log predictions *before* acting, resolve *before* rationalising.
