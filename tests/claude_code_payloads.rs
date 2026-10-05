//! The hook contract, pinned against payloads captured from a real Claude Code
//! session.
//!
//! `tests/fixtures/claude-code-2.1.251/` holds what Claude Code 2.1.251 actually
//! sent to a hook, captured on 2026-10-04 by running throwaway sessions with
//! project-local hooks that dumped their stdin, then stripped of paths and ids.
//!
//! Before these existed the post-tool hook was tested only against payloads this
//! repo wrote for itself, and it read a field, `tool_result_exit_code`, that Claude
//! Code never sends. In 689 real claims, none was ever graded by exit status. What
//! Claude Code really sends is this:
//!
//! - a command that exits 0 arrives as `PostToolUse`, with no status at all;
//! - a command that exits non-zero arrives as `PostToolUseFailure`, with the code
//!   only inside the text of `error` (`"Exit code 101\n..."`);
//! - a failing test piped to `tail`, or followed by `|| true`, arrives as an
//!   ordinary `PostToolUse`, because the shell's status was 0.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ANA: &str = env!("CARGO_BIN_EXE_ana");

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claude-code-2.1.251")
        .join(format!("{name}.json"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

struct Setup {
    dir: PathBuf,
    ledger: PathBuf,
}

/// A ledger holding one open `kind:tests-pass` claim for the fixtures' project
/// (their `cwd` is `/work/project`, so the project slug is `project`).
fn setup(name: &str) -> Setup {
    let dir = std::env::temp_dir().join(format!("ana_payload_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let ledger = dir.join("agent.json");
    let claim = serde_json::json!({
        "id": "testspass1", "statement": "the suite passes",
        "created_at": "2024-01-01T00:00:00Z", "resolve_by": "2099-01-01",
        "tags": ["who:claude", "kind:tests-pass", "project:project"], "kind": "binary",
        "forecasts": [{"at": "2024-01-01T00:00:00Z", "prob": 0.8}]
    });
    fs::write(
        &ledger,
        serde_json::json!({ "claims": [claim] }).to_string(),
    )
    .unwrap();
    Setup { dir, ledger }
}

/// Run `ana hook <event>` with `payload` on stdin; return its stdout.
fn hook(s: &Setup, event: &str, payload: &str) -> String {
    let mut child = Command::new(ANA)
        .args(["hook", event])
        .env("ANAMNESIS_AGENT_DATA", &s.ledger)
        .env("HOME", &s.dir)
        .env("USERPROFILE", &s.dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ana hook");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "ana hook {event} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// The claim's resolution: `(outcome, resolved_by, note)`, or `None` if still open.
fn resolution(s: &Setup) -> Option<(String, String, String)> {
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&s.ledger).unwrap()).unwrap();
    let r = &v["claims"][0]["resolution"];
    if r.is_null() {
        return None;
    }
    let get = |k: &str| r[k].as_str().unwrap_or("").to_string();
    Some((get("outcome"), get("resolved_by"), get("note")))
}

#[test]
fn a_passing_test_run_is_graded_true_from_the_payload_claude_code_really_sends() {
    for name in ["pass_cargo_test", "pass_cd_then_cargo_test"] {
        let s = setup(name);
        let out = hook(&s, "post-tool", &fixture(name));
        let (outcome, by, note) =
            resolution(&s).unwrap_or_else(|| panic!("{name}: the claim was not graded:\n{out}"));
        assert_eq!(outcome, "true", "{name}");
        assert_eq!(
            by, "auto",
            "{name}: graded by an observed fact, not by the agent"
        );
        assert!(note.contains("exited 0"), "{name}: {note}");
        assert!(out.contains("not self-reported"), "{name}: {out}");
        let _ = fs::remove_dir_all(&s.dir);
    }
}

#[test]
fn a_failing_test_run_is_graded_false_from_the_failure_event() {
    let s = setup("fail101");
    let out = hook(&s, "post-tool-failure", &fixture("fail_cargo_test_101"));
    let (outcome, by, note) = resolution(&s).unwrap_or_else(|| panic!("not graded:\n{out}"));
    assert_eq!(outcome, "false");
    assert_eq!(by, "auto");
    assert!(note.contains("exited 101"), "{note}");
    assert!(
        out.contains("PostToolUseFailure"),
        "wrong event name in the reply: {out}"
    );
    let _ = fs::remove_dir_all(&s.dir);
}

/// The shell's status was 0, so Claude Code sent `PostToolUse` for a run in which a
/// test failed. Grading it would record the opposite of what happened.
#[test]
fn a_run_whose_exit_status_was_swallowed_is_not_graded() {
    for name in ["masked_by_pipe", "masked_by_or_true"] {
        let s = setup(name);
        let out = hook(&s, "post-tool", &fixture(name));
        assert!(
            resolution(&s).is_none(),
            "{name}: graded a status that was not the runner's"
        );
        assert!(
            out.contains("this one is on your word"),
            "{name}: it must say nothing was graded, and why:\n{out}"
        );
        let _ = fs::remove_dir_all(&s.dir);
    }
}

#[test]
fn a_failure_that_is_not_a_test_run_is_left_alone() {
    let s = setup("notatest");
    let out = hook(&s, "post-tool-failure", &fixture("fail_not_a_test"));
    assert!(
        resolution(&s).is_none(),
        "graded a command that is not a test run"
    );
    assert!(out.trim().is_empty(), "{out}");
    let _ = fs::remove_dir_all(&s.dir);
}

/// `cd tiny && cargo test` where `cd` fails exits 1 without cargo ever running.
/// Cargo's code for a failed test is 101, so exit 1 says nothing about the claim.
#[test]
fn a_run_that_never_started_is_not_graded_false() {
    let s = setup("neverran");
    let out = hook(&s, "post-tool-failure", &fixture("fail_cd_missing_exit_1"));
    assert!(
        resolution(&s).is_none(),
        "graded a run that never happened:\n{out}"
    );
    let _ = fs::remove_dir_all(&s.dir);
}

#[test]
fn an_unrelated_command_that_succeeds_is_ignored() {
    let s = setup("echo");
    let out = hook(&s, "post-tool", &fixture("pass_echo"));
    assert!(resolution(&s).is_none());
    assert!(out.trim().is_empty(), "{out}");
    let _ = fs::remove_dir_all(&s.dir);
}

/// Exit 0 with "running 0 tests" is what a filter that matches nothing produces.
/// Nothing was tested, so nothing is graded.
#[test]
fn a_run_that_ran_no_tests_is_not_graded() {
    let s = setup("zerotests");
    let mut v: serde_json::Value = serde_json::from_str(&fixture("pass_cargo_test")).unwrap();
    v["tool_input"]["command"] = "cargo test no_such_test".into();
    v["tool_response"]["stdout"] =
        "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out"
            .into();
    hook(&s, "post-tool", &v.to_string());
    assert!(
        resolution(&s).is_none(),
        "graded a run in which no test ran"
    );
    let _ = fs::remove_dir_all(&s.dir);
}

/// The field the old hook read does not exist in what Claude Code sends, and a
/// payload that merely contains it must not be believed over the event it arrived as.
#[test]
fn an_invented_exit_status_field_is_not_believed() {
    let s = setup("invented");
    let mut v: serde_json::Value = serde_json::from_str(&fixture("pass_cargo_test")).unwrap();
    v["tool_result_exit_code"] = 1.into();
    hook(&s, "post-tool", &v.to_string());
    let (outcome, ..) = resolution(&s).expect("graded");
    assert_eq!(outcome, "true", "PostToolUse means the command succeeded");
    let _ = fs::remove_dir_all(&s.dir);
}

/// The plugin delivers the event: `hooks.json` registers it, and the launcher runs the
/// engine on the real payload. A hook that exists in the binary but is not wired in
/// is how the old grader stayed dead for a release cycle.
#[test]
fn the_plugin_registers_the_failure_event_and_its_launcher_grades_a_real_failure() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin/hooks");
    let reg: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("hooks.json")).unwrap()).unwrap();
    let cmd = reg["hooks"]["PostToolUseFailure"][0]["hooks"][0]["command"]
        .as_str()
        .expect("hooks.json must register PostToolUseFailure");
    assert!(cmd.contains("post-tool-failure.sh"), "{cmd}");
    assert_eq!(reg["hooks"]["PostToolUseFailure"][0]["matcher"], "Bash");

    #[cfg(unix)]
    {
        let s = setup("launcher");
        let bin_dir = Path::new(ANA).parent().unwrap();
        let path = format!(
            "{}:{}",
            bin_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut child = Command::new("bash")
            .arg(root.join("post-tool-failure.sh"))
            .env("PATH", path)
            .env("HOME", &s.dir)
            .env("ANAMNESIS_AGENT_DATA", &s.ledger)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn launcher");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(fixture("fail_cargo_test_101").as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let (outcome, by, _) = resolution(&s)
            .unwrap_or_else(|| panic!("the launcher did not grade a real failure:\n{stdout}"));
        assert_eq!((outcome.as_str(), by.as_str()), ("false", "auto"));
        let _ = fs::remove_dir_all(&s.dir);
    }
}

/// Caught in a live session: with the project directory missing, `cargo test
/// --manifest-path tiny/Cargo.toml` exits 101 having tested nothing, and was graded
/// FALSE. Cargo's 101 covers a failing test and most other errors alike.
#[test]
fn a_cargo_error_that_is_not_a_failing_suite_is_not_graded() {
    let s = setup("manifest");
    let mut v: serde_json::Value = serde_json::from_str(&fixture("fail_cargo_test_101")).unwrap();
    v["tool_input"]["command"] = "cargo test --manifest-path tiny/Cargo.toml".into();
    v["error"] = "Exit code 101\nerror: manifest path `tiny/Cargo.toml` does not exist".into();
    hook(&s, "post-tool-failure", &v.to_string());
    assert!(resolution(&s).is_none(), "graded a run that tested nothing");

    // But code that does not compile is a suite that does not pass.
    let s2 = setup("nocompile");
    v["error"] = "Exit code 101\nerror[E0425]: cannot find value `x`\nerror: could not compile `tiny` (lib test) due to 1 previous error".into();
    hook(&s2, "post-tool-failure", &v.to_string());
    let (outcome, ..) = resolution(&s2).expect("a compile failure should grade the claim");
    assert_eq!(outcome, "false");
    let _ = fs::remove_dir_all(&s.dir);
    let _ = fs::remove_dir_all(&s2.dir);
}

/// A pinned claim is settled by `ana run` and nothing else. If a hook graded it from
/// whichever test happened to run, an agent could still choose the run after the fact:
/// the very thing pinning exists to stop.
#[test]
fn the_hook_leaves_a_pinned_claim_alone() {
    let s = setup("pinned");
    let mut v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&s.ledger).unwrap()).unwrap();
    v["claims"][0]["check"] = "cargo test".into();
    fs::write(&s.ledger, v.to_string()).unwrap();
    hook(&s, "post-tool", &fixture("pass_cargo_test"));
    assert!(
        resolution(&s).is_none(),
        "a hook graded a claim that is pinned to `ana run`"
    );
    let _ = fs::remove_dir_all(&s.dir);
}

/// A hook runs on Claude Code's clock: a command hook defaults to a 600-second timeout
/// (30 on UserPromptSubmit). Waiting forever behind a writer would stall the session, so
/// a hook that cannot get the ledger gives up, says so, and writes nothing.
#[test]
fn a_hook_that_cannot_get_the_ledger_gives_up_and_says_so() {
    let s = setup("busy");
    let held = anamnesis::store::lock(&s.ledger).unwrap();
    let t = std::time::Instant::now();
    let mut child = Command::new(ANA)
        .args(["hook", "post-tool"])
        .env("ANAMNESIS_AGENT_DATA", &s.ledger)
        .env("ANAMNESIS_HOOK_LOCK_WAIT_MS", "300")
        .env("HOME", &s.dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(fixture("pass_cargo_test").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        t.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        t.elapsed()
    );
    assert!(out.status.success(), "a busy ledger must not fail the hook");
    assert!(
        stdout.contains("not graded"),
        "it must say it gave up:\n{stdout}"
    );
    assert!(stdout.contains("busy"), "{stdout}");
    drop(held);
    assert!(
        resolution(&s).is_none(),
        "nothing may be written without the lock"
    );

    // Released, the same payload grades the claim.
    hook(&s, "post-tool", &fixture("pass_cargo_test"));
    assert!(resolution(&s).is_some());
    let _ = fs::remove_dir_all(&s.dir);
}

/// "Fails soft into silence" used to cover a ledger that could not be read at all, so a
/// user with a corrupt file had hooks that never fired and no way to find out why.
#[test]
fn a_ledger_that_cannot_be_read_is_reported_not_swallowed() {
    let s = setup("corrupt");
    fs::write(&s.ledger, "{ this is not json").unwrap();
    let before = fs::read(&s.ledger).unwrap();
    let out = hook(
        &s,
        "session-start",
        &serde_json::json!({"session_id": "c", "cwd": "/work/project"}).to_string(),
    );
    let v: serde_json::Value =
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{e}: {out}"));
    let msg = v["systemMessage"]
        .as_str()
        .unwrap_or_else(|| panic!("no message:\n{out}"));
    assert!(msg.contains("could not be read"), "{msg}");
    assert!(msg.contains("Nothing was changed"), "{msg}");
    assert_eq!(
        fs::read(&s.ledger).unwrap(),
        before,
        "the hook must never touch a ledger it cannot read"
    );
    let _ = fs::remove_dir_all(&s.dir);
}

// ──────────────── found by an independent adversarial review of 0.4.1 ────────────────

/// Feed a `cargo test` payload with `command` swapped in, and say whether it was graded.
fn graded_command(name: &str, command: &str) -> bool {
    let s = setup(name);
    let mut v: serde_json::Value = serde_json::from_str(&fixture("pass_cargo_test")).unwrap();
    v["tool_input"]["command"] = command.into();
    hook(&s, "post-tool", &v.to_string());
    let graded = resolution(&s).is_some();
    let _ = fs::remove_dir_all(&s.dir);
    graded
}

/// `cd #x && cargo test`: `#` starts a comment, so bash runs `cd` and never cargo, and
/// exits 0. `cd .&exit && cargo test` backgrounds the `cd` and then runs `exit`. The
/// matcher only counted the tokens of a `cd` segment and checked `&` in the last one.
#[test]
fn a_cd_segment_cannot_hide_an_operator() {
    for (i, cmd) in [
        "cd #x && cargo test",
        "cd .&exit && cargo test",
        "cd a;b && cargo test",
        "cd $(pwd) && cargo test",
        "cd 'a b' && cargo test",
        "cargo test # && false",
    ]
    .iter()
    .enumerate()
    {
        assert!(
            !graded_command(&format!("cd{i}"), cmd),
            "graded a run that may not have happened: {cmd}"
        );
    }
    // The honest forms still grade.
    assert!(graded_command("cdok", "cd /work/project && cargo test"));
    assert!(graded_command("cdok2", "cd tiny && cargo test -q"));
}

/// Claude Code returns a background Bash call at once with empty output. A suite that
/// later fails has already been graded TRUE by then.
#[test]
fn a_background_run_is_not_graded() {
    let s = setup("background");
    let mut v: serde_json::Value = serde_json::from_str(&fixture("pass_cargo_test")).unwrap();
    v["tool_input"]["run_in_background"] = true.into();
    hook(&s, "post-tool", &v.to_string());
    assert!(
        resolution(&s).is_none(),
        "a call that returned before the tests finished was graded"
    );
    let _ = fs::remove_dir_all(&s.dir);
}

/// A quoted flag reaches the runner unquoted, and an environment variable can change what
/// the runner does: `cargo test '--no-run'` and `PYTEST_ADDOPTS=--co pytest` both exit 0
/// without running a test.
#[test]
fn quoting_and_environment_cannot_turn_a_runner_into_a_non_run() {
    for (i, cmd) in [
        "cargo test '--no-run'",
        "cargo test -- '--list'",
        "cargo test \"--no-run\"",
        "cargo test --no\\-run",
        "PYTEST_ADDOPTS=--co pytest",
        "GOFLAGS=-run=^$ go test ./...",
        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER=true cargo test",
        "pytest --setup-plan",
        "pytest --fixtures",
    ]
    .iter()
    .enumerate()
    {
        assert!(
            !graded_command(&format!("q{i}"), cmd),
            "graded a command that may not run tests: {cmd}"
        );
    }
    // Harmless, listed environment prefixes still grade.
    assert!(graded_command(
        "envok",
        "RUST_BACKTRACE=1 CI=true cargo test"
    ));
}

fn failure_graded(name: &str, command: &str, error: &str) -> Option<String> {
    let s = setup(name);
    let mut v: serde_json::Value = serde_json::from_str(&fixture("fail_cargo_test_101")).unwrap();
    v["tool_input"]["command"] = command.into();
    v["error"] = error.into();
    hook(&s, "post-tool-failure", &v.to_string());
    let r = resolution(&s).map(|(o, ..)| o);
    let _ = fs::remove_dir_all(&s.dir);
    r
}

/// 0.4.1 fixed this for cargo: exit 101 is not always a failed suite. Every other runner
/// still graded FALSE on any exit 1, which is also what a missing directory, a missing
/// module and a project with no test script produce.
#[test]
fn other_runners_need_evidence_of_a_failed_suite_too() {
    // Not a failed suite.
    for (i, (cmd, err)) in [
        (
            "cd /nonexistent && pytest",
            "Exit code 1\nbash: line 1: cd: /nonexistent: No such file or directory",
        ),
        (
            "python3 -m pytest",
            "Exit code 1\n/usr/bin/python3: No module named pytest",
        ),
        (
            "npm test",
            "Exit code 1\nnpm error Missing script: \"test\"",
        ),
        ("npm test", "Exit code 1\nError: no test specified"),
        (
            "go test ./...",
            "Exit code 1\ngo: go.mod file not found in current directory or any parent directory",
        ),
        ("pytest", "Exit code 1\nbash: pytest: command not found"),
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(
            failure_graded(&format!("nf{i}"), cmd, err),
            None,
            "graded FALSE without a failing suite: {cmd}"
        );
    }
    // A failed suite, with the runner's own evidence.
    for (i, (cmd, err)) in [
        ("pytest -q", "Exit code 1\nFAILED tests/test_x.py::test_a - assert 1 == 2\n=== 1 failed, 3 passed in 0.12s ==="),
        ("go test ./...", "Exit code 1\n--- FAIL: TestX (0.00s)\nFAIL\tpkg\t0.004s"),
        ("npx jest", "Exit code 1\nTests:       1 failed, 4 passed, 5 total"),
        ("npm test", "Exit code 1\n  2 passing\n  1 failing"),
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(failure_graded(&format!("f{i}"), cmd, err).as_deref(), Some("false"), "a real failure was not graded: {cmd}");
    }
}

/// A pass needs a test to have visibly run, so a pass and a failure are judged by the same
/// standard. `cargo test > log 2>&1` hid the output: the failure stayed open, but the pass
/// was graded TRUE on nothing.
#[test]
fn a_cargo_pass_needs_to_have_run_a_test() {
    let s = setup("redirected");
    let mut v: serde_json::Value = serde_json::from_str(&fixture("pass_cargo_test")).unwrap();
    v["tool_input"]["command"] = "cargo test > log 2>&1".into();
    v["tool_response"]["stdout"] = "".into();
    hook(&s, "post-tool", &v.to_string());
    assert!(resolution(&s).is_none(), "graded a pass nobody could see");

    // Only ignored tests: 0 passed.
    v["tool_input"]["command"] = "cargo test".into();
    v["tool_response"]["stdout"] =
        "running 1 test\ni\ntest result: ok. 0 passed; 0 failed; 1 ignored; 0 measured".into();
    hook(&s, "post-tool", &v.to_string());
    assert!(
        resolution(&s).is_none(),
        "graded a run in which no test passed"
    );

    // Several binaries, the last of them doc-tests with nothing in them.
    v["tool_response"]["stdout"] = "running 40 tests\n........\ntest result: ok. 40 passed; 0 failed; 0 ignored\n\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored".into();
    hook(&s, "post-tool", &v.to_string());
    assert!(
        resolution(&s).is_some(),
        "a suite with 40 passing tests and empty doc-tests must grade"
    );
    let _ = fs::remove_dir_all(&s.dir);
}

/// nextest exits 100 on a failed run and prints a summary, not `test result: FAILED`, so
/// a failure was never graded while a pass was. Not recognised yet: neither is graded.
#[test]
fn nextest_is_not_graded_in_either_direction() {
    assert!(!graded_command("nextest_pass", "cargo nextest run"));
    assert_eq!(
        failure_graded(
            "nextest_fail",
            "cargo nextest run",
            "Exit code 100\nSummary 5 tests run: 4 passed, 1 failed"
        ),
        None
    );
}
