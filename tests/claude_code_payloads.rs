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
    let mut v: serde_json::Value = serde_json::from_str(&fixture("pass_echo")).unwrap();
    v["tool_input"]["command"] = "cargo test".into();
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
