# Contributing

## Build and test

```bash
cargo build --release          # binary at target/release/ana
cargo test --all
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

The Python binding is a standalone crate with its own workspace, so a bare
`cargo build` of the core never touches pyo3:

```bash
cd bindings/python && maturin develop --release && pytest -q
```

MSRV is Rust 1.89 — `File::lock` is std from there, and `src/store.rs` depends on
it. CI checks that floor rather than assuming it.

## What CI checks beyond the tests

Three guards exist because this project has already been bitten by each of them.
They are ordinary scripts; run them locally before opening a PR.

```bash
./scripts/check-versions.sh        # one version across Cargo.toml, the plugin
                                   # manifest, the marketplace entry, and both
                                   # Python files
./scripts/check-test-count.sh      # runs the suite AND fails if the number of
                                   # tests that RAN drops below a floor
./scripts/check-banned-phrases.sh  # fails if the program, or a hook script it
                                   # ships, can tell a user they are "well
                                   # calibrated"
```

`check-test-count.sh` exists because a test here lost its `#[test]` attribute
during an edit and kept passing by not existing. A green suite looks identical
whether it ran 120 tests or 3; only the count distinguishes them. If you add
tests, raise `MIN` in the same commit. Lowering it should be visible in review.

`check-banned-phrases.sh` exists because the claim "well calibrated" reached
users three separate times by three different routes. A quiet sequential test is
absence of evidence, not evidence of calibration — see
[docs/METHODS.md](docs/METHODS.md). Comments and tests may name the phrase; code
that can print it may not. The scope includes the shell scripts, markdown and JSON
under `plugin/`, because that is where the phrase actually shipped: the 0.3.0 hook
scripts printed it into every session, and kept doing so against a 0.4.0 engine.

## Generated content is generated, not written

Two things in the repo are produced by scripts and must never be edited by hand.
CI fails if either is stale.

```bash
./scripts/regen-examples.sh   # the example output inside README.md, between
                              # <!-- BEGIN:name --> and <!-- END:name -->
./scripts/regen-demo.sh       # docs/assets/demo.gif, from docs/demo-session.sh
                              # and the fictional docs/demo-ledger.csv
```

Hand-writing example output is how `35 resolved · 6 open` sat in the docs directly
above `88% graded (44 of 50)` until an audit caught it. `regen-demo.sh` needs
`asciinema` and `agg`; it builds a throwaway fictional ledger and refuses to
commit a recording containing a real path or tag.

## Changing the scoring core

`src/scoring.rs` is pure `std`: no I/O, no serde, no clap. It is the part that has
to be trustworthy and the easiest to unit-test, so keep it that way. Add a metric
as a pure function with a test against a hand-computed value or a slow oracle —
the fast `auc` is validated against an obviously-correct `O(n^2)` version kept
specifically as a test oracle.

Anything that changes what a number *means* also needs a scenario in
`tests/hn_scenarios.rs` driving the real binary. Every defect this project has
found was found by running the tool, not by reading it.

## Docs

Diagrams live in `docs/*.md`, never in `README.md`: the README is published to
crates.io and PyPI, neither of which renders Mermaid, so a diagram there ships as
a raw code fence. There are three diagrams in the repo and that is the cap. Each
one needs `accTitle` and `accDescr`, a paragraph before it carrying the same
content in prose, and the name of the implementing function so drift is visible
at review time.

## Reporting a problem

Two issue templates: an ordinary bug, and "the verdict looks wrong", which asks
for an anonymized ledger. `ana export --anonymize` strips every free-text field,
replaces ids, rounds dates to the day and keeps only whitelisted tag namespaces,
leaving the numbers untouched — so you can send the shape of a record without
sending what it was about. `ana where` prints both ledger paths and any
environment variables overriding them; include its output.
