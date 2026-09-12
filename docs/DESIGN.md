# Design notes

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

## The loop

You only ever do four things: log a belief, revise it, resolve it, and act on
what the record says. The engine does the rest, and the lesson feeds back into
the next prediction.

```
ana add      belief, probability, and why        before the outcome is known
ana update   revise as evidence arrives          the old forecast is kept
ana resolve  once reality answers                with a post-mortem note
ana report   Brier, calibration, discrimination  is it real, and the correction
ana decide   proceed / verify / abstain          corrected by your track record
```

Everything is append-only and timestamped, so the record of what you believed,
and how sure you were, survives your own hindsight. The headline score grades the
**first** forecast on each claim, which is what makes `update` safe to use freely:
revising is read as learning something, never as having been right all along.

---

## Design choices

- **No LLM, no network, no telemetry.** The whole point is an honest, auditable mirror. A black box that *told* you "you seem overconfident" would be the opposite of the thing.
- **Pure-`std` scoring engine.** The four dependencies (`clap`, `serde`, `serde_json`, `chrono`) handle the CLI, storage, and dates — none touch the math. A tool meant to outlast your forgetting shouldn't rot when a dependency does.
- **Two claim shapes, both *properly* scored.** A yes/no proposition (probability → Brier/log) or a quantity (credible interval → Winkler score). Both use strictly proper scoring rules, so stating your true belief is the score-maximising move — and "sort of happened" has nowhere to hide.
- **Plain text storage.** You can read, grep, back up, and version your own ledger forever.

---

## Limitations, and where it could go

- Full distributional forecasts (a whole predictive distribution, not a single interval) and multi-category outcomes. Note CRPS is *deliberately not* added: for the interval format Anamnesis actually logs, the Winkler interval score already **is** its specialization (the weighted interval score converges to CRPS as you add quantile levels), so a CRPS over an *assumed* distribution shape would be more math for no new information — it would only fool you that you'd recorded a distribution you didn't.
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
