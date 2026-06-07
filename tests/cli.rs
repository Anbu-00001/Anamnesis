//! End-to-end tests that drive the compiled `ana` binary exactly as a user
//! would, against a throwaway ledger file. These prove the whole pipeline —
//! parsing, storage, the immutability of resolved history, and reporting —
//! not just the math in isolation.

use std::io::Write;
use std::process::{Command, Stdio};

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

/// Add a binary claim at `prob` and immediately resolve it `outcome` (yes/no),
/// going through the real CLI both times — the canned way to grow a ledger.
fn add_resolve(data: &str, prob: &str, outcome: &str) {
    let (o, _, ok) = ana(data, &["add", "canned claim", "-p", prob]);
    assert!(ok, "add({prob}) failed: {o}");
    let id = extract_id(&o);
    let (o, e, ok) = ana(data, &["resolve", &id, outcome]);
    assert!(ok, "resolve({id},{outcome}) failed: {o}{e}");
}

/// Drive the `ana mcp` JSON-RPC server: write each request line to stdin, close
/// the pipe, and return everything it wrote to stdout.
fn ana_mcp(data: &str, requests: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ana"))
        .arg("--data")
        .arg(data)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn `ana mcp`");
    {
        let stdin = child.stdin.as_mut().expect("mcp stdin");
        for r in requests {
            writeln!(stdin, "{r}").expect("write mcp request");
        }
    } // stdin dropped here → EOF → server loop exits
    let out = child.wait_with_output().expect("wait for `ana mcp`");
    String::from_utf8_lossy(&out.stdout).into_owned()
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

/// End-to-end proof of Tier 1: the report surfaces the anytime-valid e-process
/// and gates the recalibration map on real evidence — through the actual binary.
#[test]
fn tier1_report_surfaces_eprocess_and_recalibration() {
    let dir = std::env::temp_dir().join(format!("ana_t1_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ledger.json");
    let data = path.to_str().unwrap();

    // 1) Too few, unremarkable resolutions → the e-process must report NO evidence
    //    and the report must NOT offer a correction (never recalibrate on noise).
    add_resolve(data, "0.6", "yes");
    add_resolve(data, "0.5", "no");
    add_resolve(data, "0.7", "yes");
    let (o, _, ok) = ana(data, &["report"]);
    assert!(ok, "report should succeed");
    assert!(o.contains("Is it real?"), "e-process line missing:\n{o}");
    assert!(
        o.contains("no real evidence"),
        "small n should read as no evidence:\n{o}"
    );
    assert!(
        !o.contains("Recalibration"),
        "must NOT offer a correction without evidence:\n{o}"
    );

    // 2) Pile on gross, consistent miscalibration — "10% sure" but it always
    //    happens — until the e-value blows past the significance threshold.
    for _ in 0..15 {
        add_resolve(data, "0.1", "yes");
    }
    let (o, _, ok) = ana(data, &["report"]);
    assert!(ok);
    assert!(
        o.contains("REAL"),
        "gross miscalibration should read as REAL:\n{o}"
    );
    assert!(
        o.contains("Recalibration"),
        "with evidence, a correction should appear:\n{o}"
    );
    // Tier 2 surfaces: a bootstrap band on the Brier, a recency trend (n ≥ 10),
    // and the confidence-vocabulary line.
    assert!(o.contains("bootstrap band"), "Brier band missing:\n{o}");
    assert!(o.contains("Lately"), "recency trend missing at n≥10:\n{o}");
    assert!(o.contains("Confidence vocab"), "vocab line missing:\n{o}");
    assert!(
        o.contains("Selective"),
        "selective-prediction line missing:\n{o}"
    );

    // 3) The JSON view exposes both, machine-readable, for any agent.
    let (o, _, ok) = ana(data, &["--json", "report"]);
    assert!(ok);
    assert!(
        o.contains("\"eprocess\""),
        "JSON must carry the e-value:\n{o}"
    );
    assert!(
        o.contains("\"recalibration\""),
        "JSON must carry the map:\n{o}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// End-to-end proof that the MCP `recalibrate` tool works over real stdio JSON-RPC
/// and honours the evidence gate (unchanged on noise, corrected once earned).
#[test]
fn mcp_recalibrate_tool_end_to_end() {
    let dir = std::env::temp_dir().join(format!("ana_mcp_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ledger.json");
    let data = path.to_str().unwrap();

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#;
    let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"recalibrate","arguments":{"prob":0.6}}}"#;

    // Empty ledger → no evidence → the tool hands the number back UNCHANGED.
    let resp = ana_mcp(data, &[init, call]);
    assert!(
        resp.contains("\"protocolVersion\""),
        "initialize must respond:\n{resp}"
    );
    assert!(
        resp.contains("unchanged"),
        "no evidence ⇒ unchanged:\n{resp}"
    );

    // Now teach it a gross, consistent miscalibration and the tool must correct.
    for _ in 0..15 {
        add_resolve(data, "0.1", "yes");
    }
    let resp = ana_mcp(data, &[init, call]);
    assert!(
        resp.contains("corrected from"),
        "with evidence the tool should correct:\n{resp}"
    );
    // The advertised tool list must include recalibrate.
    let list = ana_mcp(data, &[r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#]);
    assert!(
        list.contains("\"recalibrate\""),
        "tools/list must advertise recalibrate:\n{list}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// End-to-end proof of stakes-weighting: a stake-weighted Brier appears once
/// stakes vary, and a negative stake is rejected — through the real binary.
#[test]
fn stakes_weighting_end_to_end() {
    let dir = std::env::temp_dir().join(format!("ana_stake_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ledger.json");
    let data = path.to_str().unwrap();

    // Three ordinary (default-stake) calls…
    for _ in 0..3 {
        add_resolve(data, "0.6", "yes");
    }
    // …and one high-stake call that gets blown.
    let (o, _, ok) = ana(data, &["add", "high stakes", "-p", "0.9", "--stake", "5"]);
    assert!(ok, "add --stake should succeed: {o}");
    let id = extract_id(&o);
    let (_, e, ok) = ana(data, &["resolve", &id, "no"]);
    assert!(ok, "resolve should succeed: {e}");

    // The stake-weighted Brier line appears only because stakes vary.
    let (o, _, ok) = ana(data, &["report"]);
    assert!(ok);
    assert!(
        o.contains("Stake-weighted"),
        "stake-weighted Brier should appear when stakes vary:\n{o}"
    );

    // A negative stake is a friendly error.
    let (_, e, ok) = ana(data, &["add", "bad", "-p", "0.5", "--stake=-1"]);
    assert!(
        !ok && e.contains("stake must be"),
        "negative stake must be rejected: {e}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// End-to-end proof of the elicitation protocol: a second "consider the opposite"
/// estimate is averaged into the logged probability, and the outside-view
/// reference class is recorded in the reasoning trail.
#[test]
fn dialectical_elicitation_end_to_end() {
    let dir = std::env::temp_dir().join(format!("ana_elicit_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ledger.json");
    let data = path.to_str().unwrap();

    let (o, _, ok) = ana(
        data,
        &[
            "add",
            "the refactor's tests pass first try",
            "-p",
            "0.7",
            "--second-prob",
            "0.5",
            "--reference-class",
            "similar refactors pass ~60%",
            "--because",
            "looks clean",
        ],
    );
    assert!(ok, "add with elicitation should succeed: {o}");
    // 0.7 and 0.5 average to 0.60 — that is what gets logged.
    assert!(
        o.contains("60%"),
        "logged probability should be the dialectical mean 60%:\n{o}"
    );
    let id = extract_id(&o);

    // The reasoning trail records the outside view, both estimates, and the note.
    let (o, _, ok) = ana(data, &["show", &id]);
    assert!(ok);
    assert!(
        o.contains("outside view: similar refactors pass ~60%"),
        "{o}"
    );
    assert!(o.contains("dialectical 0.70 & 0.50 → 0.60"), "{o}");
    assert!(o.contains("looks clean"), "{o}");

    std::fs::remove_dir_all(&dir).ok();
}

/// Add a binary claim carrying tags and resolve it, through the real CLI.
fn add_resolve_tagged(data: &str, prob: &str, outcome: &str, tags: &str) {
    let (o, _, ok) = ana(data, &["add", "canned claim", "-p", prob, "--tags", tags]);
    assert!(ok, "tagged add({prob}) failed: {o}");
    let id = extract_id(&o);
    let (o, e, ok) = ana(data, &["resolve", &id, outcome]);
    assert!(ok, "resolve({id},{outcome}) failed: {o}{e}");
}

/// Add an 80% interval and resolve it to a value that falls outside — a miss.
fn add_miss_interval(data: &str, interval: &str, value: &str) {
    let (o, _, ok) = ana(data, &["add", "canned interval", "--interval", interval]);
    assert!(ok, "interval add failed: {o}");
    let id = extract_id(&o);
    let (o, e, ok) = ana(data, &["resolve", &id, "--value", value]);
    assert!(ok, "resolve interval({id}) failed: {o}{e}");
}

/// Tier 3 end-to-end: the per-kind multicalibration verdict names the kind that is
/// *really* miscalibrated (anytime-valid, so a fluky small kind cannot trip it), and
/// the numeric side earns a conformal width correction once coverage evidence is real.
#[test]
fn tier3_multicalibration_and_conformal_recalibration() {
    let dir = std::env::temp_dir().join(format!("ana_t3_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ledger.json");
    let data = path.to_str().unwrap();

    // One kind is grossly overconfident ("10% sure" yet it always happens); another
    // is fine. The verdict must single out the bad kind, by name and direction.
    for _ in 0..12 {
        add_resolve_tagged(data, "0.1", "yes", "kind:bug-hypothesis");
    }
    add_resolve_tagged(data, "0.7", "yes", "kind:tests-pass");
    add_resolve_tagged(data, "0.3", "no", "kind:tests-pass");

    let (o, _, ok) = ana(data, &["report"]);
    assert!(ok, "report should succeed:\n{o}");
    assert!(
        o.contains("By prediction kind"),
        "per-kind table missing:\n{o}"
    );
    assert!(
        o.contains("'bug-hypothesis' is really overconfident"),
        "multicalibration verdict should name the bad kind:\n{o}"
    );

    // Eight 80% intervals that all miss → coverage evidence becomes REAL and the
    // conformal correction tells the agent to WIDEN (residual ratio 2.0 ⇒ ×2.00).
    for _ in 0..8 {
        add_miss_interval(data, "0..10", "15");
    }
    let (o, _, ok) = ana(data, &["report"]);
    assert!(ok);
    assert!(
        o.contains("Numeric forecasts"),
        "numeric section missing:\n{o}"
    );
    assert!(
        o.contains("Recalibration: WIDEN"),
        "conformal width correction should fire on real evidence:\n{o}"
    );
    assert!(
        o.contains("multiply your interval half-widths by 2.00"),
        "width factor should be the residual quantile 2.00:\n{o}"
    );

    // The JSON view exposes the Tier 3 fields, machine-readable.
    let (o, _, ok) = ana(data, &["--json", "report"]);
    assert!(ok);
    for key in [
        "\"coverage_shrunk\"",
        "\"coverage_eprocess\"",
        "\"width_factor\"",
        "\"eprocess\"",
    ] {
        assert!(o.contains(key), "JSON must carry {key}:\n{o}");
    }

    std::fs::remove_dir_all(&dir).ok();
}

/// End-to-end proof of the decision gate: stakes raise the bar to proceed, and an
/// earned recalibration discounts a stated number into a different action — through
/// the real binary and the MCP `decide` tool.
#[test]
fn decision_gate_end_to_end() {
    let dir = std::env::temp_dir().join(format!("ana_decide_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ledger.json");
    let data = path.to_str().unwrap();

    // 1) On a clean ledger (no correction earned), the stakes alone move the bar.
    let (o, _, ok) = ana(data, &["decide", "-p", "0.85"]);
    assert!(
        ok && o.contains("PROCEED"),
        "0.85 at ordinary stake should proceed:\n{o}"
    );
    let (o, _, ok) = ana(data, &["decide", "-p", "0.85", "--stake", "5"]);
    assert!(
        ok && o.contains("VERIFY") && o.contains("96%"),
        "high stakes should demand ~96% and force a verify:\n{o}"
    );
    // A long shot is sent back to replan, not verified.
    let (o, _, _) = ana(data, &["decide", "-p", "0.30"]);
    assert!(o.contains("ABSTAIN"), "a long shot should abstain:\n{o}");

    // 2) Teach the ledger gross overconfidence ("90% sure" that keeps failing). Now
    //    the gate must DISCOUNT a stated 0.9 through the earned correction.
    for _ in 0..12 {
        add_resolve(data, "0.9", "no");
    }
    let (o, _, ok) = ana(data, &["--json", "decide", "-p", "0.9"]);
    assert!(ok, "decide should succeed:\n{o}");
    assert!(
        o.contains("\"used_recalibration\":true"),
        "with evidence the gate must use the correction:\n{o}"
    );
    assert!(
        !o.contains("\"act\":\"proceed\""),
        "a discounted 0.9 must no longer be a blind proceed:\n{o}"
    );

    // 3) The MCP decide tool works over real JSON-RPC stdio and is advertised.
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#;
    let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"decide","arguments":{"prob":0.9,"stake":1}}}"#;
    let resp = ana_mcp(data, &[init, call]);
    assert!(
        resp.contains("corrected"),
        "MCP decide must apply the earned correction:\n{resp}"
    );
    let list = ana_mcp(data, &[r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#]);
    assert!(
        list.contains("\"decide\""),
        "tools/list must advertise decide:\n{list}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
