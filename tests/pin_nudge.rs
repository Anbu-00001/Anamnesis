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
    // A test must not inherit the Claude Code session of whoever runs it.
    cmd.env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("AI_AGENT");
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

/// Shapes taken from real sessions. The first version looked only at the start of the last
/// `&&` part, and 382 of 471 real `ana run` calls never reached the log.
#[test]
fn ana_run_is_counted_in_any_segment_and_behind_wrappers() {
    let s = setup("anarunshapes");
    for cmd in [
        "cd /w/showcase; ana run 5d40cf -- .venv/bin/python -m pytest -q 2>&1 | tail -30",
        "cd /w/showcase && nice -n 19 ana run 39f906 -- .venv/bin/python -m pytest -q 2>&1 | tail -45",
        "ID=$(ana add \"suite passes\" --prob 0.8 --check \"cargo test\" | grep -o '[0-9a-f]\\{6\\}') && ana run \"$ID\" -- cargo test",
        "ana run 393876 -- cargo test 2>&1 | tail -3 && git add README.md",
        "S=/tmp/x\nana run abc123 -- cargo test\necho done",
        "timeout 900 ana run abc123 -- cargo test",
        "pgrep -c cargo; (ana run 7c758b -- env CARGO_BUILD_JOBS=3 nice -n 19 cargo test)",
    ] {
        let out = hook(&s, "pre-tool", &payload("s", cmd), true);
        assert!(out.trim().is_empty(), "{cmd}: {out}");
    }
    let l = log(&s);
    assert_eq!(l.len(), 7, "{l:?}");
    assert!(l.iter().all(|r| r["action"] == "ana_run"), "{l:?}");
    assert_eq!(l[1]["runner"], "pytest");
    assert_eq!(l[2]["runner"], "cargo");
    let _ = fs::remove_dir_all(&s.dir);
}

#[test]
fn a_wrapped_bare_test_run_is_still_a_test_run() {
    for cmd in [
        "nice -n 19 cargo test",
        "timeout -k 5 600 pytest -q",
        "env -u CI cargo test",
        "(cd tiny && cargo test)",
        "{ cargo test; } 2>&1 | tail",
    ] {
        let s = setup("wrapped");
        let reason = denied(&hook(&s, "pre-tool", &payload("w", cmd), true))
            .unwrap_or_else(|| panic!("not seen as a test run: {cmd}"));
        assert!(
            !reason.contains("nice") && !reason.contains("timeout"),
            "{reason}"
        );
        let _ = fs::remove_dir_all(&s.dir);
    }
}

#[test]
fn a_heredoc_body_is_text_not_a_command() {
    let s = setup("heredoc");
    for cmd in [
        "cat > notes.md <<'EOF'\nRun the suite with:\ncargo test\nor ana run abc123 -- cargo test\nEOF",
        "python3 - <<PY\nimport subprocess\nPY\necho ok",
    ] {
        let out = hook(&s, "pre-tool", &payload("h", cmd), true);
        assert!(out.trim().is_empty(), "{cmd}: {out}");
    }
    assert!(log(&s).is_empty(), "{:?}", log(&s));
    // ... and the command after the body still counts.
    let out = hook(
        &s,
        "pre-tool",
        &payload("h", "cat <<EOF\nx\nEOF\ncargo test"),
        true,
    );
    assert!(denied(&out).is_some(), "{out}");
    let _ = fs::remove_dir_all(&s.dir);
}

/// A subagent's call names its parent's transcript; reading that credited 18 of 33 real
/// subagent runs to the wrong model. The payload is the one Claude Code 2.1.251 sent.
#[test]
fn a_subagent_run_is_credited_to_the_subagent_model() {
    let s = setup("subagent");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claude-code-2.1.251/pre_tool_subagent.json");
    let mut p: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(fixture).unwrap()).unwrap();
    let parent = s.dir.join("sess.jsonl");
    let line = |m: &str| {
        format!("{{\"type\":\"assistant\",\"message\":{{\"model\":\"{m}\",\"content\":[]}}}}\n")
    };
    fs::write(&parent, line("claude-opus-5-5")).unwrap();
    p["transcript_path"] = parent.display().to_string().into();

    // Its own transcript is not there (yet): say nothing rather than name the parent's model.
    hook(&s, "pre-tool", &p.to_string(), true);
    let sub_dir = s.dir.join("sess").join("subagents");
    fs::create_dir_all(&sub_dir).unwrap();
    fs::write(
        sub_dir.join("agent-a3d9bf2dfac5286db.jsonl"),
        line("claude-haiku-5-5"),
    )
    .unwrap();
    hook(&s, "pre-tool", &p.to_string(), true);
    // A hostile id never becomes a path.
    p["agent_id"] = "../../sess".into();
    hook(&s, "pre-tool", &p.to_string(), true);
    // The main agent still reads its own transcript.
    p.as_object_mut().unwrap().remove("agent_id");
    hook(&s, "pre-tool", &p.to_string(), true);

    let l = log(&s);
    let got: Vec<_> = l
        .iter()
        .map(|r| (r["model"].clone(), r["subagent"].clone()))
        .collect();
    assert_eq!(
        got,
        [
            (serde_json::Value::Null, true.into()),
            ("claude-haiku-5-5".into(), true.into()),
            (serde_json::Value::Null, true.into()),
            ("claude-opus-5-5".into(), false.into()),
        ]
    );
    let _ = fs::remove_dir_all(&s.dir);
}

#[test]
fn a_virtualenv_pytest_is_a_test_run_and_the_pin_keeps_its_path() {
    let s = setup("venv");
    let reason = denied(&hook(
        &s,
        "pre-tool",
        &payload(
            "v",
            "cd /w/showcase && .venv/bin/python -m pytest -q 2>&1 | tail -5",
        ),
        true,
    ))
    .expect("a virtualenv pytest is a test run");
    assert!(
        reason.contains("--check \".venv/bin/python -m pytest -q\""),
        "{reason}"
    );
    assert_eq!(log(&s)[0]["runner"], "pytest");
    let _ = fs::remove_dir_all(&s.dir);
}

/// Each of these was a real test run that the log still did not see. The commands are from
/// sessions between 2026-10-20 and 2026-11-04.
#[test]
fn real_test_commands_the_second_logger_still_missed_are_seen() {
    for (cmd, runner) in [
        (
            "python3 -m unittest discover -s tests 2>&1 | tail -5",
            "python",
        ),
        (
            "timeout 60 nice -n 19 python3 -m unittest discover -s tests",
            "python",
        ),
        (
            "PYTHONDONTWRITEBYTECODE=1 .venv/bin/python -B -m pytest -q -p no:cacheprovider",
            "pytest",
        ),
        ("python -W ignore -X dev -m pytest -q", "pytest"),
        ("$P -m pytest -q", "python"),
        (
            "for i in 1 2; do npx vitest run 2>&1 | grep x; done",
            "node",
        ),
        ("if ! pytest -q; then echo no; fi", "pytest"),
        ("run(){ .venv/bin/python -m pytest -q; }; run", "pytest"),
        (
            "r=$(.venv/bin/python -m pytest -q -x tests/test_a.py)",
            "pytest",
        ),
        ("timeout 200 node --test test/core.test.js", "node"),
        ("uv run pytest -q", "pytest"),
        ("npm run test:unit", "node"),
        ("ctest --output-on-failure", "c"),
        (
            "nohup bash -c \"ulimit -v 6000000; .venv/bin/python -m pytest -q\"",
            "pytest",
        ),
    ] {
        let s = setup("seen");
        assert!(
            denied(&hook(&s, "pre-tool", &payload("x", cmd), true)).is_some(),
            "not seen as a test run: {cmd}"
        );
        assert_eq!(log(&s)[0]["runner"], runner, "{cmd}");
        let _ = fs::remove_dir_all(&s.dir);
    }
}

#[test]
fn ana_run_inside_eval_or_a_shell_is_counted() {
    let s = setup("evalrun");
    for cmd in [
        "id=$(ana add x --prob 0.5 | head -1); eval ana run $id -- $CMD 2>&1 | tail",
        "nohup bash -c \"ana run 69440c -- nice -n 19 cargo test\" > /tmp/x.log 2>&1 &",
        "sh -c 'ana run abc123 -- cargo test'",
    ] {
        let out = hook(&s, "pre-tool", &payload("e", cmd), true);
        assert!(out.trim().is_empty(), "{cmd}: {out}");
    }
    let l = log(&s);
    assert_eq!(l.len(), 3, "{l:?}");
    assert!(l.iter().all(|r| r["action"] == "ana_run"), "{l:?}");
    assert_eq!(l[1]["runner"], "cargo");
    let _ = fs::remove_dir_all(&s.dir);
}

/// Widening what counts must not widen what is refused: a refusal on a command that runs no
/// tests costs a turn, and a false `ana_run` flatters the compliance figure.
#[test]
fn commands_that_only_look_like_runs_stay_out_of_the_log() {
    let s = setup("lookalike");
    for cmd in [
        "ana add \"the suite will run\" --prob 0.7 --tags who:claude",
        "ana add \"pytest passes\" --prob 0.7 --check \"cargo test\"",
        "bash -c \"echo hi\"",
        "bash tools/release.sh",
        "git commit -m \"bash -c cargo test\"",
        "python -c \"import pytest\"",
        "python3 -B script.py",
        "python3 -m pip install pytest",
        "uv run ruff check .",
        "npm run build",
        "npm run lint:test-names --silent --version",
        "for t in \"ana add\" \"ana run\"; do echo $t; done",
        "x=$(git rev-parse HEAD)",
        "if [ -f pytest.ini ]; then echo yes; fi",
        "node --version",
    ] {
        let out = hook(&s, "pre-tool", &payload("l", cmd), true);
        assert!(out.trim().is_empty(), "{cmd}: {out}");
    }
    assert!(log(&s).is_empty(), "{:?}", log(&s));
    let _ = fs::remove_dir_all(&s.dir);
}

/// One codebase looked like 23 projects because `project` is the folder a command ran in.
#[test]
fn the_log_names_the_repository_through_subfolders_cd_and_worktrees() {
    let s = setup("repo");
    let main = s.dir.join("MainRepo");
    fs::create_dir_all(main.join(".git")).unwrap();
    fs::create_dir_all(main.join("backend/app")).unwrap();
    let tree = s.dir.join("feature-branch");
    fs::create_dir_all(&tree).unwrap();
    fs::write(
        tree.join(".git"),
        format!("gitdir: {}/.git/worktrees/feature-branch\n", main.display()),
    )
    .unwrap();
    let elsewhere = s.dir.join("not-a-repo");
    fs::create_dir_all(&elsewhere).unwrap();

    let with_cwd = |sess: &str, cwd: &std::path::Path, cmd: &str| {
        let mut p: serde_json::Value = serde_json::from_str(&payload(sess, cmd)).unwrap();
        p["cwd"] = cwd.display().to_string().into();
        hook(&s, "pre-tool", &p.to_string(), true);
    };
    with_cwd("1", &main.join("backend/app"), "cargo test");
    with_cwd("2", &tree, "cargo test");
    with_cwd(
        "3",
        &elsewhere,
        &format!("cd {} && cargo test", main.join("backend").display()),
    );
    with_cwd("4", &main, "X=1; cd backend/app; cargo test");
    with_cwd("5", &elsewhere, "cargo test");

    let got: Vec<_> = log(&s).iter().map(|r| r["repo"].clone()).collect();
    assert_eq!(
        got,
        [
            "mainrepo".into(),
            "mainrepo".into(),
            "mainrepo".into(),
            "mainrepo".into(),
            serde_json::Value::Null
        ]
    );
    let _ = fs::remove_dir_all(&s.dir);
}

fn tool_use(id: &str, name: &str, command: &str) -> String {
    serde_json::json!({"type": "assistant", "message": {"model": "claude-opus-5-5", "role": "assistant",
        "content": [{"type": "tool_use", "id": id, "name": name, "input": {"command": command}}]}})
    .to_string()
}

fn tool_result(id: &str, text: &str) -> String {
    serde_json::json!({"type": "user", "message": {"role": "user",
        "content": [{"type": "tool_result", "tool_use_id": id, "content": text, "is_error": false}]}})
    .to_string()
}

/// The rule covers runs that check a change, and every bare run used to count against it.
#[test]
fn the_log_says_whether_anything_was_edited_since_the_last_run() {
    let cases: Vec<(Vec<String>, &str, serde_json::Value)> = vec![
        (vec![tool_use("a", "Edit", "")], "cargo test", true.into()),
        (vec![tool_use("a", "Agent", "")], "cargo test", true.into()),
        (
            vec![tool_use("a", "Bash", "sed -i s/a/b/ f.rs")],
            "cargo test",
            true.into(),
        ),
        (
            vec![tool_use("a", "Bash", "ls")],
            "sed -i s/a/b/ f.rs && cargo test",
            true.into(),
        ),
        (
            vec![
                tool_use("a", "Edit", ""),
                tool_use("b", "Bash", "cargo test"),
            ],
            "cargo test",
            false.into(),
        ),
        (
            vec![
                tool_use("a", "Bash", "cargo test"),
                tool_use("b", "Write", ""),
            ],
            "cargo test",
            true.into(),
        ),
        (
            vec![tool_use("a", "Bash", "ana run x -- cargo test")],
            "cargo test",
            false.into(),
        ),
        (
            vec![tool_use("a", "Bash", "ls")],
            "cargo test",
            false.into(),
        ),
        // A run a hook refused never ran, so the edit before it still stands.
        (
            vec![
                tool_use("a", "Edit", ""),
                tool_use("b", "Bash", "cargo test"),
                tool_result(
                    "b",
                    "PreToolUse:Bash hook error: anamnesis: log a pinned prediction first",
                ),
            ],
            "ana run x -- cargo test",
            true.into(),
        ),
        // The call being judged is already in the transcript when the hook runs.
        (
            vec![
                tool_use("a", "Edit", ""),
                tool_use("now", "Bash", "cargo test"),
            ],
            "cargo test",
            true.into(),
        ),
    ];
    for (i, (lines, cmd, want)) in cases.iter().enumerate() {
        let s = setup(&format!("edit{i}"));
        let transcript = s.dir.join("t.jsonl");
        fs::write(&transcript, lines.join("\n") + "\n").unwrap();
        let mut p: serde_json::Value = serde_json::from_str(&payload("e", cmd)).unwrap();
        p["transcript_path"] = transcript.display().to_string().into();
        p["tool_use_id"] = "now".into();
        hook(&s, "pre-tool", &p.to_string(), true);
        assert_eq!(
            &log(&s)[0]["after_edit"],
            want,
            "case {i}: {cmd} after {lines:?}"
        );
        let _ = fs::remove_dir_all(&s.dir);
    }
}

#[test]
fn the_edit_question_goes_unanswered_when_the_transcript_is_cut_or_missing() {
    let s = setup("editcut");
    let transcript = s.dir.join("t.jsonl");
    // Longer than the tail that is read, with nothing to answer it inside that tail.
    let filler = tool_use("f", "Read", "") + "\n";
    fs::write(&transcript, filler.repeat(300 * 1024 / filler.len() + 1)).unwrap();
    let mut p: serde_json::Value = serde_json::from_str(&payload("c", "cargo test")).unwrap();
    p["transcript_path"] = transcript.display().to_string().into();
    hook(&s, "pre-tool", &p.to_string(), true);
    hook(&s, "pre-tool", &payload("d", "cargo test"), true);
    let l = log(&s);
    assert!(l[0]["after_edit"].is_null(), "{l:?}");
    assert_eq!(l[0]["model"], "claude-opus-5-5");
    assert!(l[1]["after_edit"].is_null(), "{l:?}");
    assert_eq!(l[0]["v"], 3);
    let _ = fs::remove_dir_all(&s.dir);
}

fn add(s: &Setup, cwd: &std::path::Path, env: &[(&str, &str)]) -> serde_json::Value {
    let mut cmd = Command::new(ANA);
    // A test must not inherit the Claude Code session of whoever runs it.
    cmd.env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("AI_AGENT");
    cmd.current_dir(cwd)
        .args(["--data", s.ledger.to_str().unwrap()])
        .args(["add", "the suite passes", "--prob", "0.8"])
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("AI_AGENT");
    for (k, v) in env {
        cmd.env(k, v);
    }
    assert!(cmd.output().unwrap().status.success());
    let led: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&s.ledger).unwrap()).unwrap();
    led["claims"].as_array().unwrap().last().unwrap().clone()
}

/// A claim had only an agent-written date tag, so nothing could say how many of them came
/// from one long session.
#[test]
fn a_claim_logged_inside_claude_code_is_stamped_and_others_are_left_alone() {
    let s = setup("origin");
    let repo = s.dir.join("MyRepo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(repo.join("sub")).unwrap();

    let plain = add(&s, &repo.join("sub"), &[]);
    assert!(plain.get("origin").is_none(), "{plain}");

    let c = add(
        &s,
        &repo.join("sub"),
        &[
            ("CLAUDE_CODE_SESSION_ID", "sess-1"),
            ("AI_AGENT", "claude-code_2-1-318_agent"),
        ],
    );
    assert_eq!(c["origin"]["session"], "sess-1");
    assert_eq!(c["origin"]["repo"], "myrepo");
    assert_eq!(c["origin"]["client"], "claude-code_2-1-318_agent");

    // It survives every later save, and the report still reads the ledger.
    let id = c["id"].as_str().unwrap();
    let ok = Command::new(ANA)
        .env_remove("CLAUDE_CODE_SESSION_ID")
        .env_remove("AI_AGENT")
        .args(["--data", s.ledger.to_str().unwrap(), "resolve", id, "true"])
        .output()
        .unwrap();
    assert!(ok.status.success());
    let led: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&s.ledger).unwrap()).unwrap();
    assert_eq!(led["claims"][1]["origin"]["session"], "sess-1");
    let _ = fs::remove_dir_all(&s.dir);
}
