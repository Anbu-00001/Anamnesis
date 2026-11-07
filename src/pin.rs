//! The opt-in "pin first" reminder, `ana hook pre-tool`.
//!
//! The grader only has something to grade if a pinned prediction was logged *before*
//! the tests ran. A standing instruction in a global file was followed by one model and
//! ignored by another in the same task, and a reminder that arrives after the run is too
//! late: a prediction written after the outcome is not a prediction. PreToolUse can deny
//! a call and tell Claude why, before anything runs, so the first bare test run of a
//! session is stopped once, with the exact commands to use. A model that follows is never
//! bothered again; one that ignores it is not nagged, and its run goes ahead.
//!
//! Off unless `ANAMNESIS_PIN_NUDGE` is set. It keeps a small local log beside the ledger
//! (`protocol.jsonl`: when, which session, which project and model, which runner, what
//! happened; never a command, a path or any output) so that a week of data can tell "the
//! agent ignored the instruction" from "nobody ran any tests".

use std::path::Path;

use chrono::{Duration, Utc};
use serde_json::{json, Value};

use crate::hook::{
    assignment, dirs_counters, has_non_running_flag, is_ana_run, project_slug, runner_of, sanitize,
};
use crate::model::{Claim, Ledger};
use crate::{store, untrusted};

/// A pin logged within this long is taken to be for the run about to happen.
const PIN_WINDOW_MINUTES: i64 = 30;

/// Whether the reminder is switched on.
pub fn enabled() -> bool {
    matches!(
        std::env::var("ANAMNESIS_PIN_NUDGE").as_deref(),
        Ok("1" | "on" | "true" | "yes")
    )
}

/// Split a shell line at `|`, `;`, `&` and newlines that are not inside quotes.
///
/// Cutting inside quotes would find a runner in `echo "a | cargo test"`, and a reminder
/// that fires on an `echo` or a `git commit -m "fix the jest config"` is worse than none.
fn segments(cmd: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let (mut single, mut double, mut escaped) = (false, false, false);
    for c in cmd.chars() {
        let cur = out.last_mut().expect("never empty");
        if escaped {
            cur.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' if !single => {
                cur.push(c);
                escaped = true;
            }
            '\'' if !double => {
                single = !single;
                cur.push(c);
            }
            '"' if !single => {
                double = !double;
                cur.push(c);
            }
            '|' | ';' | '&' | '\n' if !single && !double => out.push(String::new()),
            _ => cur.push(c),
        }
    }
    out
}

/// A short, stable name for the kind of runner, for the log. Never the command.
fn runner_label(toks: &[&str]) -> &'static str {
    match toks
        .first()
        .map_or("", |t| t.rsplit('/').next().unwrap_or(t))
    {
        "cargo" => "cargo",
        "pytest" | "py.test" | "python" | "python3" => "pytest",
        "go" => "go",
        "npm" | "pnpm" | "yarn" | "bun" | "npx" | "pnpx" | "bunx" | "jest" | "vitest" => "node",
        "flutter" | "dart" => "flutter",
        "gradle" | "gradlew" | "mvn" | "mvnw" => "jvm",
        _ => "other",
    }
}

/// Test commands the grader's own matcher does not parse but a reminder should still catch:
/// pinning works for any command, judged by its exit status. The person works across stacks,
/// and a runner the reminder does not know is a run it never sees.
fn other_runner(t: &[&str]) -> Option<&'static str> {
    match t {
        ["dotnet" | "deno", "test", ..] => Some("dotnet"),
        ["bundle", "exec", "rspec" | "rake", ..]
        | ["rspec", ..]
        | ["rake", "test" | "spec", ..] => Some("ruby"),
        ["phpunit" | "./vendor/bin/phpunit", ..] => Some("php"),
        ["tox" | "nox", ..]
        | ["python" | "python3", "-m", "unittest", ..]
        | ["python" | "python3", "manage.py", "test", ..] => Some("python"),
        [var, "-m", "pytest" | "unittest", ..] if var.trim_start_matches('"').starts_with('$') => {
            Some("python")
        }
        ["node", "--test", ..]
        | ["pnpm" | "yarn" | "bun", "jest" | "vitest", ..]
        | ["pnpm" | "yarn" | "bun", "exec", "jest" | "vitest", ..] => Some("node"),
        ["npm" | "pnpm" | "yarn" | "bun", "run" | "run-script", script, ..]
            if script.starts_with("test") =>
        {
            Some("node")
        }
        ["ctest", ..] => Some("c"),
        ["make", target, ..] | ["just", target, ..]
            if matches!(*target, "test" | "tests" | "check" | "ci") =>
        {
            Some("make")
        }
        _ => None,
    }
}

/// `cmd` with redirections that contain `&` blanked, so `segments` does not cut at them, and
/// without the bodies of its here-documents: a `cargo test` line inside `cat <<EOF` is text.
fn flat(cmd: &str) -> String {
    let mut out = String::new();
    let mut until: Option<String> = None;
    for line in cmd.lines() {
        if let Some(end) = &until {
            if line.trim() == end {
                until = None;
            }
            continue;
        }
        until = heredoc_word(line);
        out.push_str(line);
        out.push('\n');
    }
    out.replace("2>&1", " ")
        .replace("1>&2", " ")
        .replace(">&2", " ")
        .replace("&>", " ")
}

/// The word that ends a here-document opened on `line` (`<<EOF`, `<<-'PY'`), if one is.
fn heredoc_word(line: &str) -> Option<String> {
    let (_, rest) = line.split_once("<<")?;
    if rest.starts_with('<') {
        return None; // `<<<` is a here-string, on one line
    }
    let rest = rest.strip_prefix('-').unwrap_or(rest).trim_start();
    let word: String = rest
        .chars()
        .filter(|c| !matches!(c, '\'' | '"'))
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!word.is_empty()).then_some(word)
}

/// What a segment runs, past `NAME=value` and the wrappers that run a command unchanged:
/// `nice -n 19`, `timeout 600`, `env -u X`, `time`, `nohup`, `command`, `exec`, `eval`, a shell
/// keyword (`do npx vitest run`, `if ! pytest`) and `uv run`. Agents keep heavy suites polite
/// with `nice -n 19`, and that is still the suite.
fn command_in<'a, 'b>(mut toks: &'b [&'a str]) -> &'b [&'a str] {
    while let Some((first, rest)) = toks.split_first() {
        if assignment(first).is_some() {
            toks = rest;
            continue;
        }
        if matches!(*first, "uv" | "poetry" | "pipenv" | "pdm" | "hatch")
            && rest.first() == Some(&"run")
        {
            toks = &rest[1..];
            continue;
        }
        let takes_value: &[&str] = match *first {
            "nice" => &["-n"],
            "timeout" => &["-s", "-k"],
            "env" => &["-u"],
            "time" | "nohup" | "command" | "exec" | "eval" | "if" | "elif" | "else" | "then"
            | "do" | "while" | "until" => &[],
            _ => break,
        };
        let mut rest = rest;
        while let Some((flag, after)) = rest.split_first() {
            if !flag.starts_with('-') {
                break;
            }
            rest = if takes_value.contains(flag) {
                after.get(1..).unwrap_or(&[])
            } else {
                after
            };
        }
        if *first == "timeout" {
            rest = rest.get(1..).unwrap_or(&[]); // the duration
        }
        toks = rest;
    }
    toks
}

/// The words of a segment, with the brackets of a subshell, group, function or command
/// substitution taken off: `(ana run x -- cargo test)` runs `ana run`, `{ cargo test; }` and
/// `run(){ cargo test; }` run `cargo test`, and so does `r=$(cargo test)`.
fn words(seg: &str) -> Vec<&str> {
    seg.split_whitespace()
        .filter(|t| !t.ends_with("(){") && !t.ends_with("()"))
        .map(|t| match assignment(t) {
            Some((_, value)) if value.starts_with("$(") => &value[2..],
            _ => t,
        })
        .map(|t| t.trim_start_matches('(').trim_end_matches(')'))
        .filter(|t| !t.is_empty() && !matches!(*t, "{" | "}" | "!"))
        .collect()
}

/// The script inside `bash -c "..."`, which `segments` rightly leaves whole. Quotes are what
/// keep `git commit -m "fix cargo test"` from counting, so only a shell's `-c` is opened.
fn shell_body(seg: &str) -> Option<String> {
    let all = words(seg);
    let toks = command_in(&all);
    if !matches!(program(toks.first()?), "bash" | "sh" | "zsh") {
        return None;
    }
    let flag = *toks.get(1)?;
    if !matches!(flag, "-c" | "-lc" | "-ic") {
        return None;
    }
    let at = seg.find(&format!(" {flag} "))? + flag.len() + 2;
    let body = seg[at..].trim();
    match body.chars().next()? {
        q @ ('"' | '\'') => body[1..].rsplit_once(q).map(|(b, _)| b.to_string()),
        _ => Some(body.to_string()),
    }
}

/// A Python interpreter's own switches, which come before `-m`: `python -B -m pytest`.
fn without_interpreter_flags(named: Vec<&str>) -> Vec<&str> {
    if !matches!(named.first(), Some(&"python" | &"python3")) {
        return named;
    }
    let mut out = vec![named[0]];
    let mut rest = named[1..].iter();
    while let Some(t) = rest.next() {
        match *t {
            "-W" | "-X" => {
                rest.next();
            }
            "-m" | "-c" | "-" => {
                out.push(t);
                break;
            }
            f if f.starts_with('-') => {}
            _ => {
                out.push(t);
                break;
            }
        }
    }
    out.extend(rest);
    out
}

/// A program's name without its directory, and `python3.12` as `python3`.
fn program(t: &str) -> &str {
    let name = t.rsplit('/').next().unwrap_or(t);
    match name {
        n if n.starts_with("python3") => "python3",
        n if n.starts_with("python") => "python",
        n => n,
    }
}

/// The runner `cmd` sends through `ana run`, if it does so in any segment: `cd x; ana run ...`,
/// `ID=$(ana add ...) && ana run "$ID" -- ...` and `nice -n 19 ana run ...` all count. Looking
/// only at the start of the last `&&` part, as the first version did, left 382 of 471 real
/// `ana run` calls out of the log, and the log is what the compliance figures are read from.
fn ana_run_in(cmd: &str) -> Option<&'static str> {
    segments(&flat(cmd)).iter().find_map(|seg| {
        if let Some(body) = shell_body(seg) {
            return ana_run_in(&body);
        }
        let all = words(seg);
        let toks = command_in(&all);
        is_ana_run(&toks.join(" ")).then(|| {
            toks.iter()
                .position(|t| *t == "--")
                .and_then(|i| test_run_in(&toks[i + 1..].join(" ")))
                .map_or("other", |(r, _)| r)
        })
    })
}

/// The test run in `cmd`, if there is one: its runner label and the plain command to pin.
///
/// Redirections and everything after them are not part of what gets pinned: an agent that
/// ran `cargo test 2>&1 | tail -60` is told to pin `cargo test`.
pub(crate) fn test_run_in(cmd: &str) -> Option<(&'static str, String)> {
    for seg in segments(&flat(cmd)) {
        if let Some(body) = shell_body(&seg) {
            match test_run_in(&body) {
                None => continue,
                found => return found,
            }
        }
        let all = words(&seg);
        let toks = command_in(&all);
        if toks.first().is_some_and(|t| *t == "cd") {
            continue;
        }
        let label = |t: &[&str]| {
            if runner_of(t).is_some() {
                Some(runner_label(t))
            } else if has_non_running_flag(t) {
                None
            } else {
                other_runner(t)
            }
        };
        // `.venv/bin/python -m pytest` is how most real pytest runs here were written, and
        // the grader's matcher knows only a bare `python`: match it by the program's name.
        let named: Vec<&str> = toks
            .iter()
            .enumerate()
            .map(|(i, t)| if i == 0 { program(t) } else { t })
            .collect();
        let named = without_interpreter_flags(named);
        if let Some(label) = label(toks).or_else(|| label(&named)) {
            let plain: Vec<&str> = toks
                .iter()
                .copied()
                .take_while(|t| !t.starts_with(['>', '<']) && !t.starts_with("2>"))
                .collect();
            return Some((label, plain.join(" ")));
        }
    }
    None
}

/// The most recent pinned, unsettled claim, if it was logged recently enough to be for
/// this run.
fn recent_open_pin(ledger: &Ledger) -> Option<&Claim> {
    let cutoff = Utc::now() - Duration::minutes(PIN_WINDOW_MINUTES);
    ledger
        .claims
        .iter()
        .filter(|c| c.is_open() && !c.is_void() && c.check.is_some() && c.created_at > cutoff)
        .max_by_key(|c| c.created_at)
}

/// Where the log lives: beside the ledger it describes, so a scratch ledger gets a scratch
/// log and the real one is never touched by a test.
fn log_path(ledger_path: &Path) -> std::path::PathBuf {
    ledger_path.with_file_name("protocol.jsonl")
}

/// What every line of the log says about the run, read once a command is known to be one.
struct Run<'a> {
    session: &'a str,
    project: String,
    repo: Option<String>,
    model: Option<String>,
    subagent: bool,
    after_edit: Option<bool>,
}

/// Bumped when the log's meaning changes, so a reader can tell which logger wrote a line.
/// Lines with no `v` predate it; 3 added `repo` and `after_edit` and widened what is a run.
const LOG_VERSION: u32 = 3;

fn log(ledger_path: &Path, run: &Run, runner: &str, action: &str) {
    let rec = json!({
        "v": LOG_VERSION,
        "at": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "session": untrusted::tag(run.session),
        "project": untrusted::tag(&run.project),
        "repo": run.repo.as_deref().map(untrusted::tag),
        "model": run.model,
        "subagent": run.subagent,
        "after_edit": run.after_edit,
        "runner": runner,
        "action": action,
    });
    let _ = store::append_private(&log_path(ledger_path), &rec.to_string());
}

/// One line per test run the post-tool hook sees, in `outcomes.jsonl` beside the ledger.
///
/// `protocol.jsonl` records what the agent did before a run (pinned or not); this records
/// how the run ended, so a later analysis has an outcome that does not depend on whether a
/// prediction was logged (docs/MEASUREMENT.md, addendum 4). Never the command or its
/// output: the runner label, how it was run, the exit status and what the grader made of
/// it. It is written whether or not the pin reminder is on, and it never prints anything.
pub(crate) fn log_outcome(
    input: &Value,
    ledger_path: &Path,
    cmd: &str,
    exit: Option<i64>,
    graded: &str,
    prompt: Option<u64>,
) {
    let (via, runner) = match ana_run_in(cmd) {
        Some(r) => ("ana_run", r),
        None => match test_run_in(cmd) {
            Some((r, _)) => ("bare", r),
            None => return,
        },
    };
    let session = input
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("default");
    let cwd = input.get("cwd").and_then(Value::as_str);
    let tail = transcript_tail(input);
    let rec = json!({
        "v": 1,
        "at": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "session": untrusted::tag(session),
        "prompt": prompt,
        "project": untrusted::tag(&project_slug(cwd)),
        "repo": cwd
            .and_then(|d| repo_of(&run_dir(cmd, Path::new(d))))
            .as_deref()
            .map(untrusted::tag),
        "model": tail.as_ref().and_then(|(t, _)| model_of(t)),
        "subagent": input.get("agent_id").is_some(),
        "after_edit": tail.as_ref().and_then(|(t, whole)| {
            after_edit(cmd, t, *whole, input.get("tool_use_id").and_then(Value::as_str))
        }),
        "runner": runner,
        "via": via,
        "exit": exit,
        "graded": graded,
    });
    let _ = store::append_private(
        &ledger_path.with_file_name("outcomes.jsonl"),
        &rec.to_string(),
    );
}

/// The repository a directory belongs to, by its folder name and nothing else of its path.
/// A linked worktree counts as its main repository: `project`, the folder a command ran in,
/// made one codebase look like 23 projects, which is the error a per-repo reading must avoid.
pub(crate) fn repo_of(dir: &Path) -> Option<String> {
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_lowercase());
    for d in dir.ancestors() {
        let dot = d.join(".git");
        if dot.is_dir() {
            return name(d);
        }
        if dot.is_file() {
            let text = std::fs::read_to_string(&dot).ok()?;
            let gitdir = text.strip_prefix("gitdir:")?.trim();
            return match gitdir.split_once("/.git/worktrees/") {
                Some((main, _)) => name(Path::new(main)),
                None => name(d),
            };
        }
    }
    None
}

/// Where `cmd` really runs: the payload's `cwd`, moved by any `cd` it opens with.
fn run_dir(cmd: &str, cwd: &Path) -> std::path::PathBuf {
    let mut dir = cwd.to_path_buf();
    for seg in segments(&flat(cmd)) {
        let toks = words(&seg);
        match toks.as_slice() {
            [] => {}
            ["cd", to] if !to.contains('$') && !to.starts_with('~') => {
                dir = dir.join(to.trim_matches(['"', '\'']));
            }
            all if all.iter().all(|t| assignment(t).is_some()) => {}
            _ => break,
        }
    }
    dir
}

/// What stamps a claim logged from inside Claude Code: which session, repository and client.
///
/// A claim has only an agent-written `session:<date>` tag, so nothing could say how many of
/// 605 exit-graded claims came from one long session. Claude Code gives its child processes
/// `CLAUDE_CODE_SESSION_ID`; older versions did not, and then nothing is stamped. Outside
/// Claude Code this is empty, so a person's ledger stays as it was. It goes in the claim's
/// `extra`, which every version keeps through a save and nothing renders.
pub fn origin_here() -> crate::model::Extra {
    let mut extra = crate::model::Extra::new();
    let Some(session) = std::env::var("CLAUDE_CODE_SESSION_ID")
        .ok()
        .filter(|s| !s.is_empty())
    else {
        return extra;
    };
    let mut origin = serde_json::Map::new();
    origin.insert("session".into(), untrusted::tag(&session).into());
    let repo = std::env::current_dir().ok().and_then(|d| repo_of(&d));
    if let Some(repo) = repo {
        origin.insert("repo".into(), untrusted::tag(&repo).into());
    }
    if let Some(client) = std::env::var("AI_AGENT").ok().filter(|s| !s.is_empty()) {
        origin.insert("client".into(), untrusted::tag(&client).into());
    }
    extra.insert("origin".into(), origin.into());
    extra
}

fn marker(session: &str, ext: &str) -> Option<std::path::PathBuf> {
    dirs_counters().map(|d| d.join(format!("{}.{ext}", sanitize(session))))
}

/// The end of this agent's transcript, and whether that is all of it. Only the last 256 KB is
/// read, so a long session costs the same as a short one.
///
/// A subagent's call carries `agent_id` but its parent's `transcript_path` (checked against
/// a real 2.1.251 payload); reading that credited 18 of 33 subagent runs to the parent's
/// model. Its own transcript is `<session>/subagents/agent-<agent_id>.jsonl`, beside the
/// parent's, and if that is not there the model is not reported rather than guessed.
fn transcript_tail(input: &Value) -> Option<(String, bool)> {
    use std::io::{Read, Seek, SeekFrom};
    const TAIL: u64 = 256 * 1024;
    let parent = Path::new(input.get("transcript_path")?.as_str()?);
    let path = match input.get("agent_id").and_then(Value::as_str) {
        None => parent.to_path_buf(),
        Some(id)
            if !id.is_empty()
                && id.len() <= 64
                && id.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            parent
                .with_extension("")
                .join("subagents")
                .join(format!("agent-{id}.jsonl"))
        }
        Some(_) => return None,
    };
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut bytes = Vec::new();
    f.take(TAIL).read_to_end(&mut bytes).ok()?;
    // The cut may land inside a line or a character; those lines simply do not parse.
    Some((String::from_utf8_lossy(&bytes).into_owned(), len <= TAIL))
}

/// The model that produced this session's messages, read from the end of its transcript.
///
/// SessionStart is not told the model (checked against a real payload: its keys are `cwd`,
/// `hook_event_name`, `session_id`, `source`, `transcript_path`), but both hooks are told
/// where the transcript is, and every assistant message in it records the model that wrote
/// it. What separates "this model ignored the instruction" from "nobody ran any tests" is
/// knowing which model it was, and this is the harness's own record, not the agent's account.
fn model_of(tail: &str) -> Option<String> {
    tail.lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find_map(|v| {
            let m = v.pointer("/message/model")?.as_str()?;
            (!m.is_empty() && !m.starts_with('<')).then(|| untrusted::tag(m))
        })
}

/// Whether a shell command plainly changes files. A floor, not a census: a script can do it
/// any way it likes, so `after_edit: false` means no edit was seen.
fn edits_by_shell(cmd: &str) -> bool {
    [
        "sed -i",
        "perl -i",
        "cat >",
        "tee ",
        "git apply",
        "patch ",
        " - <<",
    ]
    .iter()
    .any(|m| cmd.contains(m))
}

/// Was anything edited since the last test run? The rule covers runs that check a change, and
/// every bare run used to count against it, so this separates the two.
///
/// Walks the transcript backwards: an edit (or a subagent's work) answers yes, an earlier test
/// run answers no, and a tail that runs out before either answers nothing, unless it is the
/// whole transcript. A call a hook refused never ran, so it answers nothing either way.
fn after_edit(cmd: &str, tail: &str, whole: bool, current: Option<&str>) -> Option<bool> {
    if edits_by_shell(cmd) {
        return Some(true);
    }
    let mut refused = std::collections::HashSet::new();
    for line in tail.lines().rev() {
        if !line.contains("\"tool_use\"") && !line.contains("\"tool_result\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(content) = v.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        for c in content.iter().rev() {
            let id = c.get("id").and_then(Value::as_str);
            match c.get("type").and_then(Value::as_str) {
                Some("tool_result") => {
                    if c.get("content")
                        .is_some_and(|t| t.to_string().contains("hook error"))
                    {
                        if let Some(used) = c.get("tool_use_id").and_then(Value::as_str) {
                            refused.insert(used.to_string());
                        }
                    }
                }
                Some("tool_use") if id != current && !id.is_some_and(|i| refused.contains(i)) => {
                    match c.get("name").and_then(Value::as_str) {
                        Some(
                            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "Agent" | "Task",
                        ) => return Some(true),
                        Some("Bash") => {
                            let run = c.pointer("/input/command").and_then(Value::as_str)?;
                            if ana_run_in(run).is_some() || test_run_in(run).is_some() {
                                return Some(false);
                            }
                            if edits_by_shell(run) {
                                return Some(true);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    whole.then_some(false)
}

/// True the first time it is asked about a session: the reminder is said once.
fn first_denial_of_session(session: &str) -> bool {
    let Some(path) = marker(session, "pindenied") else {
        return true;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .is_ok()
}

/// Whether `s` can sit inside the double quotes of a command it is suggested in.
fn quotable(s: &str) -> bool {
    !s.is_empty() && !s.contains(['"', '\\', '$', '`', '\n'])
}

fn deny(reason: String) -> Value {
    json!({ "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": reason,
    } })
}

/// The rule as injected at the start of a session. Static text: nothing from the ledger.
pub fn standing_rule() -> String {
    "⟢ Standing rule on this machine, set by its owner and not optional: every time you are about to run a project's tests to check a change you made, FIRST log a pinned prediction, then run the tests through `ana run`.\n  1. ana add \"<what you expect, e.g. all tests pass first try>\" --prob <your honest probability> --by <today's date> --tags who:claude,kind:tests-pass,project:<repo folder name, lowercase>,session:<today's date>,model:<your model> --check \"<the exact test command>\"\n  2. ana run <the id it printed> -- <that exact command>   (from the directory the tests run in)\nThe prediction must exist BEFORE the run. The first bare test run of a session is refused with these steps; a run through `ana run` never is.".to_string()
}

/// How long a per-session marker is kept. The files are a few bytes each, one set per
/// session, and would otherwise pile up forever.
const STATE_TTL_DAYS: u64 = 14;

/// Delete this tool's own stale per-session files from the counters directory: only the
/// four kinds it writes, only older than [`STATE_TTL_DAYS`], and never anything else there.
pub fn prune_state() {
    let Some(dir) = dirs_counters() else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let cutoff =
        std::time::SystemTime::now() - std::time::Duration::from_secs(STATE_TTL_DAYS * 24 * 3600);
    for e in entries.flatten() {
        let path = e.path();
        let ours = path
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| matches!(x, "count" | "stop" | "pindenied"));
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t < cutoff);
        if ours && stale && path.is_file() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// What `ana hook pre-tool` says about a Bash call: a denial, or nothing.
pub fn pre_tool(input: &Value, ledger: &Ledger, ledger_path: &Path) -> Option<Value> {
    let cmd = input
        .pointer("/tool_input/command")
        .and_then(Value::as_str)?;
    let session = input
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("default");
    // Decide first: this runs before every Bash call, and most are not test runs.
    let ana_runner = ana_run_in(cmd);
    let bare = match ana_runner {
        Some(_) => None,
        None => Some(test_run_in(cmd)?),
    };
    let cwd = input.get("cwd").and_then(Value::as_str);
    let tail = transcript_tail(input);
    let run = Run {
        session,
        project: project_slug(cwd),
        repo: cwd.and_then(|d| repo_of(&run_dir(cmd, Path::new(d)))),
        model: tail.as_ref().and_then(|(t, _)| model_of(t)),
        subagent: input.get("agent_id").is_some(),
        after_edit: tail.as_ref().and_then(|(t, whole)| {
            let current = input.get("tool_use_id").and_then(Value::as_str);
            after_edit(cmd, t, *whole, current)
        }),
    };

    // Already going through `ana run`: that is the behaviour wanted. Count it.
    if let Some(runner) = ana_runner {
        log(ledger_path, &run, runner, "ana_run");
        return None;
    }
    let (runner, plain) = bare?;

    if !first_denial_of_session(session) {
        log(ledger_path, &run, runner, "allowed_after_nudge");
        return None;
    }
    let shown = untrusted::line(&plain, 120);
    let pin = recent_open_pin(ledger);
    let action = if pin.is_some() {
        "denied_unrun_pin"
    } else {
        "denied_no_pin"
    };
    log(ledger_path, &run, runner, action);
    Some(deny(match pin {
        Some(c) => format!(
            "anamnesis: you already logged the pinned prediction [{id}], but this is the bare command, so no one would grade it. Nothing was run. Run it as: ana run {id} -- {check} (from the directory the tests run in). It takes the verdict from the exit status and prints the command's own output. (Said once per session.)",
            id = untrusted::tag(&c.id),
            check = untrusted::line(c.check.as_deref().unwrap_or(""), untrusted::MAX_LINE),
        ),
        None => {
            let cmd_for_pin = if quotable(&shown) {
                shown
            } else {
                "<the plain test command>".to_string()
            };
            format!(
                "anamnesis: before you run tests to check a change, log a pinned prediction first, so the machine grades it and you do not. Nothing was run. Do this, then run the tests again:\n  1. ana add \"<what you expect, e.g. all tests pass first try>\" --prob <your honest probability> --by <today's date> --tags who:claude,kind:tests-pass,project:<repo folder name, lowercase>,session:<today's date>,model:<your model> --check \"{cmd_for_pin}\"\n  2. ana run <the id it printed> -- {cmd_for_pin}\nRun `ana run` from the directory the tests run in; it takes the verdict from the process's exit status and prints the command's own output. The prediction must be logged BEFORE the run: one written after the outcome is not a prediction. (Said once per session; ANAMNESIS_PIN_NUDGE=off turns it off.)"
            )
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_in(input: &Value) -> Option<String> {
        transcript_tail(input).and_then(|(tail, _)| model_of(&tail))
    }

    fn found(cmd: &str) -> Option<(&'static str, String)> {
        test_run_in(cmd)
    }

    #[test]
    fn a_test_run_is_found_inside_a_longer_command_line() {
        assert_eq!(found("cargo test").unwrap(), ("cargo", "cargo test".into()));
        assert_eq!(
            found("cd tiny && cargo test 2>&1 | tail -60").unwrap(),
            ("cargo", "cargo test".into())
        );
        assert_eq!(
            found("RUST_LOG=debug cargo test -p foo -- --nocapture")
                .unwrap()
                .1,
            "cargo test -p foo -- --nocapture"
        );
        assert_eq!(
            found("python3 -m pytest -q tests/ > out.txt").unwrap(),
            ("pytest", "python3 -m pytest -q tests/".into())
        );
        assert_eq!(found("time npm test").unwrap().0, "node");
        assert_eq!(
            found("make && go test ./...").unwrap(),
            ("go", "go test ./...".into())
        );
    }

    #[test]
    fn a_runner_named_inside_quotes_or_an_argument_is_not_a_test_run() {
        for cmd in [
            "echo \"a | cargo test --lib\"",
            "echo 'cargo test'",
            "git commit -m \"fix; cargo test\"",
            "grep -rn 'go test' docs/",
            "cat tests/test_pytest_plugin.py",
            "cargo build",
            "cargo test --no-run",
            "ls; echo done",
            "",
        ] {
            assert!(found(cmd).is_none(), "must not be a test run: {cmd}");
        }
    }

    #[test]
    fn the_model_is_read_from_the_end_of_the_transcript() {
        let dir = std::env::temp_dir().join(format!("ana_model_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let line = |m: &str| {
            format!("{{\"type\":\"assistant\",\"message\":{{\"model\":\"{m}\",\"content\":[]}}}}\n")
        };
        // A long early stretch by one model, a synthetic entry, then the model in use now.
        let mut text = String::new();
        for _ in 0..4000 {
            text.push_str(&line("claude-opus-5"));
        }
        text.push_str("{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n");
        text.push_str(&line("<synthetic>"));
        text.push_str(&line("claude-sonnet-5"));
        std::fs::write(&path, text).unwrap();
        let input = json!({ "transcript_path": path.display().to_string() });
        assert_eq!(model_in(&input).as_deref(), Some("claude-sonnet-5"));
        // No transcript, a missing file, or one with no model: nothing, never an error.
        assert_eq!(model_in(&json!({})), None);
        assert_eq!(
            model_in(&json!({"transcript_path": "/nonexistent/x.jsonl"})),
            None
        );
        std::fs::write(&path, "not json at all\n").unwrap();
        assert_eq!(model_in(&input), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_suggestion_that_would_break_the_quoting_is_not_offered() {
        assert!(quotable("cargo test -p foo"));
        assert!(!quotable("pytest -k \"a b\""));
        assert!(!quotable("echo $HOME"));
        assert!(!quotable(""));
    }
}
