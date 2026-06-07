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
  `Recalibration::apply`: `p ↦ σ(a+b·logit p)`, the mechanical self-correction).
  This is the load-bearing core; everything else is plumbing. Types: `Sample`
  (binary), `NumericSample` (interval).
- [src/model.rs](src/model.rs) — domain types + serde. `Claim` is a palimpsest
  (forecasts appended, never overwritten). `ClaimKind::{Binary,Numeric}`. `Forecast`
  holds `Option<prob>` xor `Option<interval>`; `Resolution` holds `Option<outcome>`
  xor `Option<value>`. `Ledger::index_of` resolves id prefixes.
- [src/store.rs](src/store.rs) — one JSON file, atomic write (temp + rename),
  missing file = empty ledger.
- [src/report.rs](src/report.rs) — **compute once into `ReportData`, render twice**
  (`render` = text, `render_json` = JSON). Never compute metrics in a renderer.
- [src/main.rs](src/main.rs) — clap CLI: `add/update/resolve/list/show/report/mcp`,
  global `--data` and `--json`. `list --tag` filters by tag; the agent ledger is
  `~/.anamnesis/agent.json` (`ANAMNESIS_AGENT_DATA`).
- [src/mcp.rs](src/mcp.rs) — `ana mcp`: a hand-rolled Model Context Protocol
  server over newline-delimited JSON-RPC stdio (no new deps), exposing
  predict/resolve/calibration/**recalibrate**/list as tools for any MCP agent.
  `recalibrate` returns a stated probability unchanged until the e-process finds
  real evidence (shares `report::RECAL_MIN_E`/`RECAL_MIN_N` with the report so
  they never diverge). The cross-agent reach surface; reuses scoring/report as the
  single source of truth.
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

- Your self-ledger lives at
  `~/.claude/projects/-home-anbu/memory/anamnesis_self_ledger.json` (see
  `MEMORY.md`). Drive it with `ana --data <that path>`.
- **Protocol**: at the start of a non-trivial task, log honest predictions *before*
  acting — "tests pass first try", "this needs N tool-calls" (`--interval`), "the
  migration is backward compatible". Tag them `who:claude,session:<date>`. Resolve
  each the moment reality answers, *before* you rationalise. Then
  `ana --data <self> report --tag who:claude` to see your standing miscalibration.
- The lesson compounds: if your `personal`/self predictions are overconfident (they
  were, in the demo seed), plan with more slack next time.
