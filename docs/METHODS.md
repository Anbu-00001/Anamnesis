# Methods — what each number is, and why it is that number

Every figure quoted here came out of a script in [`validation/`](../validation/) or
a test in [`tests/hn_scenarios.rs`](../tests/hn_scenarios.rs). None of them is from
memory. If a claim here has no number behind it, it should not be here.

The scoring core ([`src/scoring.rs`](../src/scoring.rs)) is pure `std`: no I/O, no
network, no model. That is what makes it testable, and it is why the interesting
arguments below are all about *which* statistic to compute, not about the code.

---

## 1. Which forecast gets graded

**The first one.** The belief you recorded before you knew the answer.

The alternative — scoring your latest forecast — is what most tools do, and it is
trivially exploitable. Log ten claims at 0.5, update each to 0.99 once the answer
is obvious, resolve them all YES:

| scored on | Brier |
|---|---|
| final forecast | **0.000** |
| first forecast | **0.250** |

0.250 is the score for knowing nothing, which is exactly what those ten claims
demonstrate. The old behaviour also printed *"your updates moved you TOWARD the
truth. Good — you changed your mind well."*

Two secondary numbers are reported beside it, neither of them the headline:

- **`brier_final`** — the same score on your last forecast, labelled *not graded*.
  Useful for seeing whether you update in the right direction. Not a score,
  because a forecaster who learns the answer early can drive it to zero.
- **`brier_time_avg`** — each forecast weighted by how long it stood, the way
  Metaculus time-averages a question. It rewards updating *early*, which is a real
  skill. It is still secondary, for the same reason: the window closes at the
  earlier of the resolution and the end of the resolve-by day, so post-deadline
  revisions carry no weight, but an early-known answer can still be farmed.

**`late_updates`** counts revisions made after the deadline, within 24 hours of the
resolution, or in the last 10% of the claim's window. It is reported as a neutral
fact. A late update can be perfectly honest; the ledger cannot tell, so it does
not judge — it just declines to score it.

---

## 2. The decomposition: CORP, not exact-value grouping

`Brier = Miscalibration − Discrimination + Uncertainty`, computed by isotonic
regression (pool-adjacent-violators), following Dimitriadis, Gneiting & Jordan,
*Stable reliability diagrams for probabilistic classifiers*, PNAS 118(8) 2021
([doi:10.1073/pnas.2016191118](https://doi.org/10.1073/pnas.2016191118); the
preprint, [arXiv:2008.03033](https://arxiv.org/abs/2008.03033), carries the
earlier title *Evaluating probabilistic classifiers*).

- **MCB** = S(your forecasts) − S(isotonically recalibrated) — your calibration error
- **DSC** = S(base rate) − S(recalibrated) — how much your forecasts sort anything
- **UNC** = S(base rate) — the difficulty of the questions you chose

The identity holds **exactly**, with no bins and no tuning parameter. Measured
residual across every fixture in the test suite: `0.000e0`.

### Why the old version had to go

The previous decomposition grouped forecasts by their *exact* value, which made
its identity exact too — but at the sample sizes a person or an agent actually
has, most exact-value groups hold one or two claims, and a group of one has an
observed frequency of 0 or 1, so its "calibration error" is as large as it can
possibly be.

Simulated: a **perfectly calibrated** forecaster using two-decimal probabilities,
where the true calibration error is 0.000
([`validation/sims.py`](../validation/sims.py)):

| n | exact-value grouping | CORP MCB |
|---|---|---|
| 20 | 0.158 | 0.057 |
| 50 | 0.137 | 0.034 |
| 200 | 0.073 | 0.014 |
| 1000 | 0.017 | 0.004 |

The shipped number was telling well-calibrated users they had a calibration
problem. `reliability` and `resolution` remain in the JSON as deprecated aliases
for one release.

### The noise floor

MCB is fitted on the same data it scores, so it is never zero even for a perfect
forecaster. How far above zero depends on `n` and on the spread of your
forecasts — which nobody can hold in their head. So the report prints the floor
beside the value:

> miscalibration 0.034 — at or below the 0.050 a perfectly calibrated forecaster
> would score making these same calls

`mcb_null_q95` is the 95th percentile of MCB over 400 redraws of
`y ~ Bernoulli(p)` from *your own* forecasts, with a fixed seed so the number
never moves under a re-run.

---

## 3. The evidence test: is the miscalibration real?

An **e-process**: a betting martingale whose value is the wealth of a gambler
betting against the hypothesis that you are calibrated. By Ville's inequality it
exceeds `1/α` with probability at most `α` under the null **at any stopping
time** — so unlike a fixed-n test, it survives being looked at every session,
which is exactly how this tool is used.

- Arnold, Henzi & Ziegel (2023), *Sequentially valid tests for forecast
  calibration*, Annals of Applied Statistics 17(3):1909–1935
  ([doi:10.1214/22-AOAS1697](https://doi.org/10.1214/22-AOAS1697))
- Henzi & Ziegel, *Valid sequential inference on probability forecast
  performance*, Biometrika 109
  ([doi:10.1093/biomet/asab047](https://doi.org/10.1093/biomet/asab047))

Measured against a fixed-n alternative, peeking after every outcome, 1000 reps
([`bindings/python/validation/validate_guarantees.py`](../bindings/python/validation/validate_guarantees.py)):

| test | false-alarm rate (target ≤ 0.05) |
|---|---|
| e-process | **0.011** |
| naive z-test | **0.345** |

### 3a. Mixing betting strategies

The wealth process bets on `h(p)·(y − p)` for each of three strategies `h`:

| `h(p)` | catches |
|---|---|
| `1` | being too high, or too low, on the whole |
| `sign(0.5 − p)` | being too *sure*, in either direction |
| `2(0.5 − p)` | the same, weighted by how extreme the forecast is |

crossed with a grid of betting fractions λ, averaged in log space.

Every `h` depends only on the **stated forecast**, which is fixed before the
outcome — so each wealth process is still a non-negative martingale under the
null, and an average of e-processes over the same data is still an e-process.
Mixing costs validity nothing.

It is necessary because a single strategy is blind to *symmetric* overconfidence.
A forecaster who says 90% when the truth is 65% **and** 10% when the truth is 35%
has errors that cancel exactly:

| ledger | n | single strategy | mixture |
|---|---|---|---|
| symmetric 90/10 overconfidence | 1000 | **0.087** | ~10⁶⁵ |
| agent straddling 0.5 | 600 | **0.296** | ~10²⁹ |
| 90% coin flips + 60% sure things | 100 | **0.233** | **48.9** |
| calibrated forecaster (null) | 300 | 0.232 | 0.309 |

Power against symmetric overconfidence at n = 100 went from 2% to 100%, while the
null false-alarm rate stayed inside the bound (measured 0.0–2.3%).

That case is not exotic. It is what an agent looks like the moment it logs "this
will fail" at 0.15 alongside "this will pass" at 0.85.

### 3b. The order the evidence arrives in

Claims enter in order of their **due key**: their `resolve_by` date, or their
creation date plus a 30-day grace horizon when they have none. Both are chosen
before the answer is known. The sequence stops at the first claim whose due key
has passed and which is still ungraded.

#### Why that is sufficient

The reported e-value is always `M_k`, a **prefix product of one fixed sequence**.
Ville's maximal inequality bounds

```
P( ∃k : M_k ≥ 1/α )  ≤  α
```

over all `k` **simultaneously**. So it does not matter how `k` comes to be chosen,
or whether the choice correlates with the outcomes — every prefix is already
covered by the same bound. There is no separate "stopping time" condition to
discharge, and consequently no deadline gate is needed: the deadline was only ever
a way of *enforcing the prefix property*, and stopping at the first unresolved
claim enforces it directly.

This is also precisely why the original code was wrong. Sorting by resolution time
does not produce a prefix of a fixed sequence — it produces **a different
sequence each time**, reordered by something the outcome touches. Ville says
nothing about that, because there is no single `M_k` to bound.

Everything else in this section follows from that argument rather than merely
supporting it.

#### Measured anyway

A perfectly calibrated forecaster, one batch of 60 same-deadline claims, the
report re-read every 5 resolutions, alarm at e ≥ 20
([`validation/peeking.py`](../validation/peeking.py)):

| order | false alarms | median peak e |
|---|---|---|
| resolution time | **100%** | 443 |
| due key | **0%** | 1.24 |

For "will X happen by DATE" questions, YES tends to resolve the day it happens
while NO waits for the deadline, so every early prefix of the resolution-time
ordering is YES-heavy even for a perfect forecaster.

Note that at a *fixed* n none of this shows up — a product commutes, and two
ledgers with identical claims in shuffled resolution order return the same
e-value. That is why it hid for so long: the damage appears only under repeated
looking, which is the property being advertised.

#### Claims with no deadline

They are **included**, ordered by creation date plus a grace horizon
(`ANAMNESIS_HORIZON_DAYS`, default 30). Excluding them would have discarded every
claim in every ledger written before `--by` was encouraged — including all 35
graded claims in this tool's own demo ledger.

The grace horizon exists to remove a cliff. Admitted on the day it is written, a
single open no-deadline claim blocks everything created after it, so logging
"will we hit 10k users by 2028?" in week one freezes the evidence test forever.
With the horizon, that claim sits unadmitted until its key passes; a forgotten
one then starts applying pressure, which is the right nudge. The key is still
fixed at creation, so every reported value is still a prefix of a fixed sequence
and the argument above is untouched.

Stress-tested where resolution *speed* is perfectly correlated with the outcome —
the agent's normal case, since "the tests pass" is known in seconds and "the tests
fail" after an hour of debugging — peeking 200 times across 80 seeds:

| forecasts | n | false alarms |
|---|---|---|
| p ~ U(0.05, 0.95) | 200 | 0.0% |
| all p = 0.5 | 200 | 0.0% |
| all p = 0.9 | 200 | 1.2% |

#### The one operation that can break it: `void`

Voiding punches a hole in exactly this property, because it edits the sequence
after the fact.

- **Voided before it resolved** — safe. The claim carries no outcome, so removing
  it cannot move the e-value in any direction. Excluded from the sequence.
- **Voided after it resolved** — an outcome deleted from the record *after being
  seen*. The direction of abuse is self-flattery rather than a false alarm: you
  would void the ones that went badly, and the e-value would fall. Either way the
  prefix is no longer fixed.

So a claim voided after resolution **stays in the evidence sequence**, and the
report prints how many there are. The scores forget it — annulling a question is
what void is for — but the sequential test does not, and the edit is never
silent. If someone voids six resolved claims, the report says so out loud.

### 3c. Multiplicity

Per-`kind:` e-values search K subgroups for the worst one, which is K tests, not
one. A row must clear `20 × K` — Ville plus a union bound — before it is reported
as a finding. Below that the per-kind numbers are shown as descriptive only.

### 3d. The assumption, stated plainly

The guarantee needs each outcome to be calibrated **given the earlier ones**.
Several claims about one underlying event are correlated and can trip the test
even for a well-calibrated forecaster. Log one claim per event, or group them
under a `group:` tag and read the per-group numbers.

---

## 4. The verdict

One function, [`report::verdict`](../src/report.rs), is the only thing that decides
whether you are calibrated. The plain report, the cat, the badge, the HTML card,
`--json`, the MCP `calibration` tool and the hooks all read it.

| state | condition |
|---|---|
| `insufficient_data` | fewer than 20 graded calls |
| `no_evidence_of_miscalibration` | MCB at or below its floor, e below 20 |
| `calibrated_but_uninformative` | the above, and DSC ≈ 0 |
| `overconfident` / `underconfident` | e ≥ 20, MCB above floor, with a clear direction |
| `biased_yes` / `biased_no` | as above, with a directional lean dominating |
| `miscalibrated_both_ways` | as above, but the gap is too small to name a direction |

The words "well calibrated" never appear below 50 graded calls, and no verdict at
all appears below 20.

**Why the confidence gap is not the verdict.** The gap is `mean(boldness) −
accuracy`, an aggregate — and aggregates cancel. A ledger of 50 calls at 0.9 on
coin flips plus 50 at 0.6 on sure things has a Brier skill of −0.520, a
calibration error of 0.160, DSC of exactly 0.000 — and a confidence gap of
−0.000000000000001. Every surface keyed off that gap, so all of them announced
`[DIALED IN] · well calibrated` and advised "keep doing what you're doing".

`miscalibrated_both_ways` exists for the same reason: on that ledger the sign of
the remaining gap is floating-point noise, and the direction it happened to give
("underconfident") would have sent the reader to raise the very 0.9s that were the
problem.

---

## 5. The decision gate

Recalibrate the stated probability, then apply Chow's reject rule:

```
τ = 1 − verify_cost / stake      proceed iff p̂ ≥ τ ;  abstain below even odds
```

- Ferrer & Ramos, *Evaluating Posterior Probabilities: Decision Theory, Proper
  Scoring Rules, and Calibration*, TMLR 2025
  ([arXiv:2408.02841](https://arxiv.org/abs/2408.02841))
  — **not** to be confused with Ferro & Fricker (2012), *A bias-corrected
  decomposition of the Brier score*, QJRMS
  ([doi:10.1002/qj.1924](https://doi.org/10.1002/qj.1924)), which is a different
  paper by different authors on a different subject. The near-homonym is the exact
  shape of citation error that gets torn apart, so both are named here.

  That paper argues calibration metrics capture only one aspect of posterior
  quality, ignore discrimination, and should play no role in assessing posteriors.
  This design agrees with the premise and answers it structurally: **the headline
  is a proper scoring rule** (Brier), and CORP reports discrimination *alongside*
  calibration rather than instead of it — which is why `DSC ≈ 0` outranks any
  statement about the level. See §4.

The correction is **evidence-gated**: `scoring::gate_recalibration` hands the
number back unchanged until the e-process finds real evidence of miscalibration,
so it will not "correct" on noise. That gate is one function, shared by the CLI,
both MCP tools and the Python binding.

Measured, 300 reps of an overconfident agent at n = 600, stake 3, verify cost 0.6:

| policy | mean decision cost |
|---|---|
| act on the raw stated number | 0.6556 |
| recalibrate, then threshold | **0.6005** |

The gate is cheaper in 100% of runs.

### The recalibration fit

`p ↦ σ(a + b·logit p)`, fitted by ridge-penalised logistic regression with damped
Newton and a backtracking line search.

The line search is not decoration. Undamped Newton **diverges** whenever the fit
wanders into the saturated region: there every `mu` is ~0 or ~1, so the Hessian
weights `mu(1−mu)` vanish while the gradient does not, and each step overshoots
further. Measured on 200 calls all stated at 0.9 that came true half the time — an
agent with a narrow confidence vocabulary — it returned `a = 66, b = 146`: a map
that corrected 0.9 **up to 1.0** for a forecaster who was right half the time.
With the line search it returns 0.498, against a truth of 0.5. If the solver does
not converge, the identity map is returned instead: "no correction" is the honest
answer when the fit will not settle, and a confidently wrong one is not.

---

## 6. Intervals

For a numeric claim you record a credible interval at a stated level. The
**Winkler interval score** (Winkler 1972) charges the width plus a miscoverage
penalty.

The headline is the **unitless ratio** `winkler / width`: `1.00` means the value
landed inside, and anything above is the miss penalty in multiples of your own
stated width. The raw score carries the units of whatever is being predicted, so
averaging it across claims let one question measured in dollars dominate one
measured in milliseconds. Raw Winkler is still shown per claim in `ana show`.

**`conformal_width_factor`** is the split-conformal quantile of standardized
residuals: the multiplier on your half-widths that would make your coverage hit
its nominal level. It is gated on the coverage e-process, which reuses the binary
test with `prob = level, outcome = contained`.

CRPS is deliberately not implemented: for the interval format recorded here, the
Winkler score already *is* its specialisation (WIS → CRPS as the number of
intervals grows), and CRPS needs a distribution shape that was never recorded.

---

## 7. Everything else

| number | what it is |
|---|---|
| **Brier score** | mean squared error of your probabilities. 0 perfect, 0.25 = always saying 50/50 |
| **Log score** | punishes confident misses far harder; `eps = 1e-6` clamps p to keep it finite |
| **Brier skill** | `1 − brier/uncertainty`. Negative means you did worse than always guessing the base rate |
| **AUC** | rank-based discrimination, ties handled. 0.5 = your confidence sorts nothing |
| **Bootstrap band** | 2000 seeded resamples of the Brier — how far luck alone could move it |
| **EWMA "lately"** | recency-weighted Brier, half-life 5. **Descriptive only**, not a control chart: an alarm at an agent's n would false-alarm constantly |
| **Wilson interval** | small-n interval on the base rate |
| **Risk–coverage** | error among your most-confident calls vs all — when to trust your own judgement |
| **Stake-weighted Brier** | are you miscalibrated on the calls that *matter*? |
| **`asmd`** | standardized difference in boldness between graded and ungraded calls — the missing-not-at-random check |
| **`dialectical_mean`** | averages a first estimate with a deliberate "consider the opposite" second (Herzog & Hertwig 2009). An elicitation aid, not a score |

## 8. Deliberately not built

- **CRPS** — see §6.
- **CUSUM / control-chart alarms, and Adaptive Conformal Inference** — both
  false-alarm at the sample sizes this tool actually sees. The EWMA line is
  descriptive, and the pooled conformal factor is more stable than ACI's
  learning-rate knob.
- **An exact incomplete-beta coverage interval** — Wilson is within a hair of
  Jeffreys at small n, and the e-process is the better peek-proof gate anyway.
