//! The hostile-review test matrix.
//!
//! Every test here is a scenario a skeptical reader would try in the first five
//! minutes, each one derived from a defect that was measured against a real
//! binary rather than imagined. They exist so that a fix cannot silently rot:
//! the numbers in the assertions are the numbers that were measured.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ANA: &str = env!("CARGO_BIN_EXE_ana");

fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ana_hn_{}_{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(ledger: &Path, args: &[&str]) -> (String, String, bool) {
    let out = Command::new(ANA)
        .arg("--data")
        .arg(ledger)
        .args(args)
        .output()
        .expect("failed to run ana");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

fn claim_count(ledger: &Path) -> usize {
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(ledger).unwrap()).unwrap();
    v["claims"].as_array().unwrap().len()
}

// ---------------------------------------------------------------- P0-1 storage

/// Scenario 10. Before the lock existed, 40 parallel `ana add` calls left 7-19
/// claims of 40: every writer loaded the same ledger and the last rename won.
/// Almost none of them reported an error, which is the part that makes it a
/// data-loss bug rather than a usability one.
#[test]
fn concurrent_adds_keep_every_claim() {
    for round in 0..3 {
        let dir = workdir(&format!("conc{round}"));
        let ledger = dir.join("ledger.json");
        let mut kids = Vec::new();
        for i in 0..40 {
            kids.push(
                Command::new(ANA)
                    .arg("--data")
                    .arg(&ledger)
                    .args(["add", &format!("claim {i}"), "--prob", "0.5"])
                    .spawn()
                    .expect("spawn"),
            );
        }
        for mut k in kids {
            assert!(k.wait().unwrap().success(), "a concurrent add failed");
        }
        assert_eq!(claim_count(&ledger), 40, "round {round}: claims were lost");

        let strays: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(
            strays.is_empty(),
            "round {round}: stray temp files {strays:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

/// Scenario 11. A ledger that will not parse is a data-loss emergency: refuse to
/// write, say where the file is, and leave the bytes untouched.
#[test]
fn corrupt_ledger_is_not_overwritten() {
    let dir = workdir("corrupt");
    let ledger = dir.join("ledger.json");
    let garbage = "{ this is not json";
    fs::write(&ledger, garbage).unwrap();

    let (_, stderr, ok) = run(&ledger, &["add", "anything", "--prob", "0.5"]);
    assert!(!ok, "a mutating command on a corrupt ledger must fail");
    assert!(
        stderr.contains("was NOT modified"),
        "error must promise the ledger is untouched, got: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(&ledger).unwrap(),
        garbage,
        "the corrupt ledger was rewritten"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Scenario 11, second half. `ana mcp` serves the *agent* ledger, so a corrupt
/// human ledger must not be able to stop it starting.
#[test]
fn corrupt_human_ledger_does_not_break_mcp() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;

    let dir = workdir("mcphome");
    fs::write(dir.join(".anamnesis.json"), "{ not json at all").unwrap();

    let mut child = Command::new(ANA)
        .arg("mcp")
        .env("HOME", &dir)
        .env("USERPROFILE", &dir)
        .env_remove("ANAMNESIS_DATA")
        .env("ANAMNESIS_AGENT_DATA", dir.join("agent.json"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mcp");

    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18"}}}}"#
    )
    .unwrap();
    stdin.flush().unwrap();

    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();

    let v: serde_json::Value = serde_json::from_str(&line).expect("mcp must answer valid JSON");
    assert!(
        v.get("result").is_some(),
        "mcp must serve despite a corrupt human ledger, got: {line}"
    );
    let _ = fs::remove_dir_all(&dir);
}

// ───────────────────────── ledger fixtures ──────────────────────────────────

/// Write a ledger of binary claims directly, so a scenario is a table of
/// `(probability, outcome)` rather than a few hundred subprocess calls.
///
/// Every claim gets a resolve-by date in the past and a resolution after it, in
/// creation order, which is what puts it in the evidence sequence.
fn write_ledger(path: &Path, claims: &[(f64, bool)]) {
    let mut out = String::from("{\"claims\":[");
    for (i, (p, y)) in claims.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let day = 1 + (i % 27);
        out.push_str(&format!(
            r#"{{"id":"c{i:05}","statement":"claim {i}","created_at":"2024-01-{day:02}T00:00:00Z","resolve_by":"2024-03-{day:02}","tags":["who:test"],"kind":"binary","forecasts":[{{"at":"2024-01-{day:02}T00:00:00Z","prob":{p}}}],"resolution":{{"at":"2024-04-{day:02}T00:00:00Z","outcome":"{}"}}}}"#,
            if *y { "true" } else { "false" }
        ));
    }
    out.push_str("]}");
    fs::write(path, out).unwrap();
}

/// A deterministic uniform stream — the same one `scoring` uses — so a scenario
/// that depends on chance still produces the same ledger on every machine.
fn splitmix(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

fn report_json(ledger: &Path) -> serde_json::Value {
    let out = Command::new(ANA)
        .args(["--json", "--data"])
        .arg(ledger)
        .arg("report")
        .output()
        .expect("run report");
    serde_json::from_slice(&out.stdout).expect("report must emit valid JSON")
}

fn all_views(ledger: &Path) -> Vec<String> {
    [
        "report",
        "report --plain",
        "report --badge",
        "report --html",
    ]
    .iter()
    .map(|cmd| {
        let args: Vec<&str> = cmd.split(' ').collect();
        run(ledger, &args).0
    })
    .chain(std::iter::once(run(ledger, &["--json", "report"]).0))
    .collect()
}

/// The strings that must never appear over a ledger that is not calibrated.
const PRAISE: [&str; 6] = [
    "DIALED IN",
    "Well calibrated",
    "well calibrated",
    "well-calibrated",
    "confidence is honest",
    "Keep doing what you're doing",
];

// ───────────────────────── P0-2 hindsight ───────────────────────────────────

/// Scenario 7. Ten claims logged at 0.5, updated to 0.99, then resolved YES.
/// This scored `Brier 0.000` and drew the compliment "your updates moved you
/// TOWARD the truth", which is the most obvious way to fool the tool and the
/// first thing a reader will try.
#[test]
fn hindsight_updates_do_not_improve_the_headline() {
    let dir = workdir("hindsight");
    let ledger = dir.join("ledger.json");
    for i in 0..10 {
        let (o, _, ok) = run(
            &ledger,
            &[
                "--json",
                "add",
                &format!("claim {i}"),
                "--prob",
                "0.5",
                "--by",
                "2020-01-01",
            ],
        );
        assert!(ok);
        let id = serde_json::from_str::<serde_json::Value>(&o).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(run(&ledger, &["update", &id, "--prob", "0.99"]).2);
        assert!(run(&ledger, &["resolve", &id, "yes"]).2);
    }

    let j = report_json(&ledger);
    assert_eq!(j["score_basis"], "first");
    let brier = j["brier"].as_f64().unwrap();
    assert!(
        (brier - 0.25).abs() < 1e-9,
        "headline must grade the pre-registered 0.5, got {brier}"
    );
    assert_eq!(j["brier_first"].as_f64().unwrap(), brier);
    assert!(
        j["brier_final"].as_f64().unwrap() < 0.01,
        "the final-forecast figure should still be shown for comparison"
    );
    assert_eq!(j["late_updates"].as_u64().unwrap(), 10);

    for view in all_views(&ledger) {
        assert!(
            !view.contains("TOWARD the truth"),
            "the praise must be gone from every view:\n{view}"
        );
    }
}

// ───────────────────── P0-5 the verdict, every surface ──────────────────────

/// Scenario 6. The ledger that produced `[DIALED IN] · well calibrated` while
/// its Brier skill was −0.520.
#[test]
fn coin_flips_at_90_and_sure_things_at_60_are_never_praised() {
    let dir = workdir("verdictb");
    let ledger = dir.join("ledger.json");
    let mut claims: Vec<(f64, bool)> = (0..50).map(|i| (0.9, i % 2 == 0)).collect();
    claims.extend((0..50).map(|_| (0.6, true)));
    write_ledger(&ledger, &claims);

    let j = report_json(&ledger);
    // DSC is exactly 0: higher stated confidence went with a LOWER hit rate, so
    // isotonic regression collapses both groups into one value. That outranks
    // anything about the level — shading these numbers down would not help,
    // because they carry no information about which calls come true.
    assert_eq!(j["verdict"], "calibrated_but_uninformative");
    assert_eq!(j["dsc"].as_f64().unwrap(), 0.0, "confidence orders nothing");
    assert!(j["mcb"].as_f64().unwrap() > j["mcb_null_q95"].as_f64().unwrap());
    assert!(j["eprocess"].as_f64().unwrap() >= 20.0);

    for view in all_views(&ledger) {
        for banned in PRAISE {
            assert!(!view.contains(banned), "{banned:?} appeared:\n{view}");
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

/// Scenario 1. Every forecast 0.5 on a fair coin: perfectly calibrated, and
/// completely uninformative. Brier 0.25 here is the best achievable score, not a
/// failure, and the report must never call this overconfident.
#[test]
fn always_fifty_fifty_on_a_fair_coin_is_uninformative_not_overconfident() {
    let dir = workdir("uninformative");
    let ledger = dir.join("ledger.json");
    write_ledger(
        &ledger,
        &(0..100).map(|i| (0.5, i % 2 == 0)).collect::<Vec<_>>(),
    );

    let j = report_json(&ledger);
    assert_eq!(j["verdict"], "calibrated_but_uninformative");
    assert!((j["brier"].as_f64().unwrap() - 0.25).abs() < 1e-9);
    for view in all_views(&ledger) {
        assert!(
            !view.contains("OVERCONFIDENT"),
            "not overconfident:\n{view}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

/// Scenario 2. Every forecast 0.5 while 80% of them come true: calibrated in no
/// sense, and the verdict must say so.
#[test]
fn always_fifty_fifty_on_an_eighty_percent_base_rate_is_miscalibrated() {
    let dir = workdir("biased");
    let ledger = dir.join("ledger.json");
    write_ledger(
        &ledger,
        &(0..100).map(|i| (0.5, i % 5 != 0)).collect::<Vec<_>>(),
    );

    let j = report_json(&ledger);
    let v = j["verdict"].as_str().unwrap();
    assert!(
        v == "biased_no" || v == "underconfident",
        "a 0.5-on-0.8 forecaster is miscalibrated, got {v}"
    );
    for view in all_views(&ledger) {
        for banned in PRAISE {
            assert!(!view.contains(banned), "{banned:?} appeared:\n{view}");
        }
    }
    let _ = fs::remove_dir_all(&dir);
}

/// Scenario 5. Symmetric overconfidence — 90% when the truth is 65%, 10% when
/// the truth is 35%. The old evidence test was structurally blind to it because
/// the two errors cancel.
#[test]
fn symmetric_overconfidence_is_detected() {
    let dir = workdir("symmetric");
    let ledger = dir.join("ledger.json");
    let mut st = 42u64;
    let claims: Vec<(f64, bool)> = (0..200)
        .map(|i| {
            let (p, truth) = if i % 2 == 0 { (0.9, 0.65) } else { (0.1, 0.35) };
            (p, splitmix(&mut st) < truth)
        })
        .collect();
    write_ledger(&ledger, &claims);

    let j = report_json(&ledger);
    assert!(
        j["eprocess"].as_f64().unwrap() >= 20.0,
        "the evidence test must see it: e={}",
        j["eprocess"]
    );
    assert_eq!(j["verdict"], "overconfident");
    let _ = fs::remove_dir_all(&dir);
}

/// Scenario 9. A calibrated forecaster using two-decimal probabilities — the way
/// an agent actually writes them — must not be told it has a calibration problem.
/// The old exact-value grouping reported 0.073 of calibration error at n=200
/// where the true value is 0.
#[test]
fn a_calibrated_two_decimal_forecaster_is_not_accused() {
    let dir = workdir("calibrated");
    let ledger = dir.join("ledger.json");
    let mut st = 2024u64;
    let claims: Vec<(f64, bool)> = (0..200)
        .map(|_| {
            let p = ((splitmix(&mut st) * 0.9 + 0.05) * 100.0).round() / 100.0;
            (p, splitmix(&mut st) < p)
        })
        .collect();
    write_ledger(&ledger, &claims);

    let j = report_json(&ledger);
    assert_eq!(j["verdict"], "no_evidence_of_miscalibration");
    assert!(j["eprocess"].as_f64().unwrap() < 20.0);
    assert!(
        j["mcb"].as_f64().unwrap() <= j["mcb_null_q95"].as_f64().unwrap(),
        "calibration error {} should sit at or under its noise floor {}",
        j["mcb"],
        j["mcb_null_q95"]
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Scenario 4. A confident miss at p = 1.0 must not produce an infinite log
/// score, and must not serialise as `null`.
#[test]
fn a_confident_miss_keeps_the_log_score_finite() {
    let dir = workdir("confidentmiss");
    let ledger = dir.join("ledger.json");
    let mut claims: Vec<(f64, bool)> = (0..30).map(|_| (0.99, true)).collect();
    claims.push((1.0, false));
    write_ledger(&ledger, &claims);

    let j = report_json(&ledger);
    let ls = j["log_score"].as_f64().expect("log score must not be null");
    assert!(ls.is_finite(), "log score must be finite, got {ls}");
    let _ = fs::remove_dir_all(&dir);
}

// ─────────────────── P0-3 the evidence test, end to end ─────────────────────

/// Scenario 8. One batch of same-deadline questions from a PERFECTLY calibrated
/// forecaster, with the report re-read as answers arrive. Ordering the evidence
/// by resolution time raised a false alarm in 100% of simulated runs, because
/// YES answers land early and NO answers wait for the deadline.
#[test]
fn peeking_at_a_calibrated_batch_raises_no_alarm() {
    let dir = workdir("peeking");
    let ledger = dir.join("ledger.json");
    let mut st = 7u64;
    let claims: Vec<(f64, bool)> = (0..60)
        .map(|_| {
            let p = (splitmix(&mut st) * 0.9 + 0.05 * 1.0).min(0.95);
            let p = (p * 100.0).round() / 100.0;
            (p, splitmix(&mut st) < p)
        })
        .collect();

    // Look again after every five more answers, exactly as a user would.
    for k in (5..=claims.len()).step_by(5) {
        write_ledger(&ledger, &claims[..k]);
        let e = report_json(&ledger)["eprocess"].as_f64().unwrap_or(0.0);
        assert!(
            e < 20.0,
            "false alarm on a calibrated forecaster at n={k}: e={e}"
        );
    }
    let _ = fs::remove_dir_all(&dir);
}

/// Finding D, through the real gate. An agent that says 0.85 when it succeeds
/// 65% of the time and 0.15 when it succeeds 35% of the time used to be told
/// `{"act":"proceed","used_recalibration":false}` — the uncertainty was
/// measured, and then not acted on, which is the whole failure the decision gate
/// exists to prevent.
#[test]
fn the_decision_gate_corrects_a_straddling_agent() {
    let dir = workdir("gate");
    let ledger = dir.join("ledger.json");
    let mut st = 11u64;
    let claims: Vec<(f64, bool)> = (0..600)
        .map(|_| {
            let high = splitmix(&mut st) < 0.5;
            let (p, truth) = if high { (0.85, 0.65) } else { (0.15, 0.35) };
            (p, splitmix(&mut st) < truth)
        })
        .collect();
    write_ledger(&ledger, &claims);

    let (o, _, ok) = run(
        &ledger,
        &[
            "--json",
            "decide",
            "--prob",
            "0.85",
            "--stake",
            "3",
            "--verify-cost",
            "0.6",
        ],
    );
    assert!(ok);
    let j: serde_json::Value = serde_json::from_str(&o).unwrap();
    assert_eq!(j["used_recalibration"], true, "the gate must open: {o}");
    assert_ne!(j["act"], "proceed", "0.85 is really 0.65 here: {o}");
    assert!(
        j["adjusted"].as_f64().unwrap() < 0.75,
        "the correction should pull 0.85 well down: {o}"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Claims with no `--by` date fall back to their creation date plus a grace
/// horizon, which is equally fixed before the outcome — so they count, and the
/// tool still works on every ledger written before `--by` was encouraged.
/// Discarding them (the first design) would have thrown away all 35 graded claims
/// in the tool's own demo ledger.
#[test]
fn claims_without_a_deadline_still_count_as_evidence() {
    let dir = workdir("nodeadline");
    let ledger = dir.join("ledger.json");
    // Backdated well past the 30-day grace, and carrying no resolve_by at all.
    let mut out = String::from("{\"claims\":[");
    for i in 0..25 {
        if i > 0 {
            out.push(',');
        }
        let day = 1 + (i % 27);
        out.push_str(&format!(
            r#"{{"id":"n{i:05}","statement":"c{i}","created_at":"2024-01-{day:02}T00:00:00Z","tags":[],"kind":"binary","forecasts":[{{"at":"2024-01-{day:02}T00:00:00Z","prob":0.9}}],"resolution":{{"at":"2024-02-{day:02}T00:00:00Z","outcome":"false"}}}}"#
        ));
    }
    out.push_str("]}");
    fs::write(&ledger, out).unwrap();

    let j = report_json(&ledger);
    assert_eq!(
        j["evidence_n"].as_u64().unwrap(),
        25,
        "no-deadline claims must still reach the evidence test"
    );
    assert!(
        j["eprocess"].as_f64().unwrap() >= 20.0,
        "0.9 that never happens is evidence"
    );
    assert_eq!(j["verdict"], "overconfident");
    let _ = fs::remove_dir_all(&dir);
}

/// The cliff the grace period exists to remove. Without it, a claim is admitted
/// the day it is written, so ONE open no-deadline claim freezes the evidence test
/// from that moment on: log "will we hit 10k users?" in week one and never see a
/// number again.
#[test]
fn a_fresh_open_claim_does_not_freeze_the_evidence_test() {
    let dir = workdir("cliff");
    let ledger = dir.join("ledger.json");
    write_ledger(
        &ledger,
        &(0..30).map(|i| (0.9, i % 4 == 0)).collect::<Vec<_>>(),
    );

    let before = report_json(&ledger)["evidence_n"].as_u64().unwrap();
    assert_eq!(before, 30);

    // Add an open claim with no date, right now — the day-one mistake.
    let (o, _, ok) = run(
        &ledger,
        &["--json", "add", "will we hit 10k users", "--prob", "0.5"],
    );
    assert!(ok, "{o}");

    let after = report_json(&ledger)["evidence_n"].as_u64().unwrap();
    assert_eq!(
        after, before,
        "a claim logged today must not freeze the whole evidence test"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Voiding is the one operation that can retroactively edit a sequence whose
/// guarantee rests on being fixed in advance. Voiding something unresolved is
/// fine — it carries no outcome. Voiding something already resolved deletes an
/// outcome after seeing it, and the direction of abuse is self-flattery: you void
/// what went badly, and the e-value falls.
#[test]
fn voiding_a_resolved_claim_cannot_quietly_lower_the_evidence() {
    let dir = workdir("voidevidence");
    let ledger = dir.join("ledger.json");
    // 40 confident calls that mostly went wrong: demonstrable overconfidence.
    write_ledger(
        &ledger,
        &(0..40).map(|i| (0.9, i % 5 == 0)).collect::<Vec<_>>(),
    );

    let before = report_json(&ledger);
    let e_before = before["eprocess"].as_f64().unwrap();
    assert!(e_before >= 20.0, "should start as demonstrated: {e_before}");

    // Void six of the ones that went badly, after the fact.
    for i in 1..7 {
        assert!(
            run(
                &ledger,
                &[
                    "void",
                    &format!("c{i:05}"),
                    "--reason",
                    "on reflection I did not like this one",
                ],
            )
            .2
        );
    }

    let after = report_json(&ledger);
    assert_eq!(
        after["evidence_n"].as_u64().unwrap(),
        before["evidence_n"].as_u64().unwrap(),
        "outcomes already seen stay in the evidence sequence"
    );
    assert_eq!(
        after["eprocess"].as_f64().unwrap(),
        e_before,
        "and the e-value does not move"
    );
    assert_eq!(after["voided_after_resolution"].as_u64().unwrap(), 6);

    // The scores DO forget them — that is what void is for — but the report says
    // out loud that six outcomes were removed after they were seen.
    assert_eq!(after["resolved"].as_u64().unwrap(), 34);
    let text = run(&ledger, &["report"]).0;
    assert!(
        text.contains("voided AFTER they had already resolved"),
        "the edit must never be silent:\n{text}"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// One ungraded, already-due claim is **priced in**, not skipped and not fatal.
///
/// Skipping it would let the user decide after the fact which calls count.
/// Stopping there prevented that, but cost the whole test: any prefix rule yields
/// about `(1−g)/g` claims, so a 35%-ungraded ledger got a two-claim sequence no
/// matter how much it held. The gap now contributes the worst factor it could
/// have, which preserves the guarantee and keeps the record.
#[test]
fn an_overdue_ungraded_claim_is_priced_in_and_says_so() {
    let dir = workdir("blocked");
    let ledger = dir.join("ledger.json");
    write_ledger(
        &ledger,
        &(0..30).map(|i| (0.9, i % 4 == 0)).collect::<Vec<_>>(),
    );

    let before = report_json(&ledger);
    assert_eq!(before["evidence_n"].as_u64().unwrap(), 30);
    assert!(before["evidence_oldest_gap"].is_null());
    let e_before = before["eprocess"].as_f64().unwrap();

    // Add one open claim dated before all of them.
    let mut v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&ledger).unwrap()).unwrap();
    v["claims"].as_array_mut().unwrap().insert(
        0,
        serde_json::json!({
            "id": "blocker", "statement": "unanswered", "created_at": "2023-01-01T00:00:00Z",
            "resolve_by": "2023-02-01", "tags": [], "kind": "binary",
            "forecasts": [{"at": "2023-01-01T00:00:00Z", "prob": 0.5}]
        }),
    );
    fs::write(&ledger, serde_json::to_string(&v).unwrap()).unwrap();

    let after = report_json(&ledger);
    assert_eq!(
        after["evidence_n"].as_u64().unwrap(),
        30,
        "every graded call still counts — the gap does not truncate the record"
    );
    assert_eq!(after["evidence_oldest_gap"], "blocker");
    assert_eq!(
        after["evidence_ungraded_due"].as_u64().unwrap(),
        1,
        "the size of the backlog, priced in"
    );

    // The gap is a price: it can only lower the e-value, never raise it. That is
    // the whole validity argument, visible from outside the binary.
    let e_after = after["eprocess"].as_f64().unwrap();
    assert!(
        e_after < e_before,
        "an ungraded claim must cost evidence ({e_after} vs {e_before})"
    );
    assert!(
        after["evidence_gap_cost_log"].as_f64().unwrap() > 0.0,
        "and the report must say what it cost"
    );

    let text = run(&ledger, &["report"]).0;
    assert!(text.contains("blocker"), "the report must name it:\n{text}");
    assert!(
        text.contains("priced in at their worst case"),
        "and explain that it is a price, not a wall:\n{text}"
    );
    assert!(
        text.contains("ana list --due"),
        "the line must name the way out:\n{text}"
    );

    // Voiding the bad question is still the documented way out, and grading it
    // gives the wealth back.
    assert!(
        run(
            &ledger,
            &["void", "blocker", "--reason", "never became knowable"]
        )
        .2
    );
    let healed = report_json(&ledger);
    assert_eq!(healed["evidence_n"].as_u64().unwrap(), 30);
    assert_eq!(
        healed["eprocess"].as_f64().unwrap(),
        e_before,
        "clearing the gap restores the evidence exactly"
    );
    let _ = fs::remove_dir_all(&dir);
}

// ─────────────────────────── P1-3 the MCP surface ───────────────────────────

/// Drive `ana mcp`: write each request, close the pipe, collect every reply.
fn mcp(ledger: &Path, requests: &[&str], env: &[(&str, &str)]) -> Vec<serde_json::Value> {
    use std::io::Write;
    use std::process::Stdio;

    let mut cmd = Command::new(ANA);
    cmd.arg("--data").arg(ledger).arg("mcp");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd
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
        .map(|l| serde_json::from_str(l).expect("every reply must be valid JSON-RPC"))
        .collect()
}

/// Scenario 14. The server used to answer an `initialize` for `"1999-01-01"` by
/// solemnly agreeing that it spoke `"1999-01-01"`, and answered the mandatory
/// `server/discover` with "method not found".
#[test]
fn mcp_negotiates_versions_and_implements_discover() {
    let dir = workdir("mcpproto");
    let ledger = dir.join("ledger.json");

    let replies = mcp(
        &ledger,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"1999-01-01"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        ],
        &[],
    );
    assert_eq!(replies.len(), 4);

    // 1) An unknown legacy version negotiates DOWN to one we really support.
    let v = replies[0]["result"]["protocolVersion"].as_str().unwrap();
    assert_ne!(v, "1999-01-01", "must not echo a version it does not speak");
    assert!(v.starts_with("202"), "got {v}");

    // 2) server/discover is mandatory in 2026-07-28 and must answer properly.
    let d = &replies[1]["result"];
    let supported: Vec<&str> = d["supportedVersions"]
        .as_array()
        .expect("supportedVersions is required")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(supported.contains(&"2026-07-28"));
    assert!(d["capabilities"]["tools"].is_object());
    assert!(d["_meta"]["io.modelcontextprotocol/serverInfo"]["name"].is_string());

    // 3) A modern request declaring a version we do not speak gets the specified
    //    error code, with the list of versions we do.
    assert_eq!(replies[2]["error"]["code"].as_i64().unwrap(), -32022);
    assert!(replies[2]["error"]["data"]["supported"].is_array());
    assert_eq!(replies[2]["error"]["data"]["requested"], "1999-01-01");

    // 4) A version we do support is echoed back unchanged.
    assert_eq!(replies[3]["result"]["protocolVersion"], "2025-06-18");
    let _ = fs::remove_dir_all(&dir);
}

/// The server tagged every prediction `who:claude`, whoever was actually
/// connected — which quietly corrupts any per-client comparison in the ledger.
#[test]
fn mcp_tags_predictions_with_the_real_client() {
    let dir = workdir("mcpwho");
    let ledger = dir.join("ledger.json");

    mcp(
        &ledger,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","clientInfo":{"name":"Cursor IDE"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"predict","arguments":{"statement":"it will work","prob":0.7,"by":"2026-01-01"}}}"#,
        ],
        &[],
    );
    let raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&ledger).unwrap()).unwrap();
    let tags = raw["claims"][0]["tags"].as_array().unwrap();
    assert!(
        tags.iter().any(|t| t == "who:cursor-ide"),
        "client name must reach the tag, got {tags:?}"
    );
    assert!(
        !tags.iter().any(|t| t == "who:claude"),
        "must not invent an identity: {tags:?}"
    );

    // An unknown client is `unknown`, not a guess.
    let l2 = dir.join("l2.json");
    mcp(
        &l2,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"predict","arguments":{"statement":"x","prob":0.5,"by":"2026-01-01"}}}"#,
        ],
        &[],
    );
    let raw2: serde_json::Value = serde_json::from_str(&fs::read_to_string(&l2).unwrap()).unwrap();
    assert!(raw2["claims"][0]["tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "who:unknown"));

    // ANAMNESIS_WHO overrides everything.
    let l3 = dir.join("l3.json");
    mcp(
        &l3,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","clientInfo":{"name":"Cursor"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"predict","arguments":{"statement":"x","prob":0.5,"by":"2026-01-01"}}}"#,
        ],
        &[("ANAMNESIS_WHO", "my-agent")],
    );
    let raw3: serde_json::Value = serde_json::from_str(&fs::read_to_string(&l3).unwrap()).unwrap();
    assert!(raw3["claims"][0]["tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "who:my-agent"));
    let _ = fs::remove_dir_all(&dir);
}

/// `void` and `amend` exist over MCP too, or an agent that notices its own bad
/// question has no way to annul it but to edit JSON behind the tool's back.
#[test]
fn mcp_mirrors_void_and_amend() {
    let dir = workdir("mcpvoid");
    let ledger = dir.join("ledger.json");

    let replies = mcp(
        &ledger,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"predict","arguments":{"statement":"teh thing works","prob":0.7,"by":"2026-01-01"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        ],
        &[],
    );
    let id = replies[0]["result"]["structuredContent"]["id"]
        .as_str()
        .expect("predict returns an id")
        .to_string();
    let names: Vec<&str> = replies[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(
        names.contains(&"void") && names.contains(&"amend"),
        "{names:?}"
    );

    let r = mcp(
        &ledger,
        &[
            &format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"amend","arguments":{{"id":"{id}","statement":"the thing works"}}}}}}"#
            ),
            &format!(
                r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"void","arguments":{{"id":"{id}","reason":"unanswerable as written"}}}}}}"#
            ),
        ],
        &[],
    );
    assert_eq!(
        r[0]["result"]["structuredContent"]["statement"],
        "the thing works"
    );
    assert_eq!(r[1]["result"]["structuredContent"]["void"], true);

    let raw = fs::read_to_string(&ledger).unwrap();
    assert!(raw.contains("teh thing works"), "the old wording is kept");
    assert!(raw.contains("unanswerable as written"));
    let _ = fs::remove_dir_all(&dir);
}

/// No surface may contradict the verdict. The demo ledger caught this: it
/// printed `VERDICT NO MISCALIBRATION FOUND` a few lines above
/// `+0.120 OVERCONFIDENT — you are bolder than you are right`, leaving the reader
/// to guess which of the two to believe.
#[test]
fn no_surface_contradicts_the_verdict() {
    let dir = workdir("consistency");

    // A gap large enough to look damning, on far too few calls to demonstrate it.
    let suggestive = dir.join("suggestive.json");
    write_ledger(
        &suggestive,
        &(0..24).map(|i| (0.8, i % 3 != 0)).collect::<Vec<_>>(),
    );
    let j = report_json(&suggestive);
    if !j["verdict"].as_str().unwrap().starts_with("over") {
        let text = run(&suggestive, &["report"]).0;
        assert!(
            !text.contains("OVERCONFIDENT"),
            "shouted a direction the verdict does not support:\n{text}"
        );
        for view in all_views(&suggestive) {
            for banned in PRAISE {
                assert!(!view.contains(banned), "{banned:?} appeared:\n{view}");
            }
        }
    }

    // And where it IS demonstrated, the direction must actually be named.
    let clear = dir.join("clear.json");
    write_ledger(&clear, &(0..200).map(|_| (0.9, false)).collect::<Vec<_>>());
    let j2 = report_json(&clear);
    assert_eq!(j2["verdict"], "overconfident");
    let text2 = run(&clear, &["report"]).0;
    assert!(text2.contains("OVERCONFIDENT"), "{text2}");
    assert!(text2.contains("VERDICT          OVERCONFIDENT"));
    let _ = fs::remove_dir_all(&dir);
}

/// Scenario 15. The installer must refuse a binary whose checksum does not match,
/// and refuse just as firmly when the release carries no checksum file at all.
/// A tool about not fooling yourself should not install an unverified binary
/// because verifying was inconvenient.
#[cfg(unix)]
#[test]
fn the_installer_fails_closed_on_a_bad_checksum() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin/install-ana.sh");
    if !script.exists() {
        return;
    }
    // The script only knows a few targets; skip where there is no mapping.
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => return,
    };

    let dir = workdir("installer");
    let rel = dir.join("rel");
    let home = dir.join("home");
    fs::create_dir_all(&rel).unwrap();
    fs::create_dir_all(&home).unwrap();
    let asset = rel.join(format!("ana-{target}"));
    fs::copy(ANA, &asset).unwrap();

    let run_installer = || {
        Command::new("bash")
            .arg(&script)
            .env("HOME", &home)
            .env(
                "ANAMNESIS_RELEASE_BASE",
                format!("file://{}", rel.display()),
            )
            .output()
            .expect("run installer")
    };
    let installed = || home.join(".anamnesis/bin/ana").exists();

    // 1. No checksum file at all → refuse.
    let out = run_installer();
    assert!(!out.status.success(), "must refuse without a checksum file");
    assert!(!installed(), "nothing may be installed");

    // 2. A checksum that does not match → refuse, and say so.
    let digest = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "cd {} && (sha256sum ana-{target} 2>/dev/null || shasum -a 256 ana-{target})",
            rel.display()
        ))
        .output()
        .expect("hash the asset");
    if !digest.status.success() {
        return; // no sha256 tool here; the shell script would skip too
    }
    let good = String::from_utf8_lossy(&digest.stdout).to_string();
    // Flip the first hex digit to something it is definitely not, rather than to
    // a fixed character it might already be.
    let mut tampered = good.clone();
    if let Some(i) = tampered.find(|c: char| c.is_ascii_hexdigit()) {
        let replacement = if tampered.as_bytes()[i] == b'a' {
            "b"
        } else {
            "a"
        };
        tampered.replace_range(i..i + 1, replacement);
    }
    assert_ne!(
        tampered, good,
        "the test's tampering must actually change it"
    );
    fs::write(rel.join("sha256.sum"), &tampered).unwrap();

    let out = run_installer();
    assert!(!out.status.success(), "must abort on a checksum mismatch");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("CHECKSUM MISMATCH"),
        "and say why: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!installed(), "nothing may be installed");

    // 3. The real checksum → installs.
    fs::write(rel.join("sha256.sum"), &good).unwrap();
    let out = run_installer();
    assert!(
        out.status.success(),
        "a matching checksum must install: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(installed(), "the binary should be in place");
    let _ = fs::remove_dir_all(&dir);
}

/// `ana export --anonymize` must remove every free-text field. This is the tool
/// for publishing a real record as evidence, so a leak here is a leak of the
/// user's actual predictions — and there is deliberately no un-anonymized mode,
/// because an export that can be run without the flag is one somebody will.
#[test]
fn export_removes_every_free_text_field() {
    let dir = workdir("export");
    let ledger = dir.join("ledger.json");

    let (o, _, ok) = run(
        &ledger,
        &[
            "--json",
            "add",
            "SECRET-STATEMENT about the acquisition",
            "--prob",
            "0.7",
            "--by",
            "2020-01-01",
            "--tags",
            "kind:estimate,who:me,private:very",
            "--because",
            "SECRET-REASONING",
        ],
    );
    assert!(ok);
    let id = serde_json::from_str::<serde_json::Value>(&o).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(run(&ledger, &["resolve", &id, "yes", "--note", "SECRET-NOTE"]).2);

    let (_, err, ok) = run(&ledger, &["export"]);
    assert!(!ok, "there must be no un-anonymized export");
    assert!(err.contains("--anonymize"), "{err}");

    let (dump, _, ok) = run(&ledger, &["export", "--anonymize"]);
    assert!(ok);
    for secret in [
        "SECRET-STATEMENT",
        "SECRET-REASONING",
        "SECRET-NOTE",
        "acquisition",
    ] {
        assert!(
            !dump.contains(secret),
            "{secret:?} survived the export:\n{dump}"
        );
    }
    assert!(
        !dump.contains(&id),
        "the original id must not survive either"
    );
    assert!(
        !dump.contains("private:very"),
        "unlisted tag namespaces are dropped"
    );

    // The numbers, which are the point, must all still be there.
    let v: serde_json::Value = serde_json::from_str(&dump).unwrap();
    let c = &v["claims"][0];
    assert_eq!(c["forecasts"][0]["prob"].as_f64().unwrap(), 0.7);
    assert_eq!(c["resolution"]["outcome"], "true");
    assert_eq!(c["kind"], "binary");
    let tags: Vec<&str> = c["tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t.as_str())
        .collect();
    assert!(tags.contains(&"kind:estimate") && tags.contains(&"who:me"));
    let _ = fs::remove_dir_all(&dir);
}

// ------------------------------------------------------- P0-2 revise-then-resolve

/// The hindsight exploit, driven the way a user actually drives it.
///
/// `resolve` reported the score of `current_prob` — the LAST forecast. With no way
/// to revise, first and last were always the same claim, so the bug was invisible:
/// a real agent ledger of 426 claims contained zero revisions because `update` was
/// CLI-only. Exposing `update` over MCP made it reachable, and the first end-to-end
/// run of predict → update → resolve printed `Brier 0.062` for a claim the record
/// scores at `0.360`.
///
/// The stored record was always right. It was the number read back to the person
/// who had just revised — the one moment they are most likely to believe it — that
/// was wrong.
#[test]
fn revising_then_resolving_reports_the_first_forecast_score() {
    let dir = workdir("revise");
    let ledger = dir.join("ledger.json");

    let (out, _, ok) = run(
        &ledger,
        &["add", "the migration is compatible", "--prob", "0.6"],
    );
    assert!(ok, "add failed: {out}");
    let id = out
        .split('[')
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap()
        .to_string();

    // Revise once the evidence arrives — the honest use of the verb.
    let (out, _, ok) = run(&ledger, &["update", &id, "--prob", "0.25"]);
    assert!(ok, "update failed: {out}");

    let (out, _, ok) = run(&ledger, &["resolve", &id, "no"]);
    assert!(ok, "resolve failed: {out}");
    assert!(
        out.contains("0.360"),
        "resolve must report the FIRST forecast's Brier (0.6 vs FALSE = 0.360):\n{out}"
    );
    assert!(
        !out.contains("you said 25%"),
        "the revised number is not the score:\n{out}"
    );
    assert!(
        out.contains("not graded"),
        "and the revision must still be visible, labelled:\n{out}"
    );

    // The same claim, the same conclusion, through the report.
    let (out, _, _) = run(&ledger, &["--json", "report"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["score_basis"], "first");
    assert!((v["brier_first"].as_f64().unwrap() - 0.36).abs() < 1e-9);
    assert!((v["brier_final"].as_f64().unwrap() - 0.0625).abs() < 1e-9);
    assert_eq!(
        v["late_updates"].as_u64().unwrap(),
        1,
        "a revision after the fact is counted and surfaced, not hidden"
    );
}

// ───────────────── the two instruments have opposite blind spots ─────────────

/// A forecaster drifting gently toward 50/50 is the e-process's blind spot, and
/// the report used to let the quiet instrument speak for both.
///
/// 10 points overconfident at n = 120: MCB 0.019 against a 0.014 noise floor,
/// while the e-process sits at 5.4 — far under the alarm at 20. Every surface
/// called such a ledger **"well calibrated"**: the verdict line, the badge, the
/// card, and the happiest cat in the program. (At 11 points and n = 160 the
/// numbers were 0.022 against 0.011 with e = 16.5 — same story, still quiet.)
///
/// This is finding B of the original audit arriving by a different road. The fix
/// is not softer wording, it is that a claim of calibration answers to BOTH
/// checks: the e-process is strong on sharp patterns and weak on gentle
/// shrinkage, while MCB-against-floor measures the size of an error but cannot
/// establish it is real.
#[test]
fn a_quiet_eprocess_never_speaks_for_the_calibration_error_too() {
    let dir = workdir("blindspot");
    let ledger = dir.join("ledger.json");

    // Deterministic: stated p cycles, truth is 11 points lower.
    let mut st = 0xABCD_u64;
    let claims: Vec<(f64, bool)> = (0..120)
        .map(|i| {
            let p = [0.60, 0.65, 0.70, 0.75, 0.80, 0.85, 0.90][i % 7];
            (p, splitmix(&mut st) < p - 0.10)
        })
        .collect();
    write_ledger(&ledger, &claims);

    let d = report_json(&ledger);
    let (mcb, floor) = (
        d["mcb"].as_f64().unwrap(),
        d["mcb_null_q95"].as_f64().unwrap(),
    );
    let e = d["eprocess"].as_f64().unwrap();

    // The construction has to actually land in the blind spot, or the test is
    // vacuous: magnitude above noise, sequential test still quiet.
    assert!(mcb > floor, "MCB {mcb} must exceed the floor {floor}");
    assert!(e < 20.0, "the e-process must still be quiet, got {e}");
    assert_eq!(d["verdict"], "no_evidence_of_miscalibration");

    // No surface may call this calibrated.
    for mode in [
        vec!["report"],
        vec!["report", "--plain"],
        vec!["report", "--badge"],
        vec!["report", "--html"],
    ] {
        let text = run(&ledger, &mode).0.to_lowercase();
        assert!(
            !text.contains("well calibrated") && !text.contains("well-calibrated"),
            "{mode:?} called a 10-points-overconfident ledger well calibrated:\n{text}"
        );
        assert!(
            !text.contains("dialed in"),
            "{mode:?} gave it the happiest face while MCB was above the floor:\n{text}"
        );
    }

    // And the disagreement is stated, not hidden behind the quiet check.
    let text = run(&ledger, &["report"]).0;
    assert!(
        text.contains("above its noise floor"),
        "the report must say the other instrument disagrees:\n{text}"
    );
    let plain = run(&ledger, &["report", "--plain"]).0;
    assert!(
        plain.contains("two checks disagree"),
        "and so must the plain view:\n{plain}"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// The per-kind breakdown describes only the calls that happen to be tagged, so
/// below real coverage it describes a self-selected slice — the same defect as a
/// calibration computed on a self-selected sample, one level down.
///
/// Measured on a real 426-claim agent ledger: 363 of 422 binary claims carried no
/// `kind:` tag at all, so the per-kind table, the per-kind e-values and the hook's
/// "worst type" line were all keyed off a field 86% of the data did not have.
#[test]
fn the_per_kind_breakdown_stays_collapsed_until_the_tag_covers_the_record() {
    let build = |path: &Path, tagged: usize, total: usize| {
        let mut out = String::from("{\"claims\":[");
        for i in 0..total {
            if i > 0 {
                out.push(',');
            }
            let day = 1 + (i % 27);
            let tags = if i < tagged {
                r#"["who:test","kind:tests-pass"]"#
            } else {
                r#"["who:test"]"#
            };
            out.push_str(&format!(
                r#"{{"id":"k{i:05}","statement":"claim {i}","created_at":"2024-01-{day:02}T00:00:00Z","resolve_by":"2024-03-{day:02}","tags":{tags},"kind":"binary","forecasts":[{{"at":"2024-01-{day:02}T00:00:00Z","prob":0.7}}],"resolution":{{"at":"2024-04-{day:02}T00:00:00Z","outcome":"{}"}}}}"#,
                if i % 10 < 7 { "true" } else { "false" }
            ));
        }
        out.push_str("]}");
        fs::write(path, out).unwrap();
    };

    let dir = workdir("kindcov");

    // 5 of 40 typed — a breakdown here would speak for an eighth of the record.
    let sparse = dir.join("sparse.json");
    build(&sparse, 5, 40);
    let d = report_json(&sparse);
    assert!((d["kind_coverage"].as_f64().unwrap() - 0.125).abs() < 1e-9);
    let text = run(&sparse, &["report"]).0;
    assert!(
        text.contains("By prediction kind   hidden"),
        "the table must stay collapsed:\n{text}"
    );
    assert!(
        text.contains("% of your graded calls carry a `kind:` tag"),
        "and say how thin the coverage is:\n{text}"
    );
    assert!(text.contains("--tags kind:"), "and how to fix it:\n{text}");

    // 30 of 40 typed — now it is describing the record, so it opens.
    let dense = dir.join("dense.json");
    build(&dense, 30, 40);
    let text = run(&dense, &["report"]).0;
    assert!(
        text.contains("By prediction kind   (gap~"),
        "past the coverage bar the breakdown appears:\n{text}"
    );
    assert!(
        !text.contains("hidden"),
        "and no longer apologises for itself:\n{text}"
    );
    let _ = fs::remove_dir_all(&dir);
}
