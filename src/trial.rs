//! The weekly ON/OFF crossover of `docs/MEASUREMENT.md`, addendum 4.
//!
//! `ana trial start` writes `trial.json` beside the agent ledger once: a start Monday and,
//! for each pair of weeks, a seeded draw of which week is ON. In an OFF week every hook
//! that speaks to the model stays silent; the grader, the ledger and the outcome log keep
//! working. Outside the trial's weeks, or with no trial at all, everything is ON, so a
//! machine that never enrols behaves exactly as before.
//!
//! Weeks run Monday to Sunday by the UTC date, the clock every hook already uses.

use chrono::{Datelike, Duration, NaiveDate, Utc, Weekday};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The protocol says the trial starts no earlier than this Monday.
pub const EARLIEST: (i32, u32, u32) = (2027, 1, 4);
/// Thirteen pairs of weeks, the number the addendum's power figures are for.
pub const DEFAULT_PAIRS: u32 = 13;

/// Which half of a pair a week is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arm {
    On,
    Off,
}

impl Arm {
    pub fn label(self) -> &'static str {
        match self {
            Arm::On => "on",
            Arm::Off => "off",
        }
    }
}

/// What `trial.json` holds. Written once; nothing rewrites it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Trial {
    pub v: u32,
    pub id: String,
    /// The draw's seed, so anyone can recompute `on_first` from it.
    pub seed: u64,
    /// A Monday.
    pub start: NaiveDate,
    /// One entry per pair: true when the pair's first week is ON.
    pub on_first: Vec<bool>,
    pub protocol: String,
    pub engine: String,
    pub created_at: String,
}

/// `trial.json` beside the ledger.
pub fn path(ledger_path: &Path) -> PathBuf {
    ledger_path.with_file_name("trial.json")
}

/// Whether pair `i` of a trial seeded `seed` opens with its ON week. SplitMix64 of
/// seed and index: reproducible, and fixed before the first week.
pub fn draw(seed: u64, i: u32) -> bool {
    let mut z = seed.wrapping_add(
        u64::from(i)
            .wrapping_add(1)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15),
    );
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    z & 1 == 1
}

impl Trial {
    /// The day after the last week.
    pub fn end(&self) -> NaiveDate {
        self.start + Duration::days(14 * self.on_first.len() as i64)
    }

    /// The arm of `day`, or `None` outside the trial's weeks.
    pub fn arm_on(&self, day: NaiveDate) -> Option<Arm> {
        if day < self.start || day >= self.end() {
            return None;
        }
        let week = (day - self.start).num_days() / 7;
        let first = *self.on_first.get((week / 2) as usize)?;
        let on = (week % 2 == 0) == first;
        Some(if on { Arm::On } else { Arm::Off })
    }

    /// Which week of the trial `day` is in, counting from 1.
    pub fn week_of(&self, day: NaiveDate) -> Option<i64> {
        self.arm_on(day)
            .map(|_| (day - self.start).num_days() / 7 + 1)
    }
}

/// The enrolled trial, if there is a readable one. An unreadable file reads as no trial,
/// so a broken `trial.json` can only leave the tool ON, never silence it.
pub fn load(ledger_path: &Path) -> Option<Trial> {
    let text = std::fs::read_to_string(path(ledger_path)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The arm in force today, `None` when no trial covers today.
pub fn arm_today(ledger_path: &Path) -> Option<Arm> {
    load(ledger_path)?.arm_on(Utc::now().date_naive())
}

/// Whether hooks may speak to the model today: false only inside an OFF week.
pub fn speaks(ledger_path: &Path) -> bool {
    arm_today(ledger_path) != Some(Arm::Off)
}

fn earliest() -> NaiveDate {
    NaiveDate::from_ymd_opt(EARLIEST.0, EARLIEST.1, EARLIEST.2).expect("a real date")
}

/// The first Monday after `today` that the protocol allows.
pub fn default_start(today: NaiveDate) -> NaiveDate {
    let mut d = (today + Duration::days(1)).max(earliest());
    while d.weekday() != Weekday::Mon {
        d += Duration::days(1);
    }
    d
}

/// Enrol: check the start, draw the schedule, and write `trial.json` once, under the
/// ledger's lock. Refuses when a trial already exists.
pub fn start(
    ledger_path: &Path,
    from: Option<NaiveDate>,
    pairs: u32,
    today: NaiveDate,
) -> Result<Trial, String> {
    let start = from.unwrap_or_else(|| default_start(today));
    if start.weekday() != Weekday::Mon {
        return Err(format!(
            "the trial starts on a Monday; {start} is a {}",
            start.weekday()
        ));
    }
    if start < earliest() {
        return Err(format!(
            "the protocol (docs/MEASUREMENT.md, addendum 4) starts the trial no earlier than {}",
            earliest()
        ));
    }
    if start <= today {
        return Err(format!(
            "{start} is not in the future: the schedule must be fixed before its first week"
        ));
    }
    if !(1..=52).contains(&pairs) {
        return Err("--pairs is between 1 and 52".into());
    }
    let _guard = crate::store::lock(ledger_path)
        .map_err(|e| format!("locking {}: {e}", ledger_path.display()))?;
    let file = path(ledger_path);
    if file.exists() {
        return Err(format!(
            "a trial is already enrolled ({}); it is written once and never rewritten",
            file.display()
        ));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seed = now ^ (u64::from(std::process::id()) << 32);
    let trial = Trial {
        v: 1,
        id: format!("{:012x}", seed & 0xffff_ffff_ffff),
        seed,
        start,
        on_first: (0..pairs).map(|i| draw(seed, i)).collect(),
        protocol: "docs/MEASUREMENT.md, addendum 4".into(),
        engine: env!("CARGO_PKG_VERSION").into(),
        created_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    };
    let text = serde_json::to_string_pretty(&trial).map_err(|e| e.to_string())?;
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file)
        .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()))
        .map_err(|e| format!("writing {}: {e}", file.display()))?;
    Ok(trial)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn trial(on_first: Vec<bool>) -> Trial {
        Trial {
            v: 1,
            id: "t".into(),
            seed: 1,
            start: d(2027, 1, 4),
            on_first,
            protocol: String::new(),
            engine: String::new(),
            created_at: String::new(),
        }
    }

    #[test]
    fn each_pair_has_one_on_week_and_one_off_week() {
        let t = trial(vec![true, false]);
        assert_eq!(t.arm_on(d(2027, 1, 3)), None, "before the start");
        assert_eq!(t.arm_on(d(2027, 1, 4)), Some(Arm::On));
        assert_eq!(
            t.arm_on(d(2027, 1, 10)),
            Some(Arm::On),
            "Sunday is the same week"
        );
        assert_eq!(t.arm_on(d(2027, 1, 11)), Some(Arm::Off));
        assert_eq!(t.arm_on(d(2027, 1, 18)), Some(Arm::Off), "pair 2 opens OFF");
        assert_eq!(t.arm_on(d(2027, 1, 25)), Some(Arm::On));
        assert_eq!(t.arm_on(d(2027, 2, 1)), None, "after the end");
        assert_eq!(t.end(), d(2027, 2, 1));
        assert_eq!(t.week_of(d(2027, 1, 25)), Some(4));
    }

    #[test]
    fn the_draw_is_reproducible_and_not_constant() {
        let a: Vec<bool> = (0..13).map(|i| draw(42, i)).collect();
        let b: Vec<bool> = (0..13).map(|i| draw(42, i)).collect();
        assert_eq!(a, b);
        assert!(a.iter().any(|x| *x) && a.iter().any(|x| !*x), "{a:?}");
    }

    #[test]
    fn the_start_is_a_future_monday_no_earlier_than_the_protocol_allows() {
        assert_eq!(default_start(d(2026, 11, 7)), d(2027, 1, 4));
        assert_eq!(default_start(d(2027, 1, 4)), d(2027, 1, 11));
        assert_eq!(default_start(d(2027, 2, 3)), d(2027, 2, 8));
        let dir = std::env::temp_dir().join(format!("ana_trial_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ledger = dir.join("agent.json");
        let today = d(2026, 11, 7);
        assert!(start(&ledger, Some(d(2027, 1, 5)), 13, today)
            .unwrap_err()
            .contains("Monday"));
        assert!(start(&ledger, Some(d(2026, 12, 28)), 13, today)
            .unwrap_err()
            .contains("no earlier"));
        let t = start(&ledger, None, 13, today).unwrap();
        assert_eq!(t.start, d(2027, 1, 4));
        assert_eq!(t.on_first.len(), 13);
        assert_eq!(load(&ledger), Some(t));
        assert!(start(&ledger, None, 13, today)
            .unwrap_err()
            .contains("already enrolled"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
