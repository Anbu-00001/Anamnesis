# FACTS

Bullet facts for the launch. **No prose, no pitch** — the human writes all of
that. Everything here is checkable, and where it is a number, it came out of a
command on this machine. Anything that could not be verified says so.

Last verified: **2026-09-12**.

---

## Basics

- **What it is, in nouns:** a command-line tool; a single JSON file; a scoring
  engine; an MCP server; a Claude Code plugin.
- **License:** MIT.
- **Dependencies:** clap, serde, serde_json, chrono. The scoring core uses none of
  them — it is `std` only.
- **MSRV:** Rust 1.89 (`File::lock` is std from there).
- **Release binary size:** 2.2 MB, x86_64 Linux, `--release` (measured 2026-09-12; it was 2.1 MB before this release's additions).
- **The demo's headline calibration error is `1.16x` its noise floor** — inside
  the 1.0–1.5 band where the report prints the ratio and deliberately says nothing
  more. Kept on purpose: the phrasing carries the meaning without prose, a ratio
  in that band is the modal presentation of real drift, and the fictional
  forecaster reading worse than this repo's author (`0.50x`) is the right way
  round. Stable — the four claims open past 2026-12-31 are unresolved, so they
  become priced gaps and move only the backlog cost (1.39x → 2.61x), not the
  ratio. Pinned by `hn_scenarios::the_demo_sits_in_the_silent_band_on_purpose`.

- **Tests:** 120 — 86 unit, 8 CLI integration, 26 hostile-review scenarios, pinned by
  `scripts/check-test-count.sh` in CI so a test that stops running is noticed, and
  `scripts/check-banned-phrases.sh` so "well calibrated" cannot come back. Plus
  36 Python binding tests. Verified 2026-09-12: wheel built with
  `maturin develop --release` (abi3, Python ≥ 3.8), `pytest` 36/36 green, and the
  wheel cross-checked against the CLI on the same ledger — `brier`, `mcb`, `dsc`,
  `unc`, `log_score`, `auc` and the e-process agree to 1e-9.

- **Platforms built in CI:** x86_64 and aarch64 Linux (gnu), x86_64 linux-musl,
  x86_64 and aarch64 macOS, x86_64 Windows.
- **Install:** `cargo install --git https://github.com/Anbu-00001/Anamnesis --locked`,
  or the checksum-verifying `plugin/install-ana.sh`.

## Commands

`add`, `update`, `resolve`, `void`, `amend`, `list`, `show`, `report`, `decide`,
`demo`, `import`, `where`, `hook`, `mcp`.

## Privacy and network

- **What leaves the machine: nothing.** No network code in the `ana` binary, no
  telemetry, no account, no phone-home. Verifiable: the dependency list is four
  crates, none of which is an HTTP client.
- The HTML card contains **zero JavaScript**.
- The ledger is plain JSON at `~/.anamnesis.json` (human) and
  `~/.anamnesis/agent.json` (agent). `ana where` prints both.

## MCP

- **Supported revisions:** `2026-07-28` (modern, stateless, per-request `_meta`,
  `server/discover`), and `2025-11-25` / `2025-06-18` / `2025-03-26` / `2024-11-05`
  via the legacy `initialize` handshake. The server is dual-era.
- Unsupported versions return `UnsupportedProtocolVersionError`, code `-32022`,
  listing what is supported.
- **Tools:** predict, update, resolve, calibration, recalibrate, decide, void, amend, list.

## The measurements

Each has a script; none is from memory.

| claim | number | how to check |
|---|---|---|
| Scoring the final forecast is exploitable | Brier 0.000 → 0.250 | `validation/repro.sh` |
| Concurrent writes lost claims | 7–19 of 40 survived; now 40/40 | `validation/repro.sh` |
| Exact-value grouping inflates calibration error | 0.073 vs CORP 0.014 at n=200, truth 0.000 | `validation/sims.py` |
| Single-strategy e-process is blind to symmetric overconfidence | e = 0.087 at n=1000 | `validation/gen_ledgers.py` + `examples/audit.rs` |
| The mixture fixes it without false alarms | power 2% → 100% at n=100; null 0.0–2.3% vs 5% bound | `validation/sims.py` |
| Resolution-order evidence false-alarms under peeking | 100% → 0% | `validation/peeking.py` |
| The e-process survives continuous peeking | 0.011 vs a z-test's 0.345 | `bindings/python/validation/validate_guarantees.py` |
| The decision gate lowers expected cost | 0.6556 → 0.6005, winning 100% of runs | same |
| The recalibration map could diverge | returned a=66, b=146; corrected 0.9 → 1.0 for a 50%-accurate forecaster. Fixed. | `scoring::tests::recalibration_does_not_diverge_on_a_narrow_vocabulary` |

## Limitations — state all of these

- **The agent benefit is unmeasured.** There is no evidence here that using this
  makes an agent or a person measurably better at anything. It measures; it does
  not claim to improve.
- **Whether quick-feedback calibration transfers to slow, real decisions is not
  measured by this project.** The forecasting literature is not settled on it.
- **The e-process assumes each outcome is calibrated given the earlier ones.**
  Several claims about one underlying event are correlated and can trip it.
- **The threat model is hindsight bias, not tampering.** Nothing stops someone
  editing their own JSON, and nothing is signed.
- **MCB is optimistically biased** (the isotonic fit is chosen on the same data),
  which is why the noise floor is printed beside it.
- **No verdict below 20 graded calls**, and the words "well calibrated" never
  appear below 50.
- **Windows is covered by CI but has not been used in anger.**

## Package names — RE-CHECK BEFORE PUBLISHING

**Both manifests currently declare a name that is taken on both registries.**
`Cargo.toml` and `bindings/python/pyproject.toml` both say `name = "anamnesis"`,
so a publish of either cannot succeed as they stand. This has to be decided and
changed before the tag, not at publish time.

Checked 2026-09-12; 404 means free.

| name | crates.io | PyPI |
|---|---|---|
| `anamnesis` | taken | taken (an unrelated project) |
| `ana` | taken | taken |
| `anamnesis-cli` | taken | free |
| `anamnesis-calibration` | **free** | **free** |
| `calibration-ledger` | **free** | **free** |

```bash
curl -s -o /dev/null -w "%{http_code}\n" -A "name-check" https://crates.io/api/v1/crates/NAME
curl -s -o /dev/null -w "%{http_code}\n" https://pypi.org/pypi/NAME/json
```

Also avoid the Python **import** name `anamnesis`: it collides at import time with
the existing PyPI project if a user has both installed.

## Renaming the packages — what it touches

`anamnesis-calibration` was still free on both registries on 2026-09-13.

**Renaming only the distribution is not enough on PyPI.** The existing `anamnesis`
1.0.4 ("Object serialisation to/from HDF5 and via MPI") installs a top-level module
named exactly `anamnesis`. Verified from its wheel: `top_level.txt` reads
`anamnesis`, and that is the wheel's only package directory. Two distributions both
writing `site-packages/anamnesis/` means whichever installs second wins, and
`import anamnesis` silently resolves to a serialisation library with the scoring
functions gone.

**Must change**

| What | Where |
|---|---|
| crates.io package name | `Cargo.toml` `[package] name` |
| Rust library name | every `anamnesis::` path (`src/main.rs`, `examples/`, `bindings/python/src/lib.rs`, a few docs) — or keep them by adding `[lib] name = "anamnesis"`. A library name may differ from its package name; a clash with the crates.io `anamnesis` crate would then fail loudly at build time rather than silently |
| path dependency | `bindings/python/Cargo.toml`: `anamnesis = { path = "../.." }`, or `anamnesis = { package = "anamnesis-calibration", path = "../.." }` |
| PyPI distribution name | `bindings/python/pyproject.toml` `[project] name` |
| Python import name | `[tool.maturin] module-name = "anamnesis._core"`, the directory `bindings/python/python/anamnesis/`, `_core.pyi`, and the cdylib `[lib] name` in `bindings/python/Cargo.toml` |
| `import anamnesis` | `bindings/python/tests/test_scoring.py`, `bindings/python/validation/validate_guarantees.py`, `validation/ratio.py`, `bindings/python/README.md`, `docs/PYTHON.md` |
| `pip install anamnesis` | `bindings/python/README.md`, `bindings/python/validation/validate_guarantees.py` |
| later | registry version badges, and the MCP registry's `server.json`, when they exist |

**Must not change**

- `ana`, the binary. It is a `[[bin]]` target, registered nowhere.
- `~/.anamnesis/`, `~/.anamnesis.json` and every `ANAMNESIS_*` variable. These are
  user data and its configuration; renaming them orphans every existing ledger.
- The GitHub repository URL, the plugin and marketplace `name`, and the MCP
  `serverInfo` name. Separate namespaces, not registry packages.

`scripts/check-versions.sh` compares versions, not names, and is unaffected.

**Why this stayed hidden:** `cargo install --git … --locked` installs from a path,
not a registry name, so the one install command that was verified by running it
would have kept working whatever the manifest said.

## Release order — fixed, because the prebuilt install line 404s until a release exists

1. Tag `v0.4.0-rc.1` and let the release pipeline run.
2. Three-runner smoke test (fresh ubuntu, macos, windows) against **that exact
   release**: the README's install line, then `ana demo`, then `ana report`.
3. Tag `v0.4.0`; the pipeline runs again.
4. Smoke test again, against `v0.4.0`.
5. Post.

Do not post on the day of the first successful release unless step 4 has run
against that release. If the pipeline fails after the final tag, the README is
wrong at the worst possible moment, and a fail-closed installer does not rescue a
first impression.

## Comparison facts

- **Fatebook** — open-source web app, Slack integration, Chrome extension, public
  API, community integrations. https://github.com/Sage-Future/fatebook
- **Metaculus** — public forecasting platform; time-averaged scoring, coverage,
  a community prediction. https://www.metaculus.com/help/scores-faq/
- **Calibration quizzes** — trivia-based, instant feedback, no record kept.
- **Anamnesis** — local JSON ledger, CLI, agent hooks and MCP, a sequential
  evidence test, stake-aware `decide`.
- **PredictionBook** — **retired.** Read-only from 2024-01-10, and the site now
  points visitors at Fatebook. Checked 2026-09-12. Do not put it in the
  comparison table as if it were a live alternative.

## Repo metadata to set (human applies)

- **Description** (≤ 120 chars) — TODO(human). It should be the same sentence as
  the one at the top of the README, so the two cannot drift. A neutral fallback,
  if you want one to edit rather than a blank page (97 chars):
  `Log a forecast before you act, resolve it after, and see where your confidence is actually wrong.`
- **Topics:** `calibration`, `forecasting`, `brier-score`, `decision-making`,
  `cli`, `rust`, `mcp`, `claude-code`, `e-values`.
- **Social preview:** 1280×640 derived from `docs/assets/card-dark.png`.
- Discussions: still to enable.
- `CONTRIBUTING.md` — **done**.
- Issue templates — **done**: `.github/ISSUE_TEMPLATE/bug.md` and
  `verdict-looks-wrong.md`, the latter asking for an `ana export --anonymize`
  ledger.
- `CITATION.cff` — **done**, but the author block is `TODO(human)`: it carries
  your name on anything that cites this.

## Prior HN threads the human may choose to link

- Calibration quiz, April 2026: https://news.ycombinator.com/item?id=47660262
- Confidence calibration game, February 2026: https://news.ycombinator.com/item?id=47164019

*(Both were listed in the pre-launch audit. Open them before linking.)*

## Verified citations

Checked against Crossref and arXiv on 2026-09-12.

| work | status |
|---|---|
| Dimitriadis, Gneiting & Jordan, *Stable reliability diagrams for probabilistic classifiers*, PNAS 118(8), 2021, doi:10.1073/pnas.2016191118 | ✅ title, authors, journal confirmed. The arXiv preprint 2008.03033 carries the earlier title *Evaluating probabilistic classifiers*. |
| Arnold, Henzi & Ziegel, *Sequentially valid tests for forecast calibration*, Ann. Appl. Stat. 17(3), 2023, doi:10.1214/22-AOAS1697 | ✅ confirmed |
| Henzi & Ziegel, *Valid sequential inference on probability forecast performance*, Biometrika 109, doi:10.1093/biomet/asab047 | ✅ confirmed |
| Ferrer & Ramos, arXiv:2408.02841 | ✅ resolves. **Correct title is *Evaluating Posterior Probabilities: Decision Theory, Proper Scoring Rules, and Calibration*** — an earlier draft of the docs had this wrong. |

**Not re-checked, do not cite without opening:** Brier (1950); Murphy (1973);
Vovk & Wang (2021); Wang & Ramdas (2022); Waudby-Smith & Ramdas; Gneiting &
Raftery (2007); Winkler (1972); Ville (1939); Herzog & Hertwig (2009);
Hébert-Johnson et al. (2018).
