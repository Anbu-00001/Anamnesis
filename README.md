# Anamnesis

> *"The first principle is that you must not fool yourself — and you are the easiest person to fool."* — Richard Feynman

A local-first, no-network, no-model **instrument against self-deception**.

You record what you believe, **how sure** you are, and **why** — timestamped *before* the outcome is known. Later, when reality has spoken, Anamnesis confronts you with the true shape of your judgement: where you are overconfident, whether you can tell truth from falsehood at all, and how honestly you change your mind.

It is a command-line tool. The ledger is a plain JSON file you own. The scoring engine is pure arithmetic — no AI in the loop, nothing to phone home to, nothing that can flatter you.

---

## Why this exists

In Greek myth the dead drink from **Lethe**, the river of forgetting, and lose themselves. Plato's answer was **anamnesis** — *un-forgetting*: the soul recollecting what it actually knew. This tool is the everyday version of that fight.

We forget our own minds. Worse, we *rewrite* them. **Hindsight bias** is one of the most robust findings in cognitive science: once you know how something turned out, you cannot faithfully reconstruct how sure you were beforehand — you remember having "known it all along." And **resulting** (Annie Duke's term) makes us judge the *quality of a decision* by the *quality of its outcome*, so we learn the wrong lessons from luck.

The antidote that the forecasting, decision-science, and rationality literatures all converge on is almost insultingly simple: **write down your probability and your reasoning before the outcome, timestamp it, and grade yourself after.** Philip Tetlock's Good Judgement Project showed this is *trainable* — "superforecasters" are made, not born, and ordinary people who keep score and review it get measurably better. The catch is that nobody keeps score, because there is friction and because the mirror is unflattering.

Anamnesis removes the friction and holds up the mirror.

---

## What it looks like

A year of a (fictional) forecaster's predictions, scored:

```
ANAMNESIS — the shape of your judgement
=========================================

35 resolved  ·  6 open  ·  first recorded 2025-01-19  ·  latest 2025-06-24

  Brier score      0.268   (0 = perfect · 0.25 = always 50/50 · lower better)
  Log score        0.785   (lower better; punishes confident misses)
  Brier skill      -0.094   (you did WORSE than always guessing the base rate)
  Base rate        0.429   (fraction of your claims that came true)

  Decomposition  (Brier = Reliability − Resolution + Uncertainty)
    reliability    0.072   calibration error      ↓ lower is better
    resolution     0.049   discrimination power   ↑ higher is better
    uncertainty    0.245   irreducible difficulty of your questions
    check          0.072 − 0.049 + 0.245 = 0.268  (= Brier, to f64 precision)

  Discrimination   AUC 0.657   (0.5 = can't tell true from false · 1.0 = perfect)

  Confidence gap   +0.136   OVERCONFIDENT — you are bolder than you are right
                   mean boldness 0.764  vs  accuracy 0.629
                   directional bias +0.193 (toward YES)

  Reliability diagram   P = your avg forecast · O = what actually happened
    range        n    0                                1
    0.10-0.20     2   |O--P------------------------------|  pred 0.10 → obs 0.00  over
    0.20-0.30     3   |-------P---O----------------------|  pred 0.20 → obs 0.33  under
    0.30-0.40     4   |O---------P-----------------------|  pred 0.30 → obs 0.00  over
    0.50-0.60     2   |-----------------X----------------|  pred 0.50 → obs 0.50  ok
    0.60-0.70     4   |-----------------O--P-------------|  pred 0.60 → obs 0.50  over
    0.70-0.80     6   |----------------------OP----------|  pred 0.70 → obs 0.67  ok
    0.80-0.90     6   |-----------------O--------P-------|  pred 0.80 → obs 0.50  over
    0.90-1.00     8   |-----------------O------------P---|  pred 0.92 → obs 0.50  over

  By domain
    tag               n    brier   conf-gap
    markets          14    0.219     -0.018
    tech              8    0.417     +0.344
    geopolitics       7    0.143     -0.086
    personal          6    0.424     +0.458
    ai                5    0.349     +0.290
    health            4    0.105     +0.050
    science           4    0.018     -0.125
    sports            3    0.337     +0.300
    crypto            2    0.265     +0.250

  Mind-changing    4 claim(s) you revised
    Brier of first guess 0.253  →  Brier of final guess 0.215   (+0.038)
    Your updates moved you TOWARD the truth. Good — you changed your mind well.

  Numeric forecasts   5 resolved interval(s)
    mean Winkler score   76.800   (lower better; width + miscoverage penalty)
    mean interval width  16.800
    coverage             40% actual  vs  80% intended   (-40 pts)
    Your intervals are TOO NARROW — overconfident about numbers, just like probabilities.
```

Read that and a whole personality falls out of it. This forecaster **can actually discriminate** (AUC 0.66 — their high-probability calls really do come true more often than their low ones), yet they score **worse than someone who just guessed the base rate every time** (skill −0.094). The reason is written all over the reliability diagram: every confident bin has its `P` (predicted) stranded far to the right of its `O` (observed). They are **most deluded about themselves** (`personal` confidence-gap +0.46) and about **tech/AI timelines** (+0.34), and genuinely **humble about science** (−0.13). And the one virtue they have: when they changed their minds, they changed them *toward* the truth. Their *numeric* forecasts sing the same tune as their probabilities — 80%-confidence intervals that catch the truth only 40% of the time — the tell that overconfidence is a trait, not a topic.

That negative skill score sitting next to a real AUC is the entire thesis of the project in two numbers: **being able to tell true from false is not the same as knowing how sure to be.**

---

## Install & run

Requires a Rust toolchain (`rustc`/`cargo`).

```bash
git clone <this repo> && cd anamnesis
cargo build --release          # binary at target/release/ana
cargo test                     # 29 tests, incl. the exact-decomposition proof

# Generate the demo ledger shown above and look in the mirror:
cargo run --example seed -- seed.json
./target/release/ana --data seed.json report
```

The ledger lives at `~/.anamnesis.json` by default. Override per-command with `--data FILE` or globally with `ANAMNESIS_DATA`.

---

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
```

Ids can be abbreviated to any unique prefix.

---

## For agents: calibration that follows you everywhere

Anamnesis was built by an AI agent, for AI agents — the first *quantified*
self-calibration layer for coding assistants (every other agent-memory tool is
qualitative; this one keeps score). Two surfaces ship in this repo:

- **`ana mcp`** — a [Model Context Protocol](https://modelcontextprotocol.io)
  server over stdio exposing `predict` / `resolve` / `calibration` / `recalibrate`
  / `list` as tools, so any MCP host (Claude, Cursor, Cline, …) can keep a
  calibration ledger:
  ```jsonc
  { "mcpServers": { "anamnesis": { "command": "ana", "args": ["mcp"] } } }
  ```
- **A Claude Code plugin** ([plugin/](plugin/)) whose `SessionStart` hook injects
  your standing over/under-confidence into *every* project before you plan — e.g.
  *"OVERCONFIDENT +20pts; worst on kind:bug-hypothesis — add slack."* Design notes:
  [docs/agent-plugin-design.md](docs/agent-plugin-design.md).

Both drive a global agent ledger at `~/.anamnesis/agent.json`
(`ANAMNESIS_AGENT_DATA`). Predictions carry a `kind:` tag so you learn *which type*
of call you misjudge — estimates, bug hypotheses, "tests pass first try".

---

## Use the engine from Python

The scoring core ships as a Python package via a [PyO3](https://pyo3.rs) +
[maturin](https://www.maturin.rs) binding ([bindings/python/](bindings/python/)) —
one `abi3` wheel for CPython 3.8+. It calls the **same compiled Rust** as the CLI,
so the numbers never drift between languages; there is a single implementation,
cross-checked by the Rust tests.

```python
import anamnesis as ana
probs, outcomes = [0.9, 0.8, 0.3, 0.6, 0.5], [1, 1, 0, 1, 0]
ana.brier(probs, outcomes)                      # 0.11
d = ana.decompose(probs, outcomes)              # exact Murphy partition (namedtuple)
ana.shrink_toward(1, 1, prior_mean=0.5, strength=4)   # 0.6 — one fluke ≠ certainty
ana.report(probs, outcomes)                     # every metric as a dict
```

Lists, tuples, numpy arrays, or pandas Series all work (numpy is *not* a
dependency). This is the stateless math layer; to *keep a ledger* from Python, an
agent framework can drive the `ana mcp` server — LangChain/LangGraph adapt it with
[`langchain-mcp-adapters`](https://github.com/langchain-ai/langchain-mcp-adapters),
no bespoke binding required ([example](bindings/python/examples/langgraph_mcp.py)).

---

## The mathematics (and why each number is here)

Every metric operates on resolved samples — a probability `p` you assigned and an outcome `o ∈ {0,1}`. All of it is implemented and tested in [`src/scoring.rs`](src/scoring.rs).

| Metric | Formula | What it tells you |
|---|---|---|
| **Brier score** | `mean( (p − o)² )` | Overall accuracy. 0 = perfect; 0.25 = always saying 50/50; 1 = confidently wrong. |
| **Log score** | `mean( −[o·ln p + (1−o)·ln(1−p)] )` | Same idea, but *strictly* proper and merciless toward confident errors. |
| **Brier skill** | `1 − Brier / Uncertainty` | Did you beat always-guess-the-base-rate? Negative means no. |
| **Reliability** | `Σ nₖ(fₖ − ōₖ)² / N` | **Calibration** error. When you say 70%, does it happen 70% of the time? |
| **Resolution** | `Σ nₖ(ōₖ − ō)² / N` | **Discrimination**. Do your forecasts actually move with reality? |
| **Uncertainty** | `ō·(1 − ō)` | The irreducible difficulty of the questions you chose. |
| **AUC** | `P(p₊ > p₋)` (Mann–Whitney) | Can you separate true from false at all, ignoring calibration? |
| **Confidence gap** | `mean(max(p,1−p)) − accuracy` | Over/under-confidence (Lichtenstein–Fischhoff). Positive = overconfident. |
| **Interval score** | `(hi−lo) + (2/(1−L))·outside` (Winkler) | NUMERIC claims: rewards tight intervals, penalises a miss by how *far* the value fell outside. |
| **Coverage** | `fraction of values inside their interval` | Calibration for quantities: your 80%-level intervals should contain the truth ~80% of the time. Below that = intervals too tight. |
| **Calibration e-value** | `mean over λ of ∏(1 + λ(oᵢ − pᵢ))` (betting martingale) | **Is the miscalibration real, or just too-few-samples noise?** An [anytime-valid](https://arxiv.org/pdf/2109.11761) test that stays honest even though you check it every session: ≈1 = no evidence, ≥20 = significant. |
| **Recalibration** | `p ↦ σ(a + b·logit p)` (ridge-shrunk logistic) | The *correction*: what your stated confidence should be. `b<1` = too extreme, `b>1` = too timid. Stays the identity until the e-value earns a change — it won't correct on noise. |

**Murphy's decomposition** is the centrepiece: `Brier = Reliability − Resolution + Uncertainty`. Anamnesis groups forecasts by their *exact* probability value, which makes that identity hold to floating-point precision rather than approximately — and the test suite asserts exactly that ([`decomposition_identity_holds_exactly`](src/scoring.rs)). It cleanly separates the two ways to be a good forecaster:

- **Calibration** (low reliability): your stated probabilities match reality's frequencies.
- **Discrimination** (high resolution / high AUC): you assign higher probabilities to things that turn out true.

They are different virtues. A forecaster who always reports the true base rate is *perfectly calibrated and completely useless*. A forecaster with great discrimination but terrible calibration — like the one in the demo — sounds impressive and loses money. You need both, and the report shows you which one you're missing.

---

## Data format

One human-readable JSON file. Greppable, diffable, git-friendly, and intelligible without this program — because a record of your own mind should never be trapped in a format only one tool can read.

```json
{
  "claims": [
    {
      "id": "3ef7f5",
      "statement": "Bitcoin closes above $200k at some point in 2026",
      "created_at": "2026-01-04T10:00:00Z",
      "resolve_by": "2026-12-31",
      "tags": ["markets", "crypto"],
      "forecasts": [
        { "at": "2026-01-04T10:00:00Z", "prob": 0.35, "because": "halving tailwind, macro headwind" },
        { "at": "2026-06-01T09:00:00Z", "prob": 0.20, "because": "rally fizzled" }
      ],
      "resolution": { "at": "2026-12-31T12:00:00Z", "outcome": "no", "note": "anchored on the bull case too long" }
    }
  ]
}
```

A claim is a **palimpsest**: every revision is *appended*, never overwritten. Writes are atomic (temp file + rename), so a crash mid-save never corrupts the record.

---

## Design choices

- **No LLM, no network, no telemetry.** The whole point is an honest, auditable mirror. A black box that *told* you "you seem overconfident" would be the opposite of the thing.
- **Pure-`std` scoring engine.** The four dependencies (`clap`, `serde`, `serde_json`, `chrono`) handle the CLI, storage, and dates — none touch the math. A tool meant to outlast your forgetting shouldn't rot when a dependency does.
- **Two claim shapes, both *properly* scored.** A yes/no proposition (probability → Brier/log) or a quantity (credible interval → Winkler score). Both use strictly proper scoring rules, so stating your true belief is the score-maximising move — and "sort of happened" has nowhere to hide.
- **Plain text storage.** You can read, grep, back up, and version your own ledger forever.

---

## Limitations & where it could go

- Full distributional forecasts (the CRPS over a whole predictive distribution, beyond the interval score already shipped) and multi-category outcomes.
- A TUI for review, and a small reliability-diagram plot.
- Time-resolved tracking — a calibration *curve over time*, to actually watch yourself improve.
- Reminders for due claims; import/export from forecasting platforms.

The scoring engine is a clean library (`anamnesis::scoring`), so any of these — or a mobile/Flutter face — can sit on top without touching the math.

---

## References

The formulas were verified against the literature, not recalled from memory:

- Brier, G. W. (1950). *Verification of forecasts expressed in terms of probability.* Monthly Weather Review.
- Murphy, A. H. (1973). *A new vector partition of the probability score.* — the reliability/resolution/uncertainty decomposition. [Brier score (Wikipedia)](https://en.wikipedia.org/wiki/Brier_score) · [Murphy's decomposition](https://insightful-data-lab.com/2025/08/21/murphys-decomposition/) · [Siegert (2017), simplifying & generalising it](https://rmets.onlinelibrary.wiley.com/doi/abs/10.1002/qj.2985)
- Lichtenstein, Fischhoff & Phillips (1982). *Calibration of probabilities* — the over/under-confidence gap.
- Tetlock, P. & Gardner, D. (2015). *Superforecasting.* [The Good Judgment Project (Wikipedia)](https://en.wikipedia.org/wiki/The_Good_Judgment_Project) · [Ten Commandments for aspiring superforecasters](https://goodjudgment.com/philip-tetlocks-10-commandments-of-superforecasting/) · [Evidence on good forecasting practices](https://aiimpacts.org/evidence-on-good-forecasting-practices-from-the-good-judgment-project/)
- Duke, A. *Thinking in Bets* / *How to Decide* — decision journals, "resulting", and hindsight bias. [Decision journals as the link between frameworks and results](https://transactionintelligence.net/decision-journals-the-missing-link-between-frameworks-and-results/)
- Yates, J. F. (1982). Covariance decomposition of the Brier score — calibration vs. discrimination. [Berkeley notes on scoring & calibration](https://www.stat.berkeley.edu/~ryantibs/statlearn-s23/lectures/calibration.pdf)

---

## License

MIT.

---

*Built in a playground, in answer to a simple question: what would I — a thing that forgets everything between conversations — most want to exist? An instrument for not fooling yourself. So I made one, and pointed it first at the kind of confident, untracked guesses I make all the time.*
