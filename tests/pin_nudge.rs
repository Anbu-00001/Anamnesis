//! The opt-in "pin first" reminder: `ana hook pre-tool`.
//!
//! The grader only has something to grade if a pinned prediction was logged *before* the
//! tests ran, and a standing instruction in a global file was followed by one model (Opus)
//! and ignored by another (Sonnet) in the same task. A reminder that arrives after the run
//! is too late, and a prediction written after the outcome is not a prediction. So the hook
//! uses PreToolUse, which can deny the call and tell Claude why: once per session, with the
//! exact commands. It is off unless `ANAMNESIS_PIN_NUDGE` is set, and it keeps a small local
//! log (no command text) so that a week of data can say how often tests ran with and
//! without a pinned prediction first, and under which model.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const ANA: &str = env!("CARGO_BIN_EXE_ana");

struct Setup {
    dir: PathBuf,
    ledger: PathBuf,
}

fn setup(name: &str) -> Setup {
    let dir = std::env::temp_dir().join(format!("ana_pin_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let ledger = dir.join("agent.json");
    fs::write(&ledger, r#"{"claims":[]}"#).unwrap();
    Setup { dir, ledger }
}

fn payload(session: &str, command: &str) -> String {
    serde_json::json!({
        "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "session_id": session, "cwd": "/work/project",
        "tool_input": { "command": command }
    })
    .to_string()
}

fn hook(s: &Setup, event: &str, stdin: &str, nudge: bool) -> String {
    let mut cmd = Command::new(ANA);
    cmd.args(["hook", event])
        .env("ANAMNESIS_AGENT_DATA", &s.ledger)
        .env("HOME", &s.dir)
        .env("USERPROFILE", &s.dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if nudge {
        cmd.env("ANAMNESIS_PIN_NUDGE", "1");
    } else {
        cmd.env_remove("ANAMNESIS_PIN_NUDGE");
    }
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn log(s: &Setup) -> Vec<serde_json::Value> {
    fs::read_to_string(s.dir.join("protocol.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn denied(out: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(out.trim()).ok()?;
    let h = &v["hookSpecificOutput"];
    (h["hookEventName"] == "PreToolUse" && h["permissionDecision"] == "deny").then(|| {
        h["permissionDecisionReason"]
            .as_str()
            .unwrap_or("")
            .to_string()
    })
}

#[test]
fn it_does_nothing_unless_it_is_switched_on() {
    let s = setup("off");
    let out = hook(&s, "pre-tool", &payload("a", "cargo test"), false);
    assert!(out.trim().is_empty(), "{out}");
    assert!(log(&s).is_empty(), "an inert hook must not even log");
    let _ = fs::remove_dir_all(&s.dir);
}

#[test]
fn the_first_bare_test_run_is_denied_with_the_exact_commands_and_the_second_is_allowed() {
    let s = setup("deny");
    let out = hook(&s, "pre-tool", &payload("sess", "cargo test"), true);
    let reason =
        denied(&out).unwrap_or_else(|| panic!("the first bare test run must be denied:\n{out}"));
    for must in [
        "ana add",
        "--check",
        "kind:tests-pass",
        "ana run",
        "--prob",
        "cargo test",
        "Nothing was run",
    ] {
        assert!(
            reason.contains(must),
            "the reminder must say {must:?}:\n{reason}"
        );
    }

    // Said once. A model that ignores it is not nagged, and the run goes ahead.
    let again = hook(&s, "pre-tool", &payload("sess", "cargo test"), true);
    assert!(
        again.trim().is_empty(),
        "denied twice in one session:\n{again}"
    );
    // A new session hears it again.
    assert!(denied(&hook(&s, "pre-tool", &payload("other", "cargo test"), true)).is_some());

    let actions: Vec<(String, String)> = log(&s)
        .iter()
        .map(|r| {
            (
                r["session"].as_str().unwrap().into(),
                r["action"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert_eq!(
        actions,
        [
            ("sess".into(), "denied_no_pin".into()),
            ("sess".into(), "allowed_after_nudge".into()),
            ("other".into(), "denied_no_pin".into()),
        ]
    );
    let _ = fs::remove_dir_all(&s.dir);
}

#[test]
fn a_pinned_run_through_ana_run_is_left_alone_and_counted() {
    let s = setup("anarun");
    for cmd in [
        "ana run abc123 -- cargo test",
        "/home/u/.local/bin/ana run abc123 -- pytest -q",
        "cd tiny && ana --data x.json run abc123 -- cargo test",
    ] {
        let out = hook(&s, "pre-tool", &payload("s", cmd), true);
        assert!(out.trim().is_empty(), "{cmd}: {out}");
    }
    let l = log(&s);
    assert_eq!(l.len(), 3);
    assert!(l.iter().all(|r| r["action"] == "ana_run"), "{l:?}");
    let _ = fs::remove_dir_all(&s.dir);
}

/// A reminder that fired on `git commit -m "fix the jest config"` would be worse than none.
#[test]
fn commands_that_merely_mention_a_runner_are_never_touched() {
    let s = setup("mention");
    for cmd in [
        "git commit -m \"fix the jest config\"",
        "echo cargo test",
        "ls -la",
        "cat tests/test_pytest_plugin.py",
        "grep -rn 'go test' docs/",
        "cargo build --release",
        "cargo test --no-run",
        "pytest --collect-only",
        "npm run build",
        "ana report --tag who:claude",
    ] {
        let out = hook(&s, "pre-tool", &payload("m", cmd), true);
        assert!(
            out.trim().is_empty(),
            "touched a command that is not a test run: {cmd}\n{out}"
        );
    }
    assert!(log(&s).is_empty(), "{:?}", log(&s));
    let _ = fs::remove_dir_all(&s.dir);
}

/// Agents pipe test output to `tail` to keep it short. That is still a test run, and the
/// reminder should suggest the plain command, which is what can be pinned.
#[test]
fn a_piped_or_chained_test_run_is_recognised_and_the_plain_command_suggested() {
    let s = setup("piped");
    let out = hook(
        &s,
        "pre-tool",
        &payload("p", "cd tiny && cargo test 2>&1 | tail -60"),
        true,
    );
    let reason = denied(&out).expect("a piped test run is a test run");
    assert!(reason.contains("--check \"cargo test\""), "{reason}");
    assert!(reason.contains("-- cargo test"), "{reason}");
    assert!(
        !reason.contains("tail"),
        "the pipe is not part of what gets pinned: {reason}"
    );
    let _ = fs::remove_dir_all(&s.dir);
}

#[test]
fn a_prediction_already_logged_but_not_run_through_ana_run_is_pointed_at_ana_run() {
    let s = setup("unrun");
    let now = chrono_now();
    fs::write(
        &s.ledger,
        serde_json::json!({"claims": [{
            "id": "pin001", "statement": "the suite passes", "created_at": now, "resolve_by": "2099-01-01",
            "tags": ["who:claude", "kind:tests-pass", "project:project"], "kind": "binary",
            "check": "cargo test",
            "forecasts": [{"at": now, "prob": 0.8}]
        }]})
        .to_string(),
    )
    .unwrap();
    let out = hook(&s, "pre-tool", &payload("u", "cargo test"), true);
    let reason = denied(&out).expect("it logged the claim but is about to run the bare command");
    assert!(reason.contains("ana run pin001 -- cargo test"), "{reason}");
    assert!(
        !reason.contains("ana add"),
        "it already logged one: {reason}"
    );
    assert_eq!(log(&s)[0]["action"], "denied_unrun_pin");

    // A pin from hours ago is not this run's pin.
    fs::write(
        &s.ledger,
        serde_json::json!({"claims": [{
            "id": "old001", "statement": "yesterday", "created_at": "2024-01-01T00:00:00Z",
            "resolve_by": "2099-01-01", "tags": ["who:claude"], "kind": "binary", "check": "cargo test",
            "forecasts": [{"at": "2024-01-01T00:00:00Z", "prob": 0.8}]
        }]})
        .to_string(),
    )
    .unwrap();
    let reason = denied(&hook(&s, "pre-tool", &payload("u2", "cargo test"), true)).unwrap();
    assert!(
        reason.contains("ana add"),
        "a stale pin must not count: {reason}"
    );
    let _ = fs::remove_dir_all(&s.dir);
}

fn chrono_now() -> String {
    // RFC 3339 UTC without a date library: `date` is not portable, so build it from the epoch.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// The model is what separates "Sonnet ignored it" from "nobody ran tests", so it is
/// recorded: SessionStart is told the model, PreToolUse is not.
#[test]
fn the_log_records_the_model_the_runner_and_the_project_but_never_the_command() {
    let s = setup("model");
    // The model is read from the session transcript: SessionStart is not told it.
    let transcript = s.dir.join("t.jsonl");
    fs::write(
        &transcript,
        "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-sonnet-5-5\",\"content\":[]}}\n",
    )
    .unwrap();
    let mut p: serde_json::Value = serde_json::from_str(&payload(
        "mm",
        "cd tiny && SECRET_TOKEN=hunter2 cargo test -p secret_crate",
    ))
    .unwrap();
    p["transcript_path"] = transcript.display().to_string().into();
    hook(&s, "pre-tool", &p.to_string(), true);
    let raw = fs::read_to_string(s.dir.join("protocol.jsonl")).unwrap();
    let r: serde_json::Value = serde_json::from_str(raw.lines().next().unwrap()).unwrap();
    assert_eq!(r["model"], "claude-sonnet-5-5");
    assert_eq!(r["runner"], "cargo");
    assert_eq!(r["project"], "project");
    assert!(r["at"].as_str().unwrap().ends_with('Z'));
    for secret in ["hunter2", "SECRET_TOKEN", "secret_crate", "tiny"] {
        assert!(
            !raw.contains(secret),
            "the log must not hold command text ({secret}):\n{raw}"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(s.dir.join("protocol.jsonl"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the log is private");
    }
    let _ = fs::remove_dir_all(&s.dir);
}

#[test]
fn a_hostile_session_id_cannot_escape_the_marker_directory() {
    let s = setup("traversal");
    hook(
        &s,
        "pre-tool",
        &payload("../../etc/evil", "cargo test"),
        true,
    );
    hook(&s, "pre-tool", &payload("a/b\nc", "cargo test"), true);
    assert!(!std::path::Path::new("/etc/evil.pindenied").exists());
    let _ = fs::remove_dir_all(&s.dir);
}

/// Superpowers' main mechanism is a rule injected at the start of every session; the start
/// of context is the most reliable place for an instruction a model would otherwise skip.
/// SessionStart fires again after /clear and after compaction, so the rule comes back.
#[test]
fn the_rule_is_injected_at_session_start_only_when_the_reminder_is_on() {
    let s = setup("rule");
    let start = serde_json::json!({"session_id": "r", "cwd": "/work/project", "source": "startup"})
        .to_string();
    let on = hook(&s, "session-start", &start, true);
    let v: serde_json::Value =
        serde_json::from_str(on.trim()).unwrap_or_else(|e| panic!("{e}: {on}"));
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    for must in [
        "Standing rule",
        "ana add",
        "--check",
        "ana run",
        "BEFORE the run",
        "kind:tests-pass",
    ] {
        assert!(ctx.contains(must), "the rule must say {must:?}:\n{ctx}");
    }
    assert_eq!(
        ctx.matches('⟢').count(),
        2,
        "the engine header and the rule, nothing forged:\n{ctx}"
    );
    let off = hook(&s, "session-start", &start, false);
    assert!(
        !off.contains("Standing rule"),
        "inert unless switched on:\n{off}"
    );
    let _ = fs::remove_dir_all(&s.dir);
}

/// Anthropic's security-guidance plugin keeps per-session state with a TTL. Ours are a few
/// bytes per session and would pile up for ever, so stale ones go, and nothing else does.
#[test]
fn stale_session_files_are_pruned_and_nothing_else_is() {
    let s = setup("prune");
    let dir = s.dir.join(".anamnesis").join("counters");
    fs::create_dir_all(&dir).unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(40 * 24 * 3600);
    let make = |name: &str, aged: bool| {
        let p = dir.join(name);
        fs::write(&p, "x").unwrap();
        if aged {
            fs::OpenOptions::new()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        p
    };
    let stale: Vec<_> = ["a.count", "a.stop", "a.pindenied"]
        .iter()
        .map(|n| make(n, true))
        .collect();
    let fresh = make("b.count", false);
    let foreign = make("notes.txt", true);
    hook(
        &s,
        "session-start",
        &serde_json::json!({"session_id": "p", "cwd": "/work/project"}).to_string(),
        true,
    );
    for p in &stale {
        assert!(!p.exists(), "a stale file was kept: {}", p.display());
    }
    assert!(fresh.exists(), "a recent file was deleted");
    assert!(
        foreign.exists(),
        "a file this tool did not write was deleted"
    );
    let _ = fs::remove_dir_all(&s.dir);
}

/// The person works across stacks. A runner the reminder does not know is a run it never
/// sees, and pinning works for any command.
#[test]
fn other_stacks_test_commands_are_recognised_too() {
    for (i, cmd) in [
        "dotnet test",
        "deno test",
        "bundle exec rspec",
        "rake test",
        "phpunit",
        "tox",
        "make test",
        "just check",
        "flutter test",
        "./gradlew test",
        "npm test",
        "go test ./...",
    ]
    .iter()
    .enumerate()
    {
        let s = setup(&format!("stack{i}"));
        assert!(
            denied(&hook(&s, "pre-tool", &payload("x", cmd), true)).is_some(),
            "not recognised: {cmd}"
        );
        let _ = fs::remove_dir_all(&s.dir);
    }
    let s = setup("stackneg");
    for cmd in [
        "make build",
        "make",
        "just --list",
        "dotnet build",
        "tox --version",
        "rake db:migrate",
    ] {
        assert!(
            hook(&s, "pre-tool", &payload("n", cmd), true)
                .trim()
                .is_empty(),
            "touched: {cmd}"
        );
    }
    let _ = fs::remove_dir_all(&s.dir);
}
