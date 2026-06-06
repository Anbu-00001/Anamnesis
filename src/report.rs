//! Rendering the calibration report — the moment the ledger stops being a diary
//! and becomes a mirror. Everything here is plain ASCII so it survives pipes,
//! logs, and machines with no fonts and no opinions.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::model::Ledger;
use crate::scoring::{self, Sample};

const LANE_W: usize = 34;

/// Place a predicted marker `P` and an observed marker `O` on a `[0,1]` lane.
/// When they coincide it draws `X` (that bin is calibrated). The visual gap
/// between `P` and `O` *is* the calibration error for that bin.
fn lane(pred: f64, obs: f64) -> String {
    let pos = |v: f64| -> usize { (v.clamp(0.0, 1.0) * (LANE_W as f64 - 1.0)).round() as usize };
    let mut cells = vec![b'-'; LANE_W];
    let pp = pos(pred);
    if obs.is_nan() {
        cells[pp] = b'P';
    } else {
        let op = pos(obs);
        if pp == op {
            cells[pp] = b'X';
        } else {
            cells[pp] = b'P';
            cells[op] = b'O';
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

/// Render the full report for a ledger, optionally filtered to a single tag.
pub fn render(ledger: &Ledger, tag_filter: Option<&str>, bins: usize) -> String {
    let tag_filter = tag_filter.map(|t| t.to_lowercase());

    // Gather resolved samples (and keep the claims for slicing / dates).
    let resolved: Vec<&crate::model::Claim> = ledger
        .claims
        .iter()
        .filter(|c| c.is_resolved())
        .filter(|c| match &tag_filter {
            Some(t) => c.tags.iter().any(|x| x == t),
            None => true,
        })
        .collect();
    let samples: Vec<Sample> = resolved.iter().filter_map(|c| c.sample()).collect();
    let open = ledger.claims.iter().filter(|c| c.is_open()).count();

    let mut out = String::new();
    let title = match &tag_filter {
        Some(t) => format!("ANAMNESIS — the shape of your judgement  [tag: {t}]"),
        None => "ANAMNESIS — the shape of your judgement".to_string(),
    };
    let _ = writeln!(out, "\n{title}");
    let _ = writeln!(out, "{}", "=".repeat(title.len()));

    if samples.is_empty() {
        let _ = writeln!(
            out,
            "\nNo resolved claims yet{}. {open} still open.\n\nRecord beliefs with `ana add`, and `ana resolve` them once reality speaks.\nThe mirror needs something to reflect.",
            tag_filter.as_ref().map(|t| format!(" tagged '{t}'")).unwrap_or_default()
        );
        return out;
    }

    // Period covered.
    let dates: Vec<_> = resolved.iter().map(|c| c.created_at.date_naive()).collect();
    let lo = dates.iter().min().unwrap();
    let hi = dates.iter().max().unwrap();
    let _ = writeln!(
        out,
        "\n{} resolved  ·  {} open  ·  first recorded {}  ·  latest {}",
        samples.len(),
        open,
        lo,
        hi
    );

    // --- Headline scores -----------------------------------------------------
    let brier = scoring::brier(&samples).unwrap();
    let logs = scoring::log_score(&samples, 1e-6).unwrap();
    let base = scoring::base_rate(&samples).unwrap();
    let _ = writeln!(out, "\n  Brier score      {brier:.3}   (0 = perfect · 0.25 = always 50/50 · lower better)");
    let _ = writeln!(out, "  Log score        {logs:.3}   (lower better; punishes confident misses)");
    if let Some(bss) = scoring::skill_score(&samples) {
        let tail = if bss > 0.0 {
            "you beat always-guess-the-base-rate"
        } else if bss < 0.0 {
            "you did WORSE than always guessing the base rate"
        } else {
            "no better than guessing the base rate"
        };
        let _ = writeln!(out, "  Brier skill      {bss:+.3}   ({tail})");
    }
    let _ = writeln!(out, "  Base rate        {base:.3}   (fraction of your claims that came true)");

    // --- Murphy decomposition ------------------------------------------------
    let d = scoring::decompose(&samples).unwrap();
    let _ = writeln!(out, "\n  Decomposition  (Brier = Reliability − Resolution + Uncertainty)");
    let _ = writeln!(out, "    reliability    {:.3}   calibration error      ↓ lower is better", d.reliability);
    let _ = writeln!(out, "    resolution     {:.3}   discrimination power   ↑ higher is better", d.resolution);
    let _ = writeln!(out, "    uncertainty    {:.3}   irreducible difficulty of your questions", d.uncertainty);
    let _ = writeln!(
        out,
        "    check          {:.3} − {:.3} + {:.3} = {:.3}  (= Brier, to f64 precision)",
        d.reliability, d.resolution, d.uncertainty, d.brier
    );

    // --- Discrimination ------------------------------------------------------
    match scoring::auc(&samples) {
        Some(a) => {
            let _ = writeln!(
                out,
                "\n  Discrimination   AUC {a:.3}   (0.5 = can't tell true from false · 1.0 = perfect)"
            );
        }
        None => {
            let _ = writeln!(out, "\n  Discrimination   n/a   (every claim resolved the same way)");
        }
    }

    // --- Over / under-confidence --------------------------------------------
    let o = scoring::overconfidence(&samples).unwrap();
    let _ = writeln!(
        out,
        "\n  Confidence gap   {:+.3}   {}",
        o.gap,
        verdict(o.gap)
    );
    let _ = writeln!(
        out,
        "                   mean boldness {:.3}  vs  accuracy {:.3}",
        o.mean_confidence, o.accuracy
    );
    if let Some(bias) = scoring::directional_bias(&samples) {
        let dir = if bias > 0.0 { "toward YES" } else { "toward NO" };
        let _ = writeln!(out, "                   directional bias {bias:+.3} ({dir})");
    }

    // --- Reliability diagram -------------------------------------------------
    let _ = writeln!(out, "\n  Reliability diagram   P = your avg forecast · O = what actually happened");
    let _ = writeln!(out, "    range        n    0{}1", " ".repeat(LANE_W.saturating_sub(2)));
    for b in scoring::calibration_curve(&samples, bins) {
        if b.count == 0 {
            continue;
        }
        let tail = if b.observed.is_nan() {
            String::new()
        } else {
            let gap = b.mean_pred - b.observed;
            let mark = if gap.abs() < 0.05 {
                "ok"
            } else if gap > 0.0 {
                "over"
            } else {
                "under"
            };
            format!("  pred {:.2} → obs {:.2}  {}", b.mean_pred, b.observed, mark)
        };
        let _ = writeln!(
            out,
            "    {:.2}-{:.2}  {:>4}   |{}|{}",
            b.lo,
            b.hi,
            b.count,
            lane(b.mean_pred, b.observed),
            tail
        );
    }

    // --- By tag --------------------------------------------------------------
    if tag_filter.is_none() {
        let mut by_tag: BTreeMap<&str, Vec<Sample>> = BTreeMap::new();
        for c in &resolved {
            if let Some(s) = c.sample() {
                for t in &c.tags {
                    by_tag.entry(t.as_str()).or_default().push(s);
                }
            }
        }
        if !by_tag.is_empty() {
            let mut rows: Vec<(&str, Vec<Sample>)> = by_tag.into_iter().collect();
            rows.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
            let _ = writeln!(out, "\n  By domain");
            let _ = writeln!(out, "    {:<14} {:>4}  {:>7}  {:>9}", "tag", "n", "brier", "conf-gap");
            for (tag, s) in rows {
                let br = scoring::brier(&s).unwrap();
                let g = scoring::overconfidence(&s).unwrap().gap;
                let _ = writeln!(out, "    {:<14} {:>4}  {:>7.3}  {:>+9.3}", tag, s.len(), br, g);
            }
        }
    }

    // --- The virtue of changing your mind ------------------------------------
    let revised: Vec<&crate::model::Claim> =
        resolved.iter().copied().filter(|c| c.was_revised()).collect();
    if !revised.is_empty() {
        let mut first = Vec::new();
        let mut last = Vec::new();
        for c in &revised {
            let o = c.outcome().unwrap().happened();
            first.push(Sample::new(c.first_prob().unwrap(), o));
            last.push(Sample::new(c.current_prob().unwrap(), o));
        }
        let bf = scoring::brier(&first).unwrap();
        let bl = scoring::brier(&last).unwrap();
        let improvement = bf - bl; // positive ⇒ your revisions helped
        let _ = writeln!(
            out,
            "\n  Mind-changing    {} claim(s) you revised",
            revised.len()
        );
        let _ = writeln!(
            out,
            "    Brier of first guess {bf:.3}  →  Brier of final guess {bl:.3}   ({improvement:+.3})"
        );
        let line = if improvement > 0.005 {
            "    Your updates moved you TOWARD the truth. Good — you changed your mind well."
        } else if improvement < -0.005 {
            "    Your updates moved you AWAY from the truth. Beware revising under social or emotional pressure."
        } else {
            "    Your revisions were roughly a wash."
        };
        let _ = writeln!(out, "{line}");
    }

    // --- Closing line --------------------------------------------------------
    let _ = writeln!(
        out,
        "\n  \"The first principle is that you must not fool yourself —\n   and you are the easiest person to fool.\"  — R. Feynman\n"
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Claim, Forecast, Outcome, Resolution};
    use chrono::{TimeZone, Utc};

    fn resolved_claim(id: &str, prob: f64, happened: bool, tags: &[&str]) -> Claim {
        let now = Utc.with_ymd_and_hms(2025, 1, 1, 12, 0, 0).unwrap();
        Claim {
            id: id.into(),
            statement: format!("claim {id}"),
            created_at: now,
            resolve_by: None,
            tags: tags.iter().map(|s| s.to_string()).collect(),
            forecasts: vec![Forecast { at: now, prob, because: None }],
            resolution: Some(Resolution { at: now, outcome: if happened { Outcome::True } else { Outcome::False }, note: None }),
        }
    }

    #[test]
    fn empty_ledger_renders_guidance() {
        let r = render(&Ledger::default(), None, 10);
        assert!(r.contains("No resolved claims yet"));
    }

    #[test]
    fn report_contains_core_sections() {
        let ledger = Ledger {
            claims: vec![
                resolved_claim("a", 0.9, true, &["tech"]),
                resolved_claim("b", 0.9, false, &["tech"]),
                resolved_claim("c", 0.2, false, &["world"]),
                resolved_claim("d", 0.8, true, &["world"]),
            ],
        };
        let r = render(&ledger, None, 10);
        assert!(r.contains("Brier score"));
        assert!(r.contains("Decomposition"));
        assert!(r.contains("Reliability diagram"));
        assert!(r.contains("By domain"));
        assert!(r.contains("Confidence gap"));
    }

    #[test]
    fn tag_filter_limits_and_titles() {
        let ledger = Ledger {
            claims: vec![
                resolved_claim("a", 0.9, true, &["tech"]),
                resolved_claim("b", 0.2, false, &["world"]),
            ],
        };
        let r = render(&ledger, Some("tech"), 10);
        assert!(r.contains("[tag: tech]"));
        // 'By domain' is suppressed when already filtered to one tag.
        assert!(!r.contains("By domain"));
    }
}
