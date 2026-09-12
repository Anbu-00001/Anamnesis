//! `ana` — the command-line face of Anamnesis.
//!
//! A deliberately small surface: record a belief, revise it, resolve it, and
//! then look in the mirror. Beliefs come in two flavours — **binary** (a yes/no
//! proposition with a probability) and **numeric** (a quantity with a credible
//! interval). Every command also speaks `--json`, so an agent, script, or future
//! UI can drive it without scraping prose.

use std::path::PathBuf;
use std::process::ExitCode;

use chrono::{NaiveDate, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};

use anamnesis::model::{
    compose_reasoning, gen_id, normalize_tags, Claim, ClaimKind, Forecast, NumericForecast,
    Outcome, Resolution,
};
use anamnesis::scoring::{self, NumericSample};
use anamnesis::{mcp, report, store};

#[derive(Parser)]
#[command(
    name = "ana",
    version,
    about = "Anamnesis — an instrument against self-deception",
    long_about = "Record what you believe, how sure you are, and why — before the outcome is \
known. Later, face the real shape of your judgement: where you are overconfident, whether you \
can tell truth from falsehood, and how honestly you change your mind."
)]
struct Cli {
    /// Ledger file to use (overrides $ANAMNESIS_DATA; default ~/.anamnesis.json)
    #[arg(long, global = true, value_name = "FILE")]
    data: Option<PathBuf>,

    /// Emit machine-readable JSON instead of human text
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Record a new belief. Use --prob for a yes/no claim, or --interval for a number.
    Add {
        /// The falsifiable statement
        statement: String,
        /// BINARY: probability it is true, 0..1 (e.g. 0.7)
        #[arg(short, long)]
        prob: Option<f64>,
        /// NUMERIC: credible interval "LOW..HIGH" (e.g. 120..180)
        #[arg(long, value_name = "LOW..HIGH")]
        interval: Option<String>,
        /// NUMERIC: confidence level of the interval (default 0.80)
        #[arg(long, default_value_t = 0.80)]
        level: f64,
        /// Date you expect to know the answer (YYYY-MM-DD)
        #[arg(long, value_name = "YYYY-MM-DD")]
        by: Option<String>,
        /// Comma-separated tags
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        /// The reasoning behind the forecast — what hindsight will try to erase
        #[arg(long)]
        because: Option<String>,
        /// How much this call MATTERS (≥ 0; default 1) — weights the Brier toward
        /// your consequential predictions
        #[arg(long, default_value_t = 1.0)]
        stake: f64,
        /// BINARY: a SECOND, deliberately-opposite estimate ("consider the
        /// opposite"); the logged probability is the average of the two
        /// (dialectical bootstrapping — the wisdom of your own crowd)
        #[arg(long)]
        second_prob: Option<f64>,
        /// The OUTSIDE VIEW: a reference class of similar past cases and its base
        /// rate, recorded with the forecast
        #[arg(long)]
        reference_class: Option<String>,
    },
    /// Revise a belief (history is preserved). Match the claim's kind.
    Update {
        id: String,
        /// BINARY: new probability, 0..1
        #[arg(short, long)]
        prob: Option<f64>,
        /// NUMERIC: new interval "LOW..HIGH"
        #[arg(long, value_name = "LOW..HIGH")]
        interval: Option<String>,
        /// NUMERIC: confidence level (defaults to the claim's previous level)
        #[arg(long)]
        level: Option<f64>,
        #[arg(long)]
        because: Option<String>,
    },
    /// Resolve a belief. BINARY: yes/no. NUMERIC: --value N.
    Resolve {
        id: String,
        /// BINARY outcome: yes/true or no/false
        #[arg(value_enum)]
        outcome: Option<OutcomeArg>,
        /// NUMERIC outcome: the value that occurred
        #[arg(long)]
        value: Option<f64>,
        /// A post-mortem: with hindsight, what did you miss?
        #[arg(long)]
        note: Option<String>,
    },
    /// List beliefs (default: all)
    List {
        #[arg(long)]
        open: bool,
        #[arg(long)]
        resolved: bool,
        #[arg(long)]
        due: bool,
        /// Only claims carrying this exact tag (e.g. project:anamnesis)
        #[arg(long)]
        tag: Option<String>,
        /// Only annulled claims — the questions that turned out unanswerable
        #[arg(long)]
        void: bool,
    },
    /// Show the full history of one belief — the palimpsest of your mind
    Show { id: String },
    /// The calibration report: the real shape of your judgement
    Report {
        #[arg(long)]
        tag: Option<String>,
        #[arg(long, default_value_t = 10)]
        bins: usize,
        /// Plain-English view: every number translated into what it means for you
        #[arg(long)]
        plain: bool,
        /// Self-contained, offline HTML card (redirect to a file: `> card.html`)
        #[arg(long)]
        html: bool,
        /// Embeddable 400×100 README badge as SVG (redirect: `> badge.svg`)
        #[arg(long)]
        badge: bool,
    },
    /// Should you act on it? Corrects a stated probability through your earned
    /// recalibration map, then applies a stake-aware threshold: PROCEED / VERIFY / ABSTAIN.
    Decide {
        /// Your stated success probability, 0..1
        #[arg(long, short = 'p')]
        prob: f64,
        /// Cost of a wrong action relative to one verification (1 = ordinary; raise it
        /// for consequential or irreversible calls — the bar to proceed climbs with it)
        #[arg(long, default_value_t = 1.0)]
        stake: f64,
        /// Cost of a verification step, in the same unit as stake
        #[arg(long, default_value_t = 0.2)]
        verify_cost: f64,
        /// Scope the correction map to claims carrying this tag (e.g. kind:estimate)
        #[arg(long)]
        tag: Option<String>,
    },
    /// Serve as a Model Context Protocol server over stdio — exposes
    /// predict/resolve/calibration/recalibrate/decide/list as tools for any MCP-capable agent.
    Mcp,
    /// Run a Claude Code hook: reads the hook JSON on stdin, writes the
    /// hookSpecificOutput on stdout. Replaces the shell scripts (and `jq`).
    Hook {
        /// Which hook is firing
        #[arg(value_enum)]
        event: HookEvent,
    },
    /// Try the tool on a fictional year of predictions: builds a demo ledger in
    /// a temporary directory, reports on it, and never touches your own.
    Demo {
        /// Write the demo ledger here instead of a temporary file, to poke at it
        #[arg(long)]
        keep: Option<PathBuf>,
        /// Plain-English report instead of the technical one
        #[arg(long)]
        plain: bool,
    },
    /// Export the ledger with every free-text field removed, so a real record
    /// can be published as evidence without publishing what it was about.
    Export {
        /// Strip statements, reasoning and notes; keep the numbers. Required —
        /// there is no un-anonymized export, because the obvious mistake is to
        /// publish one by accident.
        #[arg(long)]
        anonymize: bool,
        /// Write here instead of stdout
        #[arg(long)]
        out: Option<PathBuf>,
        /// Keep only these tag namespaces (default: kind, who, model, project)
        #[arg(long)]
        keep_tags: Option<String>,
    },
    /// Import an existing prediction history from CSV, so day one has a report.
    Import {
        /// CSV file. Columns: statement, prob, created, resolve_by, outcome,
        /// resolved_at, tags. Only `statement` and `prob` are required.
        file: PathBuf,
        /// Parse and report what would be imported, without writing anything
        #[arg(long)]
        dry_run: bool,
    },
    /// Annul an ambiguous or unanswerable question: it keeps its place in the
    /// history but is excluded from every score, the way a forecasting platform
    /// annuls a question rather than grading it.
    Void {
        /// Claim id, or any unambiguous prefix of one
        id: String,
        /// Why this question cannot fairly be graded
        #[arg(long)]
        reason: String,
    },
    /// Fix a typo in a statement, or correct its tags. Pre-resolution only, and
    /// never the probability or the timestamps — those are the record.
    Amend {
        /// Claim id, or any unambiguous prefix of one
        id: String,
        /// Replacement statement
        #[arg(long)]
        statement: Option<String>,
        /// Replacement tags, comma-separated (replaces the whole set)
        #[arg(long)]
        tags: Option<String>,
    },
    /// Print where everything lives: both ledger paths, which environment
    /// variables are overriding them, the lock and backup files, and the version.
    Where,
}

impl Cmd {
    /// Commands that write the ledger, and therefore need the exclusive lock
    /// held across the whole load-modify-save.
    fn mutates(&self) -> bool {
        matches!(
            self,
            Cmd::Add { .. }
                | Cmd::Update { .. }
                | Cmd::Resolve { .. }
                | Cmd::Void { .. }
                | Cmd::Amend { .. }
                | Cmd::Import { .. }
        )
    }
}

/// The hook events `ana hook` understands.
#[derive(Clone, Copy, ValueEnum)]
enum HookEvent {
    SessionStart,
    UserPrompt,
    PostTool,
    Stop,
}

impl From<HookEvent> for anamnesis::hook::Event {
    fn from(e: HookEvent) -> Self {
        match e {
            HookEvent::SessionStart => anamnesis::hook::Event::SessionStart,
            HookEvent::UserPrompt => anamnesis::hook::Event::UserPrompt,
            HookEvent::PostTool => anamnesis::hook::Event::PostTool,
            HookEvent::Stop => anamnesis::hook::Event::Stop,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum OutcomeArg {
    #[value(alias = "true")]
    Yes,
    #[value(alias = "false")]
    No,
}

impl From<OutcomeArg> for Outcome {
    fn from(o: OutcomeArg) -> Self {
        match o {
            OutcomeArg::Yes => Outcome::True,
            OutcomeArg::No => Outcome::False,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// The user's home directory.
///
/// `std::env::home_dir` reads `$HOME` on Unix and `USERPROFILE` (falling back to
/// `GetUserProfileDirectory`) on Windows. It was fixed for Windows in Rust 1.85
/// and un-deprecated in 1.87, so at our 1.89 MSRV this needs no crate.
///
/// Reading `$HOME` directly, as this used to, meant a normal Windows setup found
/// nothing and silently fell back to a ledger in the *current directory* — so
/// every folder quietly got its own separate record of the user's judgement, and
/// nothing ever said so.
fn home_dir() -> Option<PathBuf> {
    #[allow(deprecated)]
    std::env::home_dir().filter(|p| !p.as_os_str().is_empty())
}

/// Where the human ledger lives: `--data`, else `$ANAMNESIS_DATA`, else
/// `~/.anamnesis.json`. An error rather than a cwd fallback when there is no home
/// directory — a wrong ledger path is worse than a refusal.
fn data_path(cli: &Cli) -> Result<PathBuf, String> {
    if let Some(p) = &cli.data {
        return Ok(p.clone());
    }
    if let Ok(p) = std::env::var("ANAMNESIS_DATA") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    match home_dir() {
        Some(h) => Ok(h.join(".anamnesis.json")),
        None => Err(
            "no home directory found, so there is nowhere to keep your ledger.\n  \
             Pass --data <FILE>, or set ANAMNESIS_DATA, to say where it should live."
                .into(),
        ),
    }
}

/// The global agent ledger driven by `ana mcp` and the Claude Code plugin — the
/// cross-project calibration spine. `ANAMNESIS_AGENT_DATA` overrides it.
fn agent_ledger_path() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("ANAMNESIS_AGENT_DATA") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    match home_dir() {
        Some(h) => Ok(h.join(".anamnesis").join("agent.json")),
        None => Err(
            "no home directory found, so there is nowhere to keep the agent ledger.\n  \
             Pass --data <FILE>, or set ANAMNESIS_AGENT_DATA, to say where it should live."
                .into(),
        ),
    }
}

/// A corrupt ledger is a data-loss emergency, not a parse error: say exactly
/// where the JSON went wrong and where the last good copy is, and never write
/// over the file.
fn load_error(path: &std::path::Path, e: std::io::Error) -> String {
    let mut msg = format!("reading {}: {e}", path.display());
    if e.kind() == std::io::ErrorKind::InvalidData {
        let bak = store::backup_path(path);
        msg.push_str("\n  the ledger was NOT modified.");
        if bak.exists() {
            msg.push_str(&format!(
                "\n  last good copy: {} (inspect it before overwriting anything)",
                bak.display()
            ));
        }
    }
    msg
}

/// `ana where` — the first thing to ask a bug reporter for.
///
/// Deliberately runs before any ledger is loaded, so it still answers when the
/// ledger is the thing that is broken.
fn cmd_where(cli: &Cli) -> Result<(), String> {
    let human = data_path(cli);
    let agent = agent_ledger_path();
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());

    if cli.json {
        let describe = |p: &Result<PathBuf, String>| match p {
            Ok(p) => json!({
                "path": p.display().to_string(),
                "exists": p.exists(),
                "lock": store::lock_path(p).display().to_string(),
                "backup": store::backup_path(p).display().to_string(),
            }),
            Err(e) => json!({ "error": e }),
        };
        out_json(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "human_ledger": describe(&human),
            "agent_ledger": describe(&agent),
            "home": home_dir().map(|h| h.display().to_string()),
            "env": {
                "ANAMNESIS_DATA": env("ANAMNESIS_DATA"),
                "ANAMNESIS_AGENT_DATA": env("ANAMNESIS_AGENT_DATA"),
            },
            "data_flag": cli.data.as_ref().map(|p| p.display().to_string()),
        }));
        return Ok(());
    }

    println!("ana {}", env!("CARGO_PKG_VERSION"));
    println!(
        "home                 {}",
        home_dir().map_or("(none found)".into(), |h| h.display().to_string())
    );
    for (label, p, var) in [
        ("human ledger", &human, "ANAMNESIS_DATA"),
        ("agent ledger", &agent, "ANAMNESIS_AGENT_DATA"),
    ] {
        match p {
            Ok(p) => {
                println!(
                    "\n{label}         {}  {}",
                    p.display(),
                    if p.exists() {
                        "(exists)"
                    } else {
                        "(not created yet)"
                    }
                );
                println!("  lock               {}", store::lock_path(p).display());
                println!("  backup             {}", store::backup_path(p).display());
            }
            Err(e) => println!("\n{label}         ERROR: {e}"),
        }
        match env(var) {
            Some(v) => println!("  overridden by      {var}={v}"),
            None => println!("  {var} is unset"),
        }
    }
    if let Some(p) = &cli.data {
        println!("\n--data {} overrides both of the above.", p.display());
    }
    Ok(())
}

/// `ana demo` — a result before we ask for a habit.
///
/// A new user's first report is empty, and a visitor decides in about two
/// minutes. This builds a fictional year of predictions in a temporary
/// directory, reports on it, and deletes it — the real ledger is never opened,
/// never mind written.
fn cmd_demo(cli: &Cli, keep: Option<&std::path::Path>, plain: bool) -> Result<(), String> {
    let ledger = anamnesis::demo::ledger();
    let today = Utc::now().date_naive();

    if cli.json {
        println!("{}", report::render_json(&ledger, None, 10, today));
    } else if plain {
        print!("{}", report::render_plain(&ledger, None, 10, today));
    } else {
        print!("{}", report::render(&ledger, None, 10, today));
    }

    match keep {
        Some(path) => {
            store::save(path, &ledger).map_err(|e| format!("writing {}: {e}", path.display()))?;
            if !cli.json {
                println!("\nDemo ledger written to {}", path.display());
                println!("Poke at it:  ana --data {} list", path.display());
            }
        }
        None if !cli.json => {
            println!("\nThis was a fictional ledger — nothing of yours was touched.");
            println!("Your own first prediction:");
            println!("  ana add \"this refactor takes under an hour\" --prob 0.7 --by {today}");
            println!("  ana resolve <id> yes|no      # the moment reality answers");
            println!("  ana report");
            println!("\nKeep a copy to explore:  ana demo --keep demo.json");
        }
        None => {}
    }
    Ok(())
}

/// `ana export --anonymize` — publish the shape of a record without its content.
///
/// Keeps every number: probabilities, intervals, outcomes, kinds, stakes, and
/// dates rounded to the day. Drops every free-text field — statements, reasoning,
/// resolution notes, void reasons and amendments — and keeps only whitelisted tag
/// namespaces. Ids are replaced with sequential ones, so nothing can be joined
/// back to the original ledger.
///
/// `--anonymize` is required rather than default-on, because an export that can
/// be run without it is an export somebody will run without it.
fn cmd_export(
    ledger: &anamnesis::model::Ledger,
    anonymize: bool,
    out: Option<&std::path::Path>,
    keep_tags: Option<&str>,
) -> Result<(), String> {
    if !anonymize {
        return Err(
            "refusing to export raw statements. Pass --anonymize; there is no other mode.".into(),
        );
    }
    let allowed: Vec<String> = keep_tags
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_lowercase())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_else(|| {
            ["kind", "who", "model", "project"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        });

    let claims: Vec<Value> = ledger
        .claims
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let tags: Vec<&String> = c
                .tags
                .iter()
                .filter(|t| {
                    t.split_once(':')
                        .is_some_and(|(ns, _)| allowed.iter().any(|a| a == ns))
                })
                .collect();
            let forecasts: Vec<Value> = c
                .forecasts
                .iter()
                .map(|f| {
                    json!({
                        "at": f.at.date_naive().to_string(),
                        "prob": f.prob,
                        "interval": f.interval,
                    })
                })
                .collect();
            json!({
                "id": format!("c{i:05}"),
                "created_at": c.created_at.date_naive().to_string(),
                "resolve_by": c.resolve_by.map(|d| d.to_string()),
                "kind": c.kind,
                "stake": c.stake,
                "tags": tags,
                "forecasts": forecasts,
                "resolution": c.resolution.as_ref().map(|r| json!({
                    "at": r.at.date_naive().to_string(),
                    "outcome": r.outcome,
                    "value": r.value,
                    "resolved_by": r.resolved_by,
                })),
                "void": c.void.is_some(),
                "amendments": c.amendments.len(),
            })
        })
        .collect();

    let doc = json!({
        "anonymized": true,
        "note": "Statements, reasoning, notes and ids are removed; dates are rounded to the day. Numbers are unchanged.",
        "claims": claims,
    });
    let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    match out {
        Some(p) => {
            std::fs::write(p, &text).map_err(|e| format!("writing {}: {e}", p.display()))?;
            eprintln!(
                "wrote {} anonymized claim(s) to {}",
                ledger.claims.len(),
                p.display()
            );
        }
        None => println!("{text}"),
    }
    Ok(())
}

/// `ana import <file.csv>` — bring an existing prediction history in, so the
/// first report has something to say.
///
/// Columns, by header name: `statement, prob, created, resolve_by, outcome,
/// resolved_at, tags`. Only `statement` and `prob` are required. Unknown columns
/// are ignored; a row that cannot be parsed is reported by line number and
/// skipped rather than silently dropped.
fn cmd_import(
    cli: &Cli,
    path: &std::path::Path,
    ledger: &mut anamnesis::model::Ledger,
    file: &std::path::Path,
    dry_run: bool,
) -> Result<(), String> {
    let text =
        std::fs::read_to_string(file).map_err(|e| format!("reading {}: {e}", file.display()))?;
    let mut rows = text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty());
    let (_, header) = rows.next().ok_or("the file is empty")?;
    let cols: Vec<String> = split_csv(header)
        .into_iter()
        .map(|c| c.trim().to_lowercase())
        .collect();
    let col = |name: &str| cols.iter().position(|c| c == name);
    let (Some(i_stmt), Some(i_prob)) = (col("statement"), col("prob")) else {
        return Err(format!(
            "the header must name at least `statement` and `prob`; found: {}",
            cols.join(", ")
        ));
    };
    let (i_created, i_by, i_outcome, i_resolved, i_tags) = (
        col("created"),
        col("resolve_by"),
        col("outcome"),
        col("resolved_at"),
        col("tags"),
    );

    let now = Utc::now();
    let mut added = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    let mut salt = now.timestamp_nanos_opt().unwrap_or(0) as u64;

    for (n, line) in rows {
        let f = split_csv(line);
        let get = |i: Option<usize>| {
            i.and_then(|i| f.get(i))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
        };
        let lineno = n + 1;

        let Some(statement) = f.get(i_stmt).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
            skipped.push(format!("line {lineno}: empty statement"));
            continue;
        };
        let prob: f64 = match f.get(i_prob).map(|s| s.trim()).unwrap_or("").parse() {
            Ok(p) => p,
            Err(_) => {
                skipped.push(format!("line {lineno}: unreadable prob"));
                continue;
            }
        };
        if !(0.0..=1.0).contains(&prob) {
            skipped.push(format!("line {lineno}: prob {prob} is not between 0 and 1"));
            continue;
        }
        let created = match get(i_created) {
            Some(d) => match parse_date(d) {
                Ok(d) => d
                    .and_hms_opt(12, 0, 0)
                    .map(|dt| dt.and_utc())
                    .unwrap_or(now),
                Err(e) => {
                    skipped.push(format!("line {lineno}: {e}"));
                    continue;
                }
            },
            None => now,
        };
        let resolve_by = match get(i_by) {
            Some(d) => match parse_date(d) {
                Ok(d) => Some(d),
                Err(e) => {
                    skipped.push(format!("line {lineno}: {e}"));
                    continue;
                }
            },
            None => None,
        };
        let outcome = match get(i_outcome).map(|o| o.to_lowercase()) {
            None => None,
            Some(o) => match o.as_str() {
                "yes" | "true" | "1" | "y" => Some(Outcome::True),
                "no" | "false" | "0" | "n" => Some(Outcome::False),
                other => {
                    skipped.push(format!("line {lineno}: unreadable outcome '{other}'"));
                    continue;
                }
            },
        };
        let resolved_at = get(i_resolved)
            .and_then(|d| parse_date(d).ok())
            .and_then(|d| d.and_hms_opt(12, 0, 0))
            .map(|dt| dt.and_utc());
        let tags = get(i_tags)
            .map(|t| {
                normalize_tags(
                    &t.split([',', ';', ' '])
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap_or_default();

        salt = salt.wrapping_add(1);
        let mut id = gen_id(statement, salt);
        while ledger.has_id(&id) {
            salt = salt.wrapping_add(1);
            id = gen_id(statement, salt);
        }
        ledger.claims.push(Claim {
            id,
            statement: statement.to_string(),
            created_at: created,
            resolve_by,
            tags,
            kind: ClaimKind::Binary,
            stake: 1.0,
            forecasts: vec![Forecast {
                at: created,
                prob: Some(prob),
                interval: None,
                because: None,
            }],
            resolution: outcome.map(|o| Resolution {
                // A resolution with no date falls on the deadline if there is
                // one, so imported claims land in the evidence sequence in a
                // sensible place rather than all at "now".
                at: resolved_at
                    .or_else(|| {
                        resolve_by
                            .and_then(|d| d.and_hms_opt(12, 0, 0))
                            .map(|d| d.and_utc())
                    })
                    .unwrap_or(now),
                outcome: Some(o),
                value: None,
                note: None,
                resolved_by: None,
            }),
            void: None,
            amendments: Vec::new(),
        });
        added += 1;
    }

    if !dry_run {
        store::save(path, ledger).map_err(|e| format!("saving: {e}"))?;
    }

    if cli.json {
        out_json(json!({
            "imported": added,
            "skipped": skipped.len(),
            "problems": skipped,
            "dry_run": dry_run,
            "ledger": path.display().to_string(),
        }));
    } else {
        let verb = if dry_run { "would import" } else { "imported" };
        println!("{verb} {added} claim(s) from {}", file.display());
        if !skipped.is_empty() {
            println!("skipped {}:", skipped.len());
            for s in skipped.iter().take(10) {
                println!("  {s}");
            }
            if skipped.len() > 10 {
                println!("  … and {} more", skipped.len() - 10);
            }
        }
        if dry_run {
            println!("(--dry-run: nothing was written)");
        } else {
            println!("Now run:  ana report");
        }
    }
    Ok(())
}

/// Split one CSV line, honouring double-quoted fields and `""` escapes. Enough
/// for the documented column set; this is an importer, not a CSV library, and a
/// dependency for one function would not earn its place.
fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn check_prob(p: f64) -> Result<(), String> {
    if (0.0..=1.0).contains(&p) {
        Ok(())
    } else {
        Err(format!("probability must be between 0 and 1, got {p}"))
    }
}

fn check_level(l: f64) -> Result<(), String> {
    if l > 0.0 && l < 1.0 {
        Ok(())
    } else {
        Err(format!("level must be strictly between 0 and 1, got {l}"))
    }
}

fn parse_interval(s: &str) -> Result<(f64, f64), String> {
    let (lo, hi) = s
        .split_once("..")
        .ok_or_else(|| format!("interval must look like LOW..HIGH, got '{s}'"))?;
    let lo: f64 = lo
        .trim()
        .parse()
        .map_err(|_| format!("bad interval low '{lo}'"))?;
    let hi: f64 = hi
        .trim()
        .parse()
        .map_err(|_| format!("bad interval high '{hi}'"))?;
    if !lo.is_finite() || !hi.is_finite() {
        return Err("interval bounds must be finite".into());
    }
    if lo > hi {
        return Err(format!("interval low must be ≤ high ({lo} > {hi})"));
    }
    Ok((lo, hi))
}

fn parse_date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| format!("could not parse date '{s}' (expected YYYY-MM-DD)"))
}

fn pct(p: f64) -> String {
    format!("{:.0}%", p * 100.0)
}

fn out_json(v: Value) {
    println!("{}", serde_json::to_string_pretty(&v).unwrap());
}

fn run(cli: Cli) -> Result<(), String> {
    // Dispatched before any ledger load: `ana mcp` serves the AGENT ledger, so a
    // corrupt or unreadable human ledger must not be able to break it.
    if matches!(cli.cmd, Cmd::Mcp) {
        let ledger_path = match cli.data.clone() {
            Some(p) => p,
            None => agent_ledger_path()?,
        };
        return mcp::serve(ledger_path).map_err(|e| format!("mcp: {e}"));
    }

    if matches!(cli.cmd, Cmd::Where) {
        return cmd_where(&cli);
    }
    // Hooks read the AGENT ledger, like `mcp`, and must not be broken by a
    // corrupt human one.
    if let Cmd::Hook { event } = &cli.cmd {
        let ledger_path = match cli.data.clone() {
            Some(p) => p,
            None => agent_ledger_path()?,
        };
        return anamnesis::hook::run((*event).into(), &ledger_path);
    }
    // `demo` must never be able to touch a real ledger, so it is dispatched
    // before one is even resolved.
    if let Cmd::Demo { keep, plain } = &cli.cmd {
        return cmd_demo(&cli, keep.as_deref(), *plain);
    }

    let path = data_path(&cli)?;

    // Lock BEFORE loading, and hold it for the whole function, so concurrent
    // writers serialise instead of each overwriting the other's claim.
    let _guard = if cli.cmd.mutates() {
        Some(store::lock(&path).map_err(|e| format!("locking {}: {e}", path.display()))?)
    } else {
        None
    };

    let mut ledger = store::load(&path).map_err(|e| load_error(&path, e))?;

    match &cli.cmd {
        Cmd::Add {
            statement,
            prob,
            interval,
            level,
            by,
            tags,
            because,
            stake,
            second_prob,
            reference_class,
        } => {
            let statement = statement.trim().to_string();
            if statement.is_empty() {
                return Err("statement must not be empty".into());
            }
            let stake = *stake;
            if !(stake.is_finite() && stake >= 0.0) {
                return Err(format!("stake must be a finite number ≥ 0, got {stake}"));
            }
            let resolve_by = by.as_deref().map(parse_date).transpose()?;
            let now = Utc::now();

            let (kind, forecast) = match (prob, interval) {
                (Some(_), Some(_)) => {
                    return Err(
                        "give either --prob (binary) or --interval (numeric), not both".into(),
                    )
                }
                (Some(p), None) => {
                    check_prob(*p)?;
                    // Dialectical bootstrapping: if a second "consider the opposite"
                    // estimate is given, log the average of the two.
                    let (p_eff, estimates) = match second_prob {
                        Some(sp) => {
                            check_prob(*sp)?;
                            (scoring::dialectical_mean(*p, *sp), Some((*p, *sp)))
                        }
                        None => (*p, None),
                    };
                    (
                        ClaimKind::Binary,
                        Forecast {
                            at: now,
                            prob: Some(p_eff),
                            interval: None,
                            because: compose_reasoning(
                                because.as_deref(),
                                reference_class.as_deref(),
                                estimates,
                            ),
                        },
                    )
                }
                (None, Some(iv)) => {
                    check_level(*level)?;
                    let (low, high) = parse_interval(iv)?;
                    (
                        ClaimKind::Numeric,
                        Forecast {
                            at: now,
                            prob: None,
                            interval: Some(NumericForecast {
                                low,
                                high,
                                level: *level,
                            }),
                            because: compose_reasoning(
                                because.as_deref(),
                                reference_class.as_deref(),
                                None,
                            ),
                        },
                    )
                }
                (None, None) => {
                    return Err("give --prob 0.7 (binary) or --interval 120..180 (numeric)".into())
                }
            };

            let mut salt = now.timestamp_nanos_opt().unwrap_or(0) as u64;
            let id = loop {
                let candidate = gen_id(&statement, salt);
                if !ledger.has_id(&candidate) {
                    break candidate;
                }
                salt = salt.wrapping_add(1);
            };

            let disp_prob = forecast.prob;
            let disp_iv = forecast.interval;
            ledger.claims.push(Claim {
                id: id.clone(),
                statement: statement.clone(),
                created_at: now,
                resolve_by,
                tags: normalize_tags(tags),
                kind,
                stake,
                forecasts: vec![forecast],
                resolution: None,
                void: None,
                amendments: Vec::new(),
            });
            store::save(&path, &ledger).map_err(|e| format!("saving: {e}"))?;

            if cli.json {
                out_json(
                    json!({"id": id, "kind": kind, "prob": disp_prob, "interval": disp_iv, "statement": statement}),
                );
            } else {
                match kind {
                    ClaimKind::Binary => {
                        println!("added [{id}]  {}  \"{statement}\"", pct(disp_prob.unwrap()))
                    }
                    ClaimKind::Numeric => {
                        let iv = disp_iv.unwrap();
                        println!(
                            "added [{id}]  {:.0}% interval [{}, {}]  \"{statement}\"",
                            iv.level * 100.0,
                            iv.low,
                            iv.high
                        )
                    }
                }
            }
        }

        Cmd::Update {
            id,
            prob,
            interval,
            level,
            because,
        } => {
            let idx = ledger.index_of(id)?;
            if ledger.claims[idx].is_resolved() {
                return Err(format!(
                    "[{}] is already resolved; its history is final",
                    ledger.claims[idx].id
                ));
            }
            let kind = ledger.claims[idx].kind;
            let now = Utc::now();

            let (forecast, human) = match kind {
                ClaimKind::Binary => {
                    let p = prob.ok_or("this is a binary claim; revise it with --prob 0..1")?;
                    if interval.is_some() {
                        return Err("this is a binary claim; use --prob, not --interval".into());
                    }
                    check_prob(p)?;
                    let old = ledger.claims[idx].current_prob().unwrap_or(p);
                    (
                        Forecast {
                            at: now,
                            prob: Some(p),
                            interval: None,
                            because: because.clone(),
                        },
                        format!("{} → {}", pct(old), pct(p)),
                    )
                }
                ClaimKind::Numeric => {
                    let iv = interval
                        .as_deref()
                        .ok_or("this is a numeric claim; revise it with --interval LOW..HIGH")?;
                    if prob.is_some() {
                        return Err("this is a numeric claim; use --interval, not --prob".into());
                    }
                    let (low, high) = parse_interval(iv)?;
                    let lvl = level
                        .or_else(|| ledger.claims[idx].current_interval().map(|i| i.level))
                        .unwrap_or(0.80);
                    check_level(lvl)?;
                    (
                        Forecast {
                            at: now,
                            prob: None,
                            interval: Some(NumericForecast {
                                low,
                                high,
                                level: lvl,
                            }),
                            because: because.clone(),
                        },
                        format!("[{low}, {high}] @ {:.0}%", lvl * 100.0),
                    )
                }
            };

            ledger.claims[idx].forecasts.push(forecast);
            let cid = ledger.claims[idx].id.clone();
            let rev = ledger.claims[idx].forecasts.len();
            store::save(&path, &ledger).map_err(|e| format!("saving: {e}"))?;

            if cli.json {
                out_json(json!({"id": cid, "kind": kind, "revision": rev}));
            } else {
                println!("[{cid}]  {human}  (revision #{rev})");
            }
        }

        Cmd::Resolve {
            id,
            outcome,
            value,
            note,
        } => {
            let idx = ledger.index_of(id)?;
            if ledger.claims[idx].is_resolved() {
                return Err(format!("[{}] is already resolved", ledger.claims[idx].id));
            }
            let kind = ledger.claims[idx].kind;
            let now = Utc::now();
            let cid = ledger.claims[idx].id.clone();

            match kind {
                ClaimKind::Binary => {
                    if value.is_some() {
                        return Err(
                            "this is a binary claim; resolve it with yes/no, not --value".into(),
                        );
                    }
                    let o: Outcome = (*outcome
                        .as_ref()
                        .ok_or("resolve a binary claim with yes or no")?)
                    .into();
                    let prob = ledger.claims[idx].current_prob().unwrap_or(0.5);
                    ledger.claims[idx].resolution = Some(Resolution {
                        at: now,
                        outcome: Some(o),
                        value: None,
                        note: note.clone(),
                        resolved_by: None,
                    });
                    store::save(&path, &ledger).map_err(|e| format!("saving: {e}"))?;
                    let brier = (prob - if o.happened() { 1.0 } else { 0.0 }).powi(2);
                    if cli.json {
                        out_json(
                            json!({"id": cid, "kind": kind, "outcome": o.happened(), "prob": prob, "brier": brier}),
                        );
                    } else {
                        let truth = if o.happened() { "TRUE" } else { "FALSE" };
                        println!(
                            "[{cid}] resolved {truth}.  you said {}  →  Brier {:.3} on this one",
                            pct(prob),
                            brier
                        );
                    }
                }
                ClaimKind::Numeric => {
                    if outcome.is_some() {
                        return Err("this is a numeric claim; resolve it with --value N".into());
                    }
                    let v = value.ok_or("resolve a numeric claim with --value N")?;
                    let iv = ledger.claims[idx]
                        .current_interval()
                        .ok_or("numeric claim has no interval forecast")?;
                    ledger.claims[idx].resolution = Some(Resolution {
                        at: now,
                        outcome: None,
                        value: Some(v),
                        note: note.clone(),
                        resolved_by: None,
                    });
                    store::save(&path, &ledger).map_err(|e| format!("saving: {e}"))?;
                    let ns = NumericSample {
                        low: iv.low,
                        high: iv.high,
                        level: iv.level,
                        value: v,
                    };
                    let w = scoring::winkler(&ns);
                    let caught = ns.contains();
                    if cli.json {
                        out_json(
                            json!({"id": cid, "kind": kind, "value": v, "interval": iv, "inside": caught, "winkler": w}),
                        );
                    } else {
                        let verdict = if caught { "caught it" } else { "MISSED" };
                        println!(
                            "[{cid}] resolved value {v}.  interval [{}, {}] {verdict}  →  Winkler {:.3}",
                            iv.low, iv.high, w
                        );
                    }
                }
            }
        }

        Cmd::Void { id, reason } => {
            let reason = reason.trim().to_string();
            if reason.is_empty() {
                return Err("a void needs a reason — it is the whole record of why".into());
            }
            let idx = ledger.index_of(id)?;
            let c = &mut ledger.claims[idx];
            if c.is_void() {
                return Err(format!("[{}] is already void", c.id));
            }
            let cid = c.id.clone();
            let was_resolved = c.is_resolved();
            c.void = Some(anamnesis::model::Void {
                at: Utc::now(),
                reason: reason.clone(),
            });
            store::save(&path, &ledger).map_err(|e| format!("saving: {e}"))?;
            if cli.json {
                out_json(json!({"id": cid, "void": true, "reason": reason}));
            } else {
                println!("[{cid}] voided — excluded from every score, kept in history.");
                println!("  reason: {reason}");
                if was_resolved {
                    println!("  (it was resolved; that resolution stays on the record but no longer counts)");
                }
            }
        }

        Cmd::Amend {
            id,
            statement,
            tags,
        } => {
            if statement.is_none() && tags.is_none() {
                return Err("nothing to amend — pass --statement and/or --tags".into());
            }
            let idx = ledger.index_of(id)?;
            let c = &mut ledger.claims[idx];
            // Amending after the answer is known would let the record be rewritten
            // to match the outcome, which is the failure this whole tool exists to
            // prevent.
            if c.is_resolved() {
                return Err(format!(
                    "[{}] is already resolved — a resolved claim is the record, and the record does not change",
                    c.id
                ));
            }
            let cid = c.id.clone();
            let mut am = anamnesis::model::Amendment {
                at: Utc::now(),
                old_statement: None,
                new_statement: None,
                old_tags: None,
                new_tags: None,
            };
            if let Some(new) = statement {
                let new = new.trim().to_string();
                if new.is_empty() {
                    return Err("statement must not be empty".into());
                }
                am.old_statement = Some(c.statement.clone());
                am.new_statement = Some(new.clone());
                c.statement = new;
            }
            if let Some(new) = tags {
                let parsed: Vec<String> = new
                    .split(',')
                    .map(|t| t.trim().to_lowercase())
                    .filter(|t| !t.is_empty())
                    .collect();
                am.old_tags = Some(c.tags.clone());
                am.new_tags = Some(parsed.clone());
                c.tags = parsed;
            }
            c.amendments.push(am);
            let statement = ledger.claims[idx].statement.clone();
            let tags = ledger.claims[idx].tags.clone();
            store::save(&path, &ledger).map_err(|e| format!("saving: {e}"))?;
            if cli.json {
                out_json(json!({"id": cid, "statement": statement, "tags": tags}));
            } else {
                println!("[{cid}] amended — \"{statement}\"");
                if !tags.is_empty() {
                    println!("  tags: {}", tags.join(", "));
                }
                println!("  the previous wording is kept in the claim's history.");
            }
        }

        Cmd::List {
            open,
            resolved,
            due,
            tag,
            void,
        } => {
            let today = Utc::now().date_naive();
            let tagf = tag.as_ref().map(|t| t.to_lowercase());
            let keep = |c: &Claim| {
                if let Some(t) = &tagf {
                    if !c.tags.iter().any(|x| x == t) {
                        return false;
                    }
                }
                // Voided claims are shown only when asked for: they are history,
                // not open work, and they are not part of any score.
                if *void {
                    return c.is_void();
                }
                if c.is_void() {
                    return false;
                }
                if *due {
                    c.is_due(today)
                } else if *open {
                    c.is_open()
                } else if *resolved {
                    c.is_resolved()
                } else {
                    true
                }
            };

            if cli.json {
                let arr: Vec<Value> = ledger
                    .claims
                    .iter()
                    .filter(|c| keep(c))
                    .map(|c| {
                        json!({
                            "id": c.id,
                            "kind": c.kind,
                            "statement": c.statement,
                            "prob": c.current_prob(),
                            "interval": c.current_interval(),
                            "tags": c.tags,
                            "resolve_by": c.resolve_by,
                            "resolved": c.is_resolved(),
                            "outcome": c.outcome().map(|o| o.happened()),
                            "value": c.value(),
                        })
                    })
                    .collect();
                out_json(json!(arr));
                return Ok(());
            }

            let mut shown = 0;
            for c in ledger.claims.iter().filter(|c| keep(c)) {
                shown += 1;
                let belief = match c.kind {
                    ClaimKind::Binary => c.current_prob().map(pct).unwrap_or_else(|| "  ?".into()),
                    ClaimKind::Numeric => c
                        .current_interval()
                        .map(|i| format!("[{},{}]", i.low, i.high))
                        .unwrap_or_else(|| "  ?".into()),
                };
                let status = match c.kind {
                    ClaimKind::Binary => match c.outcome() {
                        Some(Outcome::True) => "✓ true".to_string(),
                        Some(Outcome::False) => "✗ false".to_string(),
                        None => open_status(c, today),
                    },
                    ClaimKind::Numeric => match c.value() {
                        Some(v) => format!("= {v}"),
                        None => open_status(c, today),
                    },
                };
                let tags = if c.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", c.tags.join(","))
                };
                println!(
                    "{:<6}  {:>9}  {:<11}  {}{tags}",
                    c.id,
                    belief,
                    status,
                    truncate(&c.statement, 52)
                );
            }
            if shown == 0 {
                println!("(no matching claims)");
            }
        }

        Cmd::Show { id } => {
            let idx = ledger.index_of(id)?;
            let c = &ledger.claims[idx];
            if cli.json {
                out_json(serde_json::to_value(c).map_err(|e| e.to_string())?);
                return Ok(());
            }
            println!("[{}]  {}", c.id, c.statement);
            println!(
                "  kind: {}",
                match c.kind {
                    ClaimKind::Binary => "binary",
                    ClaimKind::Numeric => "numeric",
                }
            );
            println!("  created {}", c.created_at.date_naive());
            if let Some(d) = c.resolve_by {
                println!("  resolve by {d}");
            }
            if !c.tags.is_empty() {
                println!("  tags: {}", c.tags.join(", "));
            }
            if let Some(v) = &c.void {
                println!(
                    "  VOID since {} — excluded from every score",
                    v.at.date_naive()
                );
                println!("    reason: {}", v.reason);
            }
            for a in &c.amendments {
                if let (Some(o), Some(n)) = (&a.old_statement, &a.new_statement) {
                    println!("  amended {}: \"{o}\" → \"{n}\"", a.at.date_naive());
                }
                if let (Some(o), Some(n)) = (&a.old_tags, &a.new_tags) {
                    println!(
                        "  amended {}: tags [{}] → [{}]",
                        a.at.date_naive(),
                        o.join(", "),
                        n.join(", ")
                    );
                }
            }
            println!("  forecasts:");
            for (i, f) in c.forecasts.iter().enumerate() {
                let marker = if i + 1 == c.forecasts.len() {
                    "→"
                } else {
                    " "
                };
                let belief = match (f.prob, f.interval) {
                    (Some(p), _) => pct(p),
                    (_, Some(iv)) => {
                        format!("[{}, {}] @ {:.0}%", iv.low, iv.high, iv.level * 100.0)
                    }
                    _ => "?".into(),
                };
                println!("    {marker} {}  {}", f.at.date_naive(), belief);
                if let Some(b) = &f.because {
                    println!("        because: {b}");
                }
            }
            match &c.resolution {
                Some(r) => {
                    match c.kind {
                        ClaimKind::Binary => {
                            let truth = if c.outcome().map(|o| o.happened()).unwrap_or(false) {
                                "TRUE"
                            } else {
                                "FALSE"
                            };
                            let how = match r.resolved_by {
                                Some(anamnesis::model::ResolvedBy::Auto) => {
                                    "  (graded automatically, not self-reported)"
                                }
                                Some(anamnesis::model::ResolvedBy::Human) => {
                                    "  (graded by a human)"
                                }
                                _ => "",
                            };
                            println!("  resolved {truth} on {}{how}", r.at.date_naive());
                            if let Some(s) = c.sample() {
                                println!(
                                    "    Brier on your FIRST forecast: {:.3}",
                                    (s.prob - s.outcome).powi(2)
                                );
                                if let Some(f) = c.sample_final() {
                                    if (f.prob - s.prob).abs() > 1e-9 {
                                        println!(
                                            "    (final forecast {:.2} would score {:.3} — shown, not graded)",
                                            f.prob,
                                            (f.prob - f.outcome).powi(2)
                                        );
                                    }
                                }
                            }
                        }
                        ClaimKind::Numeric => {
                            if let Some(ns) = c.numeric_sample() {
                                let verdict = if ns.contains() { "inside" } else { "OUTSIDE" };
                                println!(
                                    "  resolved value {} on {} ({verdict} the interval)",
                                    ns.value,
                                    r.at.date_naive()
                                );
                                println!(
                                    "    Winkler on final interval: {:.3}",
                                    scoring::winkler(&ns)
                                );
                            }
                        }
                    }
                    if let Some(n) = &r.note {
                        println!("    post-mortem: {n}");
                    }
                }
                None => println!("  (open — reality has not yet spoken)"),
            }
        }

        Cmd::Report {
            tag,
            bins,
            plain,
            html,
            badge,
        } => {
            let today = Utc::now().date_naive();
            let tag = tag.as_deref();
            if cli.json {
                println!("{}", report::render_json(&ledger, tag, *bins, today));
            } else if *html {
                print!("{}", report::render_html(&ledger, tag, *bins, today));
            } else if *badge {
                print!("{}", report::render_badge_svg(&ledger, tag, *bins, today));
            } else if *plain {
                print!("{}", report::render_plain(&ledger, tag, *bins, today));
            } else {
                print!("{}", report::render(&ledger, tag, *bins, today));
            }
        }

        Cmd::Decide {
            prob,
            stake,
            verify_cost,
            tag,
        } => {
            let p = *prob;
            if !(0.0..=1.0).contains(&p) {
                return Err(format!("prob must be between 0 and 1, got {p}"));
            }
            // Correct the number through the earned map (if any), then threshold by
            // the stakes — the same evidence gate as the report and the MCP tools.
            let (recal, earned, n, e) =
                report::earned_recalibration(&ledger, tag.as_deref(), Utc::now().date_naive());
            let map = if earned { recal } else { None };
            let d = scoring::decide(p, map, *stake, *verify_cost);
            let verb = match d.act {
                scoring::Act::Proceed => "PROCEED",
                scoring::Act::Verify => "VERIFY",
                scoring::Act::Abstain => "ABSTAIN",
            };
            if cli.json {
                println!(
                    "{}",
                    json!({
                        "act": verb.to_lowercase(),
                        "stated": p,
                        "adjusted": d.adjusted_p,
                        "proceed_threshold": d.proceed_threshold,
                        "margin": d.margin,
                        "stake": *stake,
                        "verify_cost": *verify_cost,
                        "used_recalibration": earned,
                        "n": n,
                        "eprocess": e,
                    })
                );
            } else {
                let gloss = match d.act {
                    scoring::Act::Proceed => "confidence clears the bar for the stakes",
                    scoring::Act::Verify => "in the doubt zone — verify before you commit",
                    scoring::Act::Abstain => {
                        "more likely to fail than succeed — replan or escalate"
                    }
                };
                let corr = if earned {
                    format!(
                        "  (corrected {:.0}%→{:.0}% from {n} resolved calls)",
                        p * 100.0,
                        d.adjusted_p * 100.0
                    )
                } else {
                    String::new()
                };
                println!("{verb}  —  {gloss}");
                println!(
                    "  need ≥{:.0}% to proceed at stake {:.1}; you have {:.0}%{corr}",
                    d.proceed_threshold * 100.0,
                    *stake,
                    d.adjusted_p * 100.0
                );
            }
        }

        Cmd::Import { file, dry_run } => cmd_import(&cli, &path, &mut ledger, file, *dry_run)?,

        Cmd::Export {
            anonymize,
            out,
            keep_tags,
        } => cmd_export(&ledger, *anonymize, out.as_deref(), keep_tags.as_deref())?,

        // All handled at the top of run(), before any ledger load.
        Cmd::Mcp | Cmd::Where | Cmd::Demo { .. } | Cmd::Hook { .. } => {
            unreachable!("dispatched before the ledger load")
        }
    }
    Ok(())
}

fn open_status(c: &Claim, today: NaiveDate) -> String {
    match c.resolve_by {
        Some(d) if d <= today => format!("● due {d}"),
        Some(d) => format!("○ by {d}"),
        None => "○ open".to_string(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
