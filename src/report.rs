//! Rendering the calibration report — the moment the ledger stops being a diary
//! and becomes a mirror.
//!
//! The metrics are computed exactly once into a serializable [`ReportData`]; the
//! human ([`render`]) and machine ([`render_json`]) views are two renderings of
//! that single source of truth, so they can never silently disagree. Every
//! metric that can be undefined is an `Option<f64>` — which serialises to JSON
//! `null` rather than a `NaN` that serde_json would quietly turn into `null`
//! anyway (and could never read back).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use chrono::NaiveDate;

use crate::model::Ledger;
use crate::scoring::{self, NumericSample, Sample};

const LANE_W: usize = 34;

/// Coerce a possibly-NaN/Infinite float to `Some(finite)` or `None`.
fn finite(x: f64) -> Option<f64> {
    x.is_finite().then_some(x)
}

// ─────────────────────────── computed data ──────────────────────────────────

/// One bin of the reliability diagram, machine-readable.
#[derive(Serialize, Clone, Debug)]
pub struct BinData {
    pub lo: f64,
    pub hi: f64,
    pub count: usize,
    /// Mean forecast in the bin; `None` when the bin is empty.
    pub mean_pred: Option<f64>,
    /// Observed frequency of "true" in the bin; `None` when empty.
    pub observed: Option<f64>,
}

/// Per-tag slice of the binary record.
#[derive(Serialize, Clone, Debug)]
pub struct TagStat {
    pub tag: String,
    pub n: usize,
    pub brier: Option<f64>,
    pub confidence_gap: Option<f64>,
    /// Confidence gap with accuracy shrunk toward the overall rate — the
    /// small-n-robust number to trust when `n` is tiny.
    pub confidence_gap_shrunk: Option<f64>,
    /// Anytime-valid calibration e-value *within this slice* — the multicalibration
    /// view: which kind of prediction is *really* miscalibrated, not just by luck.
    /// Valid at tiny subgroup `n` (it cannot false-alarm), unlike a raw worst-group
    /// gap. `None` for the by-tag rows, where it is not surfaced.
    pub eprocess: Option<f64>,
}

/// Summary of the claims whose probability you revised before they resolved.
#[derive(Serialize, Clone, Debug)]
pub struct MindChange {
    pub revised: usize,
    pub brier_first: f64,
    pub brier_last: f64,
    /// `brier_first − brier_last`; positive ⇒ your revisions moved toward truth.
    pub improvement: f64,
}

/// Numeric / interval-forecast summary.
#[derive(Serialize, Clone, Debug)]
pub struct NumericData {
    pub count: usize,
    pub mean_winkler: f64,
    /// Mean Winkler score as a multiple of each interval's own width — unitless,
    /// so a question measured in dollars cannot dominate one in milliseconds.
    /// `1.0` = every value landed inside.
    pub mean_winkler_ratio: Option<f64>,
    /// The median of the same quantity, which one disastrous miss cannot move.
    pub median_winkler_ratio: Option<f64>,
    /// Average nominal level of your intervals (e.g. 0.80 for 80% intervals).
    pub nominal_coverage: f64,
    /// Fraction of intervals that actually contained the outcome.
    pub empirical_coverage: f64,
    /// `empirical − nominal`; negative ⇒ your intervals are too narrow
    /// (overconfident), positive ⇒ too wide.
    pub coverage_gap: f64,
    pub mean_width: f64,
    /// Jeffreys-shrunk coverage `(k+½)/(n+1)` — the de-noised coverage point that
    /// keeps a 0-of-3 or 3-of-3 fluke from reading as 0% or 100%.
    pub coverage_shrunk: f64,
    /// Anytime-valid e-value on interval coverage (the same betting test as the
    /// binary side, fed `prob = nominal level`, `outcome = contained`): is the
    /// miscoverage real, or too few intervals to tell?
    pub coverage_eprocess: Option<f64>,
    /// Conformal width multiplier: multiply your interval half-widths by this to
    /// hit your nominal coverage (`>1` widen, `<1` sharpen). `None` below three
    /// usable intervals. Acted on only once `coverage_eprocess` finds real evidence.
    pub width_factor: Option<f64>,
}

/// Prior strength pulling the recalibration map toward the identity — high enough
/// that a handful of resolutions barely move it (you must *earn* a correction).
pub(crate) const RECAL_RIDGE: f64 = 1.5;
/// Minimum e-value (at least *suggestive* evidence) and sample count before a
/// correction is offered — never recalibrate on noise. Shared by the report and
/// the MCP `recalibrate` tool so the two can never disagree.
pub(crate) use scoring::{RECAL_MIN_E, RECAL_MIN_N};

/// The recalibration map for the resolved binary claims matching `tag` (in
/// resolution order), together with whether the e-process has **earned** applying
/// it — real evidence (`≥ RECAL_MIN_E`) over enough samples (`≥ RECAL_MIN_N`). The
/// fitted map is returned regardless; `earned` says whether to trust it. Defined
/// once here, next to the gate constants, so the report, the MCP `recalibrate`/
/// `decide` tools, and the CLI can never disagree about when a correction is real.
pub fn earned_recalibration(
    ledger: &Ledger,
    tag: Option<&str>,
    today: NaiveDate,
) -> (Option<scoring::Recalibration>, bool, usize, Option<f64>) {
    // The gate must consume exactly the sequence the report's headline test does,
    // in the same outcome-independent order, or `decide` and `report` will
    // disagree about whether the evidence exists.
    let evidence = crate::evidence::binary_evidence(&ledger.claims, today, tag);
    // The map itself is fitted on every graded claim: more data makes a better
    // correction, and it is the *gate*, not the fit, that guards validity.
    let all: Vec<Sample> = ledger
        .claims
        .iter()
        .filter(|c| tag.is_none_or(|t| c.tags.iter().any(|x| x == t)))
        .filter_map(|c| c.sample())
        .collect();
    let n = evidence.samples.len();
    let (recal, earned, e) = scoring::gate_recalibration(&all, &evidence.samples, RECAL_RIDGE);
    (recal, earned, n, e)
}

/// Bootstrap settings for the Brier band — a fixed seed makes the report
/// reproducible run-to-run (a band that jittered every run would unsettle).
const BRIER_CI_RESAMPLES: usize = 2000;
const BRIER_CI_SEED: u64 = 0xA11A_5EED_C0FF_EE00;
/// EWMA half-life (in resolutions) for the "lately" Brier, and the minimum record
/// before a recency trend is worth showing at all.
const EWMA_HALF_LIFE: f64 = 5.0;
const TREND_MIN_N: usize = 10;
/// Standardized-mean-difference threshold above which the graded and ungraded
/// forecasts count as drawn from different distributions (the conventional
/// covariate-balance / missing-not-at-random cutoff).
const SELECTION_ASMD_FLAG: f64 = 0.1;
/// Draws and seed for the MCB noise floor. Fixed, so the floor never moves under
/// a user who simply re-runs the report.
const MCB_NULL_DRAWS: usize = 400;
const MCB_NULL_SEED: u64 = 0xA11C_E5EE_D000_0001;

/// Below this many graded calls, no calibration verdict is possible at all.
pub const VERDICT_MIN_N: usize = 20;
/// And below this many, the report never says "well calibrated" — absence of
/// evidence at n = 25 is not evidence of calibration.
pub const VERDICT_CONFIDENT_N: usize = 50;
/// The e-value at which miscalibration counts as demonstrated: `1/20 = α 0.05`
/// by Ville's inequality, at any stopping time.
pub const EVIDENCE_ALARM: f64 = 20.0;
/// How far a directional (yes/no) lean has to run before it is the headline
/// rather than a footnote.
const BIAS_DOMINATES: f64 = 0.1;
/// Below this, the confidence gap is too small to name a direction from — the
/// errors have cancelled, and the sign of what is left is noise.
const DIRECTION_MIN_GAP: f64 = 0.02;
/// Below this much discrimination, the forecasts do not order the outcomes at
/// all, and that fact outranks anything to be said about their level.
const DSC_UNINFORMATIVE: f64 = 1e-9;

/// The one calibration verdict.
///
/// Every shareable surface — the plain report and its cat, the badge SVG, the
/// HTML card, `--json`, the MCP `calibration` tool, the shell hooks, the `decide`
/// explanation — used to key off the **confidence gap**, and over- and
/// under-confidence cancel inside that single number. Measured consequence: a
/// ledger of 50 calls at 0.9 on coin flips plus 50 at 0.6 on sure things, whose
/// Brier skill is −0.520 and whose calibration error is the worst in the test
/// suite, produced a gap of −0.000000000000001 and was therefore announced as
/// `[DIALED IN] · well calibrated`, badged `Well calibrated`, and advised to
/// "keep doing what you're doing".
///
/// So the pass/fail decision is made **here, once**, from evidence (`mcb` against
/// its own noise floor, and the e-process) — never from the gap. The gap is
/// consulted only to name the *direction* of a failure already established.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Too few graded calls to say anything.
    InsufficientData,
    /// No miscalibration has been demonstrated. Not the same as "calibrated":
    /// the test can miss patterns, and small records hide a lot.
    NoEvidenceOfMiscalibration,
    /// The forecasts carry no information about which calls come true: after
    /// isotonic recalibration they all collapse to one value.
    ///
    /// This outranks any statement about over- or under-confidence, including a
    /// demonstrated one. A forecaster whose confidence does not sort the outcomes
    /// has a ranking problem, and shading every number up or down cannot fix it —
    /// it just produces a well-calibrated constant. Fix the ranking first.
    CalibratedButUninformative,
    /// Demonstrably too sure of yourself.
    Overconfident,
    /// Demonstrably not sure enough.
    Underconfident,
    /// Miscalibrated with a directional lean toward YES.
    BiasedYes,
    /// Miscalibrated with a directional lean toward NO.
    BiasedNo,
    /// Demonstrably miscalibrated, with the errors running **both** ways at once
    /// — too sure at the top of the range and not sure enough at the bottom, or
    /// the reverse — so there is no single direction to shade toward.
    ///
    /// This state exists because of a measurement. The 50-coin-flips-at-0.9 plus
    /// 50-sure-things-at-0.6 ledger has a confidence gap of about −1e-15: naming
    /// a direction from it is naming the sign of floating-point noise, and the
    /// answer it happened to give ("underconfident") would have sent the reader
    /// to raise the very 0.9s that were the problem.
    MiscalibratedBothWays,
}

impl Verdict {
    /// Whether miscalibration has actually been demonstrated.
    pub fn is_miscalibrated(self) -> bool {
        matches!(
            self,
            Verdict::Overconfident
                | Verdict::Underconfident
                | Verdict::BiasedYes
                | Verdict::BiasedNo
                | Verdict::MiscalibratedBothWays
        )
    }

    /// A stable machine-readable slug, used by the hooks and the badge.
    pub fn slug(self) -> &'static str {
        match self {
            Verdict::InsufficientData => "insufficient_data",
            Verdict::NoEvidenceOfMiscalibration => "no_evidence_of_miscalibration",
            Verdict::CalibratedButUninformative => "calibrated_but_uninformative",
            Verdict::Overconfident => "overconfident",
            Verdict::Underconfident => "underconfident",
            Verdict::BiasedYes => "biased_yes",
            Verdict::BiasedNo => "biased_no",
            Verdict::MiscalibratedBothWays => "miscalibrated_both_ways",
        }
    }

    /// The short label for the badge and the card. Deliberately never the words
    /// "well calibrated" for anything short of demonstrated-and-plentiful
    /// evidence — see [`Verdict::NoEvidenceOfMiscalibration`].
    pub fn label(self, n: usize) -> &'static str {
        match self {
            Verdict::InsufficientData => "not enough data",
            Verdict::NoEvidenceOfMiscalibration if n >= VERDICT_CONFIDENT_N => "well calibrated",
            Verdict::NoEvidenceOfMiscalibration => "no miscalibration found",
            Verdict::CalibratedButUninformative => "uninformative",
            Verdict::Overconfident => "overconfident",
            Verdict::Underconfident => "underconfident",
            Verdict::BiasedYes => "leans yes",
            Verdict::BiasedNo => "leans no",
            Verdict::MiscalibratedBothWays => "miscalibrated",
        }
    }
}

/// The per-`kind:` samples, in the same outcome-independent order the headline
/// evidence test uses, so a per-kind e-value carries the same guarantee.
fn kind_evidence_order<'a>(
    claims: &'a [crate::model::Claim],
    today: NaiveDate,
    tag: Option<&str>,
) -> Vec<Vec<(&'a str, Sample)>> {
    crate::evidence::binary_evidence_claims(claims, today, tag)
        .into_iter()
        .filter_map(|c| {
            let s = c.evidence_sample()?;
            Some(
                c.tags
                    .iter()
                    .filter_map(|t| t.strip_prefix("kind:").map(|k| (k, s)))
                    .collect(),
            )
        })
        .collect()
}

/// Decide the verdict from evidence alone.
///
/// Inputs: `n` graded calls, the CORP miscalibration and its noise floor, the
/// discrimination, the e-value, and — for *direction only* — the confidence gap
/// and directional bias.
#[allow(clippy::too_many_arguments)]
pub fn verdict_from(
    n: usize,
    mcb: Option<f64>,
    mcb_floor: Option<f64>,
    dsc: Option<f64>,
    eprocess: Option<f64>,
    confidence_gap: Option<f64>,
    directional_bias: Option<f64>,
    distinct_forecasts: usize,
) -> Verdict {
    if n < VERDICT_MIN_N {
        return Verdict::InsufficientData;
    }
    let demonstrated = eprocess.is_some_and(|e| e >= EVIDENCE_ALARM);
    let above_floor = match (mcb, mcb_floor) {
        (Some(m), Some(f)) => m > f,
        (Some(m), None) => m > 0.0,
        _ => false,
    };

    // Ranking outranks calibration, whenever the forecasts sort nothing.
    //
    // On the 90%-coin-flips-plus-60%-sure-things ledger, DSC is exactly 0.0000:
    // the isotonic fit collapses every forecast to one value because higher
    // stated confidence went with a LOWER hit rate. The honest statement there is
    // not about confidence levels at all — it is that the ordering carries no
    // information, and advice about shading numbers up or down is premature. Fix
    // the ranking first; calibrating an uninformative forecast just produces a
    // well-calibrated constant.
    //
    // The guard on `distinct_forecasts` matters: with a single forecast value
    // DSC is zero by construction, because there is nothing to rank. Telling
    // someone who said 0.9 to everything and was right a third of the time to
    // "fix the ranking" would be useless — the finding there really is the level.
    if distinct_forecasts >= 2 && dsc.is_some_and(|d| d.abs() < DSC_UNINFORMATIVE) {
        return Verdict::CalibratedButUninformative;
    }

    if demonstrated && above_floor {
        // Real miscalibration. Now, and only now, look at the direction.
        let bias = directional_bias.unwrap_or(0.0);
        if bias.abs() >= BIAS_DOMINATES && bias.abs() > confidence_gap.map_or(0.0, f64::abs) {
            return if bias > 0.0 {
                Verdict::BiasedYes
            } else {
                Verdict::BiasedNo
            };
        }
        // A direction is only worth naming when there is one. Below this, the
        // over- and under-confident halves have cancelled and the sign of the
        // remainder is noise.
        let gap = confidence_gap.unwrap_or(0.0);
        if gap.abs() < DIRECTION_MIN_GAP {
            return Verdict::MiscalibratedBothWays;
        }
        return if gap > 0.0 {
            Verdict::Overconfident
        } else {
            Verdict::Underconfident
        };
    }

    // Nothing demonstrated, and the forecasts sort nothing: the textbook
    // calibrated-but-useless case, saying 0.5 to everything on a fair coin. It
    // reaches here rather than the check above because with a single forecast
    // value there was never anything to rank.
    if dsc.is_some_and(|d| d.abs() < DSC_UNINFORMATIVE) {
        return Verdict::CalibratedButUninformative;
    }
    Verdict::NoEvidenceOfMiscalibration
}

/// A learned recalibration map, machine-readable: `p ↦ σ(a + b·logit p)` plus a
/// few worked corrections for the human view.
#[derive(Serialize, Clone, Debug)]
pub struct RecalData {
    /// Log-odds bias (`≠ 0` ⇒ over/under-confident on average).
    pub a: f64,
    /// Slope (`< 1` ⇒ too extreme, `> 1` ⇒ too timid).
    pub b: f64,
    pub n: usize,
    /// `(stated, recalibrated)` pairs — what your stated confidences *should* be.
    pub examples: Vec<(f64, f64)>,
}

/// Resolution-discipline / selection-bias diagnostic: is the calibration above
/// computed on a fair sample of your calls, or a self-selected one? Every metric
/// rests only on *resolved* claims; if resolution is non-random (you grade your wins
/// and let your misses rot), the numbers flatter you. This is the honesty check.
#[derive(Serialize, Clone, Debug)]
pub struct ResolutionDiscipline {
    /// Resolved / (resolved + open) across all matching claims. Low ⇒ the report
    /// rests on a self-selected subset.
    pub resolution_rate: f64,
    pub resolved: usize,
    pub open: usize,
    /// Open claims already past their `resolve_by` date — rotting in the drawer
    /// rather than legitimately pending.
    pub overdue: usize,
    /// Mean boldness of resolved vs. still-open binary forecasts — the confidence
    /// profile of what you graded vs. what you didn't. `None` when a side is empty.
    pub resolved_boldness: Option<f64>,
    pub open_boldness: Option<f64>,
    /// Resolutions that were graded from an observed fact rather than the
    /// forecaster's own word — a test run's exit status, say. "Why trust a
    /// self-graded ledger?" is the first fair objection, and this is the part of
    /// the answer that is a number.
    pub auto_graded: usize,
    /// Standardized gap (ASMD) between those two boldness profiles — the
    /// missing-not-at-random diagnostic. `> 0.1` ⇒ your open and graded calls differ
    /// enough that the split is unlikely to be random. `None` below two per side.
    pub boldness_asmd: Option<f64>,
}

/// Selective-prediction summary: how your error falls when you act only on your
/// most-confident calls.
#[derive(Serialize, Clone, Debug)]
pub struct SelectiveData {
    /// Directional error acting on every call.
    pub risk_full: f64,
    /// Directional error acting only on your most-confident half.
    pub risk_half: f64,
    /// Area under the risk–coverage curve (lower ⇒ confidence ranks reliably).
    pub aurcc: f64,
}

/// The whole report as data — serialisable, and the single input to both views.
#[derive(Serialize, Clone, Debug)]
pub struct ReportData {
    pub tag: Option<String>,
    /// Resolved claims of **every** kind. Header arithmetic uses this, so that
    /// `resolved + open == total` actually holds: the header used to print a
    /// binary-only "resolved" beside an all-kinds "open", which is how the demo
    /// ledger came to announce `35 resolved · 6 open` for 50 claims.
    pub resolved: usize,
    /// Resolved binary claims — the `n` behind every probability metric.
    pub resolved_binary: usize,
    /// Resolved numeric (interval) claims — the `n` behind the interval metrics.
    pub resolved_numeric: usize,
    /// Open claims of every kind.
    pub open: usize,
    /// Every matching claim, resolved or not. Always `resolved + open`.
    pub total: usize,
    pub first_recorded: Option<String>,
    pub last_recorded: Option<String>,
    pub brier: Option<f64>,
    pub log_score: Option<f64>,
    pub brier_skill: Option<f64>,
    pub base_rate: Option<f64>,
    /// 95% Wilson interval on the base rate — how firmly this many resolutions
    /// pin the rate down (a wide interval means too few samples to trust).
    pub base_rate_ci: Option<(f64, f64)>,
    pub reliability: Option<f64>,
    pub resolution: Option<f64>,
    pub uncertainty: Option<f64>,
    pub auc: Option<f64>,
    pub confidence_gap: Option<f64>,
    pub mean_confidence: Option<f64>,
    pub accuracy: Option<f64>,
    pub directional_bias: Option<f64>,
    pub calibration: Vec<BinData>,
    pub by_tag: Vec<TagStat>,
    /// Per-`kind:` breakdown — calibration by *type* of prediction
    /// (estimate / tests-pass / bug-hypothesis …): the agent-facing view.
    pub by_kind: Vec<TagStat>,
    pub mind_changing: Option<MindChange>,
    pub numeric: Option<NumericData>,
    /// Anytime-valid calibration e-value — evidence that you are *mis*calibrated,
    /// valid no matter how often you check it. `≥ 20` ⇒ significant at α = 0.05.
    pub eprocess: Option<f64>,
    /// The e→p calibrated, optional-stopping-valid p-value (`1/eprocess`).
    pub eprocess_pvalue: Option<f64>,
    /// The learned correction to apply to your stated probabilities.
    pub recalibration: Option<RecalData>,
    /// Bootstrap percentile band on the Brier score — how far luck alone could
    /// move it. `None` for fewer than two resolutions.
    pub brier_ci: Option<(f64, f64)>,
    /// Recency-weighted (EWMA) Brier — "how am I doing *lately*", read against
    /// `brier`. Descriptive trend, not a significance test.
    pub recent_brier: Option<f64>,
    /// Distinct forecast probabilities used — a coarse vocabulary caps resolution.
    pub distinct_forecasts: usize,
    /// Selective-prediction summary — error among your surest calls vs all.
    pub selective: Option<SelectiveData>,
    /// Stake-weighted Brier — present only when stakes vary; compare with `brier`
    /// to see whether you are worse on the calls that actually matter.
    pub weighted_brier: Option<f64>,
    /// Resolution-discipline / selection-bias check — whether the metrics above rest
    /// on a fair sample of your calls or a self-selected one.
    pub resolution_discipline: Option<ResolutionDiscipline>,

    // ── P0-2: which forecast the headline score is computed on ──────────────
    /// Always `"first"`. The graded forecast is the one made *before* the answer
    /// was known; see [`crate::model::Claim::sample_final`] for why.
    pub score_basis: &'static str,
    /// The headline Brier, on first forecasts. Equal to `brier`.
    pub brier_first: Option<f64>,
    /// Brier on the final forecast — shown for comparison, never graded.
    pub brier_final: Option<f64>,
    /// Brier weighted by how long each forecast stood. Secondary metric.
    pub brier_time_avg: Option<f64>,
    /// Resolved claims carrying a forecast made too close to the answer to grade.
    pub late_updates: usize,

    // ── P0-4: the CORP decomposition ────────────────────────────────────────
    /// Miscalibration (CORP). Replaces `reliability` as the calibration number.
    pub mcb: Option<f64>,
    /// Discrimination (CORP). `0` ⇒ your confidence says nothing about which
    /// calls come true.
    pub dsc: Option<f64>,
    /// Uncertainty (CORP) — identical in meaning to `uncertainty`.
    pub unc: Option<f64>,
    /// The 95th percentile of `mcb` for a *calibrated* forecaster making exactly
    /// these calls: the noise floor below which `mcb` means nothing.
    pub mcb_null_q95: Option<f64>,

    // ── P0-3: what the evidence test actually consumed ──────────────────────
    /// Claims in the evidence sequence (due, graded, in deadline order).
    pub evidence_n: usize,
    /// Id of the first overdue, ungraded claim — the evidence pauses there.
    pub evidence_blocked_by: Option<String>,
    /// Id of the first claim whose due date has passed and which is still
    /// ungraded. The evidence test stops there, on purpose.
    pub evidence_waiting: usize,
    /// Whether the blocking claim has no `--by` date, in which case adding one is
    /// the other way to unblock the sequence.
    pub evidence_blocked_without_deadline: bool,
    /// Claims voided *after* they had already resolved — outcomes deleted from
    /// the record after being seen. They stay in the evidence sequence; this
    /// count exists so the edit is never silent.
    pub voided_after_resolution: usize,
    /// `ln` of the e-value, for comparing magnitudes past the display cap.
    pub eprocess_log: Option<f64>,
    /// The e-value a single per-kind subgroup must clear, after the union-bound
    /// correction for having searched `K` subgroups.
    pub kind_alarm_threshold: Option<f64>,

    // ── P0-5: the one verdict every surface must agree with ─────────────────
    /// The single calibration verdict. Every surface — text, cat, badge, card,
    /// JSON, hooks, `decide` — derives its wording from this and nothing else.
    pub verdict: Verdict,
}

impl ReportData {
    /// Compute every metric once, for the binary and numeric claims that match
    /// the optional tag filter. `today` anchors the overdue check (real wall-clock
    /// in the CLI; a fixed date in tests).
    pub fn compute(
        ledger: &Ledger,
        tag_filter: Option<&str>,
        bins: usize,
        today: NaiveDate,
    ) -> ReportData {
        let tag_filter = tag_filter.map(|t| t.to_lowercase());
        let matches_tag = |c: &&crate::model::Claim| match &tag_filter {
            Some(t) => c.tags.iter().any(|x| x == t),
            None => true,
        };

        // Voided claims are excluded everywhere: from the scores, from the counts,
        // and from the resolution-discipline check. They stay in the ledger.
        let resolved_claims: Vec<&crate::model::Claim> = ledger
            .claims
            .iter()
            .filter(|c| c.is_resolved() && !c.is_void())
            .filter(matches_tag)
            .collect();
        let samples: Vec<Sample> = resolved_claims.iter().filter_map(|c| c.sample()).collect();
        let numeric_samples: Vec<NumericSample> = resolved_claims
            .iter()
            .filter_map(|c| c.numeric_sample())
            .collect();
        let open_claims: Vec<&crate::model::Claim> = ledger
            .claims
            .iter()
            .filter(|c| c.is_open() && !c.is_void())
            .filter(matches_tag)
            .collect();
        let open = open_claims.len();

        let (first_recorded, last_recorded) = {
            let dates: Vec<_> = resolved_claims
                .iter()
                .map(|c| c.created_at.date_naive())
                .collect();
            (
                dates.iter().min().map(|d| d.to_string()),
                dates.iter().max().map(|d| d.to_string()),
            )
        };

        // The evidence sequence: ordered by resolve-by date, which is fixed when
        // the claim is created. Sorting by resolution time instead looks
        // chronological but is outcome-dependent — YES answers arrive early and
        // NO answers wait for the deadline — and that made a perfectly calibrated
        // forecaster raise a false alarm in 100% of simulated runs when the
        // report was re-read as claims resolved. See `crate::evidence`.
        let evidence =
            crate::evidence::binary_evidence(&ledger.claims, today, tag_filter.as_deref());
        let chrono_samples: Vec<Sample> = evidence.samples.clone();

        // The descriptive "lately" trend is not a test and carries no validity
        // claim, so it may keep using the order outcomes were learned in.
        let mut chrono: Vec<&crate::model::Claim> = resolved_claims.clone();
        chrono.sort_by_key(|c| c.resolution.as_ref().map(|r| r.at));
        let recent_samples: Vec<Sample> = chrono.iter().filter_map(|c| c.sample()).collect();

        let decomp = scoring::decompose(&samples);
        let over = scoring::overconfidence(&samples);
        let overall_acc = over.map(|o| o.accuracy); // shrinkage prior for per-slice gaps

        let calibration = scoring::calibration_curve(&samples, bins)
            .into_iter()
            .map(|b| BinData {
                lo: b.lo,
                hi: b.hi,
                count: b.count,
                mean_pred: (b.count > 0).then_some(b.mean_pred),
                observed: finite(b.observed),
            })
            .collect();

        // By tag (only when not already filtered to a single tag).
        let by_tag = if tag_filter.is_some() {
            Vec::new()
        } else {
            let mut map: BTreeMap<&str, Vec<Sample>> = BTreeMap::new();
            for c in &chrono {
                if let Some(s) = c.sample() {
                    for t in &c.tags {
                        if t.contains(':') {
                            continue; // structural tags (kind:/project:/who:/session:) live elsewhere
                        }
                        map.entry(t.as_str()).or_default().push(s);
                    }
                }
            }
            let mut rows: Vec<TagStat> = map
                .into_iter()
                .map(|(tag, s)| {
                    let oc = scoring::overconfidence(&s);
                    let gap_shrunk = match (oc, overall_acc) {
                        (Some(o), Some(pa)) => Some(
                            o.mean_confidence
                                - scoring::shrink_toward(
                                    o.accuracy * s.len() as f64,
                                    s.len(),
                                    pa,
                                    4.0,
                                ),
                        ),
                        _ => oc.map(|o| o.gap),
                    };
                    TagStat {
                        tag: tag.to_string(),
                        n: s.len(),
                        brier: scoring::brier(&s),
                        confidence_gap: oc.map(|o| o.gap),
                        confidence_gap_shrunk: gap_shrunk,
                        eprocess: None, // surfaced only for the per-kind view
                    }
                })
                .collect();
            rows.sort_by(|a, b| b.n.cmp(&a.n).then(a.tag.cmp(&b.tag)));
            rows
        };

        // By prediction kind (the `kind:` tag namespace) — shown regardless of any
        // project/tag filter, because calibration-per-type is the agent's lever.
        let by_kind = {
            let mut map: BTreeMap<&str, Vec<Sample>> = BTreeMap::new();
            // Built from the same outcome-independent order as the headline test,
            // so a per-kind e-value carries the same guarantee the overall one does.
            for kind in kind_evidence_order(&ledger.claims, today, tag_filter.as_deref()) {
                for (k, s) in kind {
                    map.entry(k).or_default().push(s);
                }
            }
            let mut rows: Vec<TagStat> = map
                .into_iter()
                .map(|(tag, s)| {
                    let oc = scoring::overconfidence(&s);
                    let gap_shrunk = match (oc, overall_acc) {
                        (Some(o), Some(pa)) => Some(
                            o.mean_confidence
                                - scoring::shrink_toward(
                                    o.accuracy * s.len() as f64,
                                    s.len(),
                                    pa,
                                    4.0,
                                ),
                        ),
                        _ => oc.map(|o| o.gap),
                    };
                    TagStat {
                        tag: tag.to_string(),
                        n: s.len(),
                        brier: scoring::brier(&s),
                        confidence_gap: oc.map(|o| o.gap),
                        confidence_gap_shrunk: gap_shrunk,
                        // Per-kind anytime-valid evidence, on the evidence
                        // sequence. Because K subgroups are searched at once,
                        // a single row must clear `EVIDENCE_ALARM × K` — a
                        // union bound on top of Ville — before it counts as a
                        // finding rather than a description.
                        eprocess: scoring::calibration_eprocess_v2(&s),
                    }
                })
                .collect();
            rows.sort_by(|a, b| b.n.cmp(&a.n).then(a.tag.cmp(&b.tag)));
            rows
        };

        // 95% Wilson interval on the base rate — surfaces small-n uncertainty.
        let base_rate_ci = {
            let k: f64 = samples.iter().map(|s| s.outcome).sum();
            scoring::wilson_interval(k, samples.len(), 1.96)
        };

        // Mind-changing (revised, resolved binary claims).
        let revised: Vec<&crate::model::Claim> = resolved_claims
            .iter()
            .copied()
            .filter(|c| c.was_revised() && c.sample().is_some())
            .collect();
        let mind_changing = if revised.is_empty() {
            None
        } else {
            let mut first = Vec::new();
            let mut last = Vec::new();
            for c in &revised {
                let happened = c.outcome().unwrap().happened();
                first.push(Sample::new(c.first_prob().unwrap(), happened));
                last.push(Sample::new(c.current_prob().unwrap(), happened));
            }
            let bf = scoring::brier(&first).unwrap();
            let bl = scoring::brier(&last).unwrap();
            Some(MindChange {
                revised: revised.len(),
                brier_first: bf,
                brier_last: bl,
                improvement: bf - bl,
            })
        };

        // Numeric / interval forecasts.
        let numeric = if numeric_samples.is_empty() {
            None
        } else {
            let n = numeric_samples.len() as f64;
            let mean_winkler = numeric_samples.iter().map(scoring::winkler).sum::<f64>() / n;
            // Unitless, skipping zero-width intervals where it is undefined.
            let mut ratios: Vec<f64> = numeric_samples
                .iter()
                .filter_map(scoring::winkler_ratio)
                .collect();
            ratios.sort_by(f64::total_cmp);
            let mean_winkler_ratio =
                (!ratios.is_empty()).then(|| ratios.iter().sum::<f64>() / ratios.len() as f64);
            let median_winkler_ratio = (!ratios.is_empty()).then(|| {
                let m = ratios.len() / 2;
                if ratios.len().is_multiple_of(2) {
                    (ratios[m - 1] + ratios[m]) / 2.0
                } else {
                    ratios[m]
                }
            });
            let nominal = numeric_samples.iter().map(|s| s.level).sum::<f64>() / n;
            let cov = scoring::coverage(&numeric_samples).unwrap_or(f64::NAN);
            let width = numeric_samples.iter().map(|s| s.width()).sum::<f64>() / n;
            // Jeffreys-shrunk coverage point: (k+½)/(n+1), de-noising tiny records.
            let contained = numeric_samples.iter().filter(|s| s.contains()).count() as f64;
            let coverage_shrunk =
                scoring::shrink_toward(contained, numeric_samples.len(), 0.5, 1.0);
            // Coverage is itself a calibration question: did each interval, at its
            // stated level, contain the truth? Reuse the binary e-process on
            // (prob = level, outcome = contained) for an anytime-valid coverage test.
            let coverage_samples: Vec<Sample> = numeric_samples
                .iter()
                .map(|s| Sample::new(s.level, s.contains()))
                .collect();
            Some(NumericData {
                count: numeric_samples.len(),
                mean_winkler,
                mean_winkler_ratio,
                median_winkler_ratio,
                nominal_coverage: nominal,
                empirical_coverage: cov,
                coverage_gap: cov - nominal,
                mean_width: width,
                coverage_shrunk,
                coverage_eprocess: scoring::calibration_eprocess_v2(&coverage_samples),
                width_factor: scoring::conformal_width_factor(&numeric_samples),
            })
        };

        // Anytime-valid calibration test, over the outcome-independent evidence
        // sequence, mixing betting strategies so that symmetric overconfidence
        // (too sure at 0.9 AND too sure at 0.1) cannot cancel itself out.
        let eprocess = scoring::calibration_eprocess_v2(&chrono_samples);
        let eprocess_log = scoring::calibration_log_eprocess(&chrono_samples);
        let eprocess_pvalue = eprocess.map(scoring::eprocess_pvalue);

        // CORP decomposition and the noise floor its value has to clear.
        let corp = scoring::corp_brier(&samples);
        let mcb_null_q95 =
            scoring::mcb_null_quantile(&samples, MCB_NULL_DRAWS, 0.95, MCB_NULL_SEED);

        // P0-2: the headline is the FIRST forecast; these are the comparisons.
        let final_samples: Vec<Sample> = resolved_claims
            .iter()
            .filter_map(|c| c.sample_final())
            .collect();
        let brier_final = scoring::brier(&final_samples);
        let time_avg: Vec<f64> = resolved_claims
            .iter()
            .filter_map(|c| c.brier_time_averaged())
            .collect();
        let brier_time_avg =
            (!time_avg.is_empty()).then(|| time_avg.iter().sum::<f64>() / time_avg.len() as f64);
        let late_updates = resolved_claims
            .iter()
            .filter(|c| c.has_late_update())
            .count();

        // Learned recalibration map (ridge-shrunk toward identity for small n).
        let recalibration = scoring::fit_recalibration(&samples, RECAL_RIDGE).map(|r| RecalData {
            a: r.a,
            b: r.b,
            n: r.n,
            examples: [0.5, 0.6, 0.7, 0.8, 0.9]
                .iter()
                .map(|&p| (p, r.apply(p)))
                .collect(),
        });

        // Small-sample / over-time bands (chrono order reused from the e-process).
        let brier_ci =
            scoring::brier_ci_bootstrap(&samples, 0.95, BRIER_CI_RESAMPLES, BRIER_CI_SEED);
        let recent_brier = scoring::ewma_brier(&recent_samples, EWMA_HALF_LIFE);
        let distinct_forecasts = scoring::distinct_forecasts(&samples);
        let selective = scoring::risk_coverage(&samples).map(|rc| SelectiveData {
            risk_full: rc.risk_at_full,
            risk_half: rc.risk_at(0.5),
            aurcc: rc.aurcc,
        });

        // Stake-weighted Brier — only meaningful (and only shown) when stakes vary.
        let stakes: Vec<f64> = resolved_claims
            .iter()
            .filter_map(|c| c.sample().map(|_| c.stake))
            .collect();
        let weighted_brier = if stakes.iter().any(|&w| (w - 1.0).abs() > 1e-9) {
            scoring::brier_weighted(&samples, &stakes)
        } else {
            None
        };

        // Resolution discipline: is the calibration above a fair sample, or only the
        // calls I bothered to grade? Compares the confidence profile of resolved vs.
        // still-open binary forecasts (outcome-free, so the open side is usable).
        let resolution_discipline = {
            let total = resolved_claims.len() + open_claims.len();
            (total > 0).then(|| {
                let overdue = open_claims
                    .iter()
                    .filter(|c| matches!(c.resolve_by, Some(d) if d <= today))
                    .count();
                let resolved_probs: Vec<f64> = resolved_claims
                    .iter()
                    .filter_map(|c| c.current_prob())
                    .collect();
                let open_probs: Vec<f64> = open_claims
                    .iter()
                    .filter_map(|c| c.current_prob())
                    .collect();
                ResolutionDiscipline {
                    resolution_rate: resolved_claims.len() as f64 / total as f64,
                    resolved: resolved_claims.len(),
                    open: open_claims.len(),
                    overdue,
                    auto_graded: resolved_claims
                        .iter()
                        .filter(|c| {
                            c.resolution.as_ref().and_then(|r| r.resolved_by)
                                == Some(crate::model::ResolvedBy::Auto)
                        })
                        .count(),
                    resolved_boldness: scoring::mean_boldness(&resolved_probs),
                    open_boldness: scoring::mean_boldness(&open_probs),
                    boldness_asmd: scoring::asmd(&resolved_probs, &open_probs),
                }
            })
        };

        let by_kind_len = by_kind.len();
        let by_kind_len_zero = by_kind_len == 0;

        ReportData {
            tag: tag_filter,
            resolved: resolved_claims.len(),
            resolved_binary: samples.len(),
            resolved_numeric: numeric_samples.len(),
            open,
            total: resolved_claims.len() + open,
            first_recorded,
            last_recorded,
            brier: scoring::brier(&samples),
            log_score: scoring::log_score(&samples, 1e-6),
            brier_skill: scoring::skill_score(&samples),
            base_rate: scoring::base_rate(&samples),
            base_rate_ci,
            reliability: decomp.map(|d| d.reliability),
            resolution: decomp.map(|d| d.resolution),
            uncertainty: decomp.map(|d| d.uncertainty),
            auc: scoring::auc(&samples),
            confidence_gap: over.map(|o| o.gap),
            mean_confidence: over.map(|o| o.mean_confidence),
            accuracy: over.map(|o| o.accuracy),
            directional_bias: scoring::directional_bias(&samples),
            calibration,
            by_tag,
            by_kind,
            mind_changing,
            numeric,
            eprocess,
            eprocess_pvalue,
            recalibration,
            brier_ci,
            recent_brier,
            distinct_forecasts,
            selective,
            weighted_brier,
            resolution_discipline,

            score_basis: "first",
            brier_first: scoring::brier(&samples),
            brier_final,
            brier_time_avg,
            late_updates,

            mcb: corp.as_ref().map(|c| c.mcb),
            dsc: corp.as_ref().map(|c| c.dsc),
            unc: corp.as_ref().map(|c| c.unc),
            mcb_null_q95,

            evidence_n: evidence.samples.len(),
            evidence_blocked_by: evidence.blocked_by.clone(),
            evidence_waiting: evidence.waiting,
            evidence_blocked_without_deadline: evidence.blocked_without_deadline,
            voided_after_resolution: evidence.voided_after_resolution,
            eprocess_log,
            kind_alarm_threshold: (!by_kind_len_zero)
                .then_some(EVIDENCE_ALARM * by_kind_len as f64),

            verdict: verdict_from(
                chrono_samples.len(),
                corp.as_ref().map(|c| c.mcb),
                mcb_null_q95,
                corp.as_ref().map(|c| c.dsc),
                eprocess,
                over.map(|o| o.gap),
                scoring::directional_bias(&samples),
                distinct_forecasts,
            ),
        }
    }

    /// True when there is nothing resolved (binary or numeric) to reflect.
    fn is_empty(&self) -> bool {
        self.resolved_binary == 0 && self.numeric.is_none()
    }
}

// ─────────────────────────── machine view ───────────────────────────────────

/// Pretty-printed JSON of the full [`ReportData`]. The agent-facing surface.
pub fn render_json(
    ledger: &Ledger,
    tag_filter: Option<&str>,
    bins: usize,
    today: NaiveDate,
) -> String {
    let data = ReportData::compute(ledger, tag_filter, bins, today);
    serde_json::to_string_pretty(&data).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

// ─────────────────────────── human view ─────────────────────────────────────

/// Place a predicted marker `P` and an observed marker `O` on a `[0,1]` lane;
/// `X` when they coincide. The visual gap between them *is* the calibration
/// error for that bin.
fn lane(pred: f64, obs: Option<f64>) -> String {
    let pos = |v: f64| -> usize { (v.clamp(0.0, 1.0) * (LANE_W as f64 - 1.0)).round() as usize };
    let mut cells = vec![b'-'; LANE_W];
    let pp = pos(pred);
    match obs {
        None => cells[pp] = b'P',
        Some(o) => {
            let op = pos(o);
            if pp == op {
                cells[pp] = b'X';
            } else {
                cells[pp] = b'P';
                cells[op] = b'O';
            }
        }
    }
    String::from_utf8(cells).unwrap()
}

/// Gloss for the confidence-gap line only.
///
/// The gap is an *aggregate* of bold-versus-right, and aggregates cancel, so this
/// describes the number on that one line and is never allowed to become a
/// verdict. It also has to agree with the verdict: naming a direction in capitals
/// while the verdict above says nothing has been demonstrated leaves the reader
/// to pick which of the two to believe.
fn gap_gloss(gap: f64, v: Verdict) -> String {
    let direction = if gap > 0.05 {
        "bolder than you are right"
    } else if gap < -0.05 {
        "less sure than you turn out to be"
    } else {
        "boldness and accuracy match on average"
    };
    if v.is_miscalibrated() {
        match () {
            _ if gap > 0.05 => format!("OVERCONFIDENT — {direction}"),
            _ if gap < -0.05 => format!("UNDERCONFIDENT — {direction}"),
            _ => format!("{direction} (the errors cancel here — see the verdict above)"),
        }
    } else if gap.abs() > 0.05 {
        format!("{direction}, but not by more than luck could manage at this sample size — see the verdict above")
    } else {
        format!("{direction} (an average: errors in opposite directions cancel here)")
    }
}

/// Render the full human-readable report.
pub fn render(ledger: &Ledger, tag_filter: Option<&str>, bins: usize, today: NaiveDate) -> String {
    let d = ReportData::compute(ledger, tag_filter, bins, today);
    let mut out = String::new();

    let title = match &d.tag {
        Some(t) => format!("ANAMNESIS — the shape of your judgement  [tag: {t}]"),
        None => "ANAMNESIS — the shape of your judgement".to_string(),
    };
    let _ = writeln!(out, "\n{title}");
    let _ = writeln!(out, "{}", "=".repeat(title.len()));

    if d.is_empty() {
        let scope = d
            .tag
            .as_ref()
            .map(|t| format!(" tagged '{t}'"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "\nNo resolved claims yet{scope}. {} still open.\n\nRecord beliefs with `ana add`, and `ana resolve` them once reality speaks.\nThe mirror needs something to reflect.",
            d.open
        );
        return out;
    }

    let split = if d.resolved_numeric > 0 && d.resolved_binary > 0 {
        format!(
            " ({} yes/no · {} numeric)",
            d.resolved_binary, d.resolved_numeric
        )
    } else {
        String::new()
    };
    let _ = writeln!(
        out,
        "\n{} of {} resolved{split}  ·  {} open  ·  first recorded {}  ·  latest {}",
        d.resolved,
        d.total,
        d.open,
        d.first_recorded.as_deref().unwrap_or("—"),
        d.last_recorded.as_deref().unwrap_or("—"),
    );

    // Resolution discipline — the honesty caveat, up top because it qualifies
    // everything below: do these numbers rest on a fair sample of your calls, or
    // only the ones you bothered to grade?
    if let Some(rd) = &d.resolution_discipline {
        let overdue_tail = if rd.overdue > 0 {
            format!("  ·  {} overdue", rd.overdue)
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "\n  Resolution discipline   {:.0}% graded ({} of {}){overdue_tail}",
            rd.resolution_rate * 100.0,
            rd.resolved,
            rd.resolved + rd.open,
        );
        if rd.overdue > 0 {
            let _ = writeln!(
                out,
                "    ⚠ {} claim(s) past due and ungraded — resolve them; until you do, the numbers below rest on a self-selected sample.",
                rd.overdue
            );
        }
        if d.voided_after_resolution > 0 {
            let _ = writeln!(
                out,
                "    ⚠ {} claim(s) were voided AFTER they had already resolved — an outcome removed from the record after it was seen. They are still counted in the evidence test below; voiding a claim you did not like is how this instrument would be defeated.",
                d.voided_after_resolution
            );
        }
        if rd.auto_graded > 0 {
            let _ = writeln!(
                out,
                "    {} of them ({:.0}%) were graded from an observed fact rather than your own word — those cannot flatter you.",
                rd.auto_graded,
                100.0 * rd.auto_graded as f64 / rd.resolved.max(1) as f64
            );
        }
        // Missing-not-at-random check: are the calls you left open a different breed
        // from the ones you graded? If so, the calibration sample is skewed.
        if let (Some(rb), Some(ob), Some(asmd)) =
            (rd.resolved_boldness, rd.open_boldness, rd.boldness_asmd)
        {
            if asmd > SELECTION_ASMD_FLAG {
                let (dir, caveat) = if ob > rb {
                    (
                        "BOLDER",
                        "the confident end, where overconfidence hides, is under-graded — the gap above may understate it",
                    )
                } else {
                    (
                        "more CAUTIOUS",
                        "your graded sample leans bold relative to what you actually predicted",
                    )
                };
                let _ = writeln!(
                    out,
                    "    Your ungraded calls are {dir} than your graded ones (boldness {ob:.2} vs {rb:.2}, ASMD {asmd:.2}) — {caveat}.",
                );
            }
        }
    }

    // Binary section -------------------------------------------------------
    if d.resolved_binary > 0 {
        // The verdict, first, before any number it is derived from — one line, the
        // same line every other surface prints.
        let _ = writeln!(
            out,
            "\n  VERDICT          {}",
            d.verdict.label(d.evidence_n).to_uppercase()
        );
        if let (Some(brier), Some(logs), Some(base)) = (d.brier, d.log_score, d.base_rate) {
            let _ = writeln!(out, "\n  Brier score      {brier:.3}   (0 = perfect · 0.25 = always 50/50 · lower better)");
            let _ = writeln!(
                out,
                "                   scored on your FIRST forecast — the belief you recorded before the answer was known"
            );
            if let Some(bf) = d.brier_final {
                if (bf - brier).abs() > 0.0005 {
                    let _ = writeln!(
                        out,
                        "                   on your final forecast it would be {bf:.3} (shown, not graded)"
                    );
                }
            }
            if let Some((lo, hi)) = d.brier_ci {
                let _ = writeln!(
                    out,
                    "                   95% bootstrap band [{lo:.3}, {hi:.3}] — how far luck alone could move it"
                );
            }
            if let Some(wb) = d.weighted_brier {
                let cmp = if wb > brier + 0.01 {
                    "WORSE on the calls that matter"
                } else if wb < brier - 0.01 {
                    "better on the calls that matter"
                } else {
                    "about the same across stakes"
                };
                let _ = writeln!(
                    out,
                    "  Stake-weighted   {wb:.3}   vs {brier:.3} flat → {cmp}"
                );
            }
            let _ = writeln!(
                out,
                "  Log score        {logs:.3}   (lower better; punishes confident misses)"
            );
            if let Some(bss) = d.brier_skill {
                let tail = if bss > 0.0 {
                    "you beat always-guess-the-base-rate"
                } else if bss < 0.0 {
                    "you did WORSE than always guessing the base rate"
                } else {
                    "no better than guessing the base rate"
                };
                let _ = writeln!(out, "  Brier skill      {bss:+.3}   ({tail})");
            }
            // Recency trend — descriptive only (small-n control charts false-alarm).
            if let Some(recent) = d.recent_brier {
                if d.resolved_binary >= TREND_MIN_N {
                    let delta = brier - recent; // > 0 ⇒ recent lower ⇒ improving
                    let lean = if delta > 0.02 {
                        "improving"
                    } else if delta < -0.02 {
                        "slipping"
                    } else {
                        "steady"
                    };
                    let _ = writeln!(
                        out,
                        "  Lately           {recent:.3}   recent Brier vs {brier:.3} lifetime → {lean}  (last ~5 weighted; directional, not significant)"
                    );
                }
            }
            let ci = d
                .base_rate_ci
                .map(|(lo, hi)| format!("   95% CI {lo:.2}–{hi:.2}"))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "  Base rate        {base:.3}   (fraction of your claims that came true){ci}"
            );
        }

        if let (Some(mcb), Some(dsc), Some(unc)) = (d.mcb, d.dsc, d.unc) {
            let _ = writeln!(
                out,
                "\n  Decomposition  (Brier = Miscalibration − Discrimination + Uncertainty)"
            );
            // The floor is what makes the calibration error readable. MCB is fitted
            // on the same data it scores, so it is never zero even for a perfect
            // forecaster; this says how large it would be for one.
            match d.mcb_null_q95 {
                Some(floor) if mcb <= floor => {
                    let _ = writeln!(
                        out,
                        "    miscalibration {mcb:.3}   calibration error      ↓ lower is better\n                   at or below the {floor:.3} a perfectly calibrated forecaster would score making these same calls — nothing to see"
                    );
                }
                Some(floor) => {
                    let _ = writeln!(
                        out,
                        "    miscalibration {mcb:.3}   calibration error      ↓ lower is better\n                   above the {floor:.3} luck alone would produce on these same calls"
                    );
                }
                None => {
                    let _ = writeln!(
                        out,
                        "    miscalibration {mcb:.3}   calibration error      ↓ lower is better"
                    );
                }
            }
            let dsc_note = if dsc.abs() < 1e-9 {
                "   — your confidence carries NO information about which calls come true"
            } else {
                ""
            };
            let _ = writeln!(
                out,
                "    discrimination {dsc:.3}   sorting power          ↑ higher is better{dsc_note}"
            );
            let _ = writeln!(
                out,
                "    uncertainty    {unc:.3}   irreducible difficulty of your questions"
            );
            let _ = writeln!(out, "    check          {mcb:.3} − {dsc:.3} + {unc:.3} = {:.3}  (= Brier, exactly — no bins, no tuning)", mcb - dsc + unc);
            if let (Some(rel), Some(res)) = (d.reliability, d.resolution) {
                let _ = writeln!(
                    out,
                    "    (the older exact-value grouping reads {rel:.3} / {res:.3}; it counts small groups as error and is kept only as a deprecated alias)"
                );
            }
        }

        match d.auc {
            Some(a) => {
                let _ = writeln!(out, "\n  Discrimination   AUC {a:.3}   (0.5 = can't tell true from false · 1.0 = perfect)");
            }
            None => {
                let _ = writeln!(
                    out,
                    "\n  Discrimination   n/a   (every claim resolved the same way)"
                );
            }
        }

        if let (Some(gap), Some(conf), Some(acc)) =
            (d.confidence_gap, d.mean_confidence, d.accuracy)
        {
            // The gloss is subordinate to the verdict. This line used to shout
            // "OVERCONFIDENT" from a gap of +0.12 while the verdict above said no
            // miscalibration had been demonstrated — the reader is left to guess
            // which of the two to believe, which is the whole defect P0-5 fixed.
            let _ = writeln!(
                out,
                "\n  Confidence gap   {gap:+.3}   {}",
                gap_gloss(gap, d.verdict)
            );
            let _ = writeln!(
                out,
                "                   mean boldness {conf:.3}  vs  accuracy {acc:.3}"
            );
            if let Some(bias) = d.directional_bias {
                let dir = if bias > 0.0 {
                    "toward YES"
                } else {
                    "toward NO"
                };
                let _ = writeln!(
                    out,
                    "                   directional bias {bias:+.3} ({dir})"
                );
            }
        }

        // Confidence vocabulary — a handful of distinct values caps your resolution.
        if d.distinct_forecasts > 0 {
            let _ = writeln!(
                out,
                "\n  Confidence vocab {:>4} distinct level(s) across {} call(s){}",
                d.distinct_forecasts,
                d.resolved_binary,
                if d.distinct_forecasts <= 3 && d.resolved_binary >= 6 {
                    " — coarse; more gradations would sharpen you"
                } else {
                    ""
                }
            );
        }

        // Selective prediction — error among your surest calls vs all (when to act).
        if let Some(sel) = &d.selective {
            if d.resolved_binary >= 6 {
                let verdict = if sel.risk_half < sel.risk_full - 0.05 {
                    "your confidence ranks your calls — trust the bold ones"
                } else if sel.risk_half > sel.risk_full + 0.05 {
                    "INVERTED — your most-confident calls are your worst"
                } else {
                    "confidence barely separates winners from losers"
                };
                let _ = writeln!(
                    out,
                    "\n  Selective        act on all {:.0}% error · surest half {:.0}% → {verdict}",
                    sel.risk_full * 100.0,
                    sel.risk_half * 100.0
                );
            }
        }

        // Anytime-valid significance: is the miscalibration above real, or noise?
        // Three states, each said plainly. "Too few resolutions to tell" printed
        // at n = 1000 read as a bug, because it was one.
        if let Some(e) = d.eprocess {
            let p = d.eprocess_pvalue.unwrap_or(1.0);
            let n = d.evidence_n;
            let shown = if d
                .eprocess_log
                .is_some_and(|l| l >= scoring::EPROCESS_LOG_CAP)
            {
                "> 10^12".to_string()
            } else {
                format!("{e:.1}")
            };
            let line = if e >= EVIDENCE_ALARM {
                "miscalibration is REAL — significant even though you peek every session (α=.05)"
                    .to_string()
            } else if n < VERDICT_MIN_N {
                format!(
                    "not enough graded calls yet — {n} in the test, about {} more to reach a usable {VERDICT_MIN_N}",
                    VERDICT_MIN_N.saturating_sub(n)
                )
            } else if e >= RECAL_MIN_E {
                "suggestive, not yet conclusive — keep logging".to_string()
            } else {
                "no evidence of miscalibration in {n} graded calls — the test can still miss patterns; read the calibration error above for the SIZE of any error"
                    .replace("{n}", &n.to_string())
            };
            let _ = writeln!(
                out,
                "\n  Is it real?      e-value {shown:>8}   (anytime-valid p ≤ {p:.3})"
            );
            let _ = writeln!(out, "                   {line}");
        } else if d.resolved_binary > 0 {
            let _ = writeln!(out, "\n  Is it real?      no evidence sequence yet");
        }

        // What the evidence test could and could not use. The test needs an order
        // fixed before the outcomes were known, and `--by` is what fixes it.
        if d.resolved_binary > 0 {
            if let Some(id) = &d.evidence_blocked_by {
                let fix = if d.evidence_blocked_without_deadline {
                    "resolve it, or give it a --by date so it sorts later"
                } else {
                    "resolve it, or void it if it was never answerable"
                };
                if d.evidence_waiting > 0 {
                    let _ = writeln!(
                        out,
                        "                   evidence uses {} of {} graded calls: it stops at [{id}], which is due and ungraded — {fix}.",
                        d.evidence_n, d.resolved_binary
                    );
                    let _ = writeln!(
                        out,
                        "                   the {} graded call(s) after it are waiting, not lost — skipping ahead is what would let the record be picked after the fact",
                        d.evidence_waiting
                    );
                } else {
                    let _ = writeln!(
                        out,
                        "                   [{id}] is due and ungraded; {fix} — until then the test cannot grow past {} call(s)",
                        d.evidence_n
                    );
                }
            }
        }

        // The learned correction — only once it is both trustworthy (n) and worth
        // mentioning (meaningfully off the identity).
        if let Some(r) = &d.recalibration {
            // Only offer a correction once the e-process finds at least suggestive
            // evidence — never invite acting on a map fit to noise.
            let has_evidence = d.eprocess.is_some_and(|e| e >= RECAL_MIN_E);
            if has_evidence && r.n >= RECAL_MIN_N && ((r.b - 1.0).abs() > 0.10 || r.a.abs() > 0.10)
            {
                // The Cox calibration slope (b) and intercept (a), read aloud:
                // b<1 ⇒ forecasts too extreme; the intercept is a level-independent
                // bias — a>0 means your stated numbers should be higher (you
                // under-state), a<0 lower (you over-state).
                let shape = if r.b < 1.0 {
                    "you run too extreme"
                } else {
                    "you are too timid"
                };
                let bias = if r.a > 0.10 {
                    "; under-stating on average"
                } else if r.a < -0.10 {
                    "; over-stating on average"
                } else {
                    ""
                };
                let _ = writeln!(
                    out,
                    "\n  Recalibration    stated → what it should be   (Cox slope b={:.2}: {shape}{bias})",
                    r.b
                );
                let cells: Vec<String> = r
                    .examples
                    .iter()
                    .map(|(p, q)| format!("{p:.2}→{q:.2}"))
                    .collect();
                let _ = writeln!(out, "    {}", cells.join("    "));
            }
        }

        let _ = writeln!(
            out,
            "\n  Reliability diagram   P = your avg forecast · O = what actually happened"
        );
        let _ = writeln!(
            out,
            "    range        n    0{}1",
            " ".repeat(LANE_W.saturating_sub(2))
        );
        for b in &d.calibration {
            if b.count == 0 {
                continue;
            }
            let mean_pred = b.mean_pred.unwrap_or((b.lo + b.hi) / 2.0);
            let tail = match b.observed {
                None => String::new(),
                Some(obs) => {
                    let gap = mean_pred - obs;
                    let mark = if gap.abs() < 0.05 {
                        "ok"
                    } else if gap > 0.0 {
                        "over"
                    } else {
                        "under"
                    };
                    format!("  pred {mean_pred:.2} → obs {obs:.2}  {mark}")
                }
            };
            let _ = writeln!(
                out,
                "    {:.2}-{:.2}  {:>4}   |{}|{}",
                b.lo,
                b.hi,
                b.count,
                lane(mean_pred, b.observed),
                tail
            );
        }

        if !d.by_tag.is_empty() {
            let _ = writeln!(out, "\n  By domain");
            let _ = writeln!(
                out,
                "    {:<14} {:>4}  {:>7}  {:>9}",
                "tag", "n", "brier", "conf-gap"
            );
            for t in &d.by_tag {
                let br = t
                    .brier
                    .map(|x| format!("{x:.3}"))
                    .unwrap_or_else(|| "  —".into());
                let g = t
                    .confidence_gap
                    .map(|x| format!("{x:+.3}"))
                    .unwrap_or_else(|| "   —".into());
                let _ = writeln!(out, "    {:<14} {:>4}  {:>7}  {:>9}", t.tag, t.n, br, g);
            }
        }

        if !d.by_kind.is_empty() {
            let _ = writeln!(
                out,
                "\n  By prediction kind   (gap~ = shrunk toward your overall rate; trust it at small n)"
            );
            let _ = writeln!(
                out,
                "    {:<16} {:>4}  {:>7}  {:>8}  {:>8}",
                "kind", "n", "brier", "gap", "gap~"
            );
            for t in &d.by_kind {
                let br = t
                    .brier
                    .map(|x| format!("{x:.3}"))
                    .unwrap_or_else(|| "  —".into());
                let g = t
                    .confidence_gap
                    .map(|x| format!("{x:+.3}"))
                    .unwrap_or_else(|| "   —".into());
                let gs = t
                    .confidence_gap_shrunk
                    .map(|x| format!("{x:+.3}"))
                    .unwrap_or_else(|| "   —".into());
                let _ = writeln!(
                    out,
                    "    {:<16} {:>4}  {:>7}  {:>8}  {:>8}",
                    t.tag, t.n, br, g, gs
                );
            }
            // Multicalibration verdict. The per-kind e-process is anytime-valid, so
            // the worst kind that clears the evidence bar is *genuinely* miscalibrated
            // — not the small-subgroup fluke that derails a raw worst-group gap. Stay
            // silent otherwise; the overall "Is it real?" line already covers that.
            // Searching K subgroups for the worst one is K tests, not one. Ville
            // plus a union bound says a single row has to clear
            // `EVIDENCE_ALARM × K` before it is a finding; below that the
            // per-kind numbers are descriptive only, and are labelled as such.
            let k_threshold = d.kind_alarm_threshold.unwrap_or(EVIDENCE_ALARM);
            if let Some((t, e)) = d
                .by_kind
                .iter()
                .filter_map(|t| t.eprocess.map(|e| (t, e)))
                .filter(|&(_, e)| e >= k_threshold)
                .max_by(|a, b| a.1.total_cmp(&b.1))
            {
                let dir = match t.confidence_gap {
                    Some(g) if g > 0.0 => "overconfident",
                    Some(_) => "underconfident",
                    None => "miscalibrated",
                };
                let _ = writeln!(
                    out,
                    "    → '{}' is really {dir} (e={:.0}) — trust your '{}' calls least.",
                    t.tag, e, t.tag
                );
            }
        }

        if let Some(mc) = &d.mind_changing {
            let _ = writeln!(
                out,
                "\n  Mind-changing    {} claim(s) you revised",
                mc.revised
            );
            let _ = writeln!(
                out,
                "    Brier of first guess {:.3}  (GRADED)  →  Brier of final guess {:.3}  (not graded)   ({:+.3})",
                mc.brier_first, mc.brier_last, mc.improvement
            );
            // Deliberately neutral. Praising a movement toward the truth rewards
            // the one thing this tool exists to catch: ten claims logged at 0.5,
            // updated to 0.99 and then resolved YES used to score a Brier of
            // 0.000 and be congratulated for changing their mind well. Moving
            // toward the answer is only a virtue if you did not already know it,
            // and the ledger cannot tell the difference — so it does not judge.
            let _ = writeln!(
                out,
                "    Only the first figure is scored. A revision may be genuine learning or may be hindsight; the ledger cannot tell, so it grades the belief you recorded before the answer was known."
            );
            if d.late_updates > 0 {
                let _ = writeln!(
                    out,
                    "    {} claim(s) were revised after their due date or within a day of the answer — those revisions carry no weight anywhere.",
                    d.late_updates
                );
            }
            if let Some(ta) = d.brier_time_avg {
                let _ = writeln!(
                    out,
                    "    Time-averaged Brier {ta:.3} — each forecast weighted by how long it stood, the way Metaculus rewards updating early (secondary: it can still be farmed by updating the moment you know)."
                );
            }
        }
    }

    // Numeric section ------------------------------------------------------
    if let Some(num) = &d.numeric {
        let _ = writeln!(
            out,
            "\n  Numeric forecasts   {} resolved interval(s)",
            num.count
        );
        let _ = writeln!(
            out,
            "    interval score       {}\n                         (1.00 = the value landed inside; above that is the miss penalty, in multiples of your own stated width)",
            match (num.mean_winkler_ratio, num.median_winkler_ratio) {
                (Some(mean), Some(med)) => format!("mean {mean:.2} · median {med:.2}"),
                _ => format!("{:.3} raw Winkler (units vary)", num.mean_winkler),
            }
        );
        let _ = writeln!(out, "    mean interval width  {:.3}", num.mean_width);
        let _ = writeln!(
            out,
            "    coverage             {:.0}% actual  vs  {:.0}% intended   ({:+.0} pts)",
            num.empirical_coverage * 100.0,
            num.nominal_coverage * 100.0,
            num.coverage_gap * 100.0
        );
        let line = if num.coverage_gap < -0.05 {
            "    Your intervals are TOO NARROW — overconfident about numbers, just like probabilities."
        } else if num.coverage_gap > 0.05 {
            "    Your intervals are too WIDE — you could afford to be sharper."
        } else {
            "    Your intervals are about the right width. Well judged."
        };
        let _ = writeln!(out, "{line}");
        // The conformal width correction — gated on the coverage e-process finding
        // real evidence (mirrors the binary recalibration gate), so the agent is
        // never told to resize intervals on the strength of a couple of misses.
        if let (Some(m), Some(e)) = (num.width_factor, num.coverage_eprocess) {
            if e >= RECAL_MIN_E && (m - 1.0).abs() > 0.10 {
                let verb = if m > 1.0 { "WIDEN" } else { "SHARPEN" };
                let _ = writeln!(
                    out,
                    "    Recalibration: {verb} — multiply your interval half-widths by {m:.2} (coverage e={e:.0})."
                );
            }
        }
    }

    let _ = writeln!(
        out,
        "\n  \"The first principle is that you must not fool yourself —\n   and you are the easiest person to fool.\"  — R. Feynman\n"
    );
    out
}

// ──────────────────── plain-English view (text + HTML card) ──────────────────
//
// Two more renderings of the same `ReportData`: the numbers translated into what
// they *mean* for a person who shouldn't need to know what a Brier score is. The
// prose is built once in `plain_summary`, so the text view (`render_plain`) and
// the offline HTML card (`render_html`) can never drift apart — the same
// compute-once-render-many discipline as the rest of this file.

/// Whole-percent string: `0.833 → "83%"`.
fn pct(x: f64) -> String {
    format!("{:.0}%", (x * 100.0).round())
}

/// Word-wrap `text` to `width` columns, prefixing every line with `indent`.
fn wrap(text: &str, width: usize, indent: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
        .into_iter()
        .map(|l| format!("{indent}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Escape the HTML-significant characters (the report's prose is ours, but a tag
/// name could in principle reach the card, so be safe).
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// One plain-language insight: a plain question, a plain answer, and the technical
/// term it maps to — named, not hidden, so the plain view is a *bridge* to the
/// rich report rather than a dumbing-down.
struct Insight {
    label: &'static str,
    answer: String,
    jargon: &'static str,
}

/// The whole report distilled to plain language — the single source of prose for
/// both the text view and the HTML card.
struct PlainSummary {
    headline: String,
    verdict_word: String,
    /// Severity/colour hint: `"good" | "over" | "under" | "unknown"`.
    verdict_class: &'static str,
    insights: Vec<Insight>,
    actions: Vec<String>,
    footer: String,
}

fn plain_summary(d: &ReportData) -> PlainSummary {
    let n = d.resolved_binary;
    let mut insights: Vec<Insight> = Vec::new();

    // — Honest sample? (resolution discipline) —
    if let Some(rd) = &d.resolution_discipline {
        let mut a = if rd.open == 0 {
            format!("Yes — you've graded every one of your {} call(s). This isn't a picture cherry-picked from only the predictions that went your way.", rd.resolved)
        } else {
            format!(
                "You've graded {} of {} call(s) ({}). The {} still open aren't in the numbers below.",
                rd.resolved,
                rd.resolved + rd.open,
                pct(rd.resolution_rate),
                rd.open
            )
        };
        if rd.overdue > 0 {
            a.push_str(&format!(" Watch out: {} of those are past their due date — resolve them, or these scores quietly flatter you.", rd.overdue));
        }
        if let (Some(rb), Some(ob), Some(asmd)) =
            (rd.resolved_boldness, rd.open_boldness, rd.boldness_asmd)
        {
            if asmd > SELECTION_ASMD_FLAG {
                let dir = if ob > rb { "bolder" } else { "more cautious" };
                a.push_str(&format!(
                    " And the calls you left open are {dir} than the ones you graded, so the split isn't random."
                ));
            }
        }
        insights.push(Insight {
            label: "Is this an honest sample of your calls?",
            answer: a,
            jargon: "resolution discipline / selection bias",
        });
    }

    // — Sure vs right? (calibration) —
    //
    // Word and class come from `d.verdict` and nowhere else. They used to be read
    // off the confidence gap, in which over- and under-confidence cancel: a
    // ledger of 50 calls at 0.9 on coin flips plus 50 at 0.6 on sure things has a
    // gap of roughly −1e-15, and was therefore called "Well calibrated" while its
    // Brier skill was −0.520.
    let (verdict_word, verdict_class): (String, &'static str) = match d.verdict {
        Verdict::InsufficientData => ("Not enough data".into(), "unknown"),
        Verdict::Overconfident => ("Overconfident".into(), "over"),
        Verdict::Underconfident => ("Underconfident".into(), "under"),
        Verdict::BiasedYes => ("Leans yes".into(), "over"),
        Verdict::BiasedNo => ("Leans no".into(), "under"),
        Verdict::CalibratedButUninformative => ("Uninformative".into(), "uninformative"),
        Verdict::MiscalibratedBothWays => ("Miscalibrated".into(), "both"),
        Verdict::NoEvidenceOfMiscalibration if n >= VERDICT_CONFIDENT_N => {
            ("Well calibrated".into(), "good")
        }
        Verdict::NoEvidenceOfMiscalibration => ("No miscalibration found".into(), "good"),
    };
    if let (Some(conf), Some(acc)) = (d.mean_confidence, d.accuracy) {
        let core = format!(
            "On the calls you felt about {} sure of, you turned out right about {} of the time.",
            pct(conf),
            pct(acc)
        );
        let lesson = match d.verdict {
            Verdict::Overconfident => "You OVERSELL yourself — you sound more certain than you turn out to be, so shade your confidence down.",
            Verdict::Underconfident => "You UNDERSELL yourself — reality rewards you more than you claim, so when you feel fairly sure you can trust it more.",
            Verdict::BiasedYes => "You lean toward YES — you say things will happen more often than they do.",
            Verdict::BiasedNo => "You lean toward NO — you say things won't happen more often than turns out to be true.",
            Verdict::CalibratedButUninformative => "Your confidence does not separate the calls that come true from the ones that don't — after correcting for level, every forecast you make carries the same information. That is the thing to fix first; how high or low you pitch the numbers barely matters until the ranking works.",
            Verdict::InsufficientData => "Too few graded calls to say whether that's luck or judgement.",
            Verdict::MiscalibratedBothWays => "Your errors run BOTH ways — too sure at one end of your range and not sure enough at the other. They cancel in the average, so there is no single direction to shade: look at the reliability diagram to see where each half goes wrong.",
            Verdict::NoEvidenceOfMiscalibration => "Nothing in the record shows your confidence is off. That is not the same as proof it's right — the test can miss patterns, and a small record hides a lot.",
        };
        insights.push(Insight {
            label: "When you say you're sure, should you be?",
            answer: format!("{core} {lesson}"),
            jargon: "calibration verdict (CORP miscalibration + sequential evidence test)",
        });
    }

    // — Discrimination —
    let disc = match d.auc {
        None => "Everything you've logged so far turned out the same way, so there's nothing to sort yet.".to_string(),
        Some(_) if n < 12 => format!("Can't tell from {n} call(s) yet — sorting your good calls from your bad ones needs more answers before it means anything."),
        Some(a) if a >= 0.70 => "Yes — the calls you're more confident about come true noticeably more often than the ones you're unsure of.".to_string(),
        Some(a) if a >= 0.55 => "Somewhat — your confidence points the right way, but not sharply.".to_string(),
        Some(_) => "Not really — your confidence isn't tracking which calls actually come true. Of everything here, that's the one most worth fixing.".to_string(),
    };
    insights.push(Insight {
        label: "Can you tell your right calls from your wrong ones?",
        answer: disc,
        jargon: "discrimination (AUC)",
    });

    // — Evidence (is the pattern real?) —
    if let Some(e) = d.eprocess {
        let en = d.evidence_n;
        let ev = if e >= EVIDENCE_ALARM {
            "Yes, it's real. Even though you check this every session, the pattern above is statistically solid — you can act on it.".to_string()
        } else if en < VERDICT_MIN_N {
            format!("Not yet answerable. Only {en} of your calls are in the test, and about {} more are needed before it can say anything either way.", VERDICT_MIN_N.saturating_sub(en))
        } else if e >= RECAL_MIN_E {
            "Maybe — the signs are suggestive but not yet conclusive. Keep logging.".to_string()
        } else {
            format!("No. Across {en} graded calls the test found no miscalibration. That is not proof you are calibrated — the test can miss patterns it isn't looking for — but nothing here is demonstrably off.")
        };
        insights.push(Insight {
            label: "Is any of this real, or a small-sample fluke?",
            answer: ev,
            jargon: "anytime-valid e-value",
        });
    }

    // — Headline score (Brier) —
    if let Some(b) = d.brier {
        let band = d
            .brier_ci
            .map(|(lo, hi)| {
                format!(
                    " Luck alone could place the true figure anywhere between {lo:.2} and {hi:.2}."
                )
            })
            .unwrap_or_default();
        let quality = if b < 0.10 {
            "That's sharp."
        } else if b < 0.25 {
            "Better than a coin flip."
        } else {
            "That's around coin-flip territory."
        };
        insights.push(Insight {
            label: "How good are your predictions overall?",
            answer: format!(
                "{b:.2} on the forecasting \"golf score\": 0 is a perfect prophet, 0.25 is a coin flip, lower is better. {quality}{band}"
            ),
            jargon: "Brier score",
        });
    }

    // — Numeric ranges (only if you've made any) —
    if let Some(num) = &d.numeric {
        let a = if num.coverage_gap < -0.05 {
            format!("Your number ranges are TOO NARROW — meant to contain the true value {} of the time, they only did {} of the time. You're overconfident about numbers, just like probabilities.", pct(num.nominal_coverage), pct(num.empirical_coverage))
        } else if num.coverage_gap > 0.05 {
            format!("Your number ranges are too WIDE — they caught the value {} of the time versus the {} you aimed for, so you can afford to be sharper.", pct(num.empirical_coverage), pct(num.nominal_coverage))
        } else {
            "Your number ranges are about the right width — well judged.".to_string()
        };
        insights.push(Insight {
            label: "And your number ranges?",
            answer: a,
            jargon: "interval coverage",
        });
    }

    // — What to do — tailored to the verdict, then the earned correction, then n. —
    let mut actions: Vec<String> = Vec::new();
    match verdict_class {
        "under" => actions
            .push("Lean in a little when you feel sure — you tend to undersell yourself.".into()),
        "over" => {
            actions.push("Add slack and shade your confidence down — you tend to oversell.".into())
        }
        "both" => actions.push(
            "Your errors cancel: shading everything one way will not help. Compare your high-confidence and low-confidence calls separately — the reliability diagram shows which end is wrong.".into(),
        ),
        "uninformative" => actions.push(
            "Spread your probabilities out — right now they don't distinguish the calls that come true from the ones that don't.".into(),
        ),
        "good" if n >= VERDICT_CONFIDENT_N => actions
            .push("Nothing here needs correcting — keep logging so it stays that way.".into()),
        "good" => actions.push(
            "Nothing is demonstrably off yet. Keep logging: absence of evidence at this sample size is not evidence of calibration.".into(),
        ),
        _ => {}
    }
    let earned = d.eprocess.is_some_and(|e| e >= RECAL_MIN_E)
        && d.recalibration
            .as_ref()
            .is_some_and(|r| r.n >= RECAL_MIN_N && ((r.b - 1.0).abs() > 0.10 || r.a.abs() > 0.10));
    if earned {
        if let Some((p, q)) = d
            .recalibration
            .as_ref()
            .and_then(|r| r.examples.iter().find(|(p, _)| (*p - 0.7).abs() < 1e-9))
        {
            actions.push(format!(
                "A correction has earned its place: when you'd say {}, log about {} instead.",
                pct(*p),
                pct(*q)
            ));
        }
    }
    if n < 12 {
        actions.push(format!(
            "Don't over-read {n} prediction(s) — keep logging and the picture sharpens."
        ));
    }
    actions.push("Before a costly or irreversible call, run `ana decide` to turn your confidence into a clear proceed / verify / abstain.".into());

    let headline = match d.verdict {
        Verdict::Underconfident => {
            "You undersell yourself — trust your sure calls more.".to_string()
        }
        Verdict::Overconfident => "You oversell yourself — add slack and shade down.".to_string(),
        Verdict::BiasedYes => {
            "You lean toward yes — you predict things happen more than they do.".to_string()
        }
        Verdict::BiasedNo => {
            "You lean toward no — you predict things don't happen more than holds up.".to_string()
        }
        Verdict::CalibratedButUninformative => {
            "Right on average, but your confidence doesn't separate the good calls from the bad."
                .to_string()
        }
        Verdict::MiscalibratedBothWays => {
            "Miscalibrated in both directions at once — the errors cancel in the average."
                .to_string()
        }
        Verdict::NoEvidenceOfMiscalibration if n >= VERDICT_CONFIDENT_N => {
            "Your confidence is honest — well calibrated.".to_string()
        }
        Verdict::NoEvidenceOfMiscalibration => {
            "Nothing shows your confidence is off — but this is still a small record.".to_string()
        }
        Verdict::InsufficientData => {
            "Too few resolved calls to read your calibration yet.".to_string()
        }
    };
    let evidence_tag = match d.eprocess {
        Some(e) if e >= EVIDENCE_ALARM => "evidence: solid",
        Some(e) if e >= RECAL_MIN_E => "evidence: suggestive",
        _ if d.evidence_n < VERDICT_MIN_N => "evidence: too few to tell",
        _ => "evidence: no miscalibration found",
    };
    let footer = format!(
        "{} resolved · {} · offline & local — nothing left your machine",
        d.resolved_binary, evidence_tag
    );

    PlainSummary {
        headline,
        verdict_word,
        verdict_class,
        insights,
        actions,
        footer,
    }
}

// A calibration cat whose face is the state of your judgement. Its mood is chosen
// by a 0–100 calibration score — `|confidence gap|` folded into four 25-point bands
// — so the agent's standing reads at a glance before a single number is parsed.
// Pure ASCII faces, no emoji: emoji width is font-dependent and misaligns or boxes
// in many terminals (and looks broken inside a Markdown code block). The mood NAME
// and gloss carry the meaning; the emoji legend lives only in prose docs.
const CAT_DIALED: &str = "   /\\_/\\\n  ( ^.^ ) <3\n   > ^ <";
const CAT_CLOSE: &str = "   /\\_/\\\n  ( o.o )\n   > . <";
const CAT_DRIFT: &str = "   /\\_/\\\n  ( ;_; )\n   > _ <";
const CAT_OFF: &str = "   /\\_/\\\n  ( @_@ )\n   >< ><";
const CAT_SLEEPY: &str = "   /\\_/\\\n  ( -.- )  zzz\n   > ~ <";

struct Mood {
    art: &'static str,
    name: &'static str,
    gloss: String,
}

/// A calibration error this far above its own noise floor scores 0/100. Roughly
/// the miscalibration of saying 90% to coin flips.
const MOOD_WORST_EXCESS: f64 = 0.10;

/// Pick the cat's mood from the **size of the calibration error**, measured
/// against the noise floor for a record of this size and shape, and from
/// [`ReportData::verdict`] for the wording.
///
/// It used to key off `|confidence gap|`, which is why a ledger with a 0.160
/// calibration error and a −0.520 skill score displayed `[DIALED IN] calibration
/// 100/100 · well calibrated`: its overconfident half and its underconfident half
/// cancelled to a gap of −1e-15. A cat is the most screenshot-able thing this
/// program produces, so it is the last place a cancelling statistic belongs.
fn mood(d: &ReportData) -> Mood {
    let excess = match (d.mcb, d.mcb_null_q95) {
        (Some(m), Some(f)) => Some((m - f).max(0.0)),
        (Some(m), None) => Some(m.max(0.0)),
        _ => None,
    };
    let score = if d.verdict == Verdict::InsufficientData {
        None
    } else {
        excess.map(|e| (100.0 * (1.0 - (e / MOOD_WORST_EXCESS).min(1.0))).round() as u32)
    };
    let dir = match d.verdict {
        Verdict::InsufficientData => "not enough graded calls to judge yet",
        Verdict::Overconfident => "overconfident — you oversell",
        Verdict::Underconfident => "underconfident — you undersell",
        Verdict::BiasedYes => "leans yes — you over-predict that things happen",
        Verdict::BiasedNo => "leans no — you over-predict that things don't",
        Verdict::CalibratedButUninformative => "your confidence sorts nothing — fix that first",
        Verdict::NoEvidenceOfMiscalibration => "no miscalibration found",
        Verdict::MiscalibratedBothWays => {
            "miscalibrated both ways — the errors cancel in the average"
        }
    };
    // The face always follows the size of the calibration error; only the NAME
    // changes when the forecasts sort nothing. An inverted forecaster — 0.95s
    // that never happen, 0.05s that always do — is uninformative AND badly
    // miscalibrated, and should not get a gentle face just because the headline
    // finding is about ranking.
    let (art, band) = match score {
        None => (CAT_SLEEPY, "WARMING UP"),
        // "DIALED IN" is the same claim as "well calibrated", so it answers to
        // the same evidence bar: absence of evidence at 24 graded calls is not
        // evidence of calibration, and the friendliest face in the program should
        // not say otherwise.
        Some(s) if s >= 75 && d.evidence_n >= VERDICT_CONFIDENT_N => (CAT_DIALED, "DIALED IN"),
        Some(s) if s >= 75 => (CAT_CLOSE, "NOTHING OFF YET"),
        Some(s) if s >= 50 => (CAT_CLOSE, "CLOSE"),
        Some(s) if s >= 25 => (CAT_DRIFT, "DRIFTING"),
        Some(_) => (CAT_OFF, "WAY OFF"),
    };
    let (art, name) = match d.verdict {
        Verdict::InsufficientData => (CAT_SLEEPY, "WARMING UP"),
        Verdict::CalibratedButUninformative => (art, "UNINFORMATIVE"),
        _ => (art, band),
    };
    let gloss = match score {
        Some(s) => format!("calibration {s}/100 · {dir}"),
        None => dir.to_string(),
    };
    Mood { art, name, gloss }
}

/// The report in plain English — for a developer skimming, or a newcomer who has
/// never heard of a Brier score. A rendering of the same `ReportData`; it computes
/// nothing.
pub fn render_plain(
    ledger: &Ledger,
    tag_filter: Option<&str>,
    bins: usize,
    today: NaiveDate,
) -> String {
    let d = ReportData::compute(ledger, tag_filter, bins, today);
    let mut out = String::new();
    let scope = d
        .tag
        .as_deref()
        .map(|t| format!("  [tag: {t}]"))
        .unwrap_or_default();
    let title = format!("ANAMNESIS — your judgement, in plain English{scope}");
    let _ = writeln!(out, "\n{title}");
    let _ = writeln!(out, "{}", "=".repeat(title.chars().count()));

    if d.is_empty() {
        let scope = d
            .tag
            .as_ref()
            .map(|t| format!(" tagged '{t}'"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "\nNothing to reflect yet{scope}. You have {} open prediction(s) but none resolved.\nLog a belief with `ana add`, then `ana resolve` it once you find out — the\nmirror needs something to reflect.\n",
            d.open
        );
        return out;
    }

    let m = mood(&d);
    let _ = writeln!(out, "\n{}", m.art);
    let _ = writeln!(out, "  [{}]  {}", m.name, m.gloss);

    let s = plain_summary(&d);
    let _ = writeln!(out, "\n{}", s.headline);
    let _ = writeln!(
        out,
        "Based on {} resolved prediction(s).",
        d.resolved_binary
    );
    for ins in &s.insights {
        let _ = writeln!(out, "\n{}", ins.label);
        let _ = writeln!(out, "{}", wrap(&ins.answer, 74, "  "));
        let _ = writeln!(out, "  ↳ technical name: {}", ins.jargon);
    }
    let _ = writeln!(out, "\nSo what should you do?");
    for act in &s.actions {
        let _ = writeln!(out, "{}", wrap(act, 72, "    ").replacen("    ", "  • ", 1));
    }
    let _ = writeln!(out, "\n{}\n", s.footer);
    out
}

/// The editorial CSS for the calibration card — verbatim from the Claude Design
/// handoff (`Calibration Card.template.html`), with the single severity-driven
/// `--accent-seed` left as `__ACCENT__` for substitution. Inline only: no fonts,
/// scripts, or network; light + dark via `prefers-color-scheme`.
const CARD_CSS: &str = r#"  /* ============================================================
     ANAMNESIS · CALIBRATION CARD
     One file. Inline CSS only. No fonts, no scripts, no network.
     Opens offline by double-click. A mirror, not a dashboard.
     ============================================================ */

  :root {
    color-scheme: light dark;

    /* --- the one value driven by the verdict's severity --- */
    --accent-seed: __ACCENT__;         /* e.g. #1565c0 */
    --accent: var(--accent-seed);

    /* --- light palette (paper & ink) --- */
    --paper:    #efece4;   /* page behind the card */
    --card:     #fbfaf6;   /* the card surface     */
    --ink:      #1d1b16;   /* primary text         */
    --ink-soft: #3f3c34;   /* answers, body        */
    --muted:    #75716a;   /* captions, labels     */
    --faint:    #a39e93;   /* technical terms      */
    --hairline: #e2ddd2;   /* rules & ticks        */
    --shadow:   0 1px 2px rgba(40,34,20,.05), 0 18px 50px -28px rgba(40,34,20,.30);

    /* --- type --- */
    --serif: "Iowan Old Style", "Palatino Linotype", Palatino, "Book Antiqua", Georgia, "Times New Roman", serif;
    --sans: system-ui, -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    --mono: ui-monospace, "SF Mono", "JetBrains Mono", Menlo, Consolas, "Liberation Mono", monospace;
  }

  @media (prefers-color-scheme: dark) {
    :root {
      --paper:    #0f1012;
      --card:     #16181b;
      --ink:      #eae6dd;
      --ink-soft: #c4c0b6;
      --muted:    #8f8b81;
      --faint:    #6a665e;
      --hairline: #2a2c30;
      --shadow:   0 1px 2px rgba(0,0,0,.4), 0 24px 60px -30px rgba(0,0,0,.7);
      /* lift the accent off the dark ground so it reads as light */
      --accent: color-mix(in oklab, var(--accent-seed), white 30%);
    }
  }

  * { box-sizing: border-box; }

  html, body { margin: 0; }

  body {
    background: var(--paper);
    color: var(--ink);
    font-family: var(--sans);
    font-size: 16px;
    line-height: 1.5;
    -webkit-font-smoothing: antialiased;
    text-rendering: optimizeLegibility;
    min-height: 100vh;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: clamp(16px, 5vw, 64px);
  }

  .card {
    width: 100%;
    max-width: 660px;
    background: var(--card);
    border: 1px solid var(--hairline);
    border-radius: 7px;
    box-shadow: var(--shadow);
    overflow: hidden;
    position: relative;
  }

  /* the severity colour as a quiet spine across the top edge */
  .card::before {
    content: "";
    position: absolute;
    inset: 0 0 auto 0;
    height: 3px;
    background: var(--accent);
    opacity: .9;
  }

  .inner {
    padding: clamp(30px, 6.5vw, 60px) clamp(26px, 6vw, 60px) clamp(28px, 5vw, 48px);
  }

  /* ---------- masthead ---------- */
  .masthead {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: clamp(30px, 6vw, 52px);
  }
  .wordmark {
    font-family: var(--serif);
    font-size: 16px;
    letter-spacing: .02em;
    color: var(--ink);
  }
  .wordmark .dot { color: var(--accent); }
  .kicker {
    font-family: var(--sans);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: .22em;
    text-transform: uppercase;
    color: var(--muted);
  }

  /* ---------- verdict + headline ---------- */
  .verdict {
    font-family: var(--serif);
    font-weight: 600;
    color: var(--accent);
    font-size: clamp(40px, 9.5vw, 66px);
    line-height: .98;
    letter-spacing: -.018em;
    margin: 0;
  }
  .headline {
    font-family: var(--serif);
    font-weight: 400;
    color: var(--ink);
    font-size: clamp(20px, 4.1vw, 27px);
    line-height: 1.34;
    letter-spacing: -.005em;
    max-width: 30ch;
    text-wrap: balance;
    margin: clamp(18px, 3.5vw, 26px) 0 0;
  }

  /* ---------- a hairline rule ---------- */
  .rule {
    height: 1px;
    background: var(--hairline);
    border: 0;
    margin: clamp(30px, 6vw, 46px) 0;
  }

  /* ---------- brier stat ---------- */
  .stat-label {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: .2em;
    text-transform: uppercase;
    color: var(--muted);
    margin-bottom: 10px;
  }
  .brier {
    display: flex;
    align-items: baseline;
    gap: 18px;
    flex-wrap: wrap;
  }
  .brier-num {
    font-family: var(--sans);
    font-weight: 600;
    font-size: clamp(46px, 11vw, 64px);
    line-height: 1;
    letter-spacing: -.03em;
    color: var(--ink);
    font-variant-numeric: tabular-nums lining-nums;
  }
  .brier-caption {
    font-size: 13.5px;
    line-height: 1.45;
    color: var(--muted);
    max-width: 30ch;
  }

  /* ---------- confidence gauge ---------- */
  .gauge {
    margin-top: clamp(26px, 5vw, 38px);
    --pos: 50%;
  }
  .gauge-track {
    position: relative;
    height: 30px;
  }
  /* the baseline */
  .gauge-track::before {
    content: "";
    position: absolute;
    left: 0; right: 0; top: 50%;
    height: 2px;
    transform: translateY(-50%);
    background: var(--hairline);
    border-radius: 2px;
  }
  /* the "calibrated" reference at centre */
  .gauge-center {
    position: absolute;
    left: 50%; top: 50%;
    width: 1px; height: 22px;
    transform: translate(-50%, -50%);
    background: var(--faint);
  }
  /* the marker: a plumb line + dot, in the accent */
  .gauge-marker {
    position: absolute;
    left: var(--pos); top: 50%;
    transform: translate(-50%, -50%);
    width: 2px; height: 26px;
    background: var(--accent);
    border-radius: 2px;
  }
  .gauge-marker::after {
    content: "";
    position: absolute;
    left: 50%; top: 50%;
    width: 13px; height: 13px;
    transform: translate(-50%, -50%);
    background: var(--accent);
    border-radius: 50%;
    box-shadow: 0 0 0 4px var(--card);
  }
  .gauge-labels {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    margin-top: 12px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: .14em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .gauge-labels span:nth-child(1) { text-align: left; }
  .gauge-labels span:nth-child(2) { text-align: center; color: var(--faint); }
  .gauge-labels span:nth-child(3) { text-align: right; }

  /* ---------- insight list ---------- */
  .insights {
    display: flex;
    flex-direction: column;
  }
  .insight {
    padding: clamp(22px, 4vw, 30px) 0;
    border-top: 1px solid var(--hairline);
  }
  .insight:first-child { border-top: 0; padding-top: 0; }
  .insight:last-child  { padding-bottom: 0; }
  .insight-q {
    font-family: var(--serif);
    font-size: clamp(18px, 3.4vw, 21px);
    font-weight: 600;
    line-height: 1.3;
    letter-spacing: -.005em;
    color: var(--ink);
    margin: 0;
    text-wrap: pretty;
  }
  .insight-a {
    font-family: var(--sans);
    font-size: 15.5px;
    line-height: 1.6;
    color: var(--ink-soft);
    margin: 10px 0 0;
    max-width: 56ch;
    text-wrap: pretty;
  }
  .insight-term {
    font-family: var(--mono);
    font-size: 11.5px;
    letter-spacing: .01em;
    color: var(--faint);
    margin: 12px 0 0;
  }

  /* ---------- what to do ---------- */
  .todo-label {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: .2em;
    text-transform: uppercase;
    color: var(--muted);
    margin-bottom: clamp(16px, 3vw, 22px);
  }
  .actions {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .actions li {
    position: relative;
    padding-left: 26px;
    font-size: 16.5px;
    line-height: 1.5;
    color: var(--ink);
    max-width: 54ch;
    text-wrap: pretty;
  }
  .actions li::before {
    content: "";
    position: absolute;
    left: 4px; top: .62em;
    width: 6px; height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }

  /* ---------- footer ---------- */
  .footer {
    margin-top: clamp(30px, 6vw, 46px);
    padding-top: clamp(20px, 4vw, 26px);
    border-top: 1px solid var(--hairline);
    font-size: 12.5px;
    line-height: 1.6;
    color: var(--muted);
    letter-spacing: .005em;
  }

  @media (max-width: 480px) {
    .masthead { flex-direction: column; gap: 6px; }
  }
"#;

/// A self-contained, offline HTML calibration card — the Claude Design "mirror"
/// rendered for real from this report. One file, inline CSS only, zero
/// JS/fonts/network; opens by double-click; light **and** dark. Your ledger never
/// leaves your machine. Built from the same `plain_summary` prose as the text view
/// so the two can't drift — Brier is the headline stat here, so it is dropped from
/// the insight list, exactly as the design lays it out.
pub fn render_html(
    ledger: &Ledger,
    tag_filter: Option<&str>,
    bins: usize,
    today: NaiveDate,
) -> String {
    let d = ReportData::compute(ledger, tag_filter, bins, today);
    let summary = (!d.is_empty()).then(|| plain_summary(&d));

    // Verdict + accent are defined even for an empty ledger.
    let (verdict_word, verdict_class): (String, &str) = match &summary {
        Some(s) => (s.verdict_word.clone(), s.verdict_class),
        None => ("Not enough data".to_string(), "unknown"),
    };
    let accent = match verdict_class {
        "under" => "#1565c0",
        "over" => "#c62828",
        "uninformative" => "#ef6c00",
        "both" => "#c62828",
        "good" => "#2e7d32",
        _ => "#757575",
    };
    let severity_class = match verdict_class {
        "under" => "underconfident",
        "over" => "overconfident",
        "uninformative" => "uninformative",
        "both" => "miscalibrated",
        "good" => "well-calibrated",
        _ => "not-enough-data",
    };

    let mut b = String::new();
    b.push_str(&format!(
        "<main class=\"card\" data-severity=\"{severity_class}\">\n    <div class=\"inner\">\n\
      <header class=\"masthead\">\n        <span class=\"wordmark\">Anamnesis<span class=\"dot\">.</span></span>\n        <span class=\"kicker\">Calibration</span>\n      </header>\n\
      <h1 class=\"verdict\">{}</h1>\n",
        esc(&verdict_word)
    ));

    match &summary {
        None => {
            b.push_str("      <p class=\"headline\">Log a belief and resolve it once you know — the mirror needs something to reflect.</p>\n");
            b.push_str(&format!(
                "      <p class=\"footer\">{} open · 0 resolved · offline &amp; local — nothing left your machine</p>\n",
                d.open
            ));
        }
        Some(s) => {
            b.push_str(&format!(
                "      <p class=\"headline\">{}</p>\n      <hr class=\"rule\" />\n",
                esc(&s.headline)
            ));
            // Brier headline stat + gauge. The design's scale: gauge_pos = 50 + gap·100
            // (so a −0.17 gap sits at 33, left of the calibrated centre).
            if let Some(brier) = d.brier {
                b.push_str(&format!(
                    "      <div>\n        <div class=\"stat-label\">Brier score</div>\n        <div class=\"brier\">\n          <span class=\"brier-num\">{brier:.2}</span>\n          <span class=\"brier-caption\">forecasting golf score — 0 perfect, 0.25 coin-flip, lower better</span>\n        </div>\n"
                ));
                if let Some(gap) = d.confidence_gap {
                    let pos = (50.0 + gap * 100.0).clamp(3.0, 97.0);
                    b.push_str(&format!(
                        "        <div class=\"gauge\" style=\"--pos: {pos:.0}%\">\n          <div class=\"gauge-track\">\n            <span class=\"gauge-center\"></span>\n            <span class=\"gauge-marker\"></span>\n          </div>\n          <div class=\"gauge-labels\">\n            <span>Undersell</span>\n            <span>Calibrated</span>\n            <span>Oversell</span>\n          </div>\n        </div>\n"
                    ));
                }
                b.push_str("      </div>\n      <hr class=\"rule\" />\n");
            }
            // Insights — Brier is the headline stat above, so leave it out of the list.
            let items: Vec<&Insight> = s
                .insights
                .iter()
                .filter(|i| i.jargon != "Brier score")
                .collect();
            if !items.is_empty() {
                b.push_str("      <section class=\"insights\">\n");
                for i in &items {
                    b.push_str(&format!(
                        "        <article class=\"insight\">\n          <p class=\"insight-q\">{}</p>\n          <p class=\"insight-a\">{}</p>\n          <p class=\"insight-term\">{}</p>\n        </article>\n",
                        esc(i.label),
                        esc(&i.answer),
                        esc(i.jargon)
                    ));
                }
                b.push_str("      </section>\n      <hr class=\"rule\" />\n");
            }
            // What to do.
            if !s.actions.is_empty() {
                b.push_str(
                    "      <div class=\"todo-label\">What to do</div>\n      <ul class=\"actions\">\n",
                );
                for a in &s.actions {
                    b.push_str(&format!("        <li>{}</li>\n", esc(a)));
                }
                b.push_str("      </ul>\n");
            }
            b.push_str(&format!(
                "      <p class=\"footer\">{}</p>\n",
                esc(&s.footer)
            ));
        }
    }

    b.push_str("    </div>\n  </main>");

    let css = CARD_CSS.replace("__ACCENT__", accent);
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\" />\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />\n<title>Anamnesis — Calibration Card</title>\n<style>\n{css}</style>\n</head>\n<body>\n  {b}\n</body>\n</html>\n"
    )
}

/// An embeddable 400×100 README badge — the SVG cousin of the card: verdict word,
/// Brier, and the same undersell↔oversell gauge. Verbatim from the Claude Design
/// `badge.template.svg`; every colour is a presentation attribute (not CSS) so it
/// survives GitHub's SVG sanitizer. Gauge marker at `gauge_x = 28 + 344·pos/100`,
/// the same `pos = 50 + gap·100` scale as the card.
pub fn render_badge_svg(
    ledger: &Ledger,
    tag_filter: Option<&str>,
    bins: usize,
    today: NaiveDate,
) -> String {
    let d = ReportData::compute(ledger, tag_filter, bins, today);
    let summary = (!d.is_empty()).then(|| plain_summary(&d));
    let (verdict_word, verdict_class): (String, &str) = match &summary {
        Some(s) => (s.verdict_word.clone(), s.verdict_class),
        None => ("Not enough data".to_string(), "unknown"),
    };
    let accent = match verdict_class {
        "under" => "#1565c0",
        "over" => "#c62828",
        "good" => "#2e7d32",
        _ => "#757575",
    };
    let brier = d
        .brier
        .map(|b| format!("{b:.2}"))
        .unwrap_or_else(|| "—".into());
    let pos = (50.0 + d.confidence_gap.unwrap_or(0.0) * 100.0).clamp(3.0, 97.0);
    let gauge_x = 28.0 + 344.0 * pos / 100.0;
    let v = esc(&verdict_word);
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="100" viewBox="0 0 400 100" role="img" aria-label="Anamnesis calibration: {v}, Brier {brier}">
  <title>Anamnesis calibration — {v} · Brier {brier}</title>

  <clipPath id="r"><rect x="1" y="1" width="398" height="98" rx="11"></rect></clipPath>
  <rect class="card" x="1" y="1" width="398" height="98" rx="11" fill="#fbfaf6" stroke="#e2ddd2"></rect>
  <g clip-path="url(#r)"><rect class="spine" x="0" y="0" width="5" height="100" fill="{accent}"></rect></g>

  <text class="verdict" x="28" y="40" fill="{accent}" font-family="&#39;Iowan Old Style&#39;,Palatino,Georgia,&#39;Times New Roman&#39;,serif" font-size="25" font-weight="600">{v}</text>
  <text class="lab" x="372" y="24" text-anchor="end" fill="#75716a" font-family="system-ui,-apple-system,&#39;Segoe UI&#39;,Roboto,sans-serif" font-size="9.5" font-weight="600" letter-spacing="2">BRIER</text>
  <text class="brier" x="372" y="45" text-anchor="end" fill="#1d1b16" font-family="system-ui,-apple-system,&#39;Segoe UI&#39;,Roboto,sans-serif" font-size="26" font-weight="600" letter-spacing="-1">{brier}</text>

  <line class="track" x1="28" y1="66" x2="372" y2="66" stroke="#e2ddd2" stroke-width="2" stroke-linecap="round"></line>
  <line class="center" x1="200" y1="59" x2="200" y2="73" stroke="#a39e93" stroke-width="1"></line>
  <line class="mark" x1="{gauge_x:.1}" y1="57" x2="{gauge_x:.1}" y2="75" stroke="{accent}" stroke-width="2" stroke-linecap="round"></line>
  <circle class="dot" cx="{gauge_x:.1}" cy="66" r="5" fill="{accent}" stroke="#fbfaf6" stroke-width="3"></circle>

  <text class="foot" x="28" y="90" fill="#75716a" font-family="system-ui,-apple-system,&#39;Segoe UI&#39;,Roboto,sans-serif" font-size="11" letter-spacing="0.3">anamnesis · calibration · offline &amp; local</text>
</svg>
"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Claim, Forecast, NumericForecast, Outcome, Resolution};
    use chrono::{TimeZone, Utc};

    fn now() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 1, 1, 12, 0, 0).unwrap()
    }

    /// Fixed "today" for deterministic reports (overdue is relative to this).
    fn td() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
    }

    fn binary(id: &str, prob: f64, happened: bool, tags: &[&str]) -> Claim {
        Claim {
            id: id.into(),
            statement: format!("claim {id}"),
            created_at: now(),
            // A past due date, so the claim enters the evidence sequence. The
            // sequential test orders by resolve-by precisely because that date is
            // chosen before the outcome is known.
            resolve_by: Some(NaiveDate::from_ymd_opt(2025, 6, 1).unwrap()),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            kind: crate::model::ClaimKind::Binary,
            stake: 1.0,
            forecasts: vec![Forecast {
                at: now(),
                prob: Some(prob),
                interval: None,
                because: None,
            }],
            resolution: Some(Resolution {
                at: now(),
                outcome: Some(if happened {
                    Outcome::True
                } else {
                    Outcome::False
                }),
                value: None,
                note: None,
                resolved_by: None,
            }),
            void: None,
            amendments: Vec::new(),
        }
    }

    fn numeric(id: &str, low: f64, high: f64, level: f64, value: f64) -> Claim {
        Claim {
            id: id.into(),
            statement: format!("num {id}"),
            created_at: now(),
            resolve_by: None,
            tags: vec![],
            kind: crate::model::ClaimKind::Numeric,
            stake: 1.0,
            forecasts: vec![Forecast {
                at: now(),
                prob: None,
                interval: Some(NumericForecast { low, high, level }),
                because: None,
            }],
            resolution: Some(Resolution {
                at: now(),
                outcome: None,
                value: Some(value),
                note: None,
                resolved_by: None,
            }),
            void: None,
            amendments: Vec::new(),
        }
    }

    #[test]
    fn empty_ledger_renders_guidance() {
        let r = render(&Ledger::default(), None, 10, td());
        assert!(r.contains("No resolved claims yet"));
        // JSON of an empty ledger is still well-formed with null metrics.
        let j = render_json(&Ledger::default(), None, 10, td());
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["resolved"], 0);
        assert!(v["brier"].is_null());
    }

    #[test]
    fn json_has_no_nan_and_parses() {
        let ledger = Ledger {
            claims: vec![
                binary("a", 0.9, true, &["tech"]),
                binary("b", 0.9, false, &["tech"]),
                binary("c", 0.2, false, &["world"]),
                binary("d", 0.8, true, &["world"]),
            ],
        };
        let j = render_json(&ledger, None, 10, td());
        // Must be valid JSON and contain no bare NaN/Infinity tokens.
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert!(!j.contains("NaN") && !j.contains("Infinity"));
        assert_eq!(v["resolved"], 4);
        assert!(v["brier"].as_f64().unwrap() > 0.0);
        assert!(v["auc"].as_f64().is_some());
        // Empty calibration bins serialise observed as null, not NaN.
        let cal = v["calibration"].as_array().unwrap();
        assert!(cal.iter().any(|b| b["observed"].is_null()));
    }

    #[test]
    fn text_and_json_agree_on_brier() {
        let ledger = Ledger {
            claims: vec![binary("a", 0.7, true, &[]), binary("b", 0.3, false, &[])],
        };
        let data = ReportData::compute(&ledger, None, 10, td());
        let v: serde_json::Value =
            serde_json::from_str(&render_json(&ledger, None, 10, td())).unwrap();
        assert_eq!(v["brier"].as_f64().unwrap(), data.brier.unwrap());
    }

    #[test]
    fn numeric_section_appears_and_scores() {
        // Two 80% intervals; one misses badly → coverage 50% < 80% (overconfident).
        let ledger = Ledger {
            claims: vec![
                numeric("n1", 10.0, 20.0, 0.8, 15.0), // inside
                numeric("n2", 10.0, 20.0, 0.8, 40.0), // outside
            ],
        };
        let r = render(&ledger, None, 10, td());
        assert!(r.contains("Numeric forecasts"));
        assert!(r.contains("TOO NARROW"));
        let v: serde_json::Value =
            serde_json::from_str(&render_json(&ledger, None, 10, td())).unwrap();
        assert_eq!(v["numeric"]["count"], 2);
        assert_eq!(v["numeric"]["empirical_coverage"].as_f64().unwrap(), 0.5);
    }

    #[test]
    fn tag_filter_limits_and_titles() {
        let ledger = Ledger {
            claims: vec![
                binary("a", 0.9, true, &["tech"]),
                binary("b", 0.2, false, &["world"]),
            ],
        };
        let r = render(&ledger, Some("tech"), 10, td());
        assert!(r.contains("[tag: tech]"));
        assert!(!r.contains("By domain")); // suppressed when filtered to one tag
    }

    #[test]
    fn plain_and_html_views_explain_and_render() {
        // Enough claims, and enough miscalibration, to reach a real verdict:
        // three claims is `insufficient_data`, which is its own test below.
        let ledger = Ledger {
            claims: (0..40)
                .map(|i| binary(&format!("c{i}"), 0.9, i % 3 == 0, &[]))
                .collect(),
        };
        let p = render_plain(&ledger, None, 10, td());
        assert!(p.contains("plain English"));
        assert!(p.contains("golf score")); // Brier explained, never left bare
        assert!(p.contains("technical name:")); // jargon named, not hidden
        assert!(p.contains("what should you do")); // the action list
                                                   // The calibration cat shows one of its moods.
        assert!(["DIALED IN", "CLOSE", "DRIFTING", "WAY OFF", "WARMING UP"]
            .iter()
            .any(|m| p.contains(m)));

        let h = render_html(&ledger, None, 10, td());
        assert!(h.contains("<!DOCTYPE html"));
        assert!(h.contains("class=\"card\""));
        assert!(h.contains("class=\"verdict\""));
        assert!(h.contains("golf score")); // Brier caption
        assert!(!h.contains("__ACCENT__")); // the accent placeholder was filled
        assert!(h.contains("#c62828")); // this ledger is overconfident → red accent seed
                                        // Brier is the headline stat, so it must NOT also appear as an insight term.
        assert!(!h.contains(">Brier score</p>"));

        // Scenario 3: a handful of sure-and-right calls is not a verdict.
        let tiny = Ledger {
            claims: vec![
                binary("a", 1.0, true, &[]),
                binary("b", 1.0, true, &[]),
                binary("c", 1.0, true, &[]),
                binary("d", 0.0, false, &[]),
                binary("e", 0.0, false, &[]),
            ],
        };
        let td5 = ReportData::compute(&tiny, None, 10, td());
        assert_eq!(td5.verdict, Verdict::InsufficientData);
        assert!(
            td5.log_score.unwrap().is_finite(),
            "log score must not be inf"
        );
        let tp = render_plain(&tiny, None, 10, td());
        // No verdict, no praise — five correct calls is five correct calls.
        assert!(
            tp.contains("Too few resolved calls") && tp.contains("WARMING UP"),
            "must say so plainly:\n{tp}"
        );
        for banned in ["DIALED IN", "Well calibrated", "confidence is honest"] {
            assert!(!tp.contains(banned), "{banned:?} at n=5:\n{tp}");
        }
        assert!(render_badge_svg(&tiny, None, 10, td()).contains("Not enough data"));

        // Both views survive an empty ledger without panicking.
        assert!(render_plain(&Ledger::default(), None, 10, td()).contains("Nothing to reflect"));
        assert!(render_html(&Ledger::default(), None, 10, td()).contains("<!DOCTYPE html"));

        // Badge: valid SVG, real verdict + accent, no leftover template placeholders.
        let svg = render_badge_svg(&ledger, None, 10, td());
        assert!(svg.contains("<svg") && svg.contains("</svg>"));
        assert!(svg.contains("Overconfident")); // this ledger is overconfident
        assert!(svg.contains("#c62828")); // → red accent
        assert!(!svg.contains("{{")); // every placeholder filled
        assert!(render_badge_svg(&Ledger::default(), None, 10, td()).contains("Not enough data"));
    }

    /// The old version of this test was called `mood_bands_track_the_confidence_gap`,
    /// and the name was the bug: the gap cancels over- against under-confidence,
    /// so the friendliest face in the program sat on top of the worst ledger in
    /// the test suite. The mood now tracks the SIZE of the calibration error
    /// against the noise floor for a record of that size.
    #[test]
    fn mood_tracks_the_size_of_the_calibration_error() {
        // Four sure-and-right calls is not calibration evidence, it is four calls.
        let tiny = Ledger {
            claims: vec![
                binary("a", 1.0, true, &[]),
                binary("b", 1.0, true, &[]),
                binary("c", 0.0, false, &[]),
                binary("d", 0.0, false, &[]),
            ],
        };
        assert_eq!(
            mood(&ReportData::compute(&tiny, None, 10, td())).name,
            "WARMING UP",
            "a four-claim ledger must not earn the best face in the program"
        );

        // Wildly overconfident, and enough of it to be demonstrated.
        let bad = Ledger {
            claims: (0..40)
                .map(|i| {
                    binary(
                        &format!("c{i}"),
                        if i % 2 == 0 { 0.95 } else { 0.05 },
                        i % 2 != 0,
                        &[],
                    )
                })
                .collect(),
        };
        // Its 0.95s never happen and its 0.05s always do: perfectly inverted, so
        // the isotonic fit pools everything and DSC is 0. The headline finding is
        // that the ranking carries no information — but the FACE still tracks the
        // size of the calibration error, which is enormous.
        let d = ReportData::compute(&bad, None, 10, td());
        assert_eq!(d.verdict, Verdict::CalibratedButUninformative);
        assert_eq!(mood(&d).name, "UNINFORMATIVE");
        assert_eq!(
            mood(&d).art,
            CAT_OFF,
            "an inverted forecaster must not look calm"
        );

        // Nothing to judge → the sleepy face.
        assert_eq!(
            mood(&ReportData::compute(&Ledger::default(), None, 10, td())).name,
            "WARMING UP"
        );
    }

    /// Finding B, as a unit test: the single ledger that most embarrassed the
    /// old code must never again be praised on any surface.
    #[test]
    fn cancelling_errors_are_never_called_well_calibrated() {
        let mut claims: Vec<Claim> = Vec::new();
        for i in 0..50 {
            claims.push(binary(&format!("f{i}"), 0.9, i % 2 == 0, &[]));
        }
        for i in 0..50 {
            claims.push(binary(&format!("s{i}"), 0.6, true, &[]));
        }
        let ledger = Ledger { claims };
        let d = ReportData::compute(&ledger, None, 10, td());

        // The gap that fooled the old code is still ~0 — that is the point.
        assert!(d.confidence_gap.unwrap().abs() < 1e-9);
        // DSC is exactly 0 here — higher stated confidence went with a LOWER hit
        // rate, so isotonic regression collapses both groups to one value. That
        // outranks anything about the level: telling this forecaster to shade
        // their numbers down would not help, because their numbers carry no
        // information about which calls come true in the first place.
        assert_eq!(d.verdict, Verdict::CalibratedButUninformative);
        assert!(d.mcb.unwrap() > d.mcb_null_q95.unwrap());
        assert_eq!(d.dsc.unwrap(), 0.0, "the forecasts order nothing");

        for view in [
            render(&ledger, None, 10, td()),
            render_plain(&ledger, None, 10, td()),
            render_html(&ledger, None, 10, td()),
            render_badge_svg(&ledger, None, 10, td()),
            render_json(&ledger, None, 10, td()),
        ] {
            for banned in [
                "DIALED IN",
                "Well calibrated",
                "well calibrated",
                "well-calibrated",
                "confidence is honest",
                "Keep doing what you're doing",
            ] {
                assert!(!view.contains(banned), "{banned:?} appeared in:\n{view}");
            }
        }
    }
}
