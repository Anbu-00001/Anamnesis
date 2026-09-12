# Good first issues — drafts for the human to post

Each one was checked against the code on 2026-09-12, not invented to fill a
quota. Each is small, self-contained, has an obvious place to put the test, and
does not touch the parts of the scoring core that are load-bearing for the
guarantee. Re-check before posting; the repo may have moved.

---

## 1. `ana report --since <DATE>` to look at a window of the record

**Labels:** `good first issue`, `enhancement`

`ana report` has `--tag`, `--bins`, and the four renderer flags, but no way to
ask "how have I been doing lately" as anything other than the built-in `Lately`
EWMA line. A date window is the obvious missing filter, and a natural first
change: `ReportData::compute` already takes `today`, so the plumbing for
date-awareness exists.

The one thing to be careful about, and worth saying in the issue: **filtering by
date changes which claims enter the evidence sequence**, and the sequence's
guarantee depends on its order being fixed before any outcome was known. A
`--since` filter is safe because the cut is a date the user picks, not something
the outcomes touch — but it must filter on the *due key*, the same key
`src/evidence.rs` orders by, not on resolution time. Ordering by resolution time
is the exact defect that made a calibrated forecaster false-alarm in 100% of
simulated runs.

Where: `src/main.rs` (the `Report` variant), `src/report.rs::compute`.
Test: a scenario in `tests/hn_scenarios.rs` driving the binary.

---

## 2. Wrap the three remaining scoring functions in the Python binding

**Labels:** `good first issue`, `python`

`bindings/python` wraps most of `anamnesis::scoring`, but three public functions
have no Python surface:

- `risk_coverage` — the selective-prediction risk/coverage curve
- `calibration_log_eprocess_seq` — the log-scale gap-filled e-process
  (`calibration_eprocess_seq` is wrapped, but the log variant is what you need to
  compare magnitudes past the `1e12` cap)
- `gate_recalibration_seq`

These are thin delegates: the rule in `CONTRIBUTING.md` is one implementation,
two languages, so the Python side must call into the Rust function rather than
reimplement anything. `calibration_eprocess_seq` in `bindings/python/src/lib.rs`
is the pattern to copy, including how it maps `None` outcomes to gaps.

Where: `bindings/python/src/lib.rs`, `python/anamnesis/__init__.py`,
`python/anamnesis/_core.pyi`, plus a test in `tests/test_scoring.py`.

---

## 3. `ana import` accepts only CSV

**Labels:** `good first issue`, `enhancement`

`ana import` takes a CSV with `statement, prob, created, resolve_by, outcome,
resolved_at, tags`. Anyone arriving from a forecasting platform has a JSON
export, not that CSV, so the on-ramp costs them a conversion script.

Adding a JSON input path is mechanical. Two things worth stating in the issue so
the first attempt does not have to be rewritten:

- **Imported claims need a `horizon_days`** set at creation the same way `ana add`
  sets it, or they fall back to the current global default and their position in
  the evidence sequence can shift when that default changes.
- A claim whose import gives it a `resolve_by` in the past **and** no resolution
  is an ungraded due claim on day one, which is correct and will be priced into
  the evidence test. That is the intended behaviour, not a bug to work around.

Where: `src/main.rs::cmd_import`. Test: `tests/cli.rs`.

---

## 4. The demo ledger carries no `kind:` tags

**Labels:** `good first issue`, `docs`

`ana demo` is the first thing most people run, and its per-group breakdown falls
back to the bare topic tags (`markets`, `tech`, …) because none of its 50 claims
carry a `kind:` tag. That works — the grouping rule picks whatever the ledger
populates — but it means the flagship output never demonstrates the `kind:`
namespace the agent workflow is built around.

This is a judgement call rather than a defect, which is why it is a good first
issue rather than a bug: adding `kind:` tags to `src/demo.rs` means choosing a
vocabulary for a *human* forecaster, and the existing one (`tests-pass`,
`bug-hypothesis`, `approach`, `compat`) is agent-shaped. Worth discussing on the
issue before writing code.

Note that `src/demo.rs` is byte-identical to what `examples/seed.rs` produces, and
`tests/hn_scenarios.rs::the_demo_sits_in_the_silent_band_on_purpose` pins the
demo's headline calibration ratio at 1.16x its noise floor. Changing the claims
will move that number and fail the test on purpose — read the comment there
before updating it.

---

## Deliberately not on this list

- Anything inside `src/scoring.rs`'s e-process, ordering, or gap-pricing. Those
  carry the anytime-valid guarantee and every one of them has a measured
  false-alarm rate behind it. They are not first issues.
- "Add more tests." Not actionable.
- Anything that would need the `check-banned-phrases.sh` or `check-test-count.sh`
  guards relaxed.
