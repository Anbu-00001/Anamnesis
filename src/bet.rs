//! The person bets on the agent: `?70`, typed as a whole prompt, is a 70% forecast that
//! the agent's first graded test run after an edit, from its next turn on, passes.
//!
//! The UserPromptSubmit hook blocks that prompt, so it never reaches the model (Claude
//! Code: a blocked prompt "never reaches Claude", and the block's `reason` is "shown to
//! the user ... not added to context"), and asks for the typed text to be left out of the
//! block message. The post-tool hook grades the bet from the exit status, the same way it
//! grades the agent's own claims, and tells the person only (`systemMessage`).
//!
//! Bets live in `bets.json` beside the agent ledger, not in it: the agent's own ledger is
//! what its hooks, `ana list` and the MCP tools read, and a bet the agent could see would
//! be a bet that changes what it is about. `ana --data ~/.anamnesis/bets.json report`
//! reads them.

use crate::model::{Claim, ClaimKind, Forecast, Outcome, Resolution, ResolvedBy};
use crate::store;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// `bets.json` beside the ledger.
pub fn path(ledger_path: &Path) -> PathBuf {
    ledger_path.with_file_name("bets.json")
}

/// What the statement of every bet says, so a report of them reads plainly.
pub const STATEMENT: &str =
    "the agent's first graded test run after an edit, from its next turn on, passes";

/// `Some(Ok(p))` for a bet, `Some(Err(why))` for something that looks like one but is not
/// a usable probability, `None` for an ordinary prompt. A bet is the whole prompt: `?70`,
/// `?70%`, `? 70` or `?.7`/`?0.7`.
pub fn parse(prompt: &str) -> Option<Result<f64, String>> {
    let rest = prompt.trim().strip_prefix('?')?.trim();
    if !rest.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let (num, pct) = match rest.strip_suffix('%') {
        Some(n) => (n.trim(), true),
        None => (rest, false),
    };
    if !num.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let x: f64 = match num.parse() {
        Ok(x) => x,
        Err(_) => return Some(Err(format!("`?{rest}` is not a number"))),
    };
    let p = if pct || x > 1.0 { x / 100.0 } else { x };
    if !(p > 0.0 && p < 1.0) {
        return Some(Err(format!(
            "a bet is a chance between 1 and 99 (`?70` or `?0.7`); `?{rest}` is not"
        )));
    }
    Some(Ok(p))
}

fn bet_of(c: &Claim) -> Option<(&str, u64)> {
    let b = c.extra.get("bet")?;
    Some((
        b.get("session")?.as_str()?,
        b.get("after_prompt")?.as_u64()?,
    ))
}

/// Record a bet made at prompt number `after_prompt` of `session`. Returns its id.
pub fn record(
    ledger_path: &Path,
    session: &str,
    after_prompt: u64,
    project: &str,
    p: f64,
) -> Result<String, String> {
    let file = path(ledger_path);
    let _guard = store::lock_within(&file, std::time::Duration::from_millis(1500))
        .map_err(|e| e.to_string())?;
    let mut ledger = store::load(&file).map_err(|e| e.to_string())?;
    let now = Utc::now();
    let tags = vec![
        "who:human".to_string(),
        "kind:agent-bet".to_string(),
        format!("project:{project}"),
    ];
    let mut salt = now.timestamp_nanos_opt().unwrap_or(0) as u64;
    let mut id = crate::gen_id(STATEMENT, salt);
    while ledger.has_id(&id) {
        salt = salt.wrapping_add(1);
        id = crate::gen_id(STATEMENT, salt);
    }
    let mut extra = crate::model::Extra::new();
    extra.insert(
        "bet".into(),
        json!({ "session": session, "after_prompt": after_prompt }),
    );
    ledger.claims.push(Claim {
        id: id.clone(),
        statement: STATEMENT.into(),
        created_at: now,
        resolve_by: Some((now + Duration::days(7)).date_naive()),
        horizon_days: Some(crate::evidence::horizon_for(&tags)),
        tags,
        kind: ClaimKind::Binary,
        stake: 1.0,
        forecasts: vec![Forecast {
            at: now,
            prob: Some(p),
            interval: None,
            because: None,
        }],
        resolution: None,
        void: None,
        check: None,
        extra,
        amendments: Vec::new(),
    });
    store::save(&file, &ledger).map_err(|e| e.to_string())?;
    Ok(id)
}

/// Grade every open bet of `session` made before prompt `prompt_now` from a test run that
/// came after an edit. Returns each graded bet's id and stated chance.
pub fn grade(
    ledger_path: &Path,
    session: &str,
    prompt_now: u64,
    exit_code: i64,
) -> Result<Vec<(String, f64)>, String> {
    let file = path(ledger_path);
    if !file.exists() {
        return Ok(Vec::new());
    }
    let _guard = store::lock_within(&file, std::time::Duration::from_millis(1500))
        .map_err(|e| e.to_string())?;
    let mut ledger = store::load(&file).map_err(|e| e.to_string())?;
    let now = Utc::now();
    let mut done = Vec::new();
    for c in ledger.claims.iter_mut() {
        if !c.is_open() || c.is_void() {
            continue;
        }
        let Some((s, after)) = bet_of(c) else {
            continue;
        };
        if s != session || prompt_now <= after {
            continue;
        }
        c.resolution = Some(Resolution {
            at: now,
            outcome: Some(if exit_code == 0 {
                Outcome::True
            } else {
                Outcome::False
            }),
            value: None,
            note: Some(format!("auto: a test run after an edit exited {exit_code}")),
            resolved_by: Some(ResolvedBy::Auto),
        });
        done.push((c.id.clone(), c.forecasts[0].prob.unwrap_or(0.5)));
    }
    if !done.is_empty() {
        store::save(&file, &ledger).map_err(|e| e.to_string())?;
    }
    Ok(done)
}

/// What the person is told when a prompt was a bet. The model is told nothing.
pub fn block(reason: String) -> Value {
    json!({
        "decision": "block",
        "reason": reason,
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "suppressOriginalPrompt": true
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_whole_prompt_of_a_chance_is_a_bet() {
        assert_eq!(parse("?70"), Some(Ok(0.7)));
        assert_eq!(parse("  ?70% "), Some(Ok(0.7)));
        assert_eq!(parse("? 0.7"), Some(Ok(0.7)));
        assert_eq!(parse("?.25"), Some(Ok(0.25)));
        assert!(matches!(parse("?100"), Some(Err(_))));
        assert!(matches!(parse("?0"), Some(Err(_))));
        assert!(matches!(parse("?1.2.3"), Some(Err(_))));
        assert_eq!(parse("?"), None);
        assert_eq!(parse("? why does this fail"), None);
        assert_eq!(parse("?70 and also fix the bug"), None);
        assert_eq!(parse("what is ?70"), None);
    }
}
