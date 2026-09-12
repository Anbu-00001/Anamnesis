# Anamnesis

<!-- TODO(human): one plain sentence — what this is and who it is for.
     No "first", no superlatives, no adjectives you would not say out loud.
     Delete this comment when you write it. -->

**TODO(human)** — one-sentence description.

<!-- TODO(human): a demo recording, 20 seconds or less, generated from a
     committed script (docs/demo.tape for vhs, or asciinema) so it can be
     regenerated rather than re-recorded by hand. -->

## Install

```bash
cargo install --git https://github.com/Anbu-00001/Anamnesis --locked
```

Or download a checksum-verified binary for your platform:

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/Anbu-00001/Anamnesis/main/plugin/install-ana.sh)
```

<!-- The installer verifies the release sha256.sum and fails closed. It never
     installs an unverified binary. -->

## Try it

```bash
ana demo                      # a fictional year of predictions, reported on
                              # — nothing of yours is touched
ana add "this refactor takes under an hour" --prob 0.7 --by 2026-09-20
ana resolve <id> yes           # the moment reality answers
ana report
```

Bring an existing prediction history with `ana import history.csv`
(columns: `statement, prob, created, resolve_by, outcome, resolved_at, tags`).

## For agents

`ana mcp` is a Model Context Protocol server exposing `predict` / `resolve` /
`calibration` / `recalibrate` / `decide` / `void` / `amend`, and
[`plugin/`](plugin/) is a Claude Code plugin that injects your standing
calibration into every session and grades `kind:tests-pass` predictions from the
actual exit status. See **[docs/AGENTS.md](docs/AGENTS.md)**.

## What it does, and what it does not

**Does**

- Grades the forecast you made **before** the answer was known, never the one you
  revised afterwards.
- Separates *calibration* (how sure you should be) from *discrimination* (whether
  you can tell true from false) — they are not the same skill, and most tools
  conflate them.
- Says whether an apparent miscalibration is **real**, with a test that stays
  valid even though you check it every session.
- Prints its own noise floor, so a calibration error you cannot distinguish from
  luck is reported as exactly that.
- Refuses to give a verdict at all below 20 graded calls.

**Does not**

- Talk to the network. Ever. There is no telemetry and no account.
- Use an LLM to score anything. The engine is `std`-only arithmetic and cannot
  flatter you.
- Protect against someone editing their own JSON. The threat model is **hindsight
  bias — your own memory rewriting how sure you were**, not tampering. See
  [docs/DATA_FORMAT.md](docs/DATA_FORMAT.md).
- Claim to make you, or an agent, measurably better at anything. That is
  unmeasured. <!-- TODO(human): replace this line if and when P1-7 produces data -->

## How it compares

<!-- TODO(human): a sentence or two of your own around this table. -->

| | Anamnesis | Fatebook | Metaculus | calibration quizzes |
|---|---|---|---|---|
| where it runs | local CLI, one JSON file | hosted web app | public platform | web page |
| questions | yours | yours | community's | trivia, pre-written |
| agent surface | MCP server + editor hooks | API, Slack, Chrome extension | API | none |
| scoring basis | first forecast | latest forecast | time-averaged | instant |
| "is it real?" | sequential e-value | — | — | — |
| acts on the number | `ana decide` → proceed/verify/abstain | — | — | — |
| needs an account | no | yes | yes | no |

Fatebook is open source and considerably better at the social, share-a-prediction
side of this. Metaculus is a serious forecasting platform with a real community.
This is a smaller thing aimed at one narrow question.

---

## Usage

## Usage

```bash
# Record a belief — a falsifiable statement, your probability, and your reasoning.
ana add "Bitcoin closes above \$200k at some point in 2026" \
    --prob 0.35 --by 2026-12-31 --tags markets,crypto \
    --because "halving tailwind, but macro is a headwind"

# Revise it when evidence arrives. The old forecast is KEPT, not overwritten.
ana update 3ef7f5 --prob 0.20 --because "rally fizzled; reverting toward base rate"

# Resolve it once reality speaks, with a post-mortem you'll thank yourself for.
ana resolve 3ef7f5 no --note "I anchored on the bull case far too long"

# Not everything is yes/no. For a QUANTITY, record a credible interval instead of
# a probability — at a confidence level — and resolve it with the value that occurred.
ana add "US Fed rate cuts in 2025" --interval 1..3 --level 0.8 --tags markets \
    --because "a cut or two looks likely"
ana update 7a1c2b --interval 1..2 --because "data turned hawkish"
ana resolve 7a1c2b --value 2          # scored with the Winkler interval score

# Drive any command as JSON for an agent, script, or future UI — never scrape prose.
ana --json report
ana --json add "Brent above \$100 in 2026" --prob 0.2

# See what's open, resolved, or overdue.
ana list --open
ana list --due           # open claims whose expected-by date has passed
ana list --resolved

# The full history of one belief — the palimpsest of your changing mind.
ana show 3ef7f5

# The mirror. Slice it by domain if you like.
ana report
ana report --tag markets --bins 5

# About to act on a hunch? Run it through the gate. It corrects your number with
# your track record, then thresholds by the stakes: PROCEED / VERIFY / ABSTAIN.
ana decide --prob 0.8                    # ordinary call → need ≥80% to just proceed
ana decide --prob 0.9 --stake 5          # irreversible → bar climbs to ~96%, so: verify
```

Ids can be abbreviated to any unique prefix.

Ids can be abbreviated to any unique prefix. `ana where` prints both ledger paths
and which environment variables are overriding them — it is the first thing to
include in a bug report.

## What the report looks like

<!-- BEGIN:report -->
```

ANAMNESIS — the shape of your judgement
=========================================

44 of 50 resolved (35 yes/no · 9 numeric)  ·  6 open  ·  first recorded 2025-01-19  ·  latest 2025-06-24

  Resolution discipline   88% graded (44 of 50)  ·  2 overdue
    ⚠ 2 claim(s) past due and ungraded — resolve them; until you do, the numbers below rest on a self-selected sample.
    Your ungraded calls are more CAUTIOUS than your graded ones (boldness 0.62 vs 0.76, ASMD 0.79) — your graded sample leans bold relative to what you actually predicted.

  VERDICT          NO MISCALIBRATION FOUND

  Brier score      0.272   (0 = perfect · 0.25 = always 50/50 · lower better)
                   scored on your FIRST forecast — the belief you recorded before the answer was known
                   on your final forecast it would be 0.268 (shown, not graded)
                   95% bootstrap band [0.180, 0.365] — how far luck alone could move it
  Stake-weighted   0.277   vs 0.272 flat → about the same across stakes
  Log score        0.793   (lower better; punishes confident misses)
  Brier skill      -0.112   (you did WORSE than always guessing the base rate)
  Lately           0.244   recent Brier vs 0.272 lifetime → improving  (last ~5 weighted; directional, not significant)
  Base rate        0.429   (fraction of your claims that came true)   95% CI 0.28–0.59

  Decomposition  (Brier = Miscalibration − Discrimination + Uncertainty)
    miscalibration 0.065   calibration error      ↓ lower is better
                   1.16x the 0.056 that only 1 calibrated forecaster in 20 exceeds on these same calls
    discrimination 0.037   sorting power          ↑ higher is better
    uncertainty    0.245   irreducible difficulty of your questions
    check          0.065 − 0.037 + 0.245 = 0.272  (= Brier, exactly — no bins, no tuning)
    (the older exact-value grouping reads 0.080 / 0.053; it counts small groups as error and is kept only as a deprecated alias)

  Discrimination   AUC 0.640   (0.5 = can't tell true from false · 1.0 = perfect)

  Confidence gap   +0.120   bolder than you are right, but not by more than luck could manage at this sample size — see the verdict above
                   mean boldness 0.749  vs  accuracy 0.629
                   directional bias +0.183 (toward YES)

  Confidence vocab   11 distinct level(s) across 35 call(s)

  Selective        act on all 37% error · surest half 39% → confidence barely separates winners from losers

  Is it real?      e-value      1.8   (anytime-valid p ≤ 0.556)
                   no evidence of miscalibration in 35 graded calls — the test can still miss patterns; read the calibration error above for the SIZE of any error
                   35 of 35 graded calls counted, 2 ungraded priced in at their worst case (`ana list --due`)
                   that backlog is costing you a factor of 1.392 on the evidence — grading it is how you get it back
                   oldest is [9870c0] — resolve it, or void it if it was never answerable. an ungraded bold call costs more than a cautious one

  Reliability diagram   P = your avg forecast · O = what actually happened
    range        n    0                                1
    0.10-0.20     2   |O--P------------------------------|  pred 0.10 → obs 0.00  over
    0.20-0.30     3   |-------P---O----------------------|  pred 0.20 → obs 0.33  under
    0.30-0.40     3   |O---------P-----------------------|  pred 0.30 → obs 0.00  over
    0.40-0.50     1   |O------------P--------------------|  pred 0.40 → obs 0.00  over
    0.50-0.60     3   |-----------------P----O-----------|  pred 0.52 → obs 0.67  under
    0.60-0.70     5   |--------------------X-------------|  pred 0.60 → obs 0.60  ok
    0.70-0.80     6   |-----------------O-----P----------|  pred 0.70 → obs 0.50  over
    0.80-0.90     4   |-----------------O--------P-------|  pred 0.80 → obs 0.50  over
    0.90-1.00     8   |-----------------O------------P---|  pred 0.92 → obs 0.50  over

  By domain
    tag               n    brier   conf-gap
    markets          14    0.253     +0.046
    tech              8    0.365     +0.181
    geopolitics       7    0.223     +0.021
    personal          6    0.344     +0.258
    ai                5    0.381     +0.240
    health            4    0.105     +0.050
    science           4    0.018     -0.125
    sports            3    0.337     +0.300
    crypto            2    0.265     +0.250

  By topic            (K=9 groups · 100% covered · gap~ shrunk toward your overall rate)
    topic               n    brier       gap      gap~
    markets            14    0.253    +0.046    +0.058
    tech                8    0.365    +0.181    +0.159
    geopolitics         7    0.223    +0.021    +0.053
    personal            6    0.344    +0.258    +0.207
    ai                  5    0.381    +0.240    +0.183
    health              4    0.105    +0.050    +0.111
    science             4    0.018    -0.125    +0.061
    sports              3    0.337    +0.300    +0.131
    crypto              2    0.265    +0.250    +0.164

  Mind-changing    4 claim(s) you revised
    Brier of first guess 0.253  (GRADED)  →  Brier of final guess 0.215  (not graded)   (+0.038)
    Only the first figure is scored. A revision may be genuine learning or may be hindsight; the ledger cannot tell, so it grades the belief you recorded before the answer was known.
    Time-averaged Brier 0.271 — each forecast weighted by how long it stood, the way Metaculus rewards updating early (secondary: it can still be farmed by updating the moment you know).

  Numeric forecasts   9 resolved interval(s)
    interval score       mean 6.02 · median 5.17
                         (1.00 = the value landed inside; above that is the miss penalty, in multiples of your own stated width)
    mean interval width  13.444
    coverage             22% actual  vs  80% intended   (-58 pts)
    Your intervals are TOO NARROW — overconfident about numbers, just like probabilities.
    Recalibration: WIDEN — multiply your interval half-widths by 2.76 (coverage e=5).

  "The first principle is that you must not fool yourself —
   and you are the easiest person to fool."  — R. Feynman
```
<!-- END:report -->

There is also `--plain` (the same thing in plain English), `--html` (a
self-contained offline card, zero JavaScript), `--badge` (an SVG for a README) and
`--json`.

## Documentation

| | |
|---|---|
| [docs/METHODS.md](docs/METHODS.md) | every number, why it is that number, and the measurement behind each claim |
| [docs/AGENTS.md](docs/AGENTS.md) | MCP server, the Claude Code plugin, auto-resolution |
| [docs/DATA_FORMAT.md](docs/DATA_FORMAT.md) | the ledger format, and the threat model |
| [docs/DESIGN.md](docs/DESIGN.md) | why it is shaped this way, and the limitations |
| [docs/PYTHON.md](docs/PYTHON.md) | the scoring core as a Python library |
| [CHANGELOG.md](CHANGELOG.md) | what changed, and what it changed from |

## Reproducing the claims

Every measurement in the docs has a script:

```bash
cargo test --all                       # 100+ tests, including the hostile-review matrix
./validation/repro.sh                  # the defects, against a built binary
python3 validation/sims.py             # e-process power, CORP noise
python3 validation/peeking.py          # the ordering experiment
cargo run --release --example audit -- <ledger.json>   # old vs new metrics side by side
```

## How this was built

<!-- TODO(human): your own words, including an honest note about AI assistance.
     This section is yours; do not let it be written for you. -->

**TODO(human)**

## License

MIT.
