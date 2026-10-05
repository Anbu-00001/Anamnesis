//! `ana run`: settle a claim from the exit status of the command it was pinned to.
//!
//! The command is fixed when the claim is logged (`ana add --check "cargo test"`).
//! `ana run <id> -- cargo test` then runs what the caller passes, refuses anything that
//! is not the pinned command, and grades the claim from the process's own exit status.
//! A hook can only grade what Claude Code happens to report, and an agent that logs
//! "the tests pass" and then runs one narrow test can settle it with that. Pinning the
//! command before the outcome is known is what closes that.
//!
//! These use `true`, `false`, `sh` and `sleep`, so they run on Unix only.
#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const ANA: &str = env!("CARGO_BIN_EXE_ana");

fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ana_run_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn ana(ledger: &Path, args: &[&str]) -> Output {
    Command::new(ANA)
        .arg("--data")
        .arg(ledger)
        .args(args)
        .output()
        .expect("run ana")
}

fn text(o: &Output) -> (String, String) {
    (
        String::from_utf8_lossy(&o.stdout).to_string(),
        String::from_utf8_lossy(&o.stderr).to_string(),
    )
}

/// Log a claim pinned to `check` and return its id.
fn add_pinned(ledger: &Path, statement: &str, check: &str) -> String {
    let o = ana(
        ledger,
        &[
            "add",
            statement,
            "--prob",
            "0.8",
            "--by",
            "2099-01-01",
            "--tags",
            "kind:tests-pass",
            "--check",
            check,
        ],
    );
    let (out, err) = text(&o);
    assert!(o.status.success(), "add --check failed: {err}");
    // "added [abc123]  80%  ..."
    out.split(['[', ']'])
        .nth(1)
        .expect("an id in the reply")
        .to_string()
}

fn claim(ledger: &Path, id: &str) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(ledger).unwrap()).unwrap();
    v["claims"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no claim {id}"))
        .clone()
}

fn outcome(ledger: &Path, id: &str) -> Option<String> {
    claim(ledger, id)["resolution"]["outcome"]
        .as_str()
        .map(String::from)
}

#[test]
fn add_pins_the_check_and_show_displays_it() {
    let dir = workdir("pin");
    let ledger = dir.join("l.json");
    let id = add_pinned(&ledger, "the suite passes", "cargo test");
    assert_eq!(claim(&ledger, &id)["check"], "cargo test");
    let (shown, _) = text(&ana(&ledger, &["show", &id]));
    assert!(
        shown.contains("cargo test"),
        "show must display the pinned check:\n{shown}"
    );
    assert!(
        shown.contains(&format!("ana run {id} --")),
        "and say how to settle it:\n{shown}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_claim_logged_without_a_check_has_no_check_field_at_all() {
    let dir = workdir("nofield");
    let ledger = dir.join("l.json");
    ana(&ledger, &["add", "plain claim", "--prob", "0.6"]);
    let raw = fs::read_to_string(&ledger).unwrap();
    assert!(
        !raw.contains("\"check\""),
        "old ledgers must stay byte-identical:\n{raw}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_grades_true_from_a_zero_exit_and_passes_it_through() {
    let dir = workdir("true");
    let ledger = dir.join("l.json");
    let id = add_pinned(&ledger, "the check passes", "true");
    let o = ana(&ledger, &["run", &id, "--", "true"]);
    let (out, err) = text(&o);
    assert_eq!(o.status.code(), Some(0), "{out}{err}");
    assert_eq!(outcome(&ledger, &id).as_deref(), Some("true"), "{out}{err}");
    let r = &claim(&ledger, &id)["resolution"];
    assert_eq!(
        r["resolved_by"], "auto",
        "graded by an observed fact, not by the agent"
    );
    assert!(r["note"].as_str().unwrap().contains("exited 0"), "{r}");
    assert!(out.contains("resolved TRUE"), "{out}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_grades_false_from_a_nonzero_exit_and_passes_the_code_through() {
    let dir = workdir("false");
    let ledger = dir.join("l.json");
    let id = add_pinned(&ledger, "the check passes", "sh -c exit 3");
    let o = ana(&ledger, &["run", &id, "--", "sh", "-c", "exit 3"]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "ana run must exit with the command's own code"
    );
    assert_eq!(outcome(&ledger, &id).as_deref(), Some("false"));
    assert!(claim(&ledger, &id)["resolution"]["note"]
        .as_str()
        .unwrap()
        .contains("exited 3"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_commands_own_output_still_reaches_the_caller() {
    let dir = workdir("tee");
    let ledger = dir.join("l.json");
    let id = add_pinned(&ledger, "it prints", "echo hello-from-the-check");
    let o = ana(&ledger, &["run", &id, "--", "echo", "hello-from-the-check"]);
    let (out, _) = text(&o);
    assert!(
        out.contains("hello-from-the-check"),
        "the output was swallowed:\n{out}"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The point of pinning: the command cannot be chosen once the outcome is known. A
/// different command is refused, and it is not run at all.
#[test]
fn a_different_command_is_refused_and_never_run() {
    let dir = workdir("mismatch");
    let ledger = dir.join("l.json");
    let marker = dir.join("MARKER");
    let id = add_pinned(&ledger, "the full suite passes", "false");
    let touch = format!("touch {}", marker.display());
    let o = ana(&ledger, &["run", &id, "--", "sh", "-c", &touch]);
    let (_, err) = text(&o);
    assert!(!o.status.success());
    assert!(err.contains("pinned to `false`"), "{err}");
    assert!(
        !marker.exists(),
        "a command that did not match the pin was executed"
    );
    assert!(outcome(&ledger, &id).is_none(), "the claim must stay open");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn run_needs_an_open_claim_that_has_a_check() {
    let dir = workdir("preconditions");
    let ledger = dir.join("l.json");

    // No check pinned.
    let plain = {
        let (out, _) = text(&ana(&ledger, &["add", "no check here", "--prob", "0.6"]));
        out.split(['[', ']']).nth(1).unwrap().to_string()
    };
    let o = ana(&ledger, &["run", &plain, "--", "true"]);
    assert!(!o.status.success());
    assert!(text(&o).1.contains("no check pinned"), "{}", text(&o).1);

    // Already resolved.
    let done = add_pinned(&ledger, "resolve me first", "true");
    assert!(ana(&ledger, &["run", &done, "--", "true"]).status.success());
    let o = ana(&ledger, &["run", &done, "--", "true"]);
    assert!(!o.status.success());
    assert!(text(&o).1.contains("already resolved"), "{}", text(&o).1);

    // Void.
    let gone = add_pinned(&ledger, "a question that went away", "true");
    assert!(ana(
        &ledger,
        &["void", &gone, "--reason", "no longer answerable"]
    )
    .status
    .success());
    let o = ana(&ledger, &["run", &gone, "--", "true"]);
    assert!(!o.status.success());
    let _ = fs::remove_dir_all(&dir);
}

/// A claim with a check can be settled one way, so `resolve` must not be a way round.
#[test]
fn resolve_refuses_a_pinned_claim_and_names_the_command() {
    let dir = workdir("resolverefuses");
    let ledger = dir.join("l.json");
    let id = add_pinned(&ledger, "the suite passes", "cargo test");
    let o = ana(&ledger, &["resolve", &id, "yes"]);
    let (_, err) = text(&o);
    assert!(!o.status.success());
    assert!(err.contains("cargo test"), "{err}");
    assert!(err.contains(&format!("ana run {id} --")), "{err}");
    assert!(outcome(&ledger, &id).is_none());
    let _ = fs::remove_dir_all(&dir);
}

/// A test run can take minutes. Holding the ledger lock across it blocked every other
/// writer, and the hooks, which also take the lock, hung behind it.
#[test]
fn the_ledger_is_not_locked_while_the_command_runs() {
    let dir = workdir("nolock");
    let ledger = dir.join("l.json");
    let id = add_pinned(&ledger, "a slow check passes", "sleep 2");

    let slow = {
        let ledger = ledger.clone();
        let id = id.clone();
        std::thread::spawn(move || ana(&ledger, &["run", &id, "--", "sleep", "2"]))
    };
    std::thread::sleep(Duration::from_millis(500));
    let t = Instant::now();
    let o = ana(
        &ledger,
        &["add", "written while the check runs", "--prob", "0.5"],
    );
    let waited = t.elapsed();
    assert!(o.status.success(), "{}", text(&o).1);
    assert!(
        waited < Duration::from_millis(1200),
        "`ana add` waited {waited:?}: the ledger lock was held while the command ran"
    );

    let o = slow.join().unwrap();
    assert!(o.status.success(), "{}", text(&o).1);
    assert_eq!(outcome(&ledger, &id).as_deref(), Some("true"));
    let (listing, _) = text(&ana(&ledger, &["list"]));
    assert!(
        listing.contains("written while the check runs"),
        "the concurrent add was lost"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// A command killed by a signal has no exit status. Nothing says whether the check
/// passed, so nothing is graded.
#[test]
fn a_command_ended_by_a_signal_is_not_graded() {
    let dir = workdir("signal");
    let ledger = dir.join("l.json");
    let id = add_pinned(&ledger, "the check survives", "sh -c kill -9 $$");
    let o = ana(&ledger, &["run", &id, "--", "sh", "-c", "kill -9 $$"]);
    let (_, err) = text(&o);
    assert!(!o.status.success());
    assert!(err.contains("signal"), "{err}");
    assert!(
        outcome(&ledger, &id).is_none(),
        "graded a run that has no exit status"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_command_that_cannot_start_is_not_graded() {
    let dir = workdir("nostart");
    let ledger = dir.join("l.json");
    let id = add_pinned(&ledger, "the check runs", "no_such_binary_xyz --flag");
    let o = ana(&ledger, &["run", &id, "--", "no_such_binary_xyz", "--flag"]);
    let (_, err) = text(&o);
    assert!(!o.status.success());
    assert!(err.contains("could not start"), "{err}");
    assert!(
        outcome(&ledger, &id).is_none(),
        "graded a command that never ran"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The ledger is text anyone can write, so `ana run` must never execute what it reads.
/// The check is compared to the command it is given and never run by itself.
#[test]
fn ana_run_never_executes_text_from_the_ledger() {
    let dir = workdir("noexec");
    let ledger = dir.join("l.json");
    let marker = dir.join("PWNED");
    let evil = format!("touch {}", marker.display());
    let id = add_pinned(&ledger, "a claim with a hostile check", &evil);
    // Everything an agent or hook could do with the claim except pass that exact command.
    ana(&ledger, &["show", &id]);
    ana(&ledger, &["list"]);
    ana(&ledger, &["report"]);
    ana(&ledger, &["run", &id, "--", "true"]);
    assert!(!marker.exists(), "text read from the ledger was executed");
    let _ = fs::remove_dir_all(&dir);
}

// ───────────────────────────── over MCP ─────────────────────────────

fn mcp(ledger: &Path, requests: &[String]) -> Vec<serde_json::Value> {
    let mut child = Command::new(ANA)
        .arg("--data")
        .arg(ledger)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mcp");
    {
        let mut stdin = child.stdin.take().unwrap();
        for r in requests {
            writeln!(stdin, "{r}").unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn mcp_can_pin_a_check_and_refuses_to_resolve_it_by_hand() {
    let dir = workdir("mcpcheck");
    let ledger = dir.join("l.json");
    let predict = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"predict","arguments":{"statement":"the suite passes","prob":0.8,"by":"2099-01-01","check":"cargo test"}}}"#.to_string();
    let r = mcp(&ledger, &[predict]);
    let id = r[0]["result"]["structuredContent"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(claim(&ledger, &id)["check"], "cargo test");

    let resolve = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"resolve","arguments":{{"id":"{id}","outcome":"yes"}}}}}}"#
    );
    let r = mcp(&ledger, &[resolve]);
    let text = r[0]["result"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(r[0]["result"]["isError"], true, "{text}");
    assert!(text.contains(&format!("ana run {id} --")), "{text}");
    assert!(outcome(&ledger, &id).is_none());
    let _ = fs::remove_dir_all(&dir);
}
