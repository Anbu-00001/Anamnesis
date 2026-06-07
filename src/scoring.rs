//! Proper scoring rules and calibration diagnostics.
//!
//! Everything in this module is pure: it takes a slice of resolved
//! [`Sample`]s — a probability you assigned and what actually happened — and
//! returns numbers. No I/O, no randomness, no hidden state. That purity is the
//! point: the conclusions a person reaches about their own judgement should be
//! reproducible and auditable, not the verdict of a black box.
//!
//! References (verified against the literature, see the project README):
//!   * Brier, G.W. (1950). *Verification of forecasts expressed in terms of
//!     probability.* Monthly Weather Review.
//!   * Murphy, A.H. (1973). *A new vector partition of the probability score.*
//!     The reliability–resolution–uncertainty decomposition implemented here.
//!   * Lichtenstein, Fischhoff & Phillips (1982). Calibration / the
//!     over/under-confidence gap (mean confidence minus proportion correct).

use std::collections::BTreeMap;

/// A resolved forecast: the probability assigned to an event, and whether the
/// event happened (`1.0`) or did not (`0.0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Forecaster's probability that the event is true, in `[0, 1]`.
    pub prob: f64,
    /// Ground truth: `1.0` if the event happened, `0.0` otherwise.
    pub outcome: f64,
}

impl Sample {
    pub fn new(prob: f64, happened: bool) -> Self {
        Sample {
            prob,
            outcome: if happened { 1.0 } else { 0.0 },
        }
    }
}

/// Mean **Brier score**: the mean squared error of probabilistic forecasts.
///
/// `0.0` is perfect. Always saying `0.5` on truly 50/50 events scores `0.25`.
/// Being confidently, maximally wrong every time scores `1.0`. Lower is better.
/// Returns `None` for an empty sample.
pub fn brier(samples: &[Sample]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let sum: f64 = samples.iter().map(|s| (s.prob - s.outcome).powi(2)).sum();
    Some(sum / samples.len() as f64)
}

/// Mean **logarithmic (Good) score**: the average negative log-likelihood of
/// the outcomes under your forecasts. Lower is better.
///
/// The log score is *strictly* proper and punishes confident errors far more
/// harshly than Brier does — one `p = 0.999` forecast on an event that does not
/// happen is brutal. Because a single confident miss would otherwise yield
/// `+∞`, probabilities are clamped into `[eps, 1 - eps]`. A reasonable `eps` is
/// `1e-6`; passing `0.0` recovers the unclamped score.
pub fn log_score(samples: &[Sample], eps: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let sum: f64 = samples
        .iter()
        .map(|s| {
            let p = s.prob.clamp(eps, 1.0 - eps);
            -(s.outcome * p.ln() + (1.0 - s.outcome) * (1.0 - p).ln())
        })
        .sum();
    Some(sum / samples.len() as f64)
}

/// The **base rate**: the fraction of sampled events that actually happened.
/// This is the climatology a useful forecaster must beat.
pub fn base_rate(samples: &[Sample]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    Some(samples.iter().map(|s| s.outcome).sum::<f64>() / samples.len() as f64)
}

/// Murphy's three-way partition of the Brier score.
///
/// `brier == reliability - resolution + uncertainty`, exactly (see
/// [`decompose`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decomposition {
    /// **Reliability** (calibration error). How far your stated probabilities
    /// drift from the frequencies that actually followed them. Lower is better;
    /// `0` means perfect calibration.
    pub reliability: f64,
    /// **Resolution** (discrimination). How much your forecasts move *with*
    /// reality away from the base rate. Higher is better; `0` means you said
    /// the same thing regardless of what was true.
    pub resolution: f64,
    /// **Uncertainty.** The irreducible variance of the outcomes themselves,
    /// `base_rate * (1 - base_rate)`. Nobody can score below this by skill
    /// alone; it is the difficulty of the questions you chose.
    pub uncertainty: f64,
    /// The Brier score reconstructed from the three parts above.
    pub brier: f64,
}

/// Decompose the Brier score into reliability, resolution and uncertainty.
///
/// Forecasts are grouped by their *exact* probability value, which makes the
/// identity `BS = REL - RES + UNC` hold to floating-point precision rather than
/// approximately. (Binning into ranges, as a reliability diagram does, would
/// introduce a within-bin term and only approximate equality.) This exactness
/// is asserted in the test suite.
pub fn decompose(samples: &[Sample]) -> Option<Decomposition> {
    if samples.is_empty() {
        return None;
    }
    let n = samples.len() as f64;
    let o_bar = base_rate(samples).unwrap();

    // Group by the exact bit pattern of the forecast probability. Each group
    // shares a single representative forecast value `f_k`.
    // value = (representative prob, sum of outcomes, count)
    let mut groups: BTreeMap<u64, (f64, f64, f64)> = BTreeMap::new();
    for s in samples {
        let e = groups.entry(s.prob.to_bits()).or_insert((s.prob, 0.0, 0.0));
        e.1 += s.outcome;
        e.2 += 1.0;
    }

    let mut reliability = 0.0;
    let mut resolution = 0.0;
    for (f_k, sum_o, n_k) in groups.values() {
        let o_k = sum_o / n_k; // observed frequency within this forecast value
        reliability += n_k * (f_k - o_k).powi(2);
        resolution += n_k * (o_k - o_bar).powi(2);
    }
    reliability /= n;
    resolution /= n;
    let uncertainty = o_bar * (1.0 - o_bar);

    Some(Decomposition {
        reliability,
        resolution,
        uncertainty,
        brier: reliability - resolution + uncertainty,
    })
}

/// **Brier skill score**: `1 - brier / uncertainty`.
///
/// `0` means you did exactly as well as always predicting the base rate.
/// Positive means skill; negative means you would have been better off
/// parroting climatology. Returns `None` when uncertainty is `0` (every event
/// resolved the same way, so there was nothing to be skillful about).
pub fn skill_score(samples: &[Sample]) -> Option<f64> {
    let d = decompose(samples)?;
    if d.uncertainty == 0.0 {
        return None;
    }
    Some(1.0 - d.brier / d.uncertainty)
}

/// One cell of a reliability diagram.
#[derive(Clone, Copy, Debug)]
pub struct Bin {
    /// Inclusive lower edge of the probability range.
    pub lo: f64,
    /// Exclusive upper edge (the final bin is closed on the right).
    pub hi: f64,
    /// How many forecasts fell in this bin.
    pub count: usize,
    /// Mean forecast probability among those forecasts.
    pub mean_pred: f64,
    /// Observed frequency of "true" among those forecasts. `NaN` when empty.
    pub observed: f64,
}

/// Bucket forecasts into `n_bins` equal-width bins over `[0, 1]` and report,
/// per bin, the mean predicted probability versus the observed frequency. This
/// is the data behind a reliability diagram: on a perfectly calibrated record
/// every populated bin has `mean_pred == observed`.
pub fn calibration_curve(samples: &[Sample], n_bins: usize) -> Vec<Bin> {
    let n_bins = n_bins.max(1);
    // (count, sum_pred, sum_outcome)
    let mut acc = vec![(0usize, 0.0f64, 0.0f64); n_bins];
    for s in samples {
        let mut idx = (s.prob * n_bins as f64).floor() as isize;
        idx = idx.clamp(0, n_bins as isize - 1); // p == 1.0 lands in the last bin
        let cell = &mut acc[idx as usize];
        cell.0 += 1;
        cell.1 += s.prob;
        cell.2 += s.outcome;
    }
    acc.into_iter()
        .enumerate()
        .map(|(i, (count, sum_pred, sum_out))| {
            let lo = i as f64 / n_bins as f64;
            let hi = (i + 1) as f64 / n_bins as f64;
            let (mean_pred, observed) = if count > 0 {
                (sum_pred / count as f64, sum_out / count as f64)
            } else {
                ((lo + hi) / 2.0, f64::NAN)
            };
            Bin {
                lo,
                hi,
                count,
                mean_pred,
                observed,
            }
        })
        .collect()
}

/// **Discrimination**, as the area under the ROC curve (AUC): the probability
/// that a randomly chosen event that *did* happen was given a higher forecast
/// than a randomly chosen event that did *not*. Ties contribute `0.5`.
///
/// `0.5` is no discriminating skill (you may still be perfectly calibrated!);
/// `1.0` is perfect separation. Returns `None` when every outcome is the same,
/// because AUC is undefined without both a positive and a negative case.
///
/// Calibration and discrimination are genuinely different virtues: a forecaster
/// who always reports the true base rate is perfectly calibrated yet useless
/// (AUC `0.5`). This metric measures the part calibration cannot see.
///
/// Computed in `O(n log n)` via the Wilcoxon–Mann–Whitney rank identity
/// `AUC = (R₊ − n₊(n₊+1)/2) / (n₊·n₋)`, where `R₊` is the sum of the ranks of
/// the positive cases using *average ranks* for ties. (A self-evidently correct
/// `O(n²)` pairwise version lives in the tests and is asserted to agree exactly,
/// ties included.)
pub fn auc(samples: &[Sample]) -> Option<f64> {
    let n_pos = samples.iter().filter(|s| s.outcome >= 0.5).count();
    let n_neg = samples.len() - n_pos;
    if n_pos == 0 || n_neg == 0 {
        return None;
    }

    // Rank by forecast probability, ascending, ranks 1..=N. total_cmp gives a
    // panic-free total order; tie groups (equal prob) share their average rank.
    let mut order: Vec<usize> = (0..samples.len()).collect();
    order.sort_by(|&a, &b| samples[a].prob.total_cmp(&samples[b].prob));
    let mut rank = vec![0.0f64; samples.len()];
    let mut i = 0;
    while i < order.len() {
        let mut j = i;
        while j + 1 < order.len() && samples[order[j + 1]].prob == samples[order[i]].prob {
            j += 1;
        }
        let avg = ((i + 1) + (j + 1)) as f64 / 2.0; // average of ranks (i+1)..=(j+1)
        for k in i..=j {
            rank[order[k]] = avg;
        }
        i = j + 1;
    }

    let r_pos: f64 = samples
        .iter()
        .zip(&rank)
        .filter(|(s, _)| s.outcome >= 0.5)
        .map(|(_, r)| *r)
        .sum();
    let (np, nn) = (n_pos as f64, n_neg as f64);
    Some((r_pos - np * (np + 1.0) / 2.0) / (np * nn))
}

/// The classic over/under-confidence statistic (Lichtenstein–Fischhoff).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Overconfidence {
    /// Mean *boldness*: the average of `max(p, 1 - p)` — how far, on average,
    /// you stood from a coin-flip.
    pub mean_confidence: f64,
    /// Proportion of the side you leaned toward that turned out correct. A
    /// forecast of exactly `0.5` is scored as half-right.
    pub accuracy: f64,
    /// `mean_confidence - accuracy`. Positive ⇒ overconfident (you were bolder
    /// than you were right). Negative ⇒ underconfident.
    pub gap: f64,
}

/// Compute the over/under-confidence gap: mean confidence minus the proportion
/// of leaned-toward sides that came true. This is the headline number behind
/// "most people are overconfident": across decades of studies the gap is
/// reliably positive.
pub fn overconfidence(samples: &[Sample]) -> Option<Overconfidence> {
    if samples.is_empty() {
        return None;
    }
    let n = samples.len() as f64;
    let mut conf = 0.0;
    let mut hits = 0.0;
    for s in samples {
        conf += s.prob.max(1.0 - s.prob);
        let hit = if (s.prob - 0.5).abs() < 1e-12 {
            0.5 // no side chosen; half credit
        } else if s.prob > 0.5 {
            s.outcome // leaned "yes"
        } else {
            1.0 - s.outcome // leaned "no"
        };
        hits += hit;
    }
    Some(Overconfidence {
        mean_confidence: conf / n,
        accuracy: hits / n,
        gap: (conf - hits) / n,
    })
}

/// Calibration-in-the-large: mean forecast minus base rate. A directional bias
/// detector — positive means you systematically said "more likely" than reality
/// delivered, independent of how bold any individual call was.
pub fn directional_bias(samples: &[Sample]) -> Option<f64> {
    Some(
        samples.iter().map(|s| s.prob).sum::<f64>() / samples.len().max(1) as f64
            - base_rate(samples)?,
    )
}

// ───────────────────────── numeric / interval forecasts ─────────────────────

/// A resolved *numeric* forecast: a central credible interval `[low, high]`
/// stated at confidence `level` (e.g. `0.80` for an 80% interval), and the
/// value that actually occurred. This is the quantity-forecasting analogue of
/// [`Sample`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumericSample {
    pub low: f64,
    pub high: f64,
    /// Nominal coverage in `(0, 1)`.
    pub level: f64,
    pub value: f64,
}

impl NumericSample {
    pub fn width(&self) -> f64 {
        (self.high - self.low).max(0.0)
    }
    pub fn contains(&self) -> bool {
        self.value >= self.low && self.value <= self.high
    }
    /// `α = 1 − level`, clamped away from `0` so the penalty stays finite.
    pub fn alpha(&self) -> f64 {
        (1.0 - self.level).clamp(1e-9, 1.0)
    }
}

/// **Winkler interval score** for one numeric forecast. Lower is better.
///
/// Inside the interval the score is simply its width; outside, the width plus a
/// `2/α` penalty proportional to how far the truth fell beyond the nearer edge.
/// It is a strictly proper scoring rule for central prediction intervals, and it
/// captures the right trade-off: narrow intervals are rewarded, but only if they
/// keep catching the outcome.
pub fn winkler(s: &NumericSample) -> f64 {
    let width = s.high - s.low;
    let two_over_alpha = 2.0 / s.alpha();
    if s.value < s.low {
        width + two_over_alpha * (s.low - s.value)
    } else if s.value > s.high {
        width + two_over_alpha * (s.value - s.high)
    } else {
        width
    }
}

/// Empirical **coverage**: the fraction of intervals that actually contained
/// their outcome. Compared with the nominal level, this is interval calibration:
/// 80% intervals that catch the truth far less than 80% of the time are too
/// narrow — the numeric face of overconfidence.
pub fn coverage(samples: &[NumericSample]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    Some(samples.iter().filter(|s| s.contains()).count() as f64 / samples.len() as f64)
}

// ───────────────────────── small-sample uncertainty ─────────────────────────

/// **Wilson score interval** for a binomial proportion `successes / n`.
///
/// A confidence interval on a rate that, unlike the normal ("Wald") approximation,
/// stays inside `[0, 1]` and keeps sensible coverage at small `n` and extreme
/// proportions — exactly the regime a single agent or session lives in, where the
/// literature warns the central-limit approximation is unsafe. `z` is the
/// standard-normal quantile (≈ `1.96` for 95%). Returns `None` for `n == 0`.
pub fn wilson_interval(successes: f64, n: usize, z: f64) -> Option<(f64, f64)> {
    if n == 0 {
        return None;
    }
    let n = n as f64;
    let p = (successes / n).clamp(0.0, 1.0);
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denom;
    let margin = (z / denom) * ((p * (1.0 - p) / n) + (z2 / (4.0 * n * n))).sqrt();
    Some(((center - margin).max(0.0), (center + margin).min(1.0)))
}

/// **Empirical-Bayes-style shrinkage** of a rate toward a prior mean.
///
/// `(successes + strength·prior_mean) / (n + strength)` — equivalently a
/// `Beta(strength·prior_mean, strength·(1−prior_mean))` prior updated by the
/// data. At small `n` the estimate is pulled toward `prior_mean` (borrowing
/// strength); as `n` grows it converges to the raw rate. This is what keeps a
/// single fluky resolution from dominating a per-kind calibration number — the
/// literature finds shrinkage beats both raw rates and fixed pseudocounts.
pub fn shrink_toward(successes: f64, n: usize, prior_mean: f64, strength: f64) -> f64 {
    (successes + strength * prior_mean) / (n as f64 + strength)
}

// ─────────────────────── anytime-valid calibration test ─────────────────────

/// Fixed symmetric grid of betting fractions λ used by the mixture e-process.
/// `|λ| < 1` guarantees every factor `1 + λ(y − p)` is non-negative (since
/// `y − p ∈ [−1, 1]`), so each `∏(1 + λ(yᵢ − pᵢ))` is a valid non-negative test
/// martingale; averaging over a symmetric grid gives two-sided power without any
/// data-dependent tuning of λ.
const EPROCESS_LAMBDAS: [f64; 10] = [-0.9, -0.7, -0.5, -0.3, -0.1, 0.1, 0.3, 0.5, 0.7, 0.9];

/// **Anytime-valid calibration e-value** — a betting / test-martingale statistic
/// for the null hypothesis "these forecasts are calibrated."
///
/// Under calibration each outcome behaves like a draw with the stated
/// probability, so `E[yᵢ − pᵢ | past] = 0` and, for each betting fraction λ, the
/// wealth `∏ᵢ (1 + λ(yᵢ − pᵢ))` is a non-negative martingale starting at `1`.
/// This returns the mean wealth over [`EPROCESS_LAMBDAS`], itself a valid
/// e-process. By Ville's inequality the value exceeds `1/α` with probability at
/// most `α` *under the null at any stopping time* — so, unlike a fixed-n test
/// (Spiegelhalter's Z, a t-test), it stays valid even though you peek at it every
/// session. Read it as evidence of *mis*calibration: `≈ 1` is none, `≥ 20` is
/// "significant at α = 0.05", and it grows without bound as miscalibration
/// accumulates. Samples are consumed in the given order, so pass them
/// chronologically (the order outcomes were learned). `None` for an empty record.
pub fn calibration_eprocess(samples: &[Sample]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut wealth = [1.0f64; EPROCESS_LAMBDAS.len()];
    for s in samples {
        let z = s.outcome - s.prob; // mean zero under the calibration null
        for (w, &lam) in wealth.iter_mut().zip(EPROCESS_LAMBDAS.iter()) {
            *w *= 1.0 + lam * z;
        }
    }
    Some(wealth.iter().sum::<f64>() / EPROCESS_LAMBDAS.len() as f64)
}

/// Convert an e-value to an **anytime-valid p-value** via the canonical e→p
/// calibrator `1/e` (capped at `1`). Valid under optional stopping.
pub fn eprocess_pvalue(e: f64) -> f64 {
    (1.0 / e).min(1.0)
}

// ─────────────────────────── recalibration map ──────────────────────────────

fn sigmoid(x: f64) -> f64 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let e = x.exp();
        e / (1.0 + e)
    }
}

fn logit(p: f64) -> f64 {
    (p / (1.0 - p)).ln()
}

/// A learned **recalibration map** `p ↦ σ(a + b·logit p)` — the post-hoc
/// correction that turns your stated probabilities into ones that match your
/// realised frequencies.
///
/// `a` is bias on the log-odds scale (`≠ 0` ⇒ over/under-confident on average);
/// `b` is the slope (`b < 1` ⇒ forecasts too extreme, `b > 1` ⇒ too timid).
/// Perfect calibration is `(a, b) = (0, 1)`, the identity. Apply it with
/// [`Recalibration::apply`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recalibration {
    pub a: f64,
    pub b: f64,
    /// How many resolved samples the fit rests on — i.e. how far to trust it.
    pub n: usize,
}

impl Recalibration {
    /// The identity map — "no correction yet", the small-`n` fallback.
    pub fn identity(n: usize) -> Self {
        Recalibration { a: 0.0, b: 1.0, n }
    }

    /// Correct a single probability through the map.
    pub fn apply(&self, p: f64) -> f64 {
        sigmoid(self.a + self.b * logit(p.clamp(1e-6, 1.0 - 1e-6)))
    }
}

/// Fit a **ridge-regularised logistic recalibration** `σ(a + b·logit p)` by
/// Newton–Raphson, penalising departure from the identity `(0, 1)` with strength
/// `ridge`.
///
/// The penalty is what makes the fit safe at small `n`: with few resolutions the
/// map barely leaves the identity (you have not *earned* a correction yet), and
/// it only develops real bias/slope as evidence accumulates — the same
/// borrow-strength logic as [`shrink_toward`]. The ridge also tames the separable
/// case (every bold call right), where the unpenalised MLE would diverge to
/// infinity. Returns `None` for an empty record.
pub fn fit_recalibration(samples: &[Sample], ridge: f64) -> Option<Recalibration> {
    if samples.is_empty() {
        return None;
    }
    let xs: Vec<f64> = samples
        .iter()
        .map(|s| logit(s.prob.clamp(1e-6, 1.0 - 1e-6)))
        .collect();
    let (mut a, mut b) = (0.0f64, 1.0f64);
    for _ in 0..50 {
        // Gradient g and Hessian H of NLL + (ridge/2)·[a² + (b−1)²].
        let (mut ga, mut gb) = (ridge * a, ridge * (b - 1.0));
        let (mut haa, mut hab, mut hbb) = (ridge, 0.0, ridge);
        for (x, s) in xs.iter().zip(samples) {
            let mu = sigmoid(a + b * x);
            let w = mu * (1.0 - mu);
            let r = mu - s.outcome;
            ga += r;
            gb += r * x;
            haa += w;
            hab += w * x;
            hbb += w * x * x;
        }
        // Newton step [a, b] -= H⁻¹ g. H is SPD thanks to the ridge, so the 2×2
        // determinant is strictly positive and the inverse always exists.
        let det = haa * hbb - hab * hab;
        if det.abs() < 1e-12 {
            break;
        }
        let da = (hbb * ga - hab * gb) / det;
        let db = (haa * gb - hab * ga) / det;
        a -= da;
        b -= db;
        if da.abs() < 1e-10 && db.abs() < 1e-10 {
            break;
        }
    }
    Some(if a.is_finite() && b.is_finite() {
        Recalibration {
            a,
            b,
            n: samples.len(),
        }
    } else {
        Recalibration::identity(samples.len())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {a} ≈ {b}");
    }

    #[test]
    fn shrinkage_pulls_small_n_toward_prior() {
        // 1/1 with a 0.5 prior, strength 4 → (1+2)/(1+4) = 0.6, not 1.0.
        approx(shrink_toward(1.0, 1, 0.5, 4.0), 0.6);
        // Large n barely moves: 80/100 shrunk toward 0.5 stays ≈ 0.79.
        let s = shrink_toward(80.0, 100, 0.5, 4.0);
        assert!(s > 0.78 && s < 0.80, "s={s}");
        // strength 0 recovers the raw rate.
        approx(shrink_toward(3.0, 4, 0.5, 0.0), 0.75);
    }

    #[test]
    fn wilson_interval_known_and_bounded() {
        // 1 of 2 at 95%: symmetric about 0.5 and wide, as small n demands.
        let (lo, hi) = wilson_interval(1.0, 2, 1.96).unwrap();
        assert!((lo - 0.0945).abs() < 1e-3, "lo={lo}");
        assert!((hi - 0.9055).abs() < 1e-3, "hi={hi}");
        // 0 of 5: clamps at 0 below, stays strictly inside (0,1) above.
        let (lo0, hi0) = wilson_interval(0.0, 5, 1.96).unwrap();
        assert_eq!(lo0, 0.0);
        assert!(hi0 > 0.0 && hi0 < 1.0, "hi0={hi0}");
        // The interval always brackets the point estimate.
        let (lo2, hi2) = wilson_interval(7.0, 10, 1.96).unwrap();
        assert!(lo2 < 0.7 && 0.7 < hi2, "[{lo2},{hi2}] should bracket 0.7");
        // n = 0 → undefined.
        assert!(wilson_interval(0.0, 0, 1.96).is_none());
    }

    #[test]
    fn brier_known_values() {
        // Perfect, confident, correct.
        approx(
            brier(&[Sample::new(1.0, true), Sample::new(0.0, false)]).unwrap(),
            0.0,
        );
        // Always 0.5: squared error 0.25 every time.
        approx(
            brier(&[Sample::new(0.5, true), Sample::new(0.5, false)]).unwrap(),
            0.25,
        );
        // Confidently, maximally wrong.
        approx(
            brier(&[Sample::new(0.0, true), Sample::new(1.0, false)]).unwrap(),
            1.0,
        );
        assert!(brier(&[]).is_none());
    }

    #[test]
    fn log_score_known_values() {
        // p = 0.5 every time ⇒ -ln(0.5) = ln 2.
        approx(
            log_score(&[Sample::new(0.5, true), Sample::new(0.5, false)], 0.0).unwrap(),
            std::f64::consts::LN_2,
        );
        // Perfect & confident with clamping ⇒ ~0.
        let s = log_score(&[Sample::new(1.0, true), Sample::new(0.0, false)], 1e-12).unwrap();
        assert!(s < 1e-9, "near-perfect log score should be tiny, got {s}");
    }

    #[test]
    fn decomposition_identity_holds_exactly() {
        // The whole reason for grouping by exact forecast value: REL - RES + UNC
        // must reproduce the empirical Brier to floating-point precision, for
        // arbitrary data. Exercise a deterministic but irregular spread.
        let mut samples = Vec::new();
        let probs = [0.05, 0.2, 0.35, 0.5, 0.65, 0.8, 0.95, 0.5, 0.8, 0.2];
        for (i, &p) in probs.iter().cycle().take(137).enumerate() {
            // pseudo-random-ish but deterministic outcome
            let happened = ((i * 2654435761usize) >> 13) & 1 == 0;
            samples.push(Sample::new(p, happened));
        }
        let d = decompose(&samples).unwrap();
        let b = brier(&samples).unwrap();
        approx(d.brier, b);
        approx(d.reliability - d.resolution + d.uncertainty, b);
        // Components are non-negative.
        assert!(d.reliability >= -1e-12);
        assert!(d.resolution >= -1e-12);
        assert!(d.uncertainty >= -1e-12);
    }

    #[test]
    fn perfect_calibration_has_zero_reliability() {
        // 100 forecasts at p=0.7, exactly 70 of which come true.
        let mut samples = Vec::new();
        for i in 0..100 {
            samples.push(Sample::new(0.7, i < 70));
        }
        let d = decompose(&samples).unwrap();
        approx(d.reliability, 0.0); // perfectly calibrated at this value
    }

    #[test]
    fn auc_separates_and_handles_degenerate() {
        // Perfect separation: every true got a higher prob than every false.
        let s = [
            Sample::new(0.9, true),
            Sample::new(0.8, true),
            Sample::new(0.2, false),
            Sample::new(0.1, false),
        ];
        approx(auc(&s).unwrap(), 1.0);
        // Reversed ranking ⇒ AUC 0.0.
        let r = [
            Sample::new(0.1, true),
            Sample::new(0.2, true),
            Sample::new(0.8, false),
            Sample::new(0.9, false),
        ];
        approx(auc(&r).unwrap(), 0.0);
        // All ties at 0.5 ⇒ AUC 0.5.
        let t = [Sample::new(0.5, true), Sample::new(0.5, false)];
        approx(auc(&t).unwrap(), 0.5);
        // No negatives ⇒ undefined.
        assert!(auc(&[Sample::new(0.6, true)]).is_none());
    }

    /// Self-evidently-correct `O(n²)` reference, kept only to validate the fast
    /// rank-based `auc`.
    fn auc_pairwise(samples: &[Sample]) -> Option<f64> {
        let pos: Vec<f64> = samples
            .iter()
            .filter(|s| s.outcome >= 0.5)
            .map(|s| s.prob)
            .collect();
        let neg: Vec<f64> = samples
            .iter()
            .filter(|s| s.outcome < 0.5)
            .map(|s| s.prob)
            .collect();
        if pos.is_empty() || neg.is_empty() {
            return None;
        }
        let mut wins = 0.0;
        for &p in &pos {
            for &q in &neg {
                if p > q {
                    wins += 1.0;
                } else if p == q {
                    wins += 0.5;
                }
            }
        }
        Some(wins / (pos.len() as f64 * neg.len() as f64))
    }

    #[test]
    fn auc_rank_matches_pairwise_reference_with_ties() {
        // Deterministic datasets deliberately full of tied probabilities, so the
        // average-rank tie handling is actually exercised against the oracle.
        let probs = [0.1, 0.1, 0.4, 0.5, 0.5, 0.5, 0.7, 0.7, 0.9, 0.95];
        for seed in 0u64..64 {
            let samples: Vec<Sample> = probs
                .iter()
                .enumerate()
                .map(|(i, &p)| Sample::new(p, ((seed >> (i % 6)) & 1) == 0))
                .collect();
            match (auc(&samples), auc_pairwise(&samples)) {
                (Some(a), Some(b)) => approx(a, b),
                (None, None) => {}
                (x, y) => panic!("AUC definedness disagrees: {x:?} vs {y:?}"),
            }
        }
    }

    #[test]
    fn overconfidence_detects_boldness_gap() {
        // Bold (0.9) but right only half the time ⇒ overconfident by ~0.4.
        let mut samples = Vec::new();
        for i in 0..100 {
            samples.push(Sample::new(0.9, i < 50));
        }
        let o = overconfidence(&samples).unwrap();
        approx(o.mean_confidence, 0.9);
        approx(o.accuracy, 0.5);
        approx(o.gap, 0.4);
        // A perfectly calibrated bold forecaster has gap ~0.
        let mut cal = Vec::new();
        for i in 0..100 {
            cal.push(Sample::new(0.9, i < 90));
        }
        approx(overconfidence(&cal).unwrap().gap, 0.0);
    }

    #[test]
    fn skill_score_zero_for_base_rate_forecaster() {
        // Always predicting the base rate (0.6) ⇒ no skill ⇒ BSS == 0.
        let mut samples = Vec::new();
        for i in 0..100 {
            samples.push(Sample::new(0.6, i < 60));
        }
        approx(skill_score(&samples).unwrap(), 0.0);
    }

    #[test]
    fn calibration_curve_bins_correctly() {
        let samples = [
            Sample::new(0.05, false),
            Sample::new(0.15, false),
            Sample::new(0.95, true),
            Sample::new(1.0, true), // must land in the last bin, not overflow
        ];
        let bins = calibration_curve(&samples, 10);
        assert_eq!(bins.len(), 10);
        assert_eq!(bins[0].count, 1);
        assert_eq!(bins[1].count, 1);
        assert_eq!(bins[9].count, 2); // 0.95 and 1.0
        approx(bins[9].observed, 1.0);
    }

    #[test]
    fn winkler_inside_is_width_outside_is_penalised() {
        let inside = NumericSample {
            low: 10.0,
            high: 20.0,
            level: 0.8,
            value: 15.0,
        };
        approx(winkler(&inside), 10.0); // just the width

        // width 10 + (2/0.2)*(10 - 5) = 10 + 10*5 = 60
        let below = NumericSample {
            low: 10.0,
            high: 20.0,
            level: 0.8,
            value: 5.0,
        };
        approx(winkler(&below), 60.0);
        let above = NumericSample {
            low: 10.0,
            high: 20.0,
            level: 0.8,
            value: 25.0,
        };
        approx(winkler(&above), 60.0);

        // A tighter nominal level (larger α) penalises a miss less.
        let loose = NumericSample {
            low: 10.0,
            high: 20.0,
            level: 0.5,
            value: 5.0,
        };
        // 10 + (2/0.5)*5 = 10 + 4*5 = 30
        approx(winkler(&loose), 30.0);
    }

    #[test]
    fn coverage_counts_contained_intervals() {
        let s = [
            NumericSample {
                low: 0.0,
                high: 10.0,
                level: 0.8,
                value: 5.0,
            }, // in
            NumericSample {
                low: 0.0,
                high: 10.0,
                level: 0.8,
                value: 50.0,
            }, // out
            NumericSample {
                low: 0.0,
                high: 10.0,
                level: 0.8,
                value: 0.0,
            }, // edge = in
        ];
        approx(coverage(&s).unwrap(), 2.0 / 3.0);
        assert!(coverage(&[]).is_none());
    }

    #[test]
    fn eprocess_finds_no_evidence_on_calibrated_data() {
        // Alternating outcomes at p=0.5: calibration-in-the-large holds, and a
        // fixed (non-predictable) bettor cannot profit → wealth never grows.
        let s: Vec<Sample> = (0..60).map(|i| Sample::new(0.5, i % 2 == 0)).collect();
        let e = calibration_eprocess(&s).unwrap();
        assert!(
            e < 5.0,
            "calibrated data should not accumulate evidence, e={e}"
        );
        assert!(eprocess_pvalue(e) > 0.05);
    }

    #[test]
    fn eprocess_detects_gross_miscalibration() {
        // "10% sure" every time, but it always happens → wildly underconfident.
        let s: Vec<Sample> = (0..40).map(|_| Sample::new(0.1, true)).collect();
        let e = calibration_eprocess(&s).unwrap();
        assert!(
            e >= 20.0,
            "gross miscalibration should be significant, e={e}"
        );
        assert!(eprocess_pvalue(e) < 0.05);
    }

    #[test]
    fn eprocess_is_finite_at_extremes_and_none_when_empty() {
        assert!(calibration_eprocess(&[]).is_none());
        // p exactly 0/1 and wrong: factors stay non-negative, wealth finite.
        let s = vec![Sample::new(1.0, false), Sample::new(0.0, true)];
        let e = calibration_eprocess(&s).unwrap();
        assert!(e.is_finite() && e > 0.0, "e={e}");
    }

    #[test]
    fn recalibration_is_identity_on_calibrated_data() {
        // Forecasts that match realised frequencies ⇒ map ≈ identity.
        let mut s = Vec::new();
        for i in 0..100 {
            s.push(Sample::new(0.7, i < 70)); // 0.7 happens 70%
        }
        for i in 0..100 {
            s.push(Sample::new(0.3, i < 30)); // 0.3 happens 30%
        }
        let r = fit_recalibration(&s, 1.0).unwrap();
        assert!(
            (r.apply(0.7) - 0.7).abs() < 0.05,
            "apply(0.7)={}",
            r.apply(0.7)
        );
        assert!(
            (r.apply(0.3) - 0.3).abs() < 0.05,
            "apply(0.3)={}",
            r.apply(0.3)
        );
    }

    #[test]
    fn recalibration_corrects_underconfidence() {
        // Stated 0.6 but true rate 0.9; stated 0.4 but true rate 0.1 → too timid.
        let mut s = Vec::new();
        for i in 0..100 {
            s.push(Sample::new(0.6, i < 90));
        }
        for i in 0..100 {
            s.push(Sample::new(0.4, i < 10));
        }
        let r = fit_recalibration(&s, 1.0).unwrap();
        assert!(r.b > 1.0, "slope should exceed 1 (too timid), b={}", r.b);
        assert!(
            r.apply(0.6) > 0.75,
            "0.6 should be pushed up, got {}",
            r.apply(0.6)
        );
        assert!(
            r.apply(0.4) < 0.25,
            "0.4 should be pushed down, got {}",
            r.apply(0.4)
        );
    }

    #[test]
    fn recalibration_stays_near_identity_at_small_n() {
        // One lucky hit must not yield a confident correction.
        let r = fit_recalibration(&[Sample::new(0.6, true)], 2.0).unwrap();
        assert!(
            (r.apply(0.6) - 0.6).abs() < 0.12,
            "apply(0.6)={}",
            r.apply(0.6)
        );
    }

    #[test]
    fn recalibration_survives_perfect_separation() {
        // Unpenalised MLE would diverge; the ridge keeps it finite and monotone.
        let s = [
            Sample::new(0.2, false),
            Sample::new(0.3, false),
            Sample::new(0.7, true),
            Sample::new(0.8, true),
        ];
        let r = fit_recalibration(&s, 1.0).unwrap();
        assert!(r.a.is_finite() && r.b.is_finite(), "a={}, b={}", r.a, r.b);
        assert!(r.apply(0.8) > r.apply(0.2), "map should stay increasing");
        assert!(fit_recalibration(&[], 1.0).is_none());
    }
}
