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
pub(crate) const RECAL_MIN_E: f64 = 3.0;
pub(crate) const RECAL_MIN_N: usize = 6;

/// The recalibration map for the resolved binary claims matching `tag` (in
/// resolution order), together with whether the e-process has **earned** applying
/// it — real evidence (`≥ RECAL_MIN_E`) over enough samples (`≥ RECAL_MIN_N`). The
/// fitted map is returned regardless; `earned` says whether to trust it. Defined
/// once here, next to the gate constants, so the report, the MCP `recalibrate`/
/// `decide` tools, and the CLI can never disagree about when a correction is real.
pub fn earned_recalibration(
    ledger: &Ledger,
    tag: Option<&str>,
) -> (Option<scoring::Recalibration>, bool, usize, Option<f64>) {
    let mut claims: Vec<&crate::model::Claim> = ledger
        .claims
        .iter()
        .filter(|c| c.is_resolved())
        .filter(|c| tag.is_none_or(|t| c.tags.iter().any(|x| x == t)))
        .collect();
    claims.sort_by_key(|c| c.resolution.as_ref().map(|r| r.at));
    let samples: Vec<Sample> = claims.iter().filter_map(|c| c.sample()).collect();
    let n = samples.len();
    let e = scoring::calibration_eprocess(&samples);
    let recal = scoring::fit_recalibration(&samples, RECAL_RIDGE);
    let earned = recal.is_some() && e.is_some_and(|ev| ev >= RECAL_MIN_E) && n >= RECAL_MIN_N;
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
    pub resolved: usize,
    pub open: usize,
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
}

impl ReportData {
    /// Compute every metric once, for the binary and numeric claims that match
    /// the optional tag filter.
    pub fn compute(ledger: &Ledger, tag_filter: Option<&str>, bins: usize) -> ReportData {
        let tag_filter = tag_filter.map(|t| t.to_lowercase());
        let matches_tag = |c: &&crate::model::Claim| match &tag_filter {
            Some(t) => c.tags.iter().any(|x| x == t),
            None => true,
        };

        let resolved_claims: Vec<&crate::model::Claim> = ledger
            .claims
            .iter()
            .filter(|c| c.is_resolved())
            .filter(matches_tag)
            .collect();
        let samples: Vec<Sample> = resolved_claims.iter().filter_map(|c| c.sample()).collect();
        let numeric_samples: Vec<NumericSample> = resolved_claims
            .iter()
            .filter_map(|c| c.numeric_sample())
            .collect();
        let open = ledger
            .claims
            .iter()
            .filter(|c| c.is_open())
            .filter(matches_tag)
            .count();

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

        // Chronological (resolution-order) view, reused by the e-process and by the
        // per-slice e-values below: sequential validity is about the order outcomes
        // were *learned*, so sort by resolution time once and share it.
        let mut chrono: Vec<&crate::model::Claim> = resolved_claims.clone();
        chrono.sort_by_key(|c| c.resolution.as_ref().map(|r| r.at));
        let chrono_samples: Vec<Sample> = chrono.iter().filter_map(|c| c.sample()).collect();

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
            for c in &chrono {
                if let Some(s) = c.sample() {
                    for t in &c.tags {
                        if let Some(k) = t.strip_prefix("kind:") {
                            map.entry(k).or_default().push(s);
                        }
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
                        // Per-kind anytime-valid evidence (samples already in
                        // resolution order, inherited from `chrono`).
                        eprocess: scoring::calibration_eprocess(&s),
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
                nominal_coverage: nominal,
                empirical_coverage: cov,
                coverage_gap: cov - nominal,
                mean_width: width,
                coverage_shrunk,
                coverage_eprocess: scoring::calibration_eprocess(&coverage_samples),
                width_factor: scoring::conformal_width_factor(&numeric_samples),
            })
        };

        // Anytime-valid calibration test, fed the outcomes in the order they were
        // learned (chrono / chrono_samples computed once, above).
        let eprocess = scoring::calibration_eprocess(&chrono_samples);
        let eprocess_pvalue = eprocess.map(scoring::eprocess_pvalue);

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
        let recent_brier = scoring::ewma_brier(&chrono_samples, EWMA_HALF_LIFE);
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

        ReportData {
            tag: tag_filter,
            resolved: samples.len(),
            open,
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
        }
    }

    /// True when there is nothing resolved (binary or numeric) to reflect.
    fn is_empty(&self) -> bool {
        self.resolved == 0 && self.numeric.is_none()
    }
}

// ─────────────────────────── machine view ───────────────────────────────────

/// Pretty-printed JSON of the full [`ReportData`]. The agent-facing surface.
pub fn render_json(ledger: &Ledger, tag_filter: Option<&str>, bins: usize) -> String {
    let data = ReportData::compute(ledger, tag_filter, bins);
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

fn verdict(gap: f64) -> &'static str {
    if gap > 0.05 {
        "OVERCONFIDENT — you are bolder than you are right"
    } else if gap < -0.05 {
        "UNDERCONFIDENT — reality rewards you more than you claim"
    } else {
        "well calibrated in aggregate"
    }
}

/// Render the full human-readable report.
pub fn render(ledger: &Ledger, tag_filter: Option<&str>, bins: usize) -> String {
    let d = ReportData::compute(ledger, tag_filter, bins);
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

    let _ = writeln!(
        out,
        "\n{} resolved  ·  {} open  ·  first recorded {}  ·  latest {}",
        d.resolved,
        d.open,
        d.first_recorded.as_deref().unwrap_or("—"),
        d.last_recorded.as_deref().unwrap_or("—"),
    );

    // Binary section -------------------------------------------------------
    if d.resolved > 0 {
        if let (Some(brier), Some(logs), Some(base)) = (d.brier, d.log_score, d.base_rate) {
            let _ = writeln!(out, "\n  Brier score      {brier:.3}   (0 = perfect · 0.25 = always 50/50 · lower better)");
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
                if d.resolved >= TREND_MIN_N {
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

        if let (Some(rel), Some(res), Some(unc)) = (d.reliability, d.resolution, d.uncertainty) {
            let _ = writeln!(
                out,
                "\n  Decomposition  (Brier = Reliability − Resolution + Uncertainty)"
            );
            let _ = writeln!(
                out,
                "    reliability    {rel:.3}   calibration error      ↓ lower is better"
            );
            let _ = writeln!(
                out,
                "    resolution     {res:.3}   discrimination power   ↑ higher is better"
            );
            let _ = writeln!(
                out,
                "    uncertainty    {unc:.3}   irreducible difficulty of your questions"
            );
            let _ = writeln!(out, "    check          {rel:.3} − {res:.3} + {unc:.3} = {:.3}  (= Brier, to f64 precision)", rel - res + unc);
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
            let _ = writeln!(out, "\n  Confidence gap   {gap:+.3}   {}", verdict(gap));
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
                d.resolved,
                if d.distinct_forecasts <= 3 && d.resolved >= 6 {
                    " — coarse; more gradations would sharpen you"
                } else {
                    ""
                }
            );
        }

        // Selective prediction — error among your surest calls vs all (when to act).
        if let Some(sel) = &d.selective {
            if d.resolved >= 6 {
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
        if let Some(e) = d.eprocess {
            let p = d.eprocess_pvalue.unwrap_or(1.0);
            let verdict = if e >= 20.0 {
                "miscalibration is REAL — significant even though you peek every session (α=.05)"
            } else if e >= 3.0 {
                "suggestive, not yet conclusive — keep logging"
            } else {
                "no real evidence of miscalibration yet — too few resolutions to tell"
            };
            let _ = writeln!(
                out,
                "\n  Is it real?      e-value {e:>6.1}   (anytime-valid p ≤ {p:.3})"
            );
            let _ = writeln!(out, "                   {verdict}");
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
            if let Some((t, e)) = d
                .by_kind
                .iter()
                .filter_map(|t| t.eprocess.map(|e| (t, e)))
                .filter(|&(_, e)| e >= RECAL_MIN_E)
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
                "    Brier of first guess {:.3}  →  Brier of final guess {:.3}   ({:+.3})",
                mc.brier_first, mc.brier_last, mc.improvement
            );
            let line = if mc.improvement > 0.005 {
                "    Your updates moved you TOWARD the truth. Good — you changed your mind well."
            } else if mc.improvement < -0.005 {
                "    Your updates moved you AWAY from the truth. Beware revising under social or emotional pressure."
            } else {
                "    Your revisions were roughly a wash."
            };
            let _ = writeln!(out, "{line}");
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
            "    mean Winkler score   {:.3}   (lower better; width + miscoverage penalty)",
            num.mean_winkler
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Claim, Forecast, NumericForecast, Outcome, Resolution};
    use chrono::{TimeZone, Utc};

    fn now() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 1, 1, 12, 0, 0).unwrap()
    }

    fn binary(id: &str, prob: f64, happened: bool, tags: &[&str]) -> Claim {
        Claim {
            id: id.into(),
            statement: format!("claim {id}"),
            created_at: now(),
            resolve_by: None,
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
            }),
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
            }),
        }
    }

    #[test]
    fn empty_ledger_renders_guidance() {
        let r = render(&Ledger::default(), None, 10);
        assert!(r.contains("No resolved claims yet"));
        // JSON of an empty ledger is still well-formed with null metrics.
        let j = render_json(&Ledger::default(), None, 10);
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
        let j = render_json(&ledger, None, 10);
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
        let data = ReportData::compute(&ledger, None, 10);
        let v: serde_json::Value = serde_json::from_str(&render_json(&ledger, None, 10)).unwrap();
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
        let r = render(&ledger, None, 10);
        assert!(r.contains("Numeric forecasts"));
        assert!(r.contains("TOO NARROW"));
        let v: serde_json::Value = serde_json::from_str(&render_json(&ledger, None, 10)).unwrap();
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
        let r = render(&ledger, Some("tech"), 10);
        assert!(r.contains("[tag: tech]"));
        assert!(!r.contains("By domain")); // suppressed when filtered to one tag
    }
}
