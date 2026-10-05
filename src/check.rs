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
use std::process::{Command, Stdio};
use std::thread;

use crate::untrusted;

/// The longest pinned command.
const MAX_CHECK: usize = 500;
/// How much of a command's output is kept for judging its result.
const TAIL_BYTES: usize = 64 * 1024;

/// A command as it is pinned: trimmed, runs of whitespace collapsed, and refused if it
/// is empty, spans lines, or is longer than anyone would type.
pub fn pin(raw: &str) -> Result<String, String> {
    let c = crate::model::collapse_spaces(raw);
    if c.is_empty() {
        return Err("a check must name the command that settles the claim".into());
    }
    if raw.chars().any(char::is_control) {
        return Err("a check must be one line".into());
    }
    if c.chars().count() > MAX_CHECK {
        return Err(format!("a check is at most {MAX_CHECK} characters"));
    }
    Ok(c)
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

/// A command that has finished.
pub struct Ran {
    /// Its exit code, or `None` if a signal ended it: then there is no status to grade.
    pub code: Option<i32>,
    /// The end of what it printed (stdout, then stderr), for judging the result.
    pub tail: String,
}

/// Run `argv` directly (no shell), passing its output through as it arrives, and
/// return how it ended. The caller holds no lock: a test run can take minutes.
pub fn run_command(argv: &[String]) -> io::Result<Ran> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no command given"))?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let out = child.stdout.take().expect("piped stdout");
    let err = child.stderr.take().expect("piped stderr");
    let t_out = thread::spawn(move || forward(out, io::stdout()));
    let t_err = thread::spawn(move || forward(err, io::stderr()));
    let status = child.wait()?;
    let mut tail = t_out.join().unwrap_or_default();
    tail.extend(t_err.join().unwrap_or_default());
    Ok(Ran {
        code: status.code(),
        tail: String::from_utf8_lossy(&tail).into_owned(),
    })
}

/// Copy `from` to `to` as it arrives, keeping only the last [`TAIL_BYTES`].
fn forward(mut from: impl Read, mut to: impl Write) -> Vec<u8> {
    let mut tail: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let _ = to.write_all(&buf[..n]);
                let _ = to.flush();
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > 2 * TAIL_BYTES {
                    let cut = tail.len() - TAIL_BYTES;
                    tail.drain(..cut);
                }
            }
        }
    }
    if tail.len() > TAIL_BYTES {
        let cut = tail.len() - TAIL_BYTES;
        tail.drain(..cut);
    }
    tail
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_is_pinned_as_one_collapsed_line() {
        assert_eq!(pin("  cargo   test  ").unwrap(), "cargo test");
        assert!(pin("").is_err());
        assert!(pin("   ").is_err());
        assert!(
            pin("cargo test\nrm -rf ~").is_err(),
            "a pinned command is one line"
        );
        assert!(pin(&"x".repeat(501)).is_err());
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
    fn the_runner_keeps_only_the_end_of_a_long_output() {
        let big = vec![b'a'; 3 * TAIL_BYTES];
        let kept = forward(&big[..], io::sink());
        assert_eq!(kept.len(), TAIL_BYTES);
    }
}
