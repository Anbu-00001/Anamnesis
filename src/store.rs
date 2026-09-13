//! Persistence. The ledger lives as a single human-readable JSON file: greppable,
//! diffable, git-friendly, and intelligible without this program. A record of
//! your own judgement should never be trapped in a format only one tool can read.

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::model::Ledger;

fn invalid_data(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, e.to_string())
}

/// An exclusive advisory lock on a ledger, held for the whole read-modify-write
/// of a mutating command. Dropping it releases the lock.
///
/// Without this, two concurrent `ana add` calls both load the same ledger, each
/// appends its own claim, and the second `rename` silently discards the first
/// claim. Measured before this existed: 40 parallel adds left 11-19 claims.
#[derive(Debug)]
pub struct LedgerLock {
    _file: File,
}

/// `<path><suffix>` — appended to the *whole* file name, so `a.json` yields
/// `a.json.lock`, not `a.lock` (which `with_extension` would produce and which
/// would collide with a sibling ledger named `a.lock`).
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// Take an exclusive lock on `<ledger>.lock`, blocking until it is free.
///
/// The lock lives on a sidecar file that is never renamed or deleted, because
/// `save` replaces the ledger itself by rename — a lock held on the ledger inode
/// would be silently orphaned by the very write it is meant to guard.
///
/// `File::lock` is std since Rust 1.89 (`flock` on Unix, `LockFileEx` on
/// Windows), so this costs no dependency.
pub fn lock(path: &Path) -> io::Result<LedgerLock> {
    fs::create_dir_all(parent_dir(path))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(sidecar(path, ".lock"))?;
    file.lock()?;
    Ok(LedgerLock { _file: file })
}

/// Path of the backup `save` keeps of the last good ledger.
pub fn backup_path(path: &Path) -> PathBuf {
    sidecar(path, ".bak")
}

/// Path of the sidecar lock file.
pub fn lock_path(path: &Path) -> PathBuf {
    sidecar(path, ".lock")
}

/// Load a ledger. A missing file is treated as an empty ledger, so the very
/// first `add` just works without any `init` ceremony.
pub fn load(path: &Path) -> io::Result<Ledger> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Ledger::default()),
        Err(e) => return Err(e),
    };
    if text.trim().is_empty() {
        // An empty file holds no claims, so reading it as an empty ledger loses
        // nothing, and `touch ~/.anamnesis.json` should not greet a new user with
        // "EOF while parsing a value". Unless a backup sits beside it: then this
        // is more likely a ledger that was emptied than one never written, and the
        // next save would copy the empty file over the only good copy.
        if backup_path(path).exists() {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "the ledger file is empty, but a backup of an earlier ledger sits beside it; refusing to start a new ledger over it",
            ));
        }
        return Ok(Ledger::default());
    }
    let ledger: Ledger = serde_json::from_str(&text).map_err(invalid_data)?;
    validate(&ledger).map_err(|m| io::Error::new(ErrorKind::InvalidData, m))?;
    Ok(ledger)
}

/// Refuse a ledger whose numbers cannot be scored.
///
/// The CLI never writes these, but a hand-edited file can hold them, and scoring
/// them does not fail: it quietly returns a Brier score and a confident verdict
/// computed from a probability of 1.7. Measured before this check, a ledger with
/// two such forecasts was reported as `OVERCONFIDENT`.
fn validate(ledger: &Ledger) -> Result<(), String> {
    for c in &ledger.claims {
        for (i, f) in c.forecasts.iter().enumerate() {
            let n = i + 1;
            if let Some(p) = f.prob {
                if !(0.0..=1.0).contains(&p) {
                    return Err(format!(
                        "claim [{}] forecast {n}: probability {p} is outside 0..1",
                        c.id
                    ));
                }
            }
            if let Some(iv) = f.interval {
                if iv.low > iv.high {
                    return Err(format!(
                        "claim [{}] forecast {n}: interval {}..{} has its low end above its high end",
                        c.id, iv.low, iv.high
                    ));
                }
                if iv.level <= 0.0 || iv.level >= 1.0 {
                    return Err(format!(
                        "claim [{}] forecast {n}: level {} is not strictly between 0 and 1",
                        c.id, iv.level
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Save a ledger durably: write to a *uniquely named* temp file in the same
/// directory, fsync it, keep a `.bak` copy of the last good ledger, then rename
/// over the target and fsync the directory.
///
/// Two details matter. The temp name carries pid and nanos, because a single
/// shared `<ledger>.json.tmp` means two concurrent writers scribble over one
/// another's half-written file before either renames. And `sync_all` before the
/// rename is what makes "a crash leaves the previous ledger intact" true on a
/// real filesystem rather than only in program order.
pub fn save(path: &Path, ledger: &Ledger) -> io::Result<()> {
    let dir = parent_dir(path).to_path_buf();
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(ledger).map_err(invalid_data)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("ledger");
    let tmp = dir.join(format!(".{name}.{}.{nanos}.tmp", std::process::id()));
    {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        // Clean the temp file up if the write or fsync fails, rather than
        // leaving litter next to the ledger.
        if let Err(e) = f.write_all(&json).and_then(|()| f.sync_all()) {
            drop(f);
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    if path.exists() {
        let _ = fs::copy(path, backup_path(path));
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // Durability of the rename itself. Unix only: Windows has no directory
    // handle to sync, and NTFS orders the metadata write for us.
    #[cfg(unix)]
    File::open(&dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Claim, ClaimKind, Forecast, Outcome, Resolution};
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
                horizon_days: None,
                resolve_by: Some(chrono::NaiveDate::from_ymd_opt(2025, 4, 3).unwrap()),
                tags: vec!["weather".into()],
                kind: ClaimKind::Binary,
                stake: 1.0,
                forecasts: vec![
                    Forecast {
                        at: now,
                        prob: Some(0.4),
                        interval: None,
                        because: Some("dry front".into()),
                    },
                    Forecast {
                        at: now,
                        prob: Some(0.7),
                        interval: None,
                        because: Some("front stalled".into()),
                    },
                ],
                resolution: Some(Resolution {
                    at: now,
                    outcome: Some(Outcome::True),
                    value: None,
                    note: Some("the front stalled, as feared".into()),
                    resolved_by: None,
                }),
                void: None,
                amendments: Vec::new(),
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
