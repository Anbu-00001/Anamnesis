# PREMORTEM

Material for the questions a hostile, statistics-literate reader will ask.

**The answers are the human's to write, live, in their own words.** What is here
is only the ammunition: facts, numbers, and the commands that produce them. Do not
paste any of this as a reply.

Each row names where the number comes from, so it can be checked mid-thread rather
than asserted.

---

## "Doesn't Brier conflate knowledge with calibration?"

Yes, which is why it is decomposed rather than reported alone.

- `Brier = MCB − DSC + UNC`. MCB is calibration error, DSC is discrimination,
  UNC is the difficulty of the questions.
- The identity holds exactly — no bins, no tuning parameter. Residual `0.000e0`
  across every fixture.
- The demonstration: 50 calls at 0.9 on coin flips plus 50 at 0.6 on sure things
  returns **DSC exactly 0.0000**. The confidence carries no information about
  which calls come true, and the tool says so in those words.
- Command: `cargo run --release --example audit -- <ledger.json>`

## "Your own citation says calibration metrics shouldn't be used at all"

**Someone will read Ferrer & Ramos and use it against you.** It is cited in
`docs/METHODS.md` §5, and it argues that calibration metrics capture only one
aspect of posterior quality, ignore discrimination, and should play no role in
assessing posteriors.

Concede the premise — it is correct — and point at the design, which already
answers it:

- **The headline is a proper scoring rule**, not a calibration metric. Brier.
  Calibration is never reported as the score.
- **CORP reports discrimination alongside calibration**, not instead of it:
  `Brier = MCB − DSC + UNC`, all three printed, every time.
- **`DSC ≈ 0` outranks any statement about calibration** in the verdict. On the
  50-coin-flips-at-0.9 ledger the tool does not say "overconfident", it says the
  forecasts sort nothing and that is what to fix first. That is Ferrer & Ramos's
  point, implemented.
- The one place a calibration number *does* drive an action — the `decide` gate —
  is decision-theoretic, which is that paper's own recommended framing.

Do not get drawn into defending calibration-as-a-metric. The tool does not use it
as one.

**Do not confuse the paper with Ferro & Fricker (2012)**, *A bias-corrected
decomposition of the Brier score*, QJRMS — different authors, different subject.
Merging the two in a reply is the exact error that ends the thread badly.

## "Anytime-valid under what assumptions?"

- Each outcome must be calibrated **given the earlier ones**. Correlated claims
  about one underlying event can trip it. Stated in `docs/METHODS.md` §3d.
- The sequence order must be fixed before the outcomes are known — it is the due
  date, or the creation date when there is none.
- **The real argument is simpler than a stopping-time argument.** The reported
  e-value is always `M_k`, a prefix product of ONE fixed sequence, and Ville's
  maximal inequality bounds `P(∃k : M_k ≥ 1/α) ≤ α` over all `k` simultaneously.
  So it does not matter how `k` gets chosen or whether the choice correlates with
  outcomes — every prefix is covered. There is no separate condition to discharge.
- That is also why the old code was wrong: resolution-time sorting does not give a
  prefix of a fixed sequence, it gives a *different sequence each time*. There is
  no single `M_k` for Ville to bound.
- The stopping rule (never skip an ungraded claim) is what enforces the prefix
  property directly. The deadline was only ever one way of enforcing it.
- Measured, peeking after every outcome, 1000 reps: e-process false alarms
  **0.011**; a naive z-test **0.345**.
- Command: `python3 bindings/python/validation/validate_guarantees.py`

## "You reorder by resolution time — isn't the e-value inflated?"

Not at a fixed n: a product commutes, and two ledgers with identical claims in
shuffled resolution order return the same e-value. **That is not the problem.**

The problem is repeated looking, which is the property advertised. Ordering by
resolution time is not a prefix of any fixed order — it reorders by something the
outcome touches. A perfectly calibrated forecaster, one batch of 60 same-deadline
claims, the report re-read every 5 resolutions:

| order | false alarms | median peak e |
|---|---|---|
| resolution time | 100% | 443 |
| due key (what ships) | 0% | 1.24 |

- Command: `python3 validation/peeking.py`

## "Why would I trust a self-graded ledger?"

- The headline grades the **first** forecast, so revising once you know the answer
  does not help: ten claims at 0.5 → 0.99 → YES score 0.250, not 0.000.
- `resolved_by: auto` marks resolutions graded from a command's exit status rather
  than from anyone's account. The report shows what fraction those are.
- The threat model is stated plainly: hindsight bias, **not** tampering. Nothing
  stops someone editing their own JSON, and nothing is signed.
- Voiding a question requires a recorded reason and leaves the claim in history.
- **Voiding cannot be used to launder a bad record.** A claim voided *after* it
  resolved stays in the evidence sequence — removing an outcome you have already
  seen would retroactively edit the sequence, and the direction of abuse is
  self-flattery — and the report prints how many such voids there are. The scores
  forget it; the sequential test does not.

## "Does quick-feedback calibration transfer to slow, real decisions?"

**Not measured by this project.** Say so. The forecasting literature is not settled
on it either. Do not reach for a study unless it has been opened in the thread.

## "Does it actually make agents better?"

**Unmeasured.** The README says so in as many words. It measures; it does not
claim to improve. If someone wants the data, the honest answer is that it does not
exist yet.

## "Why not Fatebook / Metaculus?"

They are better at what they do. Fatebook is open source, hosted, and has the
social side — Slack, a Chrome extension, an API. Metaculus is a real platform with
a real community and time-averaged scoring.

The differences that are actually differences: this runs locally against a JSON
file with no account, grades the first forecast rather than the latest, tells you
whether the miscalibration is statistically real, and has an MCP server and editor
hooks so an agent can use it. The comparison table is in the README.

## "Was this built with AI?"

**The human answers this, in their own words.** Do not write it for them, do not
suggest wording, and do not soften it.

## "Why a local file / why Rust / why no sync?"

- One JSON file: greppable, diffable, git-friendly, readable without this program.
- Four dependencies; the scoring core has none. 2.1 MB binary.
- No network code at all, so "what leaves your machine" has a one-word answer.

## "Your calibration number is inflated / your stats are wrong"

The most likely specific version of this, and the honest answer: **the previous
release's calibration number *was* inflated.** Exact-value grouping reported 0.073
of calibration error for a perfectly calibrated forecaster at n=200 where the
truth is 0.000. That is fixed (CORP: 0.014), the noise floor is now printed beside
the value, and the whole thing is in `CHANGELOG.md` marked as breaking.

There is a second, worse one, and it is worth volunteering rather than waiting for:
**the recalibration map could diverge and correct in the wrong direction.**
Undamped Newton overshoots in the saturated region, and on 200 calls all stated at
0.9 that came true half the time it returned a map that corrected 0.9 **up to
1.0** — through a gate that had already decided to trust it. Every other defect in
the project failed passively; that one actively pushed a 50%-accurate forecaster
toward certainty.

Say it as evidence the thing was stress-tested, not as a confession. It is now
fixed three ways: a backtracking line search, a non-negative slope constraint, and
a set of invariants cross-checked against the isotonic (PAV) fit, which cannot
diverge by construction and therefore serves as a free oracle. If the parametric
map and the isotonic curve disagree in direction, the identity map is returned.

If a reader finds another one, that is a good outcome, not a bad one. The repo has
an issue template for "the verdict looks wrong" that asks for an anonymized ledger.

## Things that will be said that are fair

Have these ready, and concede them rather than arguing:

- 20 graded predictions is a lot to ask before the tool says anything.
- Most people will log five predictions and stop.
- The cat is not to everyone's taste.
- "Calibration" has a specific technical meaning and the plain-English mode is
  necessarily loose with it.
- There is no sync, no mobile, and no way to share a prediction with a friend.

## Things NOT to say

- No superlatives. No "the first". No "every other tool is X".
- Nothing about the research being "blunt" or "clear" without a citation that has
  been opened.
- No claim that it improves anyone's judgement.
- Do not argue with someone who found a bug. Fix it during the thread and say what
  changed — that is what went well for the comparable calibration Show HN.
