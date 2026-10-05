//! The opt-in "pin first" reminder, `ana hook pre-tool`.
//!
//! The grader only has something to grade if a pinned prediction was logged *before*
//! the tests ran. A standing instruction in a global file was followed by one model and
//! ignored by another in the same task, and a reminder that arrives after the run is too
//! late: a prediction written after the outcome is not a prediction. PreToolUse can deny
//! a call and tell Claude why, before anything runs, so the first bare test run of a
//! session is stopped once, with the exact commands to use. A model that follows is never
//! bothered again; one that ignores it is not nagged, and its run goes ahead.
//!
//! Off unless `ANAMNESIS_PIN_NUDGE` is set. It keeps a small local log beside the ledger
//! (`protocol.jsonl`: when, which session, which project and model, which runner, what
//! happened; never a command, a path or any output) so that a week of data can tell "the
//! agent ignored the instruction" from "nobody ran any tests".

use std::path::Path;

use chrono::{Duration, Utc};
use serde_json::{json, Value};

use crate::hook::{
    assignment, dirs_counters, has_non_running_flag, is_ana_run, project_slug, runner_of, sanitize,
};
use crate::model::{Claim, Ledger};
use crate::{store, untrusted};

/// A pin logged within this long is taken to be for the run about to happen.
const PIN_WINDOW_MINUTES: i64 = 30;

/// Whether the reminder is switched on.
pub fn enabled() -> bool {
    matches!(
        std::env::var("ANAMNESIS_PIN_NUDGE").as_deref(),
        Ok("1" | "on" | "true" | "yes")
    )
}

/// Split a shell line at `|`, `;`, `&` and newlines that are not inside quotes.
///
/// Cutting inside quotes would find a runner in `echo "a | cargo test"`, and a reminder
/// that fires on an `echo` or a `git commit -m "fix the jest config"` is worse than none.
fn segments(cmd: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let (mut single, mut double, mut escaped) = (false, false, false);
    for c in cmd.chars() {
        let cur = out.last_mut().expect("never empty");
        if escaped {
            cur.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' if !single => {
                cur.push(c);
                escaped = true;
            }
            '\'' if !double => {
                single = !single;
                cur.push(c);
            }
            '"' if !single => {
                double = !double;
                cur.push(c);
            }
            '|' | ';' | '&' | '\n' if !single && !double => out.push(String::new()),
            _ => cur.push(c),
        }
    }
    out
}

/// A short, stable name for the kind of runner, for the log. Never the command.
fn runner_label(toks: &[&str]) -> &'static str {
    match toks.first().copied().unwrap_or("") {
        "cargo" => "cargo",
        "pytest" | "py.test" | "python" | "python3" => "pytest",
        "go" => "go",
        "npm" | "pnpm" | "yarn" | "bun" | "npx" | "pnpx" | "bunx" | "jest" | "vitest" => "node",
        "flutter" | "dart" => "flutter",
        "gradle" | "./gradlew" | "gradlew" | "mvn" | "./mvnw" => "jvm",
        _ => "other",
    }
}

/// Test commands the grader's own matcher does not parse but a reminder should still catch:
/// pinning works for any command, judged by its exit status. The person works across stacks,
/// and a runner the reminder does not know is a run it never sees.
fn other_runner(t: &[&str]) -> Option<&'static str> {
    match t {
        ["dotnet" | "deno", "test", ..] => Some("dotnet"),
        ["bundle", "exec", "rspec" | "rake", ..]
        | ["rspec", ..]
        | ["rake", "test" | "spec", ..] => Some("ruby"),
        ["phpunit" | "./vendor/bin/phpunit", ..] => Some("php"),
        ["tox" | "nox", ..] => Some("python"),
        ["make", target, ..] | ["just", target, ..]
            if matches!(*target, "test" | "tests" | "check" | "ci") =>
        {
            Some("make")
        }
        _ => None,
    }
}

/// The test run in `cmd`, if there is one: its runner label and the plain command to pin.
///
/// Redirections and everything after them are not part of what gets pinned: an agent that
/// ran `cargo test 2>&1 | tail -60` is told to pin `cargo test`.
pub(crate) fn test_run_in(cmd: &str) -> Option<(&'static str, String)> {
    let flat = cmd
        .replace("2>&1", " ")
        .replace("1>&2", " ")
        .replace(">&2", " ")
        .replace("&>", " ");
    for seg in segments(&flat) {
        let mut toks: Vec<&str> = seg.split_whitespace().collect();
        while toks
            .first()
            .is_some_and(|t| assignment(t).is_some() || matches!(*t, "time" | "nohup"))
        {
            toks.remove(0);
        }
        if toks.first().is_some_and(|t| *t == "cd") {
            continue;
        }
        let label = if runner_of(&toks).is_some() {
            Some(runner_label(&toks))
        } else if has_non_running_flag(&toks) {
            None
        } else {
            other_runner(&toks)
        };
        if let Some(label) = label {
            let plain: Vec<&str> = toks
                .iter()
                .copied()
                .take_while(|t| !t.starts_with(['>', '<']) && !t.starts_with("2>"))
                .collect();
            return Some((label, plain.join(" ")));
        }
    }
    None
}

/// The most recent pinned, unsettled claim, if it was logged recently enough to be for
/// this run.
fn recent_open_pin(ledger: &Ledger) -> Option<&Claim> {
    let cutoff = Utc::now() - Duration::minutes(PIN_WINDOW_MINUTES);
    ledger
        .claims
        .iter()
        .filter(|c| c.is_open() && !c.is_void() && c.check.is_some() && c.created_at > cutoff)
        .max_by_key(|c| c.created_at)
}

/// Where the log lives: beside the ledger it describes, so a scratch ledger gets a scratch
/// log and the real one is never touched by a test.
fn log_path(ledger_path: &Path) -> std::path::PathBuf {
    ledger_path.with_file_name("protocol.jsonl")
}

fn log(
    ledger_path: &Path,
    session: &str,
    project: &str,
    model: Option<&str>,
    runner: &str,
    action: &str,
) {
    let rec = json!({
        "at": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "session": untrusted::tag(session),
        "project": untrusted::tag(project),
        "model": model,
        "runner": runner,
        "action": action,
    });
    let _ = store::append_private(&log_path(ledger_path), &rec.to_string());
}

fn marker(session: &str, ext: &str) -> Option<std::path::PathBuf> {
    dirs_counters().map(|d| d.join(format!("{}.{ext}", sanitize(session))))
}

/// The model that produced this session's messages, read from the end of its transcript.
///
/// SessionStart is not told the model (checked against a real payload: its keys are `cwd`,
/// `hook_event_name`, `session_id`, `source`, `transcript_path`), but both hooks are told
/// where the transcript is, and every assistant message in it records the model that wrote
/// it. What separates "this model ignored the instruction" from "nobody ran any tests" is
/// knowing which model it was, and this is the harness's own record, not the agent's account.
/// Only the last 256 KB is read, so a long session costs the same as a short one.
fn model_of(input: &Value) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    const TAIL: u64 = 256 * 1024;
    let path = input.get("transcript_path")?.as_str()?;
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut bytes = Vec::new();
    f.take(TAIL).read_to_end(&mut bytes).ok()?;
    // The cut may land inside a line or a character; those lines simply do not parse.
    String::from_utf8_lossy(&bytes)
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find_map(|v| {
            let m = v.pointer("/message/model")?.as_str()?;
            (!m.is_empty() && !m.starts_with('<')).then(|| untrusted::tag(m))
        })
}

/// True the first time it is asked about a session: the reminder is said once.
fn first_denial_of_session(session: &str) -> bool {
    let Some(path) = marker(session, "pindenied") else {
        return true;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .is_ok()
}

/// Whether `s` can sit inside the double quotes of a command it is suggested in.
fn quotable(s: &str) -> bool {
    !s.is_empty() && !s.contains(['"', '\\', '$', '`', '\n'])
}

fn deny(reason: String) -> Value {
    json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": reason,
    } })
}

/// The rule as injected at the start of a session. Static text: nothing from the ledger.
pub fn standing_rule() -> String {
    "⟢ Standing rule on this machine, set by its owner and not optional: every time you are about to run a project's tests to check a change you made, FIRST log a pinned prediction, then run the tests through `ana run`.\n  1. ana add \"<what you expect, e.g. all tests pass first try>\" --prob <your honest probability> --by <today's date> --tags who:claude,kind:tests-pass,project:<repo folder name, lowercase>,session:<today's date>,model:<your model> --check \"<the exact test command>\"\n  2. ana run <the id it printed> -- <that exact command>   (from the directory the tests run in)\nThe prediction must exist BEFORE the run. The first bare test run of a session is refused with these steps; a run through `ana run` never is.".to_string()
}

/// How long a per-session marker is kept. The files are a few bytes each, one set per
/// session, and would otherwise pile up forever.
const STATE_TTL_DAYS: u64 = 14;

/// Delete this tool's own stale per-session files from the counters directory: only the
/// four kinds it writes, only older than [`STATE_TTL_DAYS`], and never anything else there.
pub fn prune_state() {
    let Some(dir) = dirs_counters() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let cutoff =
        std::time::SystemTime::now() - std::time::Duration::from_secs(STATE_TTL_DAYS * 24 * 3600);
    for e in entries.flatten() {
        let path = e.path();
        let ours = path
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| matches!(x, "count" | "stop" | "pindenied"));
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t < cutoff);
        if ours && stale && path.is_file() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// What `ana hook pre-tool` says about a Bash call: a denial, or nothing.
pub fn pre_tool(input: &Value, ledger: &Ledger, ledger_path: &Path) -> Option<Value> {
    let cmd = input
        .pointer("/tool_input/command")
        .and_then(Value::as_str)?;
    let session = input
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("default");
    let project = project_slug(input.get("cwd").and_then(Value::as_str));
    let model = model_of(input);
    let model = model.as_deref();

    // Already going through `ana run`: that is the behaviour wanted. Count it.
    if is_ana_run(cmd) {
        let runner = cmd
            .split("--")
            .nth(1)
            .and_then(|rest| test_run_in(rest.trim()))
            .map_or("other", |(r, _)| r);
        log(ledger_path, session, &project, model, runner, "ana_run");
        return None;
    }
    let (runner, plain) = test_run_in(cmd)?;

    if !first_denial_of_session(session) {
        log(
            ledger_path,
            session,
            &project,
            model,
            runner,
            "allowed_after_nudge",
        );
        return None;
    }
    let shown = untrusted::line(&plain, 120);
    let pin = recent_open_pin(ledger);
    log(
        ledger_path,
        session,
        &project,
        model,
        runner,
        if pin.is_some() {
            "denied_unrun_pin"
        } else {
            "denied_no_pin"
        },
    );
    Some(deny(match pin {
        Some(c) => format!(
            "anamnesis: you already logged the pinned prediction [{id}], but this is the bare command, so no one would grade it. Nothing was run. Run it as: ana run {id} -- {check} (from the directory the tests run in). It takes the verdict from the exit status and prints the command's own output. (Said once per session.)",
            id = untrusted::tag(&c.id),
            check = untrusted::line(c.check.as_deref().unwrap_or(""), untrusted::MAX_LINE),
        ),
        None => {
            let cmd_for_pin = if quotable(&shown) {
                shown
            } else {
                "<the plain test command>".to_string()
            };
            format!(
                "anamnesis: before you run tests to check a change, log a pinned prediction first, so the machine grades it and you do not. Nothing was run. Do this, then run the tests again:\n  1. ana add \"<what you expect, e.g. all tests pass first try>\" --prob <your honest probability> --by <today's date> --tags who:claude,kind:tests-pass,project:<repo folder name, lowercase>,session:<today's date>,model:<your model> --check \"{cmd_for_pin}\"\n  2. ana run <the id it printed> -- {cmd_for_pin}\nRun `ana run` from the directory the tests run in; it takes the verdict from the process's exit status and prints the command's own output. The prediction must be logged BEFORE the run: one written after the outcome is not a prediction. (Said once per session; ANAMNESIS_PIN_NUDGE=off turns it off.)"
            )
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(cmd: &str) -> Option<(&'static str, String)> {
        test_run_in(cmd)
    }

    #[test]
    fn a_test_run_is_found_inside_a_longer_command_line() {
        assert_eq!(found("cargo test").unwrap(), ("cargo", "cargo test".into()));
        assert_eq!(
            found("cd tiny && cargo test 2>&1 | tail -60").unwrap(),
            ("cargo", "cargo test".into())
        );
        assert_eq!(
            found("RUST_LOG=debug cargo test -p foo -- --nocapture")
                .unwrap()
                .1,
            "cargo test -p foo -- --nocapture"
        );
        assert_eq!(
            found("python3 -m pytest -q tests/ > out.txt").unwrap(),
            ("pytest", "python3 -m pytest -q tests/".into())
        );
        assert_eq!(found("time npm test").unwrap().0, "node");
        assert_eq!(
            found("make && go test ./...").unwrap(),
            ("go", "go test ./...".into())
        );
    }

    #[test]
    fn a_runner_named_inside_quotes_or_an_argument_is_not_a_test_run() {
        for cmd in [
            "echo \"a | cargo test --lib\"",
            "echo 'cargo test'",
            "git commit -m \"fix; cargo test\"",
            "grep -rn 'go test' docs/",
            "cat tests/test_pytest_plugin.py",
            "cargo build",
            "cargo test --no-run",
            "ls; echo done",
            "",
        ] {
            assert!(found(cmd).is_none(), "must not be a test run: {cmd}");
        }
    }

    #[test]
    fn the_model_is_read_from_the_end_of_the_transcript() {
        let dir = std::env::temp_dir().join(format!("ana_model_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let line = |m: &str| {
            format!("{{\"type\":\"assistant\",\"message\":{{\"model\":\"{m}\",\"content\":[]}}}}\n")
        };
        // A long early stretch by one model, a synthetic entry, then the model in use now.
        let mut text = String::new();
        for _ in 0..4000 {
            text.push_str(&line("claude-opus-5"));
        }
        text.push_str("{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n");
        text.push_str(&line("<synthetic>"));
        text.push_str(&line("claude-sonnet-5"));
        std::fs::write(&path, text).unwrap();
        let input = json!({ "transcript_path": path.display().to_string() });
        assert_eq!(model_of(&input).as_deref(), Some("claude-sonnet-5"));
        // No transcript, a missing file, or one with no model: nothing, never an error.
        assert_eq!(model_of(&json!({})), None);
        assert_eq!(
            model_of(&json!({"transcript_path": "/nonexistent/x.jsonl"})),
            None
        );
        std::fs::write(&path, "not json at all\n").unwrap();
        assert_eq!(model_of(&input), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_suggestion_that_would_break_the_quoting_is_not_offered() {
        assert!(quotable("cargo test -p foo"));
        assert!(!quotable("pytest -k \"a b\""));
        assert!(!quotable("echo $HOME"));
        assert!(!quotable(""));
    }
}
