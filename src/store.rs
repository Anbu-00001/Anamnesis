//! Persistence. The ledger lives as a single human-readable JSON file: greppable,
//! diffable, git-friendly, and intelligible without this program. A record of
//! your own judgement should never be trapped in a format only one tool can read.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

use crate::model::Ledger;

fn invalid_data(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, e.to_string())
}

/// Load a ledger. A missing file is treated as an empty ledger, so the very
/// first `add` just works without any `init` ceremony.
pub fn load(path: &Path) -> io::Result<Ledger> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(invalid_data),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Ledger::default()),
        Err(e) => Err(e),
    }
}

/// Save a ledger atomically: write to a sibling temp file, then rename over the
/// target. A crash mid-write leaves the previous ledger intact rather than a
/// half-written one.
pub fn save(path: &Path, ledger: &Ledger) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let json = serde_json::to_string_pretty(ledger).map_err(invalid_data)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json.as_bytes())?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Claim, Forecast, Outcome, Resolution};
    use chrono::{TimeZone, Utc};

    #[test]
    fn missing_file_is_empty_ledger() {
        let p = std::env::temp_dir().join("anamnesis_does_not_exist_xyz.json");
        let _ = fs::remove_file(&p);
        let l = load(&p).unwrap();
        assert!(l.claims.is_empty());
    }

    #[test]
    fn round_trip_preserves_everything() {
        let dir = std::env::temp_dir().join(format!("anamnesis_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ledger.json");

        let now = Utc.with_ymd_and_hms(2025, 4, 2, 9, 30, 0).unwrap();
        let ledger = Ledger {
            claims: vec![Claim {
                id: "abc123".into(),
                statement: "It will rain tomorrow".into(),
                created_at: now,
                resolve_by: Some(chrono::NaiveDate::from_ymd_opt(2025, 4, 3).unwrap()),
                tags: vec!["weather".into()],
                forecasts: vec![
                    Forecast { at: now, prob: 0.4, because: Some("dry front".into()) },
                    Forecast { at: now, prob: 0.7, because: Some("front stalled".into()) },
                ],
                resolution: Some(Resolution {
                    at: now,
                    outcome: Outcome::True,
                    note: Some("the front stalled, as feared".into()),
                }),
            }],
        };

        save(&path, &ledger).unwrap();
        let back = load(&path).unwrap();
        assert_eq!(back.claims.len(), 1);
        let c = &back.claims[0];
        assert_eq!(c.id, "abc123");
        assert_eq!(c.forecasts.len(), 2);
        assert_eq!(c.current_prob(), Some(0.7));
        assert_eq!(c.outcome(), Some(Outcome::True));
        assert_eq!(c.tags, vec!["weather".to_string()]);

        let _ = fs::remove_dir_all(&dir);
    }
}
