//! The domain model: claims, the forecasts layered onto them over time, and
//! their eventual resolution. A [`Claim`] is deliberately a *palimpsest* — each
//! revision is appended, never overwritten, so the history of your own
//! mind-changing survives intact.
//!
//! A claim is either **binary** (a yes/no proposition scored with a probability)
//! or **numeric** (a quantity scored with a credible interval). Both live in the
//! same struct; the [`ClaimKind`] tag and the `Option` fields keep older,
//! binary-only ledgers loading unchanged (see the serde defaults below).

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::scoring::{NumericSample, Sample};

/// Whether a claim is scored as a yes/no proposition or as a quantity.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ClaimKind {
    /// A proposition resolved true/false, forecast with a probability.
    #[default]
    Binary,
    /// A quantity resolved to a number, forecast with a credible interval.
    Numeric,
}

/// What actually happened for a *binary* claim.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    True,
    False,
}

impl Outcome {
    pub fn happened(self) -> bool {
        matches!(self, Outcome::True)
    }
}

/// A central credible interval `[low, high]` stated at confidence `level`
/// (e.g. `0.80` for an 80% interval). The stored form of a numeric forecast.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct NumericForecast {
    pub low: f64,
    pub high: f64,
    pub level: f64,
}

/// A single dated forecast, with the reasoning that produced it. Exactly one of
/// `prob` (binary) or `interval` (numeric) is set.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Forecast {
    pub at: DateTime<Utc>,
    /// Probability the claim is true, in `[0, 1]` — for binary claims.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prob: Option<f64>,
    /// Credible interval — for numeric claims.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval: Option<NumericForecast>,
    /// The reasoning — the part hindsight will try to erase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub because: Option<String>,
}

/// The verdict of reality. Exactly one of `outcome` (binary) or `value`
/// (numeric) is set.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Resolution {
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Who graded this claim: `self` (the forecaster said so), `auto` (a machine
    /// observed it — a test run's exit status, say), or `human`.
    ///
    /// "Why would I trust a self-graded ledger?" is the first fair objection to
    /// this whole idea, and the honest answer is to record which claims did not
    /// depend on the forecaster's word. Absent in older ledgers, and not written
    /// when it is the default, so existing files stay byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<ResolvedBy>,
}

/// Provenance of a resolution. See [`Resolution::resolved_by`].
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ResolvedBy {
    /// The forecaster graded their own prediction.
    #[default]
    #[serde(rename = "self")]
    Zelf,
    /// Graded from an observed fact, with no human in the loop.
    Auto,
    /// Graded by someone other than the forecaster.
    Human,
}

/// One belief tracked over its lifetime.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Claim {
    pub id: String,
    pub statement: String,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_by: Option<NaiveDate>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Binary or numeric. Absent in older ledgers ⇒ defaults to binary.
    #[serde(default)]
    pub kind: ClaimKind,
    /// How much this call *matters* (≥ 0; default 1) — weights the Brier toward
    /// your consequential predictions. Absent in older ledgers ⇒ 1, and a
    /// default stake is not serialised, so existing ledgers stay byte-identical.
    #[serde(default = "default_stake", skip_serializing_if = "is_default_stake")]
    pub stake: f64,
    /// Every forecast you ever made, oldest first. The last is your current
    /// belief; the first is where you started.
    pub forecasts: Vec<Forecast>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
    /// Annulled: the question turned out to be ambiguous, or to be about
    /// something that never became knowable.
    ///
    /// A voided claim keeps its place in the history — nothing is ever deleted —
    /// but it is excluded from every score, exactly as a forecasting platform
    /// annuls a question rather than grading it. The alternative, hand-editing
    /// the JSON, is worse: it removes the evidence that the question was ever
    /// asked. Absent in older ledgers, and not serialised when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub void: Option<Void>,
    /// Corrections to the wording or tags, oldest first. Probabilities and
    /// timestamps are never amendable — that immutability is the entire point of
    /// the instrument — so only the parts that carry no forecast can change, and
    /// even those leave a record of what they were.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub amendments: Vec<Amendment>,
}

/// Why a claim was annulled, and when.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Void {
    pub at: DateTime<Utc>,
    pub reason: String,
}

/// One correction to a claim's wording or tags.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Amendment {
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_statement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_statement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_tags: Option<Vec<String>>,
}

fn default_stake() -> f64 {
    1.0
}

fn is_default_stake(s: &f64) -> bool {
    (*s - 1.0).abs() < f64::EPSILON
}

/// Weave the elicitation trail into the stored reasoning: the outside-view
/// *reference class*, the two *dialectical* estimates that produced the forecast,
/// and the free-text rationale — joined into one `because`. Empty pieces drop out;
/// returns `None` only when there is nothing at all to record. This keeps the CLI
/// and the MCP server recording provenance identically.
pub fn compose_reasoning(
    because: Option<&str>,
    reference_class: Option<&str>,
    estimates: Option<(f64, f64)>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(rc) = reference_class.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("outside view: {rc}"));
    }
    if let Some((p1, p2)) = estimates {
        parts.push(format!(
            "dialectical {p1:.2} & {p2:.2} → {:.2}",
            crate::scoring::dialectical_mean(p1, p2)
        ));
    }
    if let Some(b) = because.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(b.to_string());
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

impl Claim {
    pub fn current(&self) -> Option<&Forecast> {
        self.forecasts.last()
    }

    pub fn current_prob(&self) -> Option<f64> {
        self.forecasts.last().and_then(|f| f.prob)
    }

    pub fn first_prob(&self) -> Option<f64> {
        self.forecasts.first().and_then(|f| f.prob)
    }

    pub fn current_interval(&self) -> Option<NumericForecast> {
        self.forecasts.last().and_then(|f| f.interval)
    }

    pub fn is_resolved(&self) -> bool {
        self.resolution.is_some()
    }

    /// Annulled, and therefore excluded from every score.
    pub fn is_void(&self) -> bool {
        self.void.is_some()
    }

    /// Voided before the answer was known.
    ///
    /// This is the honest case, and the only one the sequential evidence test
    /// honours: a claim with no outcome carries no evidence, so removing it
    /// cannot move the e-value in any direction.
    pub fn voided_before_resolution(&self) -> bool {
        match (&self.void, &self.resolution) {
            (Some(v), Some(r)) => v.at <= r.at,
            (Some(_), None) => true,
            _ => false,
        }
    }

    /// Voided *after* it had already resolved — an outcome deleted from the
    /// record after being seen.
    ///
    /// Sometimes legitimate (the question really was ambiguous, and you only
    /// noticed once it resolved), but it retroactively edits a sequence whose
    /// whole guarantee rests on being fixed in advance, and the direction of
    /// abuse is self-flattery: you void the ones that went badly, and the e-value
    /// falls. Kept in the evidence sequence, and counted out loud.
    pub fn voided_after_resolution(&self) -> bool {
        match (&self.void, &self.resolution) {
            (Some(v), Some(r)) => v.at > r.at,
            _ => false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.resolution.is_none()
    }

    pub fn is_due(&self, today: NaiveDate) -> bool {
        self.is_open() && self.resolve_by.map(|d| d <= today).unwrap_or(false)
    }

    pub fn outcome(&self) -> Option<Outcome> {
        self.resolution.as_ref().and_then(|r| r.outcome)
    }

    pub fn value(&self) -> Option<f64> {
        self.resolution.as_ref().and_then(|r| r.value)
    }

    /// The scoreable binary sample: current probability paired with the resolved
    /// outcome. `None` unless this is a resolved binary claim.
    pub fn sample(&self) -> Option<Sample> {
        if self.kind != ClaimKind::Binary || self.is_void() {
            return None;
        }
        match (self.first_prob(), self.outcome()) {
            (Some(p), Some(o)) => Some(Sample::new(p, o.happened())),
            _ => None,
        }
    }

    /// The sample as the **sequential evidence test** sees it.
    ///
    /// Differs from [`Claim::sample`] in exactly one way: a claim voided *after*
    /// it resolved still counts here. Void annuls a question, and for every score
    /// in the report that is the right thing — but the evidence sequence's whole
    /// guarantee rests on being fixed before the outcomes are known, and dropping
    /// an outcome you have already seen edits it retroactively. So the scores
    /// forget it and the evidence test does not, and the report says how often
    /// that has happened.
    pub fn evidence_sample(&self) -> Option<Sample> {
        if self.kind != ClaimKind::Binary || self.voided_before_resolution() {
            return None;
        }
        match (self.first_prob(), self.outcome()) {
            (Some(p), Some(o)) => Some(Sample::new(p, o.happened())),
            _ => None,
        }
    }

    /// The same sample scored on the **final** forecast instead of the first.
    ///
    /// Shown, never graded. Scoring the final forecast is what let ten claims
    /// logged at 0.5, updated to 0.99 and then resolved YES earn a Brier score
    /// of 0.000 and the compliment "your updates moved you TOWARD the truth" —
    /// the exact self-deception this tool exists to prevent. The first forecast
    /// is the one made before the answer was known, so it is the one that counts.
    pub fn sample_final(&self) -> Option<Sample> {
        if self.kind != ClaimKind::Binary || self.is_void() {
            return None;
        }
        match (self.current_prob(), self.outcome()) {
            (Some(p), Some(o)) => Some(Sample::new(p, o.happened())),
            _ => None,
        }
    }

    /// Whether any forecast after the first landed too close to the answer to be
    /// an honest update: after the resolve-by date, within 24 hours of the
    /// resolution, or in the last 10% of the claim's window.
    ///
    /// This is a neutral flag, not an accusation — a genuine late update is
    /// perfectly legitimate. It exists so the report can say which forecasts are
    /// not being graded, instead of silently rewarding them.
    pub fn has_late_update(&self) -> bool {
        if self.forecasts.len() < 2 {
            return false;
        }
        let Some(res) = self.resolution.as_ref() else {
            return false;
        };
        let deadline = self
            .resolve_by
            .and_then(|d| d.and_hms_opt(23, 59, 59))
            .map(|dt| dt.and_utc());
        let window = (res.at - self.created_at).num_seconds().max(0) as f64;
        self.forecasts.iter().skip(1).any(|f| {
            if deadline.is_some_and(|d| f.at > d) {
                return true;
            }
            if (res.at - f.at).num_seconds() <= 86_400 {
                return true;
            }
            window > 0.0 && (f.at - self.created_at).num_seconds() as f64 > 0.9 * window
        })
    }

    /// Brier weighted by **how long each forecast stood**, the way Metaculus
    /// time-averages a question.
    ///
    /// The window runs from `created_at` to the earlier of the resolution and the
    /// end of the resolve-by day, so a revision made after the deadline carries
    /// no weight at all. Secondary, never the headline: someone who learns the
    /// answer early can still farm it by updating the moment they know.
    pub fn brier_time_averaged(&self) -> Option<f64> {
        if self.kind != ClaimKind::Binary || self.is_void() {
            return None;
        }
        let y = if self.outcome()?.happened() { 1.0 } else { 0.0 };
        let first = self.first_prob()?;
        let start = self.created_at;
        let mut end = self.resolution.as_ref()?.at;
        if let Some(d) = self.resolve_by {
            end = end.min(d.and_hms_opt(23, 59, 59)?.and_utc());
        }
        let total = (end - start).num_seconds();
        if total < 60 {
            // Degenerate or inverted window: nothing to weight by.
            return Some((first - y).powi(2));
        }
        let mut acc = 0.0;
        for (k, f) in self.forecasts.iter().enumerate() {
            let p = f.prob?;
            let from = if k == 0 {
                start
            } else {
                f.at.clamp(start, end)
            };
            let to = self
                .forecasts
                .get(k + 1)
                .map_or(end, |next| next.at)
                .clamp(from, end);
            acc += (p - y).powi(2) * (to - from).num_seconds() as f64;
        }
        Some(acc / total as f64)
    }

    /// The scoreable numeric sample: current interval paired with the resolved
    /// value. `None` unless this is a resolved numeric claim.
    pub fn numeric_sample(&self) -> Option<NumericSample> {
        if self.kind != ClaimKind::Numeric || self.is_void() {
            return None;
        }
        let iv = self.current_interval()?;
        let value = self.value()?;
        Some(NumericSample {
            low: iv.low,
            high: iv.high,
            level: iv.level,
            value,
        })
    }

    pub fn was_revised(&self) -> bool {
        self.forecasts.len() > 1
    }
}

/// The whole ledger.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Ledger {
    #[serde(default)]
    pub claims: Vec<Claim>,
}

impl Ledger {
    /// Resolve a (possibly abbreviated) id to a single claim index.
    pub fn index_of(&self, id_prefix: &str) -> Result<usize, String> {
        if let Some(i) = self.claims.iter().position(|c| c.id == id_prefix) {
            return Ok(i);
        }
        let hits: Vec<usize> = self
            .claims
            .iter()
            .enumerate()
            .filter(|(_, c)| c.id.starts_with(id_prefix))
            .map(|(i, _)| i)
            .collect();
        match hits.len() {
            0 => Err(format!("no claim matches id '{id_prefix}'")),
            1 => Ok(hits[0]),
            n => Err(format!("ambiguous id '{id_prefix}' matches {n} claims")),
        }
    }

    pub fn has_id(&self, id: &str) -> bool {
        self.claims.iter().any(|c| c.id == id)
    }

    pub fn resolved_samples(&self) -> Vec<Sample> {
        self.claims.iter().filter_map(|c| c.sample()).collect()
    }
}

/// Derive a short, stable, dependency-free id from a statement and a salt.
pub fn gen_id(statement: &str, salt: u64) -> String {
    let mut h = DefaultHasher::new();
    statement.hash(&mut h);
    salt.hash(&mut h);
    format!("{:06x}", h.finish() & 0xff_ffff)
}

/// Normalise tags: trim, lower-case, drop empties, de-duplicate (order kept).
pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t = t.trim().to_lowercase();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    fn binary_forecast(p: f64) -> Forecast {
        Forecast {
            at: ts(2025, 1, 1),
            prob: Some(p),
            interval: None,
            because: None,
        }
    }

    fn claim(id: &str, probs: &[f64], outcome: Option<Outcome>) -> Claim {
        Claim {
            id: id.to_string(),
            statement: format!("claim {id}"),
            created_at: ts(2025, 1, 1),
            resolve_by: None,
            tags: vec![],
            kind: ClaimKind::Binary,
            stake: 1.0,
            forecasts: probs.iter().map(|&p| binary_forecast(p)).collect(),
            resolution: outcome.map(|o| Resolution {
                at: ts(2025, 6, 1),
                outcome: Some(o),
                value: None,
                note: None,
                resolved_by: None,
            }),
            void: None,
            amendments: Vec::new(),
        }
    }

    #[test]
    fn sample_only_when_resolved_binary() {
        assert!(claim("a", &[0.7], None).sample().is_none());
        let s = claim("b", &[0.7], Some(Outcome::True)).sample().unwrap();
        assert_eq!(s, Sample::new(0.7, true));
    }

    #[test]
    fn numeric_sample_round_trips() {
        let c = Claim {
            id: "n".into(),
            statement: "how many".into(),
            created_at: ts(2025, 1, 1),
            resolve_by: None,
            tags: vec![],
            kind: ClaimKind::Numeric,
            stake: 1.0,
            forecasts: vec![Forecast {
                at: ts(2025, 1, 1),
                prob: None,
                interval: Some(NumericForecast {
                    low: 10.0,
                    high: 20.0,
                    level: 0.8,
                }),
                because: None,
            }],
            resolution: Some(Resolution {
                at: ts(2025, 6, 1),
                outcome: None,
                value: Some(13.0),
                note: None,
                resolved_by: None,
            }),
            void: None,
            amendments: Vec::new(),
        };
        assert!(c.sample().is_none()); // not a binary sample
        let ns = c.numeric_sample().unwrap();
        assert_eq!(
            (ns.low, ns.high, ns.level, ns.value),
            (10.0, 20.0, 0.8, 13.0)
        );
    }

    #[test]
    fn current_is_latest_forecast() {
        let c = claim("c", &[0.6, 0.8, 0.3], None);
        assert_eq!(c.first_prob(), Some(0.6));
        assert_eq!(c.current_prob(), Some(0.3));
        assert!(c.was_revised());
    }

    #[test]
    fn index_of_handles_prefix_and_ambiguity() {
        let ledger = Ledger {
            claims: vec![
                claim("ab12cd", &[0.5], None),
                claim("ab99ff", &[0.5], None),
                claim("ff0000", &[0.5], None),
            ],
        };
        assert_eq!(ledger.index_of("ff0000").unwrap(), 2);
        assert_eq!(ledger.index_of("ff").unwrap(), 2);
        assert!(ledger.index_of("ab").is_err());
        assert!(ledger.index_of("zz").is_err());
    }

    #[test]
    fn exact_id_beats_prefix_collision() {
        let ledger = Ledger {
            claims: vec![claim("ab", &[0.5], None), claim("ab12cd", &[0.5], None)],
        };
        assert_eq!(ledger.index_of("ab").unwrap(), 0);
    }

    #[test]
    fn tags_normalised() {
        let got = normalize_tags(&[
            " Politics ".into(),
            "politics".into(),
            "".into(),
            "TECH".into(),
        ]);
        assert_eq!(got, vec!["politics".to_string(), "tech".to_string()]);
    }

    #[test]
    fn due_logic() {
        let mut c = claim("d", &[0.5], None);
        c.resolve_by = Some(NaiveDate::from_ymd_opt(2025, 6, 1).unwrap());
        assert!(c.is_due(NaiveDate::from_ymd_opt(2025, 7, 1).unwrap()));
        assert!(!c.is_due(NaiveDate::from_ymd_opt(2025, 5, 1).unwrap()));
    }

    #[test]
    fn legacy_binary_json_still_loads() {
        // A ledger written before numeric claims existed: no `kind`, bare `prob`,
        // string `outcome`. It must load as a binary claim, unchanged.
        let legacy = r#"{
            "claims": [{
                "id": "abc123",
                "statement": "old belief",
                "created_at": "2025-01-01T12:00:00Z",
                "tags": ["legacy"],
                "forecasts": [{ "at": "2025-01-01T12:00:00Z", "prob": 0.7, "because": "why" }],
                "resolution": { "at": "2025-06-01T12:00:00Z", "outcome": "true", "note": "n" }
            }]
        }"#;
        let ledger: Ledger = serde_json::from_str(legacy).unwrap();
        let c = &ledger.claims[0];
        assert_eq!(c.kind, ClaimKind::Binary);
        assert_eq!(c.current_prob(), Some(0.7));
        assert_eq!(c.outcome(), Some(Outcome::True));
        assert_eq!(c.sample(), Some(Sample::new(0.7, true)));
        // A ledger predating stakes loads with the default stake of 1.0…
        assert_eq!(c.stake, 1.0);
        // …and a default-stake claim does not re-introduce the field on save.
        // Neither void nor amendments existed either, and a claim that has
        // neither must not grow the fields on save — an old ledger stays
        // byte-identical through a load/save round trip.
        assert!(!c.is_void());
        assert!(c.amendments.is_empty());
        let round = serde_json::to_string(&ledger).unwrap();
        assert!(
            !round.contains("void"),
            "absent void must not be serialised"
        );
        assert!(
            !round.contains("amendments"),
            "an empty amendment list must not be serialised"
        );
        assert!(
            !round.contains("stake"),
            "default stake must not be serialised"
        );
    }

    #[test]
    fn compose_reasoning_weaves_the_elicitation_trail() {
        let r = compose_reasoning(
            Some("looks clean"),
            Some("similar PRs pass 60%"),
            Some((0.7, 0.5)),
        )
        .unwrap();
        assert!(r.contains("outside view: similar PRs pass 60%"));
        assert!(r.contains("dialectical 0.70 & 0.50 → 0.60"));
        assert!(r.contains("looks clean"));
        // A single piece comes back bare; nothing-to-record is None.
        assert_eq!(compose_reasoning(Some("x"), None, None).unwrap(), "x");
        assert!(compose_reasoning(None, None, None).is_none());
        assert!(compose_reasoning(Some("  "), Some(""), None).is_none());
    }
}
