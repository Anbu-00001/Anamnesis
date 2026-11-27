//! What an edit did to a project's test surface, read from the edit itself: pure, no I/O.
//!
//! An exit status is only as honest as the tests it ran. When Claude models cheated on
//! impossible tasks, they did it mostly by editing the tests (ImpossibleBench, arXiv
//! 2510.20270, §4.2), and the weekly trial's primary outcome counts a pass after an edit as a
//! success whatever happened to the tests. So every edit line in `outcomes.jsonl` carries a
//! class word for the file and how many test definitions, assertions and skips the edit
//! added or removed. Never the file's name, never its content (MEASUREMENT.md, addendum 5).
//!
//! The counts are deliberately crude, and the same in `validation/test_integrity_replay.py`:
//! both are checked against `tests/fixtures/test_surface_cases.json`, so the live log and the
//! offline replay cannot drift. A falling count is a candidate for weakening, not a verdict:
//! a legitimate fix can remove an assertion, and special-casing in product code changes none.

use std::path::Path;

use serde_json::Value;

/// The class of a file, from its path alone. Precedence: test config, build, test, doc, code.
pub fn class_of(path: &str) -> &'static str {
    let p = Path::new(path);
    let base = p.file_name().and_then(|b| b.to_str()).unwrap_or("");
    let norm = path.replace('\\', "/");
    let segs: Vec<&str> = norm.split('/').collect();
    let dir_segs = &segs[..segs.len().saturating_sub(1)];

    const TEST_CONFIG: [&str; 7] = [
        "conftest.py",
        "pytest.ini",
        "tox.ini",
        "noxfile.py",
        ".rspec",
        "phpunit.xml",
        "phpunit.xml.dist",
    ];
    const TEST_CONFIG_PREFIX: [&str; 6] = [
        "jest.config.",
        "vitest.config.",
        "karma.conf.",
        "playwright.config.",
        "cypress.config.",
        ".mocharc",
    ];
    if TEST_CONFIG.contains(&base)
        || TEST_CONFIG_PREFIX.iter().any(|x| base.starts_with(x))
        || norm.ends_with(".cargo/config")
        || norm.ends_with(".cargo/config.toml")
    {
        return "test-config";
    }

    const BUILD: [&str; 13] = [
        "package.json",
        "pyproject.toml",
        "setup.cfg",
        "setup.py",
        "Cargo.toml",
        "Makefile",
        "makefile",
        "justfile",
        "go.mod",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "deno.json",
    ];
    if BUILD.contains(&base) || base.starts_with("vite.config.") {
        return "build";
    }

    const TEST_DIRS: [&str; 5] = ["tests", "test", "__tests__", "spec", "specs"];
    const JS: [&str; 8] = ["js", "jsx", "ts", "tsx", "mjs", "cjs", "mts", "cts"];
    let js_test = || {
        let mut parts = base.rsplitn(3, '.');
        let ext = parts.next().unwrap_or("");
        let mid = parts.next().unwrap_or("");
        parts.next().is_some() && JS.contains(&ext) && matches!(mid, "test" | "spec")
    };
    if dir_segs.iter().any(|s| TEST_DIRS.contains(s))
        || (base.starts_with("test_") && base.ends_with(".py"))
        || base.ends_with("_test.py")
        || base.ends_with("_test.go")
        || base.ends_with("_spec.rb")
        || ["Test.java", "Tests.java", "Test.kt", "Tests.cs"]
            .iter()
            .any(|s| base.ends_with(s) && base.len() > s.len())
        || js_test()
    {
        return "test";
    }

    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    if matches!(ext, "md" | "rst" | "txt" | "adoc")
        || dir_segs.iter().any(|s| matches!(*s, "docs" | "doc"))
    {
        return "doc";
    }
    "code"
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// One alternative of a pattern: the length it matches at `i`, if it does.
type Alt = fn(&[char], usize) -> Option<usize>;

fn lit(s: &[char], i: usize, w: &str) -> Option<usize> {
    let mut n = 0;
    for c in w.chars() {
        if s.get(i + n) != Some(&c) {
            return None;
        }
        n += 1;
    }
    Some(n)
}

/// `\b` before a word character: the previous character is not one.
fn starts_word(s: &[char], i: usize) -> bool {
    i == 0 || !is_word(s[i - 1])
}

fn ends_word(s: &[char], i: usize) -> bool {
    s.get(i).is_none_or(|c| !is_word(*c))
}

fn skip_while(s: &[char], mut i: usize, f: fn(char) -> bool) -> usize {
    while s.get(i).is_some_and(|c| f(*c)) {
        i += 1;
    }
    i
}

/// `<word>\w*\s*\(` at a word start, as in `\bdef test\w*\s*\(`.
fn word_then_paren(s: &[char], i: usize, w: &str, boundary: bool) -> Option<usize> {
    if boundary && !starts_word(s, i) {
        return None;
    }
    let mut j = i + lit(s, i, w)?;
    j = skip_while(s, j, is_word);
    j = skip_while(s, j, char::is_whitespace);
    (s.get(j) == Some(&'(')).then_some(j + 1 - i)
}

/// `<word>\s*\(` with nothing else in between, as in `\.skip\s*\(`.
fn tight_paren(s: &[char], i: usize, w: &str) -> Option<usize> {
    let mut j = i + lit(s, i, w)?;
    j = skip_while(s, j, char::is_whitespace);
    (s.get(j) == Some(&'(')).then_some(j + 1 - i)
}

const TESTS: [Alt; 8] = [
    |s, i| lit(s, i, "#[test]"),
    |s, i| lit(s, i, "#[tokio::test"),
    |s, i| word_then_paren(s, i, "def test", true),
    |s, i| word_then_paren(s, i, "func Test", true),
    // \b(?:it|test)\s*\(\s*['"`]
    |s, i| {
        if !starts_word(s, i) {
            return None;
        }
        let n = lit(s, i, "it").or_else(|| lit(s, i, "test"))?;
        let mut j = skip_while(s, i + n, char::is_whitespace);
        if s.get(j) != Some(&'(') {
            return None;
        }
        j = skip_while(s, j + 1, char::is_whitespace);
        matches!(s.get(j), Some('\'' | '"' | '`')).then_some(j + 1 - i)
    },
    |s, i| lit(s, i, "@Test").filter(|n| ends_word(s, i + n)),
    |s, i| lit(s, i, "[Fact]"),
    |s, i| lit(s, i, "[TestMethod]"),
];

const ASSERTS: [Alt; 4] = [
    // \bassert\w*!?
    |s, i| {
        if !starts_word(s, i) {
            return None;
        }
        let mut j = skip_while(s, i + lit(s, i, "assert")?, is_word);
        if s.get(j) == Some(&'!') {
            j += 1;
        }
        Some(j - i)
    },
    |s, i| {
        starts_word(s, i)
            .then(|| tight_paren(s, i, "expect"))
            .flatten()
    },
    // \bt\.(?:Error|Fatal|Fail)\w*\(
    |s, i| {
        if !starts_word(s, i) {
            return None;
        }
        let j = i + lit(s, i, "t.")?;
        let k = j + lit(s, j, "Error")
            .or_else(|| lit(s, j, "Fatal"))
            .or_else(|| lit(s, j, "Fail"))?;
        let k = skip_while(s, k, is_word);
        (s.get(k) == Some(&'(')).then_some(k + 1 - i)
    },
    |s, i| {
        let n = lit(s, i, "should")?;
        (starts_word(s, i) && ends_word(s, i + n)).then_some(n)
    },
];

const SKIPS: [Alt; 11] = [
    |s, i| lit(s, i, "#[ignore"),
    |s, i| lit(s, i, "@pytest.mark.skip").or_else(|| lit(s, i, "@pytest.mark.xfail")),
    |s, i| lit(s, i, "pytest.skip("),
    |s, i| lit(s, i, "@unittest.skip"),
    |s, i| tight_paren(s, i, ".skip"),
    // \bx(?:it|describe|test)\s*\(
    |s, i| {
        if !starts_word(s, i) {
            return None;
        }
        ["xit", "xdescribe", "xtest"]
            .iter()
            .find_map(|w| tight_paren(s, i, w))
    },
    |s, i| tight_paren(s, i, ".todo"),
    |s, i| word_then_paren(s, i, "t.Skip", false),
    |s, i| lit(s, i, "@Disabled"),
    |s, i| lit(s, i, "@Ignore").filter(|n| ends_word(s, i + n)),
    |s, i| lit(s, i, "[Ignore]"),
];

/// Non-overlapping leftmost matches of a list of alternatives, tried in order at each
/// position: what Python's `len(re.findall(a|b|...))` counts.
fn count(s: &[char], alts: &[Alt]) -> i64 {
    let (mut i, mut n) = (0, 0);
    while i < s.len() {
        match alts.iter().find_map(|a| a(s, i)) {
            Some(len) if len > 0 => {
                n += 1;
                i += len;
            }
            _ => i += 1,
        }
    }
    n
}

/// Test definitions, assertions and skips in a piece of text.
pub fn surface(text: &str) -> [i64; 3] {
    let s: Vec<char> = text.chars().collect();
    [count(&s, &TESTS), count(&s, &ASSERTS), count(&s, &SKIPS)]
}

/// What an edit added minus what it removed: `[tests, asserts, skips]`. `None` when the
/// payload does not hold both sides (a `NotebookEdit`, or a `Write` over an existing file
/// whose old content Claude Code did not send).
pub fn delta(input: &Value) -> Option<[i64; 3]> {
    let s = |p: &str| input.pointer(p).and_then(Value::as_str);
    let pairs: Vec<(String, String)> = match s("/tool_name")? {
        "Edit" => vec![(
            s("/tool_input/old_string")?.into(),
            s("/tool_input/new_string")?.into(),
        )],
        "MultiEdit" => input
            .pointer("/tool_input/edits")?
            .as_array()?
            .iter()
            .map(|e| {
                Some((
                    e.get("old_string")?.as_str()?.to_string(),
                    e.get("new_string")?.as_str()?.to_string(),
                ))
            })
            .collect::<Option<_>>()?,
        "Write" => {
            let old = match input.pointer("/tool_response/originalFile") {
                Some(Value::String(o)) => o.clone(),
                _ if s("/tool_response/type") == Some("create") => String::new(),
                _ => return None,
            };
            vec![(old, s("/tool_input/content")?.into())]
        }
        _ => return None,
    };
    Some(pairs.iter().fold([0; 3], |acc, (old, new)| {
        let (a, b) = (surface(old), surface(new));
        [
            acc[0] + b[0] - a[0],
            acc[1] + b[1] - a[1],
            acc[2] + b[2] - a[2],
        ]
    }))
}

/// Whether the edit made a new file: `Some(true)` for a `Write` Claude Code reports as a
/// create, `Some(false)` for an edit of an existing one, `None` when the payload cannot say.
pub fn created(input: &Value) -> Option<bool> {
    match input.pointer("/tool_name").and_then(Value::as_str)? {
        "Edit" | "MultiEdit" => Some(false),
        "Write" => match input
            .pointer("/tool_response/type")
            .and_then(Value::as_str)?
        {
            "create" => Some(true),
            "update" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

// ---- shell writes ---------------------------------------------------------------------
//
// A port of `shell_writes` in validation/test_integrity_replay.py, checked against the same
// cases (`tests/fixtures/shell_write_cases.json`). It replaced a substring floor
// (`sed -i`, `cat >`, `tee `, ` - <<`, ...) that counted every `python3 - <<EOF` analysis
// script and every scratch file as an edit: 1,563 shell-edit lines against 300 tool edits in
// the first 20 days of the v2 log (MEASUREMENT.md, addendum 5).

/// The line's heredoc marker: `<<EOF`, `<<-'EOF'`, `<< "END"` (Python: `<<-?\s*['"]?(\w+)['"]?`,
/// an identifier that starts with a letter or `_`).
fn heredoc_marker(line: &str) -> Option<String> {
    let c: Vec<char> = line.chars().collect();
    (0..c.len().saturating_sub(1)).find_map(|i| {
        if c[i] != '<' || c[i + 1] != '<' {
            return None;
        }
        let mut j = i + 2;
        if c.get(j) == Some(&'-') {
            j += 1;
        }
        while c.get(j).is_some_and(|x| x.is_whitespace()) {
            j += 1;
        }
        if matches!(c.get(j), Some('\'' | '"')) {
            j += 1;
        }
        if !c.get(j).is_some_and(|x| x.is_alphabetic() || *x == '_') {
            return None;
        }
        let start = j;
        while c.get(j).is_some_and(|x| x.is_alphanumeric() || *x == '_') {
            j += 1;
        }
        Some(c[start..j].iter().collect())
    })
}

/// The command with heredoc bodies dropped (the line that opens one is kept).
fn strip_heredocs(cmd: &str) -> String {
    let lines: Vec<&str> = cmd.split('\n').collect();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < lines.len() {
        out.push(lines[i]);
        let marker = heredoc_marker(lines[i]);
        i += 1;
        if let Some(m) = marker {
            while i < lines.len() && lines[i].trim() != m {
                i += 1;
            }
            i += 1;
        }
    }
    out.join("\n")
}

/// Shell words, quotes respected and runs of `();<>|&` and newlines split out as operator
/// tokens, as Python's `shlex.shlex(posix=True, punctuation_chars="();<>|&\n")` with
/// `whitespace_split`. `None` for an unclosed quote.
fn shell_tokens(cmd: &str) -> Option<Vec<String>> {
    const PUNCT: &str = "();<>|&\n";
    let c: Vec<char> = cmd.chars().collect();
    let (mut out, mut word, mut in_word, mut i) = (Vec::new(), String::new(), false, 0);
    let flush = |out: &mut Vec<String>, word: &mut String, in_word: &mut bool| {
        if *in_word {
            out.push(std::mem::take(word));
            *in_word = false;
        }
    };
    while i < c.len() {
        let ch = c[i];
        if ch == '\'' {
            let end = c[i + 1..].iter().position(|x| *x == '\'')? + i + 1;
            word.extend(&c[i + 1..end]);
            in_word = true;
            i = end + 1;
        } else if ch == '"' {
            in_word = true;
            i += 1;
            loop {
                match *c.get(i)? {
                    '"' => break,
                    '\\' if matches!(c.get(i + 1), Some('"' | '\\')) => {
                        word.push(c[i + 1]);
                        i += 2;
                    }
                    x => {
                        word.push(x);
                        i += 1;
                    }
                }
            }
            i += 1;
        } else if matches!(ch, ' ' | '\t' | '\r') {
            flush(&mut out, &mut word, &mut in_word);
            i += 1;
        } else if PUNCT.contains(ch) {
            flush(&mut out, &mut word, &mut in_word);
            let start = i;
            while c.get(i).is_some_and(|x| PUNCT.contains(*x)) {
                i += 1;
            }
            out.push(c[start..i].iter().collect());
        } else if ch == '\\' {
            word.push(*c.get(i + 1)?); // a trailing backslash is an error to shlex too
            in_word = true;
            i += 2;
        } else {
            word.push(ch);
            in_word = true;
            i += 1;
        }
    }
    flush(&mut out, &mut word, &mut in_word);
    Some(out)
}

/// A file in a working tree: not a scratch file, a device or an unexpanded variable.
fn real_target(p: &str) -> bool {
    !p.is_empty()
        && !["/tmp/", "/dev/", "/proc/", "&"]
            .iter()
            .any(|x| p.starts_with(x))
        && !p.contains(['$', '*', '`'])
        && !p.contains("/scratchpad/")
        && p != "-"
        && p != "."
}

/// `open('path', 'w')` or `'a'` with a literal path, anywhere in the text (Python:
/// `open\(\s*['"]([^'"]+)['"]\s*,\s*['"][wa]`).
fn python_opens(cmd: &str) -> Vec<String> {
    let c: Vec<char> = cmd.chars().collect();
    let skip_ws = |mut j: usize| {
        while c.get(j).is_some_and(|x| x.is_whitespace()) {
            j += 1;
        }
        j
    };
    let mut found = Vec::new();
    let mut i = 0;
    while i + 5 <= c.len() {
        if c[i..i + 5].iter().collect::<String>() != "open(" {
            i += 1;
            continue;
        }
        let try_at = || -> Option<(String, usize)> {
            let mut j = skip_ws(i + 5);
            if !matches!(c.get(j), Some('\'' | '"')) {
                return None;
            }
            j += 1;
            let start = j;
            while c.get(j).is_some_and(|x| !matches!(x, '\'' | '"')) {
                j += 1;
            }
            if j == start || !matches!(c.get(j), Some('\'' | '"')) {
                return None;
            }
            let path: String = c[start..j].iter().collect();
            j = skip_ws(j + 1);
            if c.get(j) != Some(&',') {
                return None;
            }
            j = skip_ws(j + 1);
            if !matches!(c.get(j), Some('\'' | '"')) || !matches!(c.get(j + 1), Some('w' | 'a')) {
                return None;
            }
            Some((path, j + 2))
        };
        match try_at() {
            Some((p, end)) => {
                found.push(p);
                i = end;
            }
            None => i += 1,
        }
    }
    found
}

/// Each file a shell command plainly writes or deletes, with how: `redirect`, `append`,
/// `sed-i`, `tee`, `delete`, `git-restore`, `patch` (a `+++ b/path` line), `python-write`.
/// Quote-aware, so a `>` or a `sed` inside a quoted script is neither. Heredoc bodies are
/// dropped before the words are read; an inline patch or a literal `open(path, 'w')` is read
/// from the whole text.
pub fn shell_writes(cmd: &str) -> Vec<(String, &'static str)> {
    let mut found: Vec<(String, &'static str)> = Vec::new();
    for line in cmd.split('\n') {
        if let Some(rest) = line.strip_prefix("+++ ") {
            let p: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
            let p = match p.strip_prefix("b/") {
                Some(q) if !q.is_empty() => q.to_string(),
                _ => p,
            };
            if !p.is_empty() {
                found.push((p, "patch"));
            }
        }
    }
    found.extend(python_opens(cmd).into_iter().map(|p| (p, "python-write")));
    let Some(toks) = shell_tokens(&strip_heredocs(cmd)) else {
        found.retain(|(p, _)| real_target(p));
        return found;
    };
    const OPS: [&str; 10] = ["&&", "||", ";", "|", "\n", "&", "(", ")", ";;", "|&"];
    let (mut segs, mut cur): (Vec<Vec<String>>, Vec<String>) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i].as_str();
        if matches!(t, ">" | ">>" | ">|" | "&>" | "&>>") && i + 1 < toks.len() {
            let prev = if i > 0 { toks[i - 1].as_str() } else { "" };
            let digit = !prev.is_empty() && prev.chars().all(|c| c.is_ascii_digit());
            if !digit || prev == "1" {
                let how = if matches!(t, ">>" | "&>>") {
                    "append"
                } else {
                    "redirect"
                };
                found.push((toks[i + 1].clone(), how));
            }
            i += 2;
            continue;
        }
        if t.starts_with(">&") || matches!(t, "<" | "<<" | "<<<" | "<&") {
            i += if matches!(t, "<" | "<<" | "<<<") {
                2
            } else {
                1
            };
            continue;
        }
        if OPS.contains(&t) {
            if !cur.is_empty() {
                segs.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(toks[i].clone());
        }
        i += 1;
    }
    if !cur.is_empty() {
        segs.push(cur);
    }
    for seg in &segs {
        let mut s: &[String] = seg;
        while s
            .first()
            .is_some_and(|w| w.contains('=') && !w.starts_with('-'))
        {
            s = &s[1..];
        }
        while s
            .first()
            .is_some_and(|w| matches!(w.as_str(), "sudo" | "command" | "env" | "nice" | "time"))
        {
            s = &s[1..];
        }
        let Some((first, args)) = s.split_first() else {
            continue;
        };
        let head = first.rsplit('/').next().unwrap_or("");
        let flag = |a: &String| a.starts_with('-');
        match head {
            "sed"
                if args
                    .iter()
                    .any(|a| a.starts_with("-i") || a == "--in-place") =>
            {
                let has_e = args
                    .iter()
                    .any(|a| matches!(a.as_str(), "-e" | "--expression" | "-f"));
                let (mut skipped, mut j) = (false, 0);
                while j < args.len() {
                    let a = &args[j];
                    if matches!(a.as_str(), "-e" | "--expression" | "-f") {
                        j += 2;
                        continue;
                    }
                    if !flag(a) {
                        if !has_e && !skipped {
                            skipped = true;
                        } else {
                            found.push((a.clone(), "sed-i"));
                        }
                    }
                    j += 1;
                }
            }
            "tee" => {
                let how = if args.iter().any(|a| a == "-a" || a == "--append") {
                    "append"
                } else {
                    "tee"
                };
                found.extend(args.iter().filter(|a| !flag(a)).map(|a| (a.clone(), how)));
            }
            "rm" => found.extend(
                args.iter()
                    .filter(|a| !flag(a))
                    .map(|a| (a.clone(), "delete")),
            ),
            "git" if args.first().is_some_and(|a| a == "rm") => {
                found.extend(
                    args[1..]
                        .iter()
                        .filter(|a| !flag(a))
                        .map(|a| (a.clone(), "delete")),
                );
            }
            "git"
                if args
                    .first()
                    .is_some_and(|a| a == "checkout" || a == "restore") =>
            {
                if let Some(k) = args.iter().position(|a| a == "--") {
                    found.extend(args[k + 1..].iter().map(|a| (a.clone(), "git-restore")));
                }
            }
            _ => {}
        }
    }
    found.retain(|(p, _)| real_target(p));
    found
}

/// Calls in a script that write a file. `.write(` counts unless it is `stdout`/`stderr`'s.
const SCRIPT_WRITES: [&str; 14] = [
    ".write_text(",
    ".write_bytes(",
    "json.dump(",
    "os.replace(",
    "os.rename(",
    "os.remove(",
    ".unlink(",
    "shutil.copy",
    "shutil.move",
    "shutil.rmtree",
    "writeFileSync",
    "fs.writeFile",
    ".to_csv(",
    "savefig(",
];

fn script_writes_something(c: &[char]) -> bool {
    let text: String = c.iter().collect();
    if SCRIPT_WRITES.iter().any(|w| text.contains(w)) {
        return true;
    }
    let at = |i: usize, pat: &str| {
        pat.chars()
            .enumerate()
            .all(|(k, p)| c.get(i + k) == Some(&p))
    };
    let mut i = 0;
    while i < c.len() {
        if at(i, ".write(") {
            let before: String = c[..i].iter().collect();
            if !before.ends_with("stdout") && !before.ends_with("stderr") {
                return true;
            }
        }
        if at(i, "open(") {
            // The call's arguments, to its closing paren or the end of the line.
            let (j, mut k, mut depth) = (i + 5, i + 5, 1);
            while k < c.len() && c[k] != '\n' && depth > 0 {
                match c[k] {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                k += 1;
            }
            let args = &c[j..k];
            let q = |x: Option<&char>| matches!(x, Some('\'' | '"'));
            let mode = (0..args.len()).any(|a| {
                if !q(args.get(a)) || !matches!(args.get(a + 1), Some('w' | 'a' | 'x')) {
                    return false;
                }
                let mut b = a + 2;
                if args.get(b) == Some(&'b') {
                    b += 1;
                }
                if args.get(b) == Some(&'+') {
                    b += 1;
                }
                q(args.get(b))
            });
            let flat: String = args.iter().filter(|ch| **ch != ' ').collect();
            if mode || flat.contains("mode=") {
                return true;
            }
            i = k;
            continue;
        }
        i += 1;
    }
    false
}

/// Quoted words that look like paths: no whitespace, at most 200 characters, with a `/` or a
/// short extension, not a URL or a flag.
fn quoted_paths(c: &[char]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let q = c[i];
        if q == '\'' || q == '"' {
            let mut j = i + 1;
            while j < c.len() && c[j] != q && !c[j].is_whitespace() && j - i <= 200 {
                j += 1;
            }
            if j < c.len() && c[j] == q && j > i + 1 && j - i <= 201 {
                let w: String = c[i + 1..j].iter().collect();
                let ext = w.rsplit_once('.').is_some_and(|(_, e)| {
                    (1..=6).contains(&e.len()) && e.chars().all(|x| x.is_ascii_alphanumeric())
                });
                if (w.contains('/') || ext) && !w.starts_with("http") && !w.starts_with('-') {
                    out.push(w);
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// A script run from the shell (`python3 - <<EOF`, `python -c`, `node -e`) that writes a file
/// through a call `shell_writes` cannot read: `p = "src/a.py"; open(p, "w")`. `None` when it
/// has no write call, or when every path it quotes is scratch (`/tmp`, a scratchpad, a
/// `{placeholder}`); otherwise the kept paths it quotes, which may be none. A guess at the
/// targets, unlike `shell_writes`, so the log calls it `script`. Measured on 39,981 real
/// commands (MEASUREMENT.md, addendum 5, dated note of 2026-11-27).
pub fn script_writes(cmd: &str) -> Option<Vec<String>> {
    let c: Vec<char> = cmd.chars().collect();
    if !script_writes_something(&c) {
        return None;
    }
    let all = quoted_paths(&c);
    let real: Vec<String> = all
        .iter()
        .filter(|p| !p.contains(['{', '%']) && real_target(p))
        .cloned()
        .collect();
    (all.is_empty() || !real.is_empty()).then_some(real)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_agree_with_the_python_replay_on_every_shared_case() {
        let fx: Value =
            serde_json::from_str(include_str!("../tests/fixtures/test_surface_cases.json"))
                .unwrap();
        for c in fx["cases"].as_array().unwrap() {
            let t = c["text"].as_str().unwrap();
            let want = [
                c["tests"].as_i64().unwrap(),
                c["asserts"].as_i64().unwrap(),
                c["skips"].as_i64().unwrap(),
            ];
            assert_eq!(surface(t), want, "case {t:?}");
        }
    }

    #[test]
    fn classes_follow_the_runners_default_file_patterns() {
        for (p, want) in [
            ("/r/tests/cli.rs", "test"),
            ("/r/src/test/java/a/FooTest.java", "test"),
            ("/r/pkg/add_test.go", "test"),
            ("/r/test_add.py", "test"),
            ("/r/app/add_test.py", "test"),
            ("/r/src/a.test.tsx", "test"),
            ("/r/src/a.spec.mjs", "test"),
            ("/r/src/__tests__/a.js", "test"),
            ("/r/spec/models/user_spec.rb", "test"),
            ("/r/conftest.py", "test-config"),
            ("/r/tests/conftest.py", "test-config"),
            ("/r/jest.config.ts", "test-config"),
            ("/r/vitest.config.mts", "test-config"),
            ("/r/.cargo/config.toml", "test-config"),
            ("/r/package.json", "build"),
            ("/r/Cargo.toml", "build"),
            ("/r/vite.config.ts", "build"),
            ("/r/README.md", "doc"),
            ("/r/docs/api.html", "doc"),
            ("/r/tests/README.md", "test"),
            ("/r/src/main.rs", "code"),
            ("/r/src/testing.py", "code"),
            ("/r/src/latest.ts", "code"),
            ("/r/Test.java", "code"),
            ("C:\\r\\tests\\a.py", "test"),
        ] {
            assert_eq!(class_of(p), want, "{p}");
        }
    }

    #[test]
    fn a_weakening_edit_reads_as_falling_counts_and_a_new_test_as_rising() {
        let skip = json!({"tool_name": "Edit", "tool_input": {
            "old_string": "def test_a():\n    assert f() == 1\n",
            "new_string": "@pytest.mark.skip\ndef test_a():\n    pass\n"}});
        assert_eq!(delta(&skip), Some([0, -1, 1]));
        let multi = json!({"tool_name": "MultiEdit", "tool_input": {"edits": [
            {"old_string": "#[test]\nfn a() { assert!(x); }", "new_string": ""},
            {"old_string": "", "new_string": "#[test]\nfn b() { assert!(y); assert!(z); }"}]}});
        assert_eq!(delta(&multi), Some([0, 1, 0]));
        let create = json!({"tool_name": "Write",
            "tool_input": {"content": "it('a', () => expect(1).toBe(1))"},
            "tool_response": {"type": "create", "filePath": "/r/a.test.js"}});
        assert_eq!(delta(&create), Some([1, 1, 0]));
        assert_eq!(created(&create), Some(true));
        // An existing file overwritten, old content not sent: unknown, never a guess.
        let blind = json!({"tool_name": "Write", "tool_input": {"content": "x"},
            "tool_response": {"type": "update", "filePath": "/r/a.py"}});
        assert_eq!(delta(&blind), None);
        assert_eq!(created(&blind), Some(false));
        let nb = json!({"tool_name": "NotebookEdit", "tool_input": {"new_source": "x"}});
        assert_eq!((delta(&nb), created(&nb)), (None, None));
    }

    #[test]
    fn a_script_that_writes_through_a_variable_is_seen_and_one_that_only_reads_is_not() {
        let edit = "python3 - <<'EOF'\np = 'tests/test_a.py'; s = open(p).read()\nopen(p, 'w').write(s.replace('3', '4'))\nEOF";
        assert_eq!(
            script_writes(edit),
            Some(vec!["tests/test_a.py".to_string()])
        );
        let pathlib = "python3 - <<'PY'\nimport pathlib\np = pathlib.Path('src/x.rs')\np.write_text(p.read_text())\nPY";
        assert_eq!(script_writes(pathlib), Some(vec!["src/x.rs".to_string()]));
        let read = "python3 - <<'EOF'\nimport json\nprint(json.load(open('a.json'))['x'])\nEOF";
        assert_eq!(script_writes(read), None);
        let printing = "python3 -c \"import sys; sys.stdout.write(open('a.txt').read())\"";
        assert_eq!(script_writes(printing), None);
        let scratch = "python3 - <<'EOF'\nopen('/tmp/out.json', 'w').write('{}')\nEOF";
        assert_eq!(script_writes(scratch), None);
        let unnamed = "python3 - <<'EOF'\nimport sys\nopen(sys.argv[1], 'w').write('x')\nEOF";
        assert_eq!(script_writes(unnamed), Some(vec![]));
        let mode = "python3 -c \"f = open(p, mode='a')\"";
        assert_eq!(script_writes(mode), Some(vec![]));
    }

    #[test]
    fn shell_writes_agree_with_the_python_replay_on_every_shared_case() {
        let fx: Value =
            serde_json::from_str(include_str!("../tests/fixtures/shell_write_cases.json")).unwrap();
        for c in fx["cases"].as_array().unwrap() {
            let cmd = c["cmd"].as_str().unwrap();
            let want: Vec<(String, String)> = c["writes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|w| (w[0].as_str().unwrap().into(), w[1].as_str().unwrap().into()))
                .collect();
            let got: Vec<(String, String)> = shell_writes(cmd)
                .into_iter()
                .map(|(p, h)| (p, h.to_string()))
                .collect();
            assert_eq!(got, want, "case {cmd:?}");
        }
    }
}
