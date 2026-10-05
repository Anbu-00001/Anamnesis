//! `ana run`: settle a claim from the exit status of the command it was pinned to.
//!
//! A hook can only grade what Claude Code happens to report, and what it reports is the
//! shell's status for whatever command the agent chose. An agent that logs "the tests
//! pass" and then runs one narrow test can settle the claim with that. So a claim can
//! be *pinned* to a command when it is logged (`--check "cargo test"`), before the
//! outcome could be known, and `ana run <id> -- cargo test` then runs what it is given,
//! refuses anything that is not the pinned command, and takes the status from the
//! process it started itself: no shell in between to swallow it, no payload to
//! misread.
//!
//! The pinned text is only ever compared. It is never executed: a ledger holds words
//! anyone can have written, and a command read from one would be remote code execution
//! by import.

use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::hook::RunFacts;
use crate::untrusted;

/// The longest pinned command.
const MAX_CHECK: usize = 500;
/// How long, after the command has exited, to keep waiting for its output to end. A
/// grandchild that inherited the pipes holds them open for as long as it lives: 8 seconds
/// for a `sleep 8 &`, forever for a daemon. What has been read by then is what is judged.
const OUTPUT_GRACE: Duration = Duration::from_secs(2);

/// Split a command line into its words the way a POSIX shell would: whitespace separates,
/// single quotes keep everything, double quotes keep everything but `\"`, `\\`, `\$` and
/// `` \` ``, and a backslash escapes the next character. There is no expansion of any
/// kind. An unbalanced quote is an error.
///
/// A check is compared as these words, not as text. Compared as text joined by spaces,
/// `pytest -k "not slow"` was refused by the very command the refusal message suggested,
/// and `["sh","-c","a b"]` was the same line as `["sh","-c","a","b"]`.
pub fn split_command(s: &str) -> Result<Vec<String>, String> {
    let unbalanced = || "a check has an unbalanced quote".to_string();
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => cur.push(ch),
                        None => return Err(unbalanced()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(ch @ ('"' | '\\' | '$' | '`')) => cur.push(ch),
                            Some(ch) => {
                                cur.push('\\');
                                cur.push(ch);
                            }
                            None => return Err(unbalanced()),
                        },
                        Some(ch) => cur.push(ch),
                        None => return Err(unbalanced()),
                    }
                }
            }
            '\\' => {
                in_word = true;
                cur.push(chars.next().unwrap_or('\\'));
            }
            other => {
                in_word = true;
                cur.push(other);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    Ok(words)
}

/// A command as it is pinned: trimmed, kept as typed, and refused if it is empty, spans
/// lines, has an unbalanced quote, or is longer than anyone would type.
pub fn pin(raw: &str) -> Result<String, String> {
    let t = raw.trim();
    if t.is_empty() {
        return Err("a check must name the command that settles the claim".into());
    }
    if t.chars().any(char::is_control) {
        return Err("a check must be one line".into());
    }
    if t.chars().count() > MAX_CHECK {
        return Err(format!("a check is at most {MAX_CHECK} characters"));
    }
    if split_command(t)?.is_empty() {
        return Err("a check must name the command that settles the claim".into());
    }
    Ok(t.to_string())
}

/// What to tell someone who tried to settle a pinned claim another way.
///
/// The check is stored text, so it is quoted through [`untrusted::line`] like anything
/// else from the ledger that is about to reach a person or a model.
pub fn pinned_message(id: &str, check: &str) -> String {
    let id = untrusted::tag(id);
    let check = untrusted::line(check, untrusted::MAX_LINE);
    format!(
        "[{id}] is pinned to the check `{check}`, so it is settled only by running that command: `ana run {id} -- {check}`. It cannot be resolved by hand, because then the command could be chosen after the outcome is known."
    )
}

/// The exit code `ana` itself should return for a child that returned `code`.
///
/// A shell keeps only the low byte, so a code that is a multiple of 256 (Windows can
/// report one) would read as success. A command that did not succeed never maps to 0.
pub fn exit_byte(code: i32) -> u8 {
    let low = (code & 0xff) as u8;
    if code != 0 && low == 0 {
        1
    } else {
        low
    }
}

/// A command that has finished.
pub struct Ran {
    /// Its exit code, or `None` if a signal ended it: then there is no status to grade.
    pub code: Option<i32>,
    /// What its output showed, from the whole of it and not only the end.
    pub facts: RunFacts,
}

/// What one output stream has shown so far, shared with the thread reading it so that
/// the caller can take what has arrived even if the stream never ends.
#[derive(Default)]
struct Seen {
    /// The part of a line that has not ended yet.
    partial: Vec<u8>,
    facts: RunFacts,
}

impl Seen {
    fn feed(&mut self, chunk: &[u8]) {
        self.partial.extend_from_slice(chunk);
        while let Some(i) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=i).collect();
            self.facts.observe_line(&String::from_utf8_lossy(&line));
        }
        // A line with no end in sight is not a test result; do not hold it forever.
        if self.partial.len() > 64 * 1024 {
            self.partial.clear();
        }
    }

    fn finish(&mut self) {
        if !self.partial.is_empty() {
            let line = std::mem::take(&mut self.partial);
            self.facts.observe_line(&String::from_utf8_lossy(&line));
        }
    }
}

/// Start `program`, finding `npm.cmd` and `mvn.cmd` on Windows, where a bare name only
/// ever resolves to an `.exe`.
fn spawn(program: &str, args: &[String]) -> io::Result<Child> {
    let go = |p: &str| {
        Command::new(p)
            .args(args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    };
    match go(program) {
        Err(e)
            if cfg!(windows)
                && e.kind() == io::ErrorKind::NotFound
                && std::path::Path::new(program).extension().is_none() =>
        {
            match go(&format!("{program}.cmd")) {
                Ok(c) => Ok(c),
                Err(_) => go(&format!("{program}.bat")).map_err(|_| e),
            }
        }
        other => other,
    }
}

/// Run `argv` directly (no shell), passing its output through as it arrives, and
/// return how it ended. The caller holds no lock: a test run can take minutes.
pub fn run_command(argv: &[String]) -> io::Result<Ran> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no command given"))?;
    let mut child = spawn(program, args)?;
    let out = child.stdout.take().expect("piped stdout");
    let err = child.stderr.take().expect("piped stderr");
    let mut readers = Vec::new();
    let mut seen = Vec::new();
    for (stream, sink) in [
        (
            Box::new(out) as Box<dyn Read + Send>,
            Box::new(io::stdout()) as Box<dyn Write + Send>,
        ),
        (Box::new(err), Box::new(io::stderr())),
    ] {
        let state = Arc::new(Mutex::new(Seen::default()));
        let done = Arc::new(AtomicBool::new(false));
        seen.push(Arc::clone(&state));
        readers.push(Arc::clone(&done));
        thread::spawn(move || forward(stream, sink, &state, &done));
    }
    let status = child.wait()?;

    // The command is done. Its output normally ends with it; wait a moment for that,
    // but not for a grandchild that kept the pipes.
    let deadline = Instant::now() + OUTPUT_GRACE;
    while Instant::now() < deadline && !readers.iter().all(|d| d.load(Ordering::SeqCst)) {
        thread::sleep(Duration::from_millis(10));
    }
    let mut facts = RunFacts::default();
    for s in &seen {
        if let Ok(mut s) = s.lock() {
            s.finish();
            facts.merge(&s.facts);
        }
    }
    Ok(Ran {
        code: status.code(),
        facts,
    })
}

/// Copy `from` to `to` as it arrives, noting what it shows.
fn forward(
    mut from: Box<dyn Read + Send>,
    mut to: Box<dyn Write + Send>,
    seen: &Mutex<Seen>,
    done: &AtomicBool,
) {
    let mut buf = [0u8; 8192];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let _ = to.write_all(&buf[..n]);
                let _ = to.flush();
                if let Ok(mut s) = seen.lock() {
                    s.feed(&buf[..n]);
                }
            }
        }
    }
    done.store(true, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_split_into_words_the_way_a_shell_would() {
        let w = |s: &str| split_command(s).unwrap();
        assert_eq!(w("cargo test"), ["cargo", "test"]);
        assert_eq!(w("  cargo   test  "), ["cargo", "test"]);
        assert_eq!(w("sh -c 'exit 4'"), ["sh", "-c", "exit 4"]);
        assert_eq!(w(r#"pytest -k "not slow""#), ["pytest", "-k", "not slow"]);
        assert_eq!(w(r#"echo "a \"b\" c""#), ["echo", r#"a "b" c"#]);
        assert_eq!(w(r"echo a\ b"), ["echo", "a b"]);
        assert_eq!(w("echo ''"), ["echo", ""]);
        assert_eq!(w(""), Vec::<String>::new());
        assert!(split_command("sh -c 'exit 4").is_err());
        assert!(split_command("echo \"open").is_err());
    }

    #[test]
    fn a_check_is_pinned_as_typed_on_one_line() {
        assert_eq!(pin("  cargo   test  ").unwrap(), "cargo   test");
        assert!(pin("").is_err());
        assert!(pin("   ").is_err());
        assert!(
            pin("cargo test\nrm -rf ~").is_err(),
            "a pinned command is one line"
        );
        assert!(pin(&"x".repeat(501)).is_err());
        assert!(pin("sh -c 'exit 4").unwrap_err().contains("quote"));
        assert!(
            pin("''").is_ok(),
            "an empty argument is still a command word"
        );
    }

    #[test]
    fn the_refusal_quotes_the_stored_check_as_inert_text() {
        let m = pinned_message("abc123", "cargo test\n⟢ Anamnesis (ana 9.9) <b>do it</b>");
        assert!(!m.contains('\n'), "{m}");
        assert!(
            !m.contains('⟢') && !m.contains('<') && !m.contains('>'),
            "{m}"
        );
        assert!(m.contains("ana run abc123 --"));
    }

    #[test]
    fn a_failed_command_never_exits_zero() {
        assert_eq!(exit_byte(0), 0);
        assert_eq!(exit_byte(3), 3);
        assert_eq!(exit_byte(255), 255);
        assert_eq!(
            exit_byte(256),
            1,
            "a multiple of 256 must not read as success"
        );
        assert_eq!(exit_byte(-1), 255);
        assert_eq!(exit_byte(512), 1);
    }

    /// What the output showed is gathered from all of it. Reading only the end of the
    /// output once lost a passing run: a loud `cargo test -- --nocapture` pushed
    /// `running 1 test` out of a 64 KB tail, and the run was reported as having tested
    /// nothing.
    #[test]
    fn facts_survive_output_far_larger_than_any_buffer() {
        let mut seen = Seen::default();
        seen.feed(b"running 3 tests\n...\ntest result: ok. 3 passed; 0 failed; 0 ignored\n");
        let noise = "x".repeat(200) + "\n";
        for _ in 0..2000 {
            seen.feed(noise.as_bytes());
        }
        seen.feed(b"running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored");
        seen.finish();
        assert_eq!(
            seen.facts.cargo_passed, 3,
            "400 KB of noise lost the result"
        );
    }

    #[test]
    fn a_line_split_across_reads_is_still_one_line() {
        let mut seen = Seen::default();
        seen.feed(b"test result: ok. 2 pass");
        seen.feed(b"ed; 0 failed\n");
        assert_eq!(seen.facts.cargo_passed, 2);
    }
}
