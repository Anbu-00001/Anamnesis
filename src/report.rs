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
            for c in &resolved_claims {
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
            for c in &resolved_claims {
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
            Some(NumericData {
                count: numeric_samples.len(),
                mean_winkler,
                nominal_coverage: nominal,
                empirical_coverage: cov,
                coverage_gap: cov - nominal,
                mean_width: width,
            })
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
