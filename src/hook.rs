//! `ana hook <event>` — the Claude Code hooks, implemented in the binary.
//!
//! These used to be three shell scripts, each re-deriving the calibration
//! verdict from `--json` output with its own `jq` expression. That meant three
//! more copies of the logic P0-5 exists to centralise, a hard dependency on `jq`,
//! and — because every script ended `exit 0` on any problem — total silence when
//! something was misconfigured. A user whose hooks never fired had no way to find
//! out why.
//!
//! Everything here reads the hook's JSON on stdin and writes one
//! `hookSpecificOutput` object on stdout. Two fields matter, per the hooks
//! reference: `additionalContext` is injected into the model's context, and
//! `systemMessage` is shown to the user. Nothing here ever blocks a tool call.

use std::io::Read;

use chrono::{NaiveDate, Utc};
use serde_json::{json, Value};

use crate::model::{Ledger, Outcome, Resolution, ResolvedBy};
use crate::report::{ReportData, Verdict};
use crate::store;

/// Which hook is firing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    SessionStart,
    UserPrompt,
    PostTool,
    Stop,
}

impl Event {
    /// The `hookEventName` the harness expects back.
    fn name(self) -> &'static str {
        match self {
            Event::SessionStart => "SessionStart",
            Event::UserPrompt => "UserPromptSubmit",
            Event::PostTool => "PostToolUse",
            Event::Stop => "Stop",
        }
    }
}

/// Below this many graded calls, a calibration line is noise and is not shown.
const MIN_N: usize = 6;
/// Re-surface the standing calibration every Nth user prompt. A counter, not
/// willpower: nobody remembers to audit themselves unprompted.
const DEFAULT_EVERY: u64 = 7;

/// The engine's own version, stamped on the first line of everything a hook
/// injects.
///
/// On the machine this was built on, the session hooks ran a 0.3.0 engine behind
/// June-era scripts for a whole release cycle, and greeted the agent in wording
/// the repo had already deleted. Nothing said so, because nothing in the output
/// named the binary that wrote it. A stale engine now announces itself.
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn env_usize(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default)
}

/// One line naming the agent's standing calibration, or `None` when there is
/// nothing honest to say.
///
/// The wording comes from [`report::Verdict`] and nowhere else, so the hook, the
/// report, the badge and the card cannot disagree about whether the agent is
/// calibrated — which they did, when each parsed the confidence gap for itself.
fn standing_line(d: &ReportData) -> Option<String> {
    if d.evidence_n < MIN_N {
        return None;
    }
    let n = d.evidence_n;
    Some(match d.verdict {
        Verdict::InsufficientData => return None,
        Verdict::NoEvidenceOfMiscalibration => format!(
            "no miscalibration found across {n} graded calls — which is not proof you are calibrated, only that nothing shows otherwise"
        ),
        Verdict::CalibratedButUninformative => format!(
            "right on average across {n} calls, but your confidence does not separate the ones that come true — spread your probabilities out"
        ),
        Verdict::Overconfident => format!(
            "OVERCONFIDENT ({n} graded): right {} of the time while claiming about {}. Add slack.",
            pct(d.accuracy),
            pct(d.mean_confidence)
        ),
        Verdict::Underconfident => format!(
            "UNDERCONFIDENT ({n} graded): right {} while claiming about {}. Trust your sure calls more.",
            pct(d.accuracy),
            pct(d.mean_confidence)
        ),
        Verdict::BiasedYes => format!(
            "LEANS YES ({n} graded): you predict things happen more often than they do."
        ),
        Verdict::BiasedNo => format!(
            "LEANS NO ({n} graded): you predict things don't happen more often than holds up."
        ),
        Verdict::MiscalibratedBothWays => format!(
            "MISCALIBRATED BOTH WAYS ({n} graded): too sure at one end of your range and not sure enough at the other. Shading everything one way will not help."
        ),
    })
}

fn pct(v: Option<f64>) -> String {
    v.map_or_else(|| "—".into(), |x| format!("{:.0}%", x * 100.0))
}

/// The worst per-kind slice, but only once it clears the multiplicity-corrected
/// bar — searching K subgroups for the worst one is K tests, not one.
fn worst_kind(d: &ReportData) -> Option<String> {
    // Naming an agent's "worst group" from a slice of the record that happens to
    // be tagged is advice drawn from a self-selected sample. Measured: 363 of 422
    // claims on a real ledger carried no `kind:` tag at all, so no grouping is
    // selected there and this stays quiet.
    let ns = d.group_by.as_deref()?;
    let threshold = d.kind_alarm_threshold?;
    let (row, e) = d
        .by_kind
        .iter()
        .filter_map(|t| t.eprocess.map(|e| (t, e)))
        .filter(|&(_, e)| e >= threshold)
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    let dir = match row.confidence_gap {
        Some(g) if g > 0.0 => "overconfident",
        Some(_) => "underconfident",
        None => "miscalibrated",
    };
    Some(format!(
        "  worst group: {ns}:{} is really {dir} (e={e:.0}, n={}, K={}) — trust those calls least",
        row.tag, row.n, d.group_k
    ))
}

/// The project slug, used to scope "what is due here".
fn project_slug(cwd: Option<&str>) -> String {
    let dir = cwd
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    dir.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "unknown".into())
}

fn due_lines(ledger: &Ledger, today: NaiveDate, slug: &str) -> Vec<String> {
    ledger
        .claims
        .iter()
        .filter(|c| !c.is_void() && c.is_due(today))
        .filter(|c| c.tags.iter().any(|t| t == &format!("project:{slug}")))
        .take(5)
        .map(|c| format!("  DUE [{}] {} — resolve it", c.id, c.statement))
        .collect()
}

/// The per-session prompt counter, kept next to the ledger. Returns the new count.
fn bump_counter(session: &str) -> u64 {
    let Some(dir) = dirs_counters() else {
        return 1;
    };
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join(format!("{}.count", sanitize(session)));
    let n = std::fs::read_to_string(&file)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
        + 1;
    let _ = std::fs::write(&file, n.to_string());
    n
}

fn dirs_counters() -> Option<std::path::PathBuf> {
    #[allow(deprecated)]
    std::env::home_dir().map(|h| h.join(".anamnesis").join("counters"))
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(64)
        .collect()
}

/// Commands whose result is the moment of truth for a `kind:tests-pass` claim.
fn is_test_command(cmd: &str) -> bool {
    const NEEDLES: [&str; 16] = [
        "cargo test",
        "cargo build",
        "cargo clippy",
        "npm test",
        "npm run build",
        "pnpm test",
        "pnpm build",
        "yarn test",
        "yarn build",
        "pytest",
        "go test",
        "gradle",
        "mvn ",
        "flutter test",
        "jest",
        "vitest",
    ];
    let lower = cmd.to_lowercase();
    NEEDLES.iter().any(|n| lower.contains(n))
}

/// Run one hook. `stdin` is the hook's JSON payload; the result is what to print.
pub fn run(event: Event, ledger_path: &std::path::Path) -> Result<(), String> {
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    let input: Value = serde_json::from_str(raw.trim()).unwrap_or_else(|_| json!({}));

    // Every path below fails soft into "say nothing" — except a missing engine,
    // which cannot happen here because the engine IS this binary. That was the
    // point of moving the logic in.
    let ledger = store::load(ledger_path).unwrap_or_default();
    let today = Utc::now().date_naive();
    let cwd = input.get("cwd").and_then(Value::as_str);
    let slug = project_slug(cwd);

    let mut context: Vec<String> = Vec::new();
    let mut user_message: Option<String> = None;

    match event {
        Event::SessionStart | Event::UserPrompt => {
            if event == Event::UserPrompt {
                let session = input
                    .get("session_id")
                    .and_then(Value::as_str)
                    .unwrap_or("default");
                let every = env_usize("ANAMNESIS_INTROSPECT_EVERY", DEFAULT_EVERY);
                let n = bump_counter(session);
                if !n.is_multiple_of(every) {
                    return Ok(()); // silent on the other six prompts in seven
                }
                context.push(format!(
                    "⟢ Anamnesis (ana {VERSION}) — self-introspection checkpoint (prompt #{n})"
                ));
            } else {
                context.push(format!(
                    "⟢ Anamnesis (ana {VERSION}) — your standing calibration"
                ));
            }

            let d = ReportData::compute(&ledger, Some("who:claude"), 10, today);
            if let Some(line) = standing_line(&d) {
                context.push(line);
                if let Some(k) = worst_kind(&d) {
                    context.push(k);
                }
            }
            if d.evidence_ungraded_due > 0 {
                if let Some(id) = &d.evidence_oldest_gap {
                    context.push(format!(
                        "  {} ungraded call(s) are costing you evidence, oldest [{id}] — resolve or void them",
                        d.evidence_ungraded_due
                    ));
                }
            }
            context.extend(due_lines(&ledger, today, &slug));
            if context.len() == 1 {
                return Ok(()); // header only: nothing worth saying
            }
            context.push(
                "Log predictions with a probability BEFORE acting; resolve them the moment reality answers.".into(),
            );
        }

        Event::PostTool => {
            let cmd = input
                .pointer("/tool_input/command")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !is_test_command(cmd) {
                return Ok(());
            }
            // The exit status is the fact. Grading a "the tests will pass"
            // prediction from the agent's own account of what happened is the
            // weakest link in a self-graded ledger; this removes it.
            let exit = input.get("tool_result_exit_code").and_then(Value::as_i64);
            match exit {
                Some(code) => {
                    let resolved = auto_resolve(ledger_path, &slug, cmd, code)?;
                    if resolved.is_empty() {
                        return Ok(());
                    }
                    let happened = code == 0;
                    context.push(format!(
                        "⟢ Anamnesis (ana {VERSION}): resolved {} prediction(s) from the command's exit status ({code}) — {}, not self-reported: {}",
                        resolved.len(),
                        if happened { "it passed" } else { "it failed" },
                        resolved.join(", ")
                    ));
                    user_message = Some(format!(
                        "anamnesis: auto-resolved {} prediction(s) from exit code {code}",
                        resolved.len()
                    ));
                }
                None => {
                    // No exit status available: fall back to a nudge, and say
                    // plainly that this one is on the agent's honour.
                    let open: Vec<String> = ledger
                        .claims
                        .iter()
                        .filter(|c| c.is_open() && !c.is_void())
                        .filter(|c| {
                            c.tags.iter().any(|t| t == "kind:tests-pass")
                                || c.tags.iter().any(|t| t == "kind:approach")
                        })
                        .take(5)
                        .map(|c| c.id.clone())
                        .collect();
                    if open.is_empty() {
                        return Ok(());
                    }
                    context.push(format!(
                        "⟢ Anamnesis (moment of truth): a test/build just ran — resolve your open prediction(s) about it NOW ({}), before hindsight rewrites how sure you were.",
                        open.join(", ")
                    ));
                }
            }
        }

        Event::Stop => {
            let overdue: Vec<&crate::model::Claim> = ledger
                .claims
                .iter()
                .filter(|c| !c.is_void() && c.is_due(today))
                .collect();
            if overdue.is_empty() {
                return Ok(());
            }
            context.push(format!(
                "⟢ Anamnesis (ana {VERSION}): {} prediction(s) are past their due date and ungraded. Until they are resolved the calibration numbers rest on a self-selected sample, and each one is priced into the evidence test at the worst factor it could have contributed.",
                overdue.len()
            ));
            for c in overdue.iter().take(5) {
                context.push(format!("  [{}] {}", c.id, c.statement));
            }
        }
    }

    if context.is_empty() {
        return Ok(());
    }
    let mut out = json!({
        "hookSpecificOutput": {
            "hookEventName": event.name(),
            "additionalContext": context.join("\n"),
        }
    });
    if let Some(msg) = user_message {
        out["systemMessage"] = json!(msg);
    }
    println!("{out}");
    Ok(())
}

/// Resolve open `kind:tests-pass` claims for this project from a command's exit
/// status, and return the ids that were graded.
///
/// Deliberately narrow: only claims explicitly tagged `kind:tests-pass`, only in
/// this project, and only when the claim was made before the command ran.
fn auto_resolve(
    ledger_path: &std::path::Path,
    slug: &str,
    cmd: &str,
    exit_code: i64,
) -> Result<Vec<String>, String> {
    let _guard = store::lock(ledger_path).map_err(|e| e.to_string())?;
    let mut ledger = store::load(ledger_path).map_err(|e| e.to_string())?;
    let now = Utc::now();
    let passed = exit_code == 0;
    let project = format!("project:{slug}");

    let mut done = Vec::new();
    for c in ledger.claims.iter_mut() {
        if !c.is_open() || c.is_void() {
            continue;
        }
        if !c.tags.iter().any(|t| t == "kind:tests-pass") {
            continue;
        }
        if !c.tags.iter().any(|t| t == &project) {
            continue;
        }
        if c.created_at > now {
            continue;
        }
        c.resolution = Some(Resolution {
            at: now,
            outcome: Some(if passed {
                Outcome::True
            } else {
                Outcome::False
            }),
            value: None,
            note: Some(format!("auto: `{}` exited {exit_code}", cmd.trim())),
            resolved_by: Some(ResolvedBy::Auto),
        });
        done.push(c.id.clone());
    }
    if !done.is_empty() {
        store::save(ledger_path, &ledger).map_err(|e| e.to_string())?;
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_test_commands_count_as_the_moment_of_truth() {
        assert!(is_test_command("cargo test --all"));
        assert!(is_test_command("  PYTHONPATH=. pytest -q "));
        assert!(is_test_command("npm test"));
        assert!(!is_test_command("ls -la"));
        assert!(!is_test_command("git commit -m 'tests'"));
    }

    #[test]
    fn the_standing_line_is_silent_on_too_little_evidence() {
        let d = ReportData::compute(&Ledger::default(), None, 10, today_for_test());
        assert!(standing_line(&d).is_none());
    }

    fn today_for_test() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
    }

    #[test]
    fn session_names_become_safe_filenames() {
        assert_eq!(sanitize("abc/../x"), "abc----x");
        assert!(!sanitize("a/b").contains('/'));
    }
}
