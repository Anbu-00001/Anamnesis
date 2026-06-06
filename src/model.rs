//! The domain model: claims, the forecasts layered onto them over time, and
//! their eventual resolution. A [`Claim`] is deliberately a *palimpsest* — each
//! revision of your probability is appended, never overwritten, so the history
//! of your own mind-changing survives intact.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::scoring::Sample;

/// What actually happened, once a claim resolves. Binary by design: forcing a
/// yes/no keeps the scoring honest and the math proper.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// The event happened / the statement turned out true.
    True,
    /// The event did not happen / the statement turned out false.
    False,
}

impl Outcome {
    pub fn happened(self) -> bool {
        matches!(self, Outcome::True)
    }
}

/// A single dated probability assignment, with the reasoning that produced it.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Forecast {
    /// When this probability was recorded.
    pub at: DateTime<Utc>,
    /// The probability assigned to the claim being true, in `[0, 1]`.
    pub prob: f64,
    /// The reasoning behind it — the part hindsight will try to erase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub because: Option<String>,
}

/// The verdict of reality.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Resolution {
    /// When the claim was resolved.
    pub at: DateTime<Utc>,
    /// True or false.
    pub outcome: Outcome,
    /// An optional post-mortem: what, with hindsight, you now think you missed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One belief tracked over its lifetime.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Claim {
    /// Short stable id (6 hex chars), unique within a ledger.
    pub id: String,
    /// The falsifiable statement, phrased so it can be scored true or false.
    pub statement: String,
    /// When the claim was first recorded.
    pub created_at: DateTime<Utc>,
    /// Optional date by which you expect to know the answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolve_by: Option<NaiveDate>,
    /// Free-form tags (lower-cased) for slicing the report by domain.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Every probability you ever assigned, in chronological order. The last is
    /// your current belief; the first is where you started.
    pub forecasts: Vec<Forecast>,
    /// Present once reality has spoken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
}

impl Claim {
    /// Your current (latest) forecast.
    pub fn current(&self) -> Option<&Forecast> {
        self.forecasts.last()
    }

    pub fn current_prob(&self) -> Option<f64> {
        self.forecasts.last().map(|f| f.prob)
    }

    pub fn first_prob(&self) -> Option<f64> {
        self.forecasts.first().map(|f| f.prob)
    }

    pub fn is_resolved(&self) -> bool {
        self.resolution.is_some()
    }

    pub fn is_open(&self) -> bool {
        self.resolution.is_none()
    }

    /// True when the claim is still open and its expected-by date has passed.
    pub fn is_due(&self, today: NaiveDate) -> bool {
        self.is_open() && self.resolve_by.map(|d| d <= today).unwrap_or(false)
    }

    pub fn outcome(&self) -> Option<Outcome> {
        self.resolution.as_ref().map(|r| r.outcome)
    }

    /// The scoreable sample for this claim: current probability paired with the
    /// resolved outcome. `None` while the claim is open.
    pub fn sample(&self) -> Option<Sample> {
        match (self.current_prob(), self.outcome()) {
            (Some(p), Some(o)) => Some(Sample::new(p, o.happened())),
            _ => None,
        }
    }

    /// True if you ever revised your probability for this claim.
    pub fn was_revised(&self) -> bool {
        self.forecasts.len() > 1
    }
}

/// The whole ledger: every claim you have ever recorded.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Ledger {
    #[serde(default)]
    pub claims: Vec<Claim>,
}

impl Ledger {
    /// Resolve a (possibly abbreviated) id to a single claim index. Returns a
    /// human-readable error on no match or an ambiguous prefix.
    pub fn index_of(&self, id_prefix: &str) -> Result<usize, String> {
        // Exact match wins outright, even if it is a prefix of another id.
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

    /// Whether any claim already uses this id.
    pub fn has_id(&self, id: &str) -> bool {
        self.claims.iter().any(|c| c.id == id)
    }

    /// All resolved claims as scoreable samples (current forecast vs outcome).
    pub fn resolved_samples(&self) -> Vec<Sample> {
        self.claims.iter().filter_map(|c| c.sample()).collect()
    }
}

/// Derive a short, stable, dependency-free id from a statement and a salt.
/// Collisions are resolved by the caller bumping the salt.
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

    fn claim(id: &str, probs: &[f64], outcome: Option<Outcome>) -> Claim {
        Claim {
            id: id.to_string(),
            statement: format!("claim {id}"),
            created_at: ts(2025, 1, 1),
            resolve_by: None,
            tags: vec![],
            forecasts: probs
                .iter()
                .map(|&p| Forecast {
                    at: ts(2025, 1, 1),
                    prob: p,
                    because: None,
                })
                .collect(),
            resolution: outcome.map(|o| Resolution {
                at: ts(2025, 6, 1),
                outcome: o,
                note: None,
            }),
        }
    }

    #[test]
    fn sample_only_when_resolved() {
        assert!(claim("a", &[0.7], None).sample().is_none());
        let s = claim("b", &[0.7], Some(Outcome::True)).sample().unwrap();
        assert_eq!(s, Sample::new(0.7, true));
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
        assert_eq!(ledger.index_of("ff").unwrap(), 2); // unique prefix
        assert!(ledger.index_of("ab").is_err()); // ambiguous
        assert!(ledger.index_of("zz").is_err()); // missing
    }

    #[test]
    fn exact_id_beats_prefix_collision() {
        // "ab" is an exact id AND a prefix of "ab12cd"; exact match must win.
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
}
