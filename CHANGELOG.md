# Changelog

All notable changes to this project are documented here.
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] — unreleased

The scoring changes in this release are **breaking**: the same ledger will produce
different numbers than 0.3.0 did. In every case the new number is the more
truthful one, and the reason is given below.

### Breaking — scoring

- **The headline score now grades your FIRST forecast, not your last.**
  Scoring the final forecast meant a claim logged at 0.5, updated to 0.99 once the
  answer was obvious, and then resolved YES earned a Brier score of 0.000 — and a
  compliment. Ten such claims scored `Brier 0.000`; they now score `0.250`.
  The final forecast is still shown, labelled "not graded", and `brier_time_avg`
  (Brier weighted by how long each forecast stood, as Metaculus time-averages) is
  reported as a secondary figure.
  JSON: `score_basis` (always `"first"`), `brier_first`, `brier_final`,
  `brier_time_avg`, `late_updates`. `brier` equals `brier_first`.

- **The Brier decomposition is now CORP** (Dimitriadis, Gneiting & Jordan, PNAS
  2021) — isotonic regression via pool-adjacent-violators, instead of grouping by
  exact forecast value. With two-decimal probabilities most exact-value groups
  hold one claim, whose observed frequency is always 0 or 1, so the old
  reliability term reported large calibration error for forecasters who had none:
  simulated at n=200, a *perfectly calibrated* forecaster scored 0.073 where the
  truth is 0.000. CORP scores 0.014. The identity `S = MCB − DSC + UNC` still
  holds exactly, with no bins and no tuning parameter.
  JSON: `mcb`, `dsc`, `unc`, `mcb_null_q95`. `reliability` and `resolution` remain
  as **deprecated aliases** for one release.

- **The calibration error is now printed against a noise floor** — the 95th
  percentile of the same statistic for a calibrated forecaster making exactly your
  calls. Without it, "your calibration error is 0.034" is unreadable.

- **The evidence test mixes betting strategies**, so symmetric overconfidence no
  longer cancels itself out. A forecaster saying 90% when the truth is 65% *and*
  10% when the truth is 35% produced e = 0.087 at n = 1000 — no evidence at all.
  It now produces a number with 65 digits. Simulated power against that pattern at
  n = 100 went from 2% to 100%, with the null false-alarm rate staying inside the
  5% Ville bound (measured 0.0–2.3%).

- **Claims with no `--by` date get a 30-day grace horizon** before they are
  admitted to the evidence sequence (`ANAMNESIS_HORIZON_DAYS`). Without it the
  rule had a cliff: a claim was admitted the day it was written, so one open
  no-deadline claim froze the evidence test from that moment on — log "will we
  hit 10k users by 2028?" in week one and never see a number again.

- **Voiding a claim that has already resolved no longer removes it from the
  evidence sequence.** Void annuls a question, and for every score that is right,
  but dropping an outcome you have already seen retroactively edits a sequence
  whose entire guarantee rests on being fixed in advance — and the direction of
  abuse is self-flattery, since the ones you would void are the ones that went
  badly. Post-resolution voids stay in the sequence and are counted in the report
  (`voided_after_resolution`), so the edit is never silent. Pre-resolution voids
  are excluded as before: they carry no outcome and cannot move the e-value.

- **The evidence test consumes claims in an outcome-independent order** — by
  resolve-by date, or creation date when there is none — and stops at the first
  due-but-ungraded claim. Ordering by resolution time looked chronological but was
  outcome-dependent: YES answers arrive early and NO answers wait for the
  deadline, so a perfectly calibrated forecaster raised a false alarm in 100% of
  simulated runs when the report was re-read as claims resolved. It is now 0%.
  JSON: `evidence_n`, `evidence_blocked_by`, `evidence_waiting`,
  `evidence_blocked_without_deadline`, `eprocess_log`.

- **Per-kind e-values need a multiplicity correction** before they count as a
  finding: searching K subgroups for the worst is K tests, so a row must clear
  `20 × K`. JSON: `kind_alarm_threshold`.

- **The interval headline is now unitless** (`winkler_ratio` = Winkler ÷ width,
  mean and median). The raw mean Winkler score carried the units of whatever was
  being predicted, so one question measured in dollars dominated one measured in
  milliseconds. Raw Winkler is still shown per claim in `show`.

- **The recalibration slope is constrained to be non-negative.** An unconstrained
  fit reaches a negative slope whenever observed frequencies do not rise with the
  forecast — measured on a 15-claim fixture where twelve 0.9s failed and a lone
  0.6 came true, it converged to `b = −0.69`, a map correcting a stated 0.9 to
  *below* a stated 0.5. At that sample size it is noise, and inverting the
  ordering is not a calibration correction. Pinned at zero the map collapses to a
  constant, which is exactly what pool-adjacent-violators does with the same data.

- **The fitted map is now validated before it is used**: it must map `[0,1]` into
  `[0,1]`, be monotone non-decreasing, and agree in direction with the isotonic
  (PAV) fit that CORP already computes. PAV cannot diverge by construction, so it
  is a free oracle. A map failing any of these is replaced by the identity.

### Breaking — the verdict

- **One verdict, computed once, for every surface** (`report::verdict`). The plain
  report, the calibration cat, the badge SVG, the HTML card, `--json`, the MCP
  `calibration` tool and the hooks all derive their wording from it.
  Previously each keyed off the *confidence gap*, in which over- and
  under-confidence cancel — so a ledger of 50 calls at 0.9 on coin flips plus 50
  at 0.6 on sure things, with a Brier skill of −0.520, had a gap of −1e-15 and was
  announced as `[DIALED IN] · well calibrated`, badged `Well calibrated`, and
  advised to "keep doing what you're doing".
  JSON: `verdict`, one of `insufficient_data`, `no_evidence_of_miscalibration`,
  `calibrated_but_uninformative`, `overconfident`, `underconfident`,
  `biased_yes`, `biased_no`, `miscalibrated_both_ways`.
- **`calibrated_but_uninformative` now outranks any statement about the level**
  whenever the forecasts sort nothing (`DSC ≈ 0`) and there was more than one
  distinct forecast value to sort by. On the coin-flips ledger the honest finding
  is not that the confidence is too high — it is that the ranking carries no
  information, and advice about shading numbers up or down is premature. Fix the
  ranking first. With a single forecast value, `DSC = 0` is degenerate rather than
  a finding, and the level verdict still applies.
- No verdict at all below 20 graded calls, and never the words "well calibrated"
  below 50.
- The calibration cat's mood now tracks the size of the calibration error against
  its noise floor, not the confidence gap. Its happiest face answers to the same
  evidence bar as its happiest words: no `[DIALED IN]` below 50 graded calls.

### Fixed

- **Concurrent writes no longer lose claims.** Every command did load → modify →
  save with no lock, and every writer shared one temp filename. Measured: 40
  parallel `ana add` calls left 7, 10 and 17 claims of 40, and almost none of them
  reported an error. Now 40 of 40, every time. Saves are durable (unique temp file,
  `fsync`, a `.bak` of the last good ledger, directory fsync on Unix).
- **A corrupt ledger is never overwritten.** The error names the file and says the
  ledger was not modified.
- **`ana mcp` no longer depends on the human ledger**, which it does not use, so a
  corrupt `~/.anamnesis.json` cannot stop the agent-facing server from starting.
- **Ledger paths work on Windows.** Both paths read `$HOME` only, so a normal
  Windows setup silently fell back to a ledger in the *current directory* — every
  folder quietly getting its own record. Now `std::env::home_dir()`, and an error
  rather than a cwd fallback when there is no home directory.
- **The report's counts add up.** The header counted binary claims as "resolved"
  and all kinds as "open", printing `35 resolved · 6 open` for a 50-claim ledger.
- **MCP version negotiation.** The server echoed back whatever protocol version
  the client asked for, including `"1999-01-01"`. It now replies with a revision it
  actually supports.
- **The recalibration map could diverge, and correct in the wrong direction.**
  Found while testing the Python binding, not predicted by the audit. Undamped
  Newton overshoots whenever the fit reaches the saturated region — every `mu` is
  ~0 or ~1, so the Hessian weights `mu(1−mu)` vanish while the gradient does not.
  Measured on 200 calls all stated at 0.9 that came true half the time (an agent
  with a narrow confidence vocabulary), it returned `a = 66, b = 146`: a map that
  corrected 0.9 **up to 1.0** for a forecaster who was right half the time, and
  the gate happily applied it. Now damped with a backtracking line search — it
  returns 0.498 against a truth of 0.5 — and a fit that does not converge yields
  the identity map rather than a confidently wrong one.

- **MCP client identity.** Every prediction was tagged `who:claude` regardless of
  which client was connected, quietly corrupting any per-client comparison. The
  tag now comes from the client's own name, overridable with `ANAMNESIS_WHO`, and
  defaults to `who:unknown`.

### Added

- **`ana demo`** — builds a fictional year of predictions in a temporary
  directory, reports on it, and never touches a real ledger.
- **`ana import <file.csv>`** — bring an existing prediction history in. Columns:
  `statement, prob, created, resolve_by, outcome, resolved_at, tags`.
- **`ana export --anonymize`** — publish the shape of a real record without its
  content: every free-text field removed, ids replaced, dates rounded to the day,
  only whitelisted tag namespaces kept, and all the numbers unchanged. There is
  deliberately no un-anonymized mode.
- **`ana hook <session-start|user-prompt|post-tool|stop>`** — the Claude Code
  hooks moved into the binary. This removes the `jq` dependency and three separate
  copies of the verdict logic, and lets the PostToolUse hook grade
  `kind:tests-pass` claims **from the command's exit status** rather than from the
  agent's account of what happened (`resolved_by: "auto"`).
- **`ana where`** — prints both ledger paths, which environment variables are
  overriding them, the lock and backup paths, and the version. Dispatched before
  any ledger is loaded, so it still answers when the ledger is what is broken.
- **`ana void <id> --reason …`** — annul an ambiguous or unanswerable question. It
  keeps its place in the history and leaves every score. `ana list --void` shows them.
- **`ana amend <id> --statement … --tags …`** — fix a typo or correct tags,
  pre-resolution only, keeping the previous wording. Probabilities and timestamps
  stay immutable.
- **MCP**: `server/discover` (mandatory as of the 2026-07-28 revision),
  `UnsupportedProtocolVersionError` (−32022) with the list of supported versions,
  per-request `_meta` protocol versions, and `void` / `amend` tools. The server is
  dual-era: it answers both `initialize` and the modern stateless requests.
- **`tests/hn_scenarios.rs`** — the hostile-review matrix, one named test per way
  a reader might try to break the tool.
- **`validation/`** — the scripts that produced every measurement quoted above.

### Also

- `plugin/install.sh` is now consensual: it prints the exact JSON it will add to
  `~/.claude/settings.json`, does nothing without `--yes`, has `--uninstall`, and
  warns when the marketplace plugin is also installed (which would fire every hook
  twice).
- `plugin/install-ana.sh` verifies the release `sha256.sum` and **fails closed** —
  no checksum file, or a mismatch, installs nothing.
- The release workflow publishes a unified `sha256.sum`, adds an
  `x86_64-unknown-linux-musl` target for older glibc, and smoke-tests the
  documented install command on Linux, macOS and Windows after tagging.
- CI runs on all three operating systems, checks the MSRV, and fails if the three
  version files disagree or the README's generated examples are stale.

### Notes

- MSRV is now **1.89** (`File::lock` is std from there).
- `predict` over MCP now asks for `by` in its schema: a claim with no date can
  still be scored, but a date is what lets the sequential test order it well.
