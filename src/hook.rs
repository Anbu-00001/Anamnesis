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

use crate::model::{Claim, Ledger, Outcome, Resolution, ResolvedBy};
use crate::report::{ReportData, Verdict};
use crate::store;
use crate::untrusted;

/// Which hook is firing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    SessionStart,
    UserPrompt,
    PostTool,
    /// A tool call that failed. Claude Code sends a command that exits non-zero here
    /// and never as `PostToolUse`, so without this event a failing test is invisible.
    PostToolFailure,
    Stop,
}

impl Event {
    /// The `hookEventName` the harness expects back.
    fn name(self) -> &'static str {
        match self {
            Event::SessionStart => "SessionStart",
            Event::UserPrompt => "UserPromptSubmit",
            Event::PostTool => "PostToolUse",
            Event::PostToolFailure => "PostToolUseFailure",
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
        Verdict::Withheld => format!(
            "NO VERDICT: {} of the {} claims that came due were voided after their due date, so no reading of your {n} graded calls can be trusted. Resolve a claim when it answers; void only a question that stopped making sense before you knew.",
            d.voids.late, d.voids.came_due
        ),
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
        "  worst group: {}:{} is really {dir} (e={e:.0}, n={}, K={}) — trust those calls least",
        untrusted::tag(ns),
        untrusted::tag(&row.tag),
        row.n,
        d.group_k
    ))
}

/// The identities that count as this agent in the standing line.
///
/// Claude Code introduces itself to an MCP server as `claude-code` (captured from
/// 2.1.251), so everything the plugin's own MCP server logs carries
/// `who:claude-code`, while claims logged from the CLI under the documented
/// protocol carry `who:claude`. The standing line read only the second, so it was
/// blind to what the plugin itself had logged: 25 MCP predictions at 95%, all
/// wrong, produced an OVERCONFIDENT report and a hook that said nothing.
const AGENT_WHO: [&str; 2] = ["who:claude", "who:claude-code"];

/// This agent's claims, under either identity, as one agent.
///
/// The ledger on disk keeps the true client on every claim. In this private copy
/// both identities are `who:claude`, so the report logic sees one agent and the
/// grouping selector cannot mistake the two names for a two-group breakdown.
fn agent_claims(ledger: &Ledger) -> Ledger {
    let mut claims = Vec::new();
    for c in &ledger.claims {
        if !c.tags.iter().any(|t| AGENT_WHO.contains(&t.as_str())) {
            continue;
        }
        let mut c = c.clone();
        let mut tags: Vec<String> = Vec::with_capacity(c.tags.len());
        for t in c.tags.drain(..) {
            let t = if t == "who:claude-code" {
                "who:claude".to_string()
            } else {
                t
            };
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
        c.tags = tags;
        claims.push(c);
    }
    Ledger { claims }
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
    let lines: Vec<String> = ledger
        .claims
        .iter()
        .filter(|c| !c.is_void() && c.is_due(today))
        .filter(|c| c.tags.iter().any(|t| t == &format!("project:{slug}")))
        .take(5)
        .map(|c| format!("  DUE {} — resolve it", quoted(c)))
        .collect();
    framed(lines)
}

/// One claim as a single quoted line of stored data: `[id] "statement"`.
fn quoted(c: &Claim) -> String {
    format!(
        "[{}] \"{}\"",
        untrusted::tag(&c.id),
        untrusted::line(&c.statement, untrusted::MAX_LINE)
    )
}

/// Put the "this is data" label in front of any block of ledger text.
fn framed(lines: Vec<String>) -> Vec<String> {
    if lines.is_empty() {
        return lines;
    }
    let mut out = vec![format!("  ({})", untrusted::FRAME)];
    out.extend(lines);
    out
}

/// A claim belongs to this project if it is tagged for it, or for no project at
/// all. A claim tagged for another project is not this session's business: the
/// ledger is global, so without this a claim logged anywhere is shown everywhere.
fn in_scope(c: &Claim, slug: &str) -> bool {
    let mine = format!("project:{slug}");
    c.tags.iter().any(|t| t == &mine) || !c.tags.iter().any(|t| t.starts_with("project:"))
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

/// The kinds of test runner whose exit status answers "did the tests pass".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Runner {
    Cargo,
    /// Everything else we recognise: pytest, go, npm and friends, flutter, gradle, maven.
    Other,
}

impl Runner {
    /// The exit codes that mean "the tests ran and at least one failed".
    ///
    /// Any other non-zero status means the run itself went wrong: a missing
    /// directory, a usage error, a crash before a test started. It says nothing about
    /// the claim, and grading it FALSE would record a failure that did not happen.
    /// Measured: `cd tiny && cargo test` with no `tiny` exits 1 without cargo ever
    /// running; cargo's code for a failing test is 101.
    fn failure_codes(self) -> &'static [i64] {
        match self {
            Runner::Cargo => &[101],
            Runner::Other => &[1],
        }
    }

    /// Whether the output of a failed run shows the claim's suite failing, not merely
    /// the runner stopping.
    ///
    /// Cargo exits 101 for a failing test, but also for a manifest it cannot find, an
    /// unknown subcommand and most other errors. Caught live: a session whose project
    /// directory was missing ran `cargo test --manifest-path tiny/Cargo.toml`, exited
    /// 101 having tested nothing, and was graded FALSE. So for cargo the output must
    /// say the tests failed, or that the code under test does not compile.
    fn confirms_failure(self, output: &str) -> bool {
        match self {
            Runner::Cargo => {
                output.contains("test result: FAILED") || output.contains("could not compile")
            }
            Runner::Other => true,
        }
    }
}

/// Flags that make a test command not run any test.
const NON_RUNNING: [&str; 10] = [
    "--no-run",
    "--collect-only",
    "--co",
    "--help",
    "-h",
    "--version",
    "-V",
    "--list",
    "--listTests",
    "--dry-run",
];

/// A test run whose exit status is a trustworthy answer, or `None`.
///
/// Claude Code sends no exit status. It sends a command that exited 0 as
/// `PostToolUse`, and one that exited non-zero as `PostToolUseFailure`, so the
/// event stands in for the status, and it stands in for the *shell's* status.
/// Anything that can make that differ from the runner's own is refused: a pipe
/// (the status is the last stage's), `||` (swallowed), `;` or `&` (a later command
/// wins, or it ran in the background), a subshell, a substitution, a negation.
/// Measured: `cargo test 2>&1 | tail -3` over a failing test and `cargo test || true`
/// both arrive as ordinary successes.
///
/// A leading `cd DIR &&` and `NAME=value` assignments are allowed, since they are how
/// agents actually run tests; the runner must then be the command itself, not a word
/// in an `echo`. Builds and lints are not tests: `cargo build` is not an answer to
/// "the tests pass".
fn test_run(cmd: &str) -> Option<Runner> {
    let cmd = cmd.trim();
    if cmd.is_empty() || cmd.contains(['|', ';', '`', '(', ')', '\n']) {
        return None;
    }
    // `2>&1` and friends are redirections, not background jobs.
    let flat = cmd
        .replace("2>&1", " ")
        .replace("1>&2", " ")
        .replace(">&2", " ")
        .replace("&>", " ");
    let mut segments: Vec<&str> = flat.split("&&").map(str::trim).collect();
    let last = segments.pop()?;
    let is_cd = |s: &str| {
        let t: Vec<&str> = s.split_whitespace().collect();
        t.len() == 2 && t[0] == "cd"
    };
    if segments.iter().any(|s| !is_cd(s)) || last.starts_with('!') || last.contains('&') {
        return None;
    }
    let mut toks: Vec<&str> = last.split_whitespace().collect();
    while toks.first().is_some_and(|t| is_assignment(t)) {
        toks.remove(0);
    }
    if toks.iter().any(|t| NON_RUNNING.contains(t)) {
        return None;
    }
    let t = toks.as_slice();
    let runner = match t {
        ["cargo", rest @ ..] => {
            // Skip a toolchain selector: `cargo +nightly test`.
            let rest: &[&str] = match rest {
                [first, tail @ ..] if first.starts_with('+') => tail,
                _ => rest,
            };
            match rest {
                ["test", ..] => Runner::Cargo,
                ["nextest", "run", ..] => Runner::Cargo,
                _ => return None,
            }
        }
        ["npm" | "pnpm" | "yarn" | "bun", "test" | "t" | "tst", ..]
        | ["npm" | "pnpm" | "yarn" | "bun", "run" | "run-script", "test", ..] => Runner::Other,
        ["pytest" | "py.test" | "jest" | "vitest", ..]
        | ["python" | "python3", "-m", "pytest", ..]
        | ["npx" | "pnpx" | "bunx", "jest" | "vitest" | "pytest", ..]
        | ["go" | "flutter" | "dart", "test", ..] => Runner::Other,
        ["gradle" | "./gradlew" | "gradlew" | "mvn" | "./mvnw", rest @ ..]
            if rest.contains(&"test") =>
        {
            Runner::Other
        }
        _ => return None,
    };
    Some(runner)
}

fn is_assignment(tok: &str) -> bool {
    match tok.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && !name.starts_with(|c: char| c.is_ascii_digit())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        None => false,
    }
}

/// Whether `cmd` mentions a test runner at all, even one whose status we will not
/// trust. Used only to say that a run was seen and could not be graded.
fn mentions_test_runner(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    [
        "cargo test",
        "cargo nextest",
        "npm test",
        "npm run test",
        "pnpm test",
        "yarn test",
        "pytest",
        "go test",
        "flutter test",
        "jest",
        "vitest",
        "gradle",
        "mvn",
    ]
    .iter()
    .any(|n| lower.contains(n))
}

/// Cargo exits 0 for a filter that matches nothing, printing `running 0 tests` for
/// every test binary. Nothing was tested, so a zero status is not a pass.
fn cargo_ran_no_tests(stdout: &str) -> bool {
    let counts: Vec<u64> = stdout
        .lines()
        .filter_map(|l| l.trim().strip_prefix("running "))
        .filter_map(|r| r.split_whitespace().next()?.parse().ok())
        .collect();
    !counts.is_empty() && counts.iter().all(|&n| n == 0)
}

/// The exit code Claude Code puts inside a failed call's `error` text, which reads
/// `"Exit code 101\n<output>"`.
fn exit_code_in(error: &str) -> Option<i64> {
    let digits: String = error
        .strip_prefix("Exit code ")?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// What a post-tool event says about the claim it might settle.
enum Grade {
    /// The runner's own exit status, taken from the event it arrived as.
    Exit(i64),
    /// A test command was seen, but its status cannot be trusted.
    Untrusted,
    /// Nothing to say.
    NotATest,
}

fn grade_of(event: Event, input: &Value, cmd: &str) -> Grade {
    let Some(runner) = test_run(cmd) else {
        return if event == Event::PostTool && mentions_test_runner(cmd) {
            Grade::Untrusted
        } else {
            Grade::NotATest
        };
    };
    match event {
        // Claude Code sends this event only for a call that succeeded.
        Event::PostTool => {
            let stdout = input
                .pointer("/tool_response/stdout")
                .and_then(Value::as_str)
                .unwrap_or("");
            if runner == Runner::Cargo && cargo_ran_no_tests(stdout) {
                Grade::Untrusted
            } else {
                Grade::Exit(0)
            }
        }
        Event::PostToolFailure => {
            if input.get("is_interrupt").and_then(Value::as_bool) == Some(true) {
                return Grade::NotATest;
            }
            let error = input.get("error").and_then(Value::as_str).unwrap_or("");
            match exit_code_in(error) {
                Some(code)
                    if runner.failure_codes().contains(&code) && runner.confirms_failure(error) =>
                {
                    Grade::Exit(code)
                }
                _ => Grade::NotATest,
            }
        }
        _ => Grade::NotATest,
    }
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

            let d = ReportData::compute(&agent_claims(&ledger), Some("who:claude"), 10, today);
            if let Some(line) = standing_line(&d) {
                context.push(line);
                if let Some(k) = worst_kind(&d) {
                    context.push(k);
                }
            }
            if d.evidence_ungraded_due > 0 {
                if let Some(id) = &d.evidence_oldest_gap {
                    context.push(format!(
                        "  {} ungraded call(s) are costing you evidence, oldest [{}] — resolve or void them",
                        d.evidence_ungraded_due,
                        untrusted::tag(id)
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

        Event::PostTool | Event::PostToolFailure => {
            let cmd = input
                .pointer("/tool_input/command")
                .and_then(Value::as_str)
                .unwrap_or("");
            // The exit status is the fact. Grading a "the tests will pass"
            // prediction from the agent's own account of what happened is the
            // weakest link in a self-graded ledger; this removes it.
            match grade_of(event, &input, cmd) {
                Grade::NotATest => return Ok(()),
                Grade::Exit(code) => {
                    let resolved = auto_resolve(ledger_path, &slug, cmd, code)?;
                    if resolved.is_empty() {
                        return Ok(());
                    }
                    let happened = code == 0;
                    context.push(format!(
                        "⟢ Anamnesis (ana {VERSION}): resolved {} prediction(s) from the command's exit status ({code}) — {}, not self-reported: {}",
                        resolved.len(),
                        if happened { "it passed" } else { "it failed" },
                        resolved
                            .iter()
                            .map(|i| untrusted::tag(i))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    user_message = Some(format!(
                        "anamnesis: auto-resolved {} prediction(s) from exit code {code}",
                        resolved.len()
                    ));
                }
                Grade::Untrusted => {
                    // A test command ran, but its exit status cannot be trusted
                    // (piped, chained, or no test ran). Say so, and say plainly
                    // that resolving it now is on the agent's word.
                    let open: Vec<String> = ledger
                        .claims
                        .iter()
                        .filter(|c| c.is_open() && !c.is_void())
                        .filter(|c| {
                            c.tags.iter().any(|t| t == "kind:tests-pass")
                                || c.tags.iter().any(|t| t == "kind:approach")
                        })
                        .take(5)
                        .map(|c| untrusted::tag(&c.id))
                        .collect();
                    if open.is_empty() {
                        return Ok(());
                    }
                    context.push(format!(
                        "⟢ Anamnesis (moment of truth): a test command just ran, but its exit status cannot be trusted here (it was piped, chained, or ran no tests), so nothing was graded automatically. Resolve your open prediction(s) about it NOW ({}), before hindsight rewrites how sure you were; this one is on your word.",
                        open.join(", ")
                    ));
                }
            }
        }

        Event::Stop => {
            let overdue: Vec<&Claim> = ledger
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
            // The count is the whole ledger's. The wording shown is only this
            // project's, and only as quoted data.
            let here: Vec<&Claim> = overdue
                .iter()
                .copied()
                .filter(|c| in_scope(c, &slug))
                .collect();
            context.extend(framed(
                here.iter()
                    .take(5)
                    .map(|c| format!("  {}", quoted(c)))
                    .collect(),
            ));
            if here.len() < overdue.len() {
                context.push(format!(
                    "  ({} more belong to other projects)",
                    overdue.len() - here.len()
                ));
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
    fn plain_test_runs_are_graded() {
        for cmd in [
            "cargo test",
            "cargo test --all -- --nocapture",
            "cargo +nightly test -p foo",
            "cargo nextest run",
            "  PYTHONPATH=. pytest -q ",
            "python3 -m pytest tests/",
            "npm test",
            "npm run test -- --watch=false",
            "pnpm test",
            "yarn test",
            "go test ./...",
            "flutter test",
            "npx vitest run",
            "./gradlew test",
            "mvn test",
            "cd /work/project && cargo test",
            "cd sub && RUST_LOG=debug cargo test 2>&1",
        ] {
            assert!(test_run(cmd).is_some(), "should be graded: {cmd}");
        }
    }

    #[test]
    fn anything_that_can_change_the_shells_status_is_not() {
        for cmd in [
            "cargo test | tail -3",
            "cargo test 2>&1 | tail -3",
            "cargo test || true",
            "cargo test; echo done",
            "cargo test &",
            "cargo test && echo ok",
            "echo cargo test",
            "! cargo test",
            "(cargo test)",
            "x=$(cargo test)",
            "cargo test\nexit 0",
            "cd a && cd b && cargo test | cat",
            "make && cargo test",
        ] {
            assert!(test_run(cmd).is_none(), "must not be graded: {cmd}");
        }
    }

    #[test]
    fn builds_lints_and_runs_that_test_nothing_are_not_tests() {
        for cmd in [
            "cargo build",
            "cargo clippy --all-targets",
            "cargo test --no-run",
            "cargo test -- --list",
            "pytest --collect-only",
            "pytest --help",
            "npm run build",
            "go build ./...",
            "ls -la",
            "git commit -m 'tests'",
            "ana run -- cargo test",
            "",
        ] {
            assert!(test_run(cmd).is_none(), "must not be graded: {cmd:?}");
        }
    }

    #[test]
    fn a_test_failure_is_only_the_runners_own_failure_code() {
        assert_eq!(test_run("cargo test").unwrap().failure_codes(), [101]);
        assert_eq!(test_run("pytest").unwrap().failure_codes(), [1]);
    }

    #[test]
    fn cargo_must_say_the_suite_failed_not_merely_that_it_stopped() {
        let cargo = Runner::Cargo;
        assert!(cargo.confirms_failure("Exit code 101\n\ntest result: FAILED. 0 passed; 1 failed"));
        assert!(cargo.confirms_failure("Exit code 101\nerror: could not compile `x` (lib test)"));
        assert!(!cargo.confirms_failure(
            "Exit code 101\nerror: manifest path `tiny/Cargo.toml` does not exist"
        ));
        assert!(!cargo.confirms_failure("Exit code 101\nerror: no such command: `tset`"));
        assert!(Runner::Other.confirms_failure("Exit code 1"));
    }

    #[test]
    fn the_exit_code_is_read_from_the_error_text() {
        assert_eq!(exit_code_in("Exit code 101\n\nrunning 1 test"), Some(101));
        assert_eq!(exit_code_in("Exit code 3"), Some(3));
        assert_eq!(exit_code_in("Command timed out"), None);
        assert_eq!(exit_code_in("exit code 3"), None);
    }

    #[test]
    fn cargo_with_nothing_to_run_is_not_a_pass() {
        assert!(cargo_ran_no_tests("running 0 tests\n\nrunning 0 tests\n"));
        assert!(!cargo_ran_no_tests("running 2 tests\n\nrunning 0 tests\n"));
        assert!(!cargo_ran_no_tests("no cargo output at all"));
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
    fn claude_and_claude_code_are_one_agent_and_other_clients_are_not() {
        let claim = |id: &str, tags: &[&str]| -> Claim {
            serde_json::from_value(serde_json::json!({
                "id": id, "statement": "s", "created_at": "2024-01-01T00:00:00Z",
                "tags": tags, "kind": "binary",
                "forecasts": [{"at": "2024-01-01T00:00:00Z", "prob": 0.7}]
            }))
            .unwrap()
        };
        let ledger = Ledger {
            claims: vec![
                claim("a", &["who:claude", "kind:x"]),
                claim("b", &["who:claude-code"]),
                claim("c", &["who:claude", "who:claude-code"]),
                claim("d", &["who:cursor"]),
                claim("e", &["kind:x"]),
            ],
        };
        let mine = agent_claims(&ledger);
        let ids: Vec<&str> = mine.claims.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        // One identity, spelled once, whichever name it arrived under.
        for c in &mine.claims {
            assert_eq!(c.tags.iter().filter(|t| t.starts_with("who:")).count(), 1);
            assert!(c.tags.contains(&"who:claude".to_string()));
        }
        assert!(ledger.claims[1]
            .tags
            .contains(&"who:claude-code".to_string()));
    }

    #[test]
    fn session_names_become_safe_filenames() {
        assert_eq!(sanitize("abc/../x"), "abc----x");
        assert!(!sanitize("a/b").contains('/'));
    }
}
