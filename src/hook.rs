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
/// How long a hook waits for the ledger lock before giving up, in milliseconds.
/// Claude Code's own default for a command hook is 600 seconds (30 on UserPromptSubmit),
/// so a hook that waited forever would stall the session instead of failing.
const DEFAULT_LOCK_WAIT_MS: u64 = 2000;

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
    Ledger {
        claims,
        ..Default::default()
    }
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

/// Whether this is the first Stop of the session, remembered in a marker file beside the
/// prompt counters. The Stop hook speaks once per session, not at the end of every turn.
/// If the marker cannot be written there is nothing to remember it by, and it speaks.
fn first_stop_of_session(session: &str) -> bool {
    let Some(dir) = dirs_counters() else {
        return true;
    };
    let _ = std::fs::create_dir_all(&dir);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(format!("{}.stop", sanitize(session))))
        .is_ok()
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
    /// Any other non-zero status means the run itself went wrong: a missing directory, a
    /// usage error, a crash before a test started. It says nothing about the claim, and
    /// grading it FALSE would record a failure that did not happen.
    fn failure_codes(self) -> &'static [i64] {
        match self {
            Runner::Cargo => &[101],
            Runner::Other => &[1],
        }
    }

    /// Whether the evidence shows the suite failing, not merely the runner stopping.
    ///
    /// Cargo exits 101 for a failing test, but also for a manifest it cannot find, an
    /// unknown subcommand and most other errors, and a live session once graded a run
    /// that tested nothing as a failed suite. Every other runner exits 1 for a missing
    /// directory, a missing module and a project with no test script, as well as for a
    /// failing test; an independent review found each of those graded FALSE. So a
    /// failure needs the runner's own words for one, and none of the words for the other.
    fn confirms_failure(self, f: &RunFacts) -> bool {
        match self {
            Runner::Cargo => f.cargo_failed || f.compile_error,
            Runner::Other => f.fail_marker && !f.env_error,
        }
    }

    /// Whether the evidence shows a test passing, so a pass and a failure are judged by
    /// the same standard. `cargo test > log 2>&1` hides the output: its failure stayed
    /// ungraded while its pass was graded TRUE on nothing.
    fn confirms_pass(self, f: &RunFacts) -> bool {
        match self {
            Runner::Cargo => f.cargo_passed > 0,
            Runner::Other => true,
        }
    }
}

/// What a runner's output says, gathered line by line so that it holds however much was
/// printed. `ana run` feeds it as the command streams past; a hook feeds it the output
/// Claude Code reports. Reading only the end of the output once lost a passing run: a
/// loud `cargo test -- --nocapture` pushed `running 1 test` out of a 64 KB tail.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunFacts {
    /// The total from every cargo `test result: ok. N passed` line.
    pub cargo_passed: u64,
    /// A cargo `test result: FAILED` line.
    pub cargo_failed: bool,
    /// Cargo's `could not compile`: the code under test does not build.
    pub compile_error: bool,
    /// Any runner's own words for a failing test: `1 failed`, `--- FAIL`, `FAIL`.
    pub fail_marker: bool,
    /// Words that mean the run never got going: `command not found`, `No module named`.
    pub env_error: bool,
}

/// Phrases that say the run itself went wrong, whatever the exit code.
const ENV_ERRORS: [&str; 10] = [
    "command not found",
    "no such file or directory",
    "no module named",
    "no test specified",
    "missing script",
    "cannot find module",
    "go.mod file not found",
    "is not recognized as",
    "permission denied",
    "enoent",
];

impl RunFacts {
    /// The facts in a whole block of output.
    pub fn of(text: &str) -> RunFacts {
        let mut f = RunFacts::default();
        for line in text.lines() {
            f.observe_line(line);
        }
        f
    }

    /// Fold one line of output in.
    pub fn observe_line(&mut self, line: &str) {
        let l = line.trim();
        if l.is_empty() {
            return;
        }
        if let Some(rest) = l.strip_prefix("test result: ok.") {
            if let Some(n) = rest
                .split(';')
                .next()
                .and_then(|part| counts_before(part, "passed").next())
            {
                self.cargo_passed = self.cargo_passed.saturating_add(n);
            }
        } else if l.starts_with("test result: FAILED") {
            self.cargo_failed = true;
        }
        if l.contains("could not compile") {
            self.compile_error = true;
        }
        let lower = l.to_ascii_lowercase();
        if lower.starts_with("--- fail")
            || l == "FAIL"
            || l.starts_with("FAIL\t")
            || l.starts_with("FAIL ")
            || l.starts_with("FAILED ")
            || ["failed", "failing", "failures"]
                .iter()
                .any(|w| counts_before(&lower, w).any(|n| n > 0))
        {
            self.fail_marker = true;
        }
        if ENV_ERRORS.iter().any(|e| lower.contains(e)) {
            self.env_error = true;
        }
    }

    /// Fold another stream's facts into these (stdout and stderr are read separately).
    pub fn merge(&mut self, other: &RunFacts) {
        self.cargo_passed = self.cargo_passed.saturating_add(other.cargo_passed);
        self.cargo_failed |= other.cargo_failed;
        self.compile_error |= other.compile_error;
        self.fail_marker |= other.fail_marker;
        self.env_error |= other.env_error;
    }
}

/// Every `N` that is directly followed by a word starting with `word`, in `text`:
/// `1 failed, 3 passed` gives 1 for "failed" and 3 for "passed".
fn counts_before<'a>(text: &'a str, word: &'a str) -> impl Iterator<Item = u64> + 'a {
    let toks: Vec<&str> = text.split_whitespace().collect();
    (0..toks.len().saturating_sub(1)).filter_map(move |i| {
        if toks[i + 1].starts_with(word) {
            toks[i]
                .trim_matches(|c: char| !c.is_ascii_digit())
                .parse()
                .ok()
        } else {
            None
        }
    })
}

/// Flags that make a test command not run any test, by exact match or by prefix.
const NON_RUNNING_EXACT: [&str; 3] = ["--co", "-h", "-V"];
const NON_RUNNING_PREFIX: [&str; 11] = [
    "--no-run",
    "--collect-only",
    "--help",
    "--version",
    "--list",
    "-list",
    "--setup-",
    "--fixtures",
    "--markers",
    "--trace-config",
    "--dry-run",
];

/// Environment variables that cannot change whether, or which, tests run. Anything else
/// can: `PYTEST_ADDOPTS=--co` and `GOFLAGS=-run=^$` turn a runner into a no-op that
/// exits 0, and `CARGO_TARGET_<triple>_RUNNER=true` replaces the test binary with `true`.
const HARMLESS_ENV: [&str; 12] = [
    "RUST_BACKTRACE",
    "RUST_LOG",
    "CI",
    "NO_COLOR",
    "FORCE_COLOR",
    "CARGO_TERM_COLOR",
    "PYTHONUNBUFFERED",
    "PYTHONDONTWRITEBYTECODE",
    "PYTHONPATH",
    "NODE_ENV",
    "TERM",
    "TZ",
];

/// Characters that let a shell do something other than run one command and report its
/// status. Quoting is refused wholesale, not parsed: a quoted flag reaches the runner
/// unquoted (`cargo test '--no-run'` is `cargo test --no-run`), so the matcher cannot
/// judge a command it has not read the way the shell will.
const SHELL_SYNTAX: [char; 14] = [
    '|', ';', '`', '(', ')', '\n', '#', '\'', '"', '\\', '$', '{', '}', '!',
];

/// A test run whose exit status is a trustworthy answer, or `None`.
///
/// Claude Code sends no exit status. It sends a command that exited 0 as `PostToolUse`,
/// and one that exited non-zero as `PostToolUseFailure`, so the event stands in for the
/// status, and it stands in for the *shell's* status. Anything that can make that differ
/// from the runner's own is refused: a pipe (the status is the last stage's), `||`
/// (swallowed), `;` or `&` (a later command wins, or it ran in the background), a
/// comment (`cd #x && cargo test` never runs cargo), a subshell, a substitution, a
/// negation, any quoting, and an environment variable that is not on a short allowlist.
/// Measured: `cargo test 2>&1 | tail -3` over a failing test and `cargo test || true`
/// both arrive as ordinary successes.
///
/// A leading `cd DIR &&` is allowed, with a plain directory word, since it is how agents
/// run tests. The runner must then be the command itself, not a word in an `echo`.
/// Builds and lints are not tests: `cargo build` is not an answer to "the tests pass".
fn test_run(cmd: &str) -> Option<Runner> {
    let cmd = cmd.trim();
    if cmd.is_empty() || cmd.contains(SHELL_SYNTAX) {
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
    if segments.iter().any(|s| !is_plain_cd(s)) || last.contains('&') {
        return None;
    }
    let mut toks: Vec<&str> = last.split_whitespace().collect();
    while let Some((name, _)) = toks.first().and_then(|t| assignment(t)) {
        if !HARMLESS_ENV.contains(&name) {
            return None;
        }
        toks.remove(0);
    }
    runner_of(&toks)
}

/// `cd DIR` where DIR is a plain path word, so nothing in it can be an operator.
fn is_plain_cd(segment: &str) -> bool {
    let t: Vec<&str> = segment.split_whitespace().collect();
    t.len() == 2
        && t[0] == "cd"
        && !t[1].is_empty()
        && t[1]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/~+-".contains(c))
}

/// The runner a command line starts, from its words. Shared by the hook, which reads
/// them out of a shell command, and `ana run`, which is handed them directly.
fn runner_of(t: &[&str]) -> Option<Runner> {
    if t.iter().any(|w| {
        NON_RUNNING_EXACT.contains(w) || NON_RUNNING_PREFIX.iter().any(|p| w.starts_with(p))
    }) {
        return None;
    }
    match t {
        ["cargo", rest @ ..] => {
            // Skip a toolchain selector: `cargo +nightly test`.
            let rest: &[&str] = match rest {
                [first, tail @ ..] if first.starts_with('+') => tail,
                _ => rest,
            };
            match rest {
                ["test", ..] => Some(Runner::Cargo),
                _ => None,
            }
        }
        ["npm" | "pnpm" | "yarn" | "bun", "test" | "t" | "tst", ..]
        | ["npm" | "pnpm" | "yarn" | "bun", "run" | "run-script", "test", ..] => {
            Some(Runner::Other)
        }
        ["pytest" | "py.test" | "jest" | "vitest", ..]
        | ["python" | "python3", "-m", "pytest", ..]
        | ["npx" | "pnpx" | "bunx", "jest" | "vitest" | "pytest", ..]
        | ["go" | "flutter" | "dart", "test", ..] => Some(Runner::Other),
        ["gradle" | "./gradlew" | "gradlew" | "mvn" | "./mvnw", rest @ ..]
            if rest.contains(&"test") =>
        {
            Some(Runner::Other)
        }
        _ => None,
    }
}

/// `NAME=value`, with a valid shell identifier for a name.
fn assignment(tok: &str) -> Option<(&str, &str)> {
    let (name, value) = tok.split_once('=')?;
    let ok = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then_some((name, value))
}

/// Whether `cmd` is `ana run ...`, which settles its own claim and needs no nudge.
fn is_ana_run(cmd: &str) -> bool {
    let toks: Vec<&str> = cmd
        .rsplit("&&")
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect();
    let is_ana = |t: &str| {
        let name = t.rsplit(['/', '\\']).next().unwrap_or(t);
        name == "ana" || name == "ana.exe"
    };
    toks.first().is_some_and(|t| is_ana(t))
        && toks
            .iter()
            .skip(1)
            .take_while(|t| **t != "--")
            .any(|t| *t == "run")
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

/// What a finished command says about the claim it was pinned to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunJudgement {
    Passed,
    Failed,
    /// The command ended, but not in a way that answers the claim; the reason says why.
    NotGraded(&'static str),
}

/// Judge a finished run of `runner` from its exit `code` and what its output showed.
fn judge(runner: Runner, code: i64, facts: &RunFacts) -> RunJudgement {
    if code == 0 {
        if runner.confirms_pass(facts) {
            RunJudgement::Passed
        } else {
            RunJudgement::NotGraded("no test was seen to pass")
        }
    } else if runner.failure_codes().contains(&code) && runner.confirms_failure(facts) {
        RunJudgement::Failed
    } else {
        RunJudgement::NotGraded(
            "that exit status is not a failed suite for this runner, or its output does not show one",
        )
    }
}

/// Judge a pinned command from its exit `code` and the facts its output showed.
///
/// `ana run` started the process itself and was handed its words directly, so there is
/// no shell to swallow the status and the shape checks a hook needs do not apply. The
/// runner knowledge does. A command that is not a test runner, `make ci` or a script, is
/// simply judged by its own exit status, since pinning it was the claim-maker's choice.
pub fn judge_run(argv: &[String], code: i64, facts: &RunFacts) -> RunJudgement {
    let toks: Vec<&str> = argv.iter().map(String::as_str).collect();
    match runner_of(&toks) {
        Some(runner) => judge(runner, code, facts),
        None if code == 0 => RunJudgement::Passed,
        None => RunJudgement::Failed,
    }
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
    // Claude Code returns a background call at once, before anything has run: a suite
    // that fails a minute later has already been "passed".
    if input
        .pointer("/tool_input/run_in_background")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Grade::NotATest;
    }
    let Some(runner) = test_run(cmd) else {
        return if event == Event::PostTool && !is_ana_run(cmd) && mentions_test_runner(cmd) {
            Grade::Untrusted
        } else {
            Grade::NotATest
        };
    };
    let text = |p: &str| input.pointer(p).and_then(Value::as_str).unwrap_or("");
    match event {
        // Claude Code sends this event only for a call that succeeded.
        Event::PostTool => {
            let out = format!(
                "{}\n{}",
                text("/tool_response/stdout"),
                text("/tool_response/stderr")
            );
            match judge(runner, 0, &RunFacts::of(&out)) {
                RunJudgement::Passed => Grade::Exit(0),
                _ => Grade::Untrusted,
            }
        }
        Event::PostToolFailure => {
            if input.get("is_interrupt").and_then(Value::as_bool) == Some(true) {
                return Grade::NotATest;
            }
            let error = text("/error");
            match exit_code_in(error) {
                Some(code) => match judge(runner, code, &RunFacts::of(error)) {
                    RunJudgement::Failed => Grade::Exit(code),
                    _ => Grade::NotATest,
                },
                None => Grade::NotATest,
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
    //
    // Except that "silent" must not mean "silently broken": a ledger that cannot be read
    // is remembered, so the session hooks can say so. A user whose hooks never fired had
    // no way to find out why, which is what moving them into the binary was meant to end.
    let (ledger, load_error) = match store::load(ledger_path) {
        Ok(l) => (l, None),
        Err(e) => (Ledger::default(), Some(e)),
    };
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

            if let Some(e) = &load_error {
                println!(
                    "{}",
                    json!({ "systemMessage": format!(
                        "anamnesis: the agent ledger could not be read ({}), so the calibration hooks have nothing to say. Nothing was changed; fix or restore it, or the hooks stay quiet.",
                        untrusted::line(&e.to_string(), 200)
                    ) })
                );
                return Ok(());
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
                    match auto_resolve(ledger_path, &slug, cmd, code) {
                        // The ledger was busy or unreadable: the run is not graded, and
                        // the user is told so rather than left to wonder.
                        Err(e) => {
                            user_message = Some(format!(
                                "anamnesis: this test run was not graded ({})",
                                untrusted::line(&e, 160)
                            ));
                        }
                        Ok(resolved) if resolved.is_empty() => return Ok(()),
                        Ok(resolved) => {
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
                    }
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
                        "⟢ Anamnesis (ana {VERSION}) (moment of truth): a test command just ran, but its exit status cannot be trusted here (it was piped, chained, or ran no tests), so nothing was graded automatically. Resolve your open prediction(s) about it NOW ({}), before hindsight rewrites how sure you were; this one is on your word.",
                        open.join(", ")
                    ));
                }
            }
        }

        Event::Stop => {
            // For a Stop hook, Claude Code reads `additionalContext` as "the conversation
            // continues so Claude can act on it". This arm used to return it whenever any
            // claim was overdue and never read `stop_hook_active`, so every turn was
            // followed by another until the turn cap: measured live, with one overdue
            // claim a one-word prompt ran 13 assistant messages. It shipped in the plugin.
            // A Stop hook here may speak to the person (`systemMessage`) and may never
            // return context.
            if input
                .get("stop_hook_active")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return Ok(());
            }
            let overdue = ledger
                .claims
                .iter()
                .filter(|c| !c.is_void() && c.is_due(today))
                .count();
            if overdue == 0 {
                return Ok(());
            }
            let session = input
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or("default");
            if !first_stop_of_session(session) {
                return Ok(());
            }
            user_message = Some(format!(
                "anamnesis (ana {VERSION}): {overdue} prediction(s) are past their due date and ungraded, which leaves the calibration numbers resting on a self-selected sample. Resolve or void them. (Said once per session.)"
            ));
        }
    }

    if context.is_empty() && user_message.is_none() {
        return Ok(());
    }
    let mut out = json!({});
    if !context.is_empty() {
        out["hookSpecificOutput"] = json!({
            "hookEventName": event.name(),
            "additionalContext": context.join("\n"),
        });
    }
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
    let wait = std::time::Duration::from_millis(env_usize(
        "ANAMNESIS_HOOK_LOCK_WAIT_MS",
        DEFAULT_LOCK_WAIT_MS,
    ));
    let _guard = store::lock_within(ledger_path, wait).map_err(|e| e.to_string())?;
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
        // A pinned claim is settled by `ana run` and by nothing else: a hook grading
        // it from whichever test happened to run would be the escape pinning closes.
        if c.check.is_some() {
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
            "RUST_BACKTRACE=1 CI=true cargo test --color always",
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
            // Found by an independent review: an operator hidden in the cd segment.
            "cd #x && cargo test",
            "cd .&exit && cargo test",
            "cd a;b && cargo test",
            "cd $(pwd) && cargo test",
            "cd 'a b' && cargo test",
            "cargo test # && false",
            // Quoting reaches the runner unquoted; an unlisted environment variable can
            // turn a runner into a no-op.
            "cargo test '--no-run'",
            "cargo test \"--no-run\"",
            "cargo test --no\\-run",
            "PYTEST_ADDOPTS=--co pytest",
            "GOFLAGS=-run=^$ go test ./...",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER=true cargo test",
            "cargo nextest run",
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
            "pytest --setup-plan",
            "pytest --fixtures",
            "go test -list .",
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
    fn ana_run_is_recognised_and_left_alone() {
        assert!(is_ana_run("ana run abc123 -- cargo test"));
        assert!(is_ana_run(
            "/home/u/.anamnesis/bin/ana run abc123 -- cargo test"
        ));
        assert!(is_ana_run("ana --data x.json run abc123 -- pytest"));
        assert!(is_ana_run("cd /work && ana run abc123 -- cargo test"));
        assert!(is_ana_run(
            "ana --data x.json --json run abc123 -- cargo test"
        ));
        assert!(!is_ana_run("ana report -- run"));
        assert!(!is_ana_run("cargo test"));
        assert!(!is_ana_run("echo ana run"));
        assert!(!is_ana_run("ana report"));
    }

    #[test]
    fn a_test_failure_is_only_the_runners_own_failure_code() {
        assert_eq!(test_run("cargo test").unwrap().failure_codes(), [101]);
        assert_eq!(test_run("pytest").unwrap().failure_codes(), [1]);
    }

    #[test]
    fn cargo_must_say_the_suite_failed_not_merely_that_it_stopped() {
        let cargo = Runner::Cargo;
        let f = RunFacts::of;
        assert!(cargo.confirms_failure(&f(
            "Exit code 101\n\ntest result: FAILED. 0 passed; 1 failed"
        )));
        assert!(
            cargo.confirms_failure(&f("Exit code 101\nerror: could not compile `x` (lib test)"))
        );
        assert!(!cargo.confirms_failure(&f(
            "Exit code 101\nerror: manifest path `tiny/Cargo.toml` does not exist"
        )));
        assert!(!cargo.confirms_failure(&f("Exit code 101\nerror: no such command: `tset`")));
    }

    #[test]
    fn other_runners_must_show_a_failing_suite_and_no_sign_the_run_never_started() {
        let other = Runner::Other;
        let f = RunFacts::of;
        assert!(other.confirms_failure(&f("=== 1 failed, 3 passed in 0.1s ===")));
        assert!(other.confirms_failure(&f("--- FAIL: TestX (0.00s)\nFAIL\tpkg\t0.004s")));
        assert!(other.confirms_failure(&f("Tests:       2 failed, 4 passed, 6 total")));
        assert!(other.confirms_failure(&f("  2 passing\n  1 failing")));
        // Nothing failed, or the run never began.
        assert!(!other.confirms_failure(&f("=== 4 passed, 0 failed in 0.1s ===")));
        assert!(!other.confirms_failure(&f("Exit code 1")));
        assert!(!other.confirms_failure(&f("bash: pytest: command not found")));
        assert!(!other.confirms_failure(&f("/usr/bin/python3: No module named pytest")));
        assert!(!other.confirms_failure(&f(
            "1 failed\nbash: cd: /nonexistent: No such file or directory"
        )));
    }

    #[test]
    fn a_pinned_command_is_judged_by_its_own_status_and_the_runners_rules() {
        use RunJudgement::*;
        let argv = |s: &str| crate::check::split_command(s).unwrap();
        let j = |cmd: &str, code: i64, out: &str| judge_run(&argv(cmd), code, &RunFacts::of(out));
        // A plain script or make target: its exit status is the answer.
        assert_eq!(j("make ci", 0, ""), Passed);
        assert_eq!(j("make ci", 2, ""), Failed);
        assert_eq!(j("./scripts/check.sh", 1, "anything"), Failed);
        // A test runner: only its own failure code, and evidence for either answer.
        assert_eq!(
            j("cargo test", 0, "test result: ok. 2 passed; 0 failed"),
            Passed
        );
        assert!(matches!(
            j("cargo test", 0, "test result: ok. 0 passed; 0 failed"),
            NotGraded(_)
        ));
        assert!(matches!(j("cargo test", 0, ""), NotGraded(_)));
        assert_eq!(
            j("cargo test", 101, "test result: FAILED. 0 passed; 1 failed"),
            Failed
        );
        assert!(matches!(
            j("cargo test", 101, "error: manifest path does not exist"),
            NotGraded(_)
        ));
        assert!(matches!(j("cargo test", 2, "usage error"), NotGraded(_)));
        assert_eq!(j("pytest -q", 1, "=== 1 failed in 0.1s ==="), Failed);
        assert!(matches!(j("pytest -q", 1, ""), NotGraded(_)));
        assert!(matches!(j("pytest -q", 5, "no tests ran"), NotGraded(_)));
        // `ana run` is handed words, so quotes are fine there: pytest is still pytest.
        assert_eq!(j(r#"pytest -k "not slow""#, 1, "1 failed"), Failed);
    }

    #[test]
    fn the_exit_code_is_read_from_the_error_text() {
        assert_eq!(exit_code_in("Exit code 101\n\nrunning 1 test"), Some(101));
        assert_eq!(exit_code_in("Exit code 3"), Some(3));
        assert_eq!(exit_code_in("Command timed out"), None);
        assert_eq!(exit_code_in("exit code 3"), None);
    }

    #[test]
    fn a_cargo_pass_is_the_sum_of_every_binary_and_needs_one_test() {
        let n = |t: &str| RunFacts::of(t).cargo_passed;
        assert_eq!(
            n("running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored\n"),
            0
        );
        assert_eq!(
            n("test result: ok. 0 passed; 0 failed; 1 ignored"),
            0,
            "only an ignored test"
        );
        assert_eq!(
            n("test result: ok. 40 passed; 0 failed\ntest result: ok. 0 passed; 0 failed"),
            40,
            "empty doc-tests after a real suite"
        );
        assert_eq!(
            n("test result: ok. 2 passed; 0 failed\ntest result: ok. 3 passed; 0 failed"),
            5
        );
        assert_eq!(n("no cargo output at all"), 0);
        assert!(!RunFacts::of("test result: ok. 4 passed; 0 failed").cargo_failed);
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
            ..Default::default()
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
