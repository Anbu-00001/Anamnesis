//! End-to-end tests that drive the compiled `ana` binary exactly as a user
//! would, against a throwaway ledger file. These prove the whole pipeline —
//! parsing, storage, the immutability of resolved history, and reporting —
//! not just the math in isolation.

use std::process::Command;

fn ana(data: &str, args: &[&str]) -> (String, String, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_ana"))
        .arg("--data")
        .arg(data)
        .args(args)
        .output()
        .expect("failed to run ana");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

/// Pull the `[id]` out of a line like: `added [abc123]  30%  "..."`.
fn extract_id(stdout: &str) -> String {
    let line = stdout.lines().next().expect("no output");
    let start = line.find('[').expect("no [") + 1;
    let end = line.find(']').expect("no ]");
    line[start..end].to_string()
}

#[test]
fn full_lifecycle() {
    let dir = std::env::temp_dir().join(format!("ana_it_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ledger.json");
    let data = path.to_str().unwrap();

    // add ----------------------------------------------------------------
    let (o, _, ok) = ana(
        data,
        &[
            "add",
            "It rains tomorrow",
            "-p",
            "0.3",
            "--tags",
            "weather",
            "--because",
            "dry front",
        ],
    );
    assert!(ok, "add should succeed");
    let id = extract_id(&o);
    assert_eq!(id.len(), 6, "id should be 6 hex chars, got '{id}'");

    // update keeps history ----------------------------------------------
    let (o, _, ok) = ana(
        data,
        &["update", &id, "-p", "0.6", "--because", "front stalled"],
    );
    assert!(ok);
    assert!(
        o.contains("30% → 60%"),
        "update should show the transition, got: {o}"
    );

    // list --open shows it ----------------------------------------------
    let (o, _, ok) = ana(data, &["list", "--open"]);
    assert!(ok && o.contains(&id), "open list should contain the claim");

    // resolve ------------------------------------------------------------
    let (o, _, ok) = ana(
        data,
        &["resolve", &id, "yes", "--note", "stalled as feared"],
    );
    assert!(ok && o.contains("resolved TRUE"), "resolve output: {o}");

    // resolved history is final -----------------------------------------
    let (_, e, ok) = ana(data, &["resolve", &id, "no"]);
    assert!(
        !ok && e.contains("already resolved"),
        "double-resolve must fail: {e}"
    );
    let (_, e, ok) = ana(data, &["update", &id, "-p", "0.9"]);
    assert!(
        !ok && e.contains("already resolved"),
        "updating resolved must fail: {e}"
    );

    // show renders the full palimpsest ----------------------------------
    let (o, _, ok) = ana(data, &["show", &id]);
    assert!(ok);
    assert!(o.contains("resolved TRUE"));
    assert!(
        o.contains("dry front") && o.contains("front stalled"),
        "both reasons should survive"
    );
    assert!(o.contains("Brier on final forecast"));

    // report works on a real ledger -------------------------------------
    let (o, _, ok) = ana(data, &["report"]);
    assert!(ok && o.contains("Brier score") && o.contains("Reliability diagram"));

    // input validation ---------------------------------------------------
    let (_, e, ok) = ana(data, &["add", "bad", "-p", "1.5"]);
    assert!(
        !ok && e.contains("between 0 and 1"),
        "out-of-range prob must be rejected: {e}"
    );

    // ambiguous / missing ids are friendly errors -----------------------
    let (_, e, ok) = ana(data, &["show", "zzzzzz"]);
    assert!(!ok && e.contains("no claim matches"));

    std::fs::remove_dir_all(&dir).ok();
}
