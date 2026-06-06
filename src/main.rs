//! `ana` — the command-line face of Anamnesis.
//!
//! A deliberately small surface: record a belief, revise it, resolve it, and
//! then look in the mirror. No accounts, no network, no model in the loop —
//! just you, a text file you own, and arithmetic that cannot flatter you.

use std::path::PathBuf;
use std::process::ExitCode;

use chrono::{NaiveDate, Utc};
use clap::{Parser, Subcommand, ValueEnum};

use anamnesis::model::{gen_id, normalize_tags, Claim, Forecast, Outcome, Resolution};
use anamnesis::{report, store};

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

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Record a new belief with a probability and your reasoning
    Add {
        /// The falsifiable statement, e.g. "Bitcoin closes above $200k in 2026"
        statement: String,
        /// Your probability it is TRUE, from 0 to 1 (e.g. 0.7)
        #[arg(short, long)]
        prob: f64,
        /// Date you expect to know the answer (YYYY-MM-DD)
        #[arg(long, value_name = "YYYY-MM-DD")]
        by: Option<String>,
        /// Comma-separated tags, e.g. markets,crypto
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        /// The reasoning behind the number — what hindsight will try to erase
        #[arg(long)]
        because: Option<String>,
    },
    /// Revise your probability for an existing belief (history is preserved)
    Update {
        /// Claim id (a unique prefix is enough)
        id: String,
        /// Your new probability, 0 to 1
        #[arg(short, long)]
        prob: f64,
        /// Why you are updating
        #[arg(long)]
        because: Option<String>,
    },
    /// Resolve a belief: record what actually happened, plus a post-mortem
    Resolve {
        /// Claim id (a unique prefix is enough)
        id: String,
        /// What happened: yes/true or no/false
        #[arg(value_enum)]
        outcome: OutcomeArg,
        /// A post-mortem: with hindsight, what do you think you missed?
        #[arg(long)]
        note: Option<String>,
    },
    /// List beliefs (default: all)
    List {
        /// Only still-open beliefs
        #[arg(long)]
        open: bool,
        /// Only resolved beliefs
        #[arg(long)]
        resolved: bool,
        /// Only open beliefs whose expected-by date has passed
        #[arg(long)]
        due: bool,
    },
    /// Show the full history of one belief — the palimpsest of your mind
    Show {
        /// Claim id (a unique prefix is enough)
        id: String,
    },
    /// The calibration report: the real shape of your judgement
    Report {
        /// Restrict to one tag
        #[arg(long)]
        tag: Option<String>,
        /// Number of bins for the reliability diagram
        #[arg(long, default_value_t = 10)]
        bins: usize,
    },
}

#[derive(Clone, ValueEnum)]
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

fn data_path(cli: &Cli) -> PathBuf {
    if let Some(p) = &cli.data {
        return p.clone();
    }
    if let Ok(p) = std::env::var("ANAMNESIS_DATA") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".anamnesis.json");
    }
    PathBuf::from("anamnesis.json")
}

fn check_prob(p: f64) -> Result<(), String> {
    if (0.0..=1.0).contains(&p) {
        Ok(())
    } else {
        Err(format!("probability must be between 0 and 1, got {p}"))
    }
}

fn parse_date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| format!("could not parse date '{s}' (expected YYYY-MM-DD)"))
}

fn pct(p: f64) -> String {
    format!("{:.0}%", p * 100.0)
}

fn run(cli: Cli) -> Result<(), String> {
    let path = data_path(&cli);
    let mut ledger = store::load(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;

    match cli.cmd {
        Cmd::Add {
            statement,
            prob,
            by,
            tags,
            because,
        } => {
            check_prob(prob)?;
            let statement = statement.trim().to_string();
            if statement.is_empty() {
                return Err("statement must not be empty".into());
            }
            let resolve_by = by.as_deref().map(parse_date).transpose()?;
            let now = Utc::now();

            // Find an unused id by bumping the salt.
            let mut salt = now.timestamp_nanos_opt().unwrap_or(0) as u64;
            let id = loop {
                let candidate = gen_id(&statement, salt);
                if !ledger.has_id(&candidate) {
                    break candidate;
                }
                salt = salt.wrapping_add(1);
            };

            ledger.claims.push(Claim {
                id: id.clone(),
                statement: statement.clone(),
                created_at: now,
                resolve_by,
                tags: normalize_tags(&tags),
                forecasts: vec![Forecast {
                    at: now,
                    prob,
                    because,
                }],
                resolution: None,
            });
            store::save(&path, &ledger).map_err(|e| format!("saving: {e}"))?;
            println!("added [{id}]  {}  \"{statement}\"", pct(prob));
        }

        Cmd::Update {
            id,
            prob,
            because,
        } => {
            check_prob(prob)?;
            let idx = ledger.index_of(&id)?;
            let claim = &mut ledger.claims[idx];
            if claim.is_resolved() {
                return Err(format!(
                    "[{}] is already resolved; its history is final",
                    claim.id
                ));
            }
            let old = claim.current_prob().unwrap_or(prob);
            claim.forecasts.push(Forecast {
                at: Utc::now(),
                prob,
                because,
            });
            let cid = claim.id.clone();
            store::save(&path, &ledger).map_err(|e| format!("saving: {e}"))?;
            println!("[{cid}]  {} → {}  (revision #{})", pct(old), pct(prob), ledger.claims[idx].forecasts.len());
        }

        Cmd::Resolve { id, outcome, note } => {
            let idx = ledger.index_of(&id)?;
            let claim = &mut ledger.claims[idx];
            if claim.is_resolved() {
                return Err(format!("[{}] is already resolved", claim.id));
            }
            let outcome: Outcome = outcome.into();
            let prob = claim.current_prob().unwrap_or(0.5);
            claim.resolution = Some(Resolution {
                at: Utc::now(),
                outcome,
                note,
            });
            let cid = claim.id.clone();
            let brier = (prob - if outcome.happened() { 1.0 } else { 0.0 }).powi(2);
            store::save(&path, &ledger).map_err(|e| format!("saving: {e}"))?;
            let truth = if outcome.happened() { "TRUE" } else { "FALSE" };
            println!(
                "[{cid}] resolved {truth}.  you said {}  →  Brier {:.3} on this one",
                pct(prob),
                brier
            );
        }

        Cmd::List {
            open,
            resolved,
            due,
        } => {
            let today = Utc::now().date_naive();
            let mut shown = 0;
            for c in &ledger.claims {
                let keep = if due {
                    c.is_due(today)
                } else if open {
                    c.is_open()
                } else if resolved {
                    c.is_resolved()
                } else {
                    true
                };
                if !keep {
                    continue;
                }
                shown += 1;
                let prob = c.current_prob().map(pct).unwrap_or_else(|| "  ?".into());
                let status = match c.outcome() {
                    Some(Outcome::True) => "✓ true".to_string(),
                    Some(Outcome::False) => "✗ false".to_string(),
                    None => match c.resolve_by {
                        Some(d) if d <= today => format!("● due {d}"),
                        Some(d) => format!("○ by {d}"),
                        None => "○ open".to_string(),
                    },
                };
                let tags = if c.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", c.tags.join(","))
                };
                let stmt = truncate(&c.statement, 56);
                println!("{:<6}  {:>4}  {:<11}  {stmt}{tags}", c.id, prob, status);
            }
            if shown == 0 {
                println!("(no matching claims)");
            }
        }

        Cmd::Show { id } => {
            let idx = ledger.index_of(&id)?;
            let c = &ledger.claims[idx];
            println!("[{}]  {}", c.id, c.statement);
            println!("  created {}", c.created_at.date_naive());
            if let Some(d) = c.resolve_by {
                println!("  resolve by {d}");
            }
            if !c.tags.is_empty() {
                println!("  tags: {}", c.tags.join(", "));
            }
            println!("  forecasts:");
            for (i, f) in c.forecasts.iter().enumerate() {
                let marker = if i + 1 == c.forecasts.len() { "→" } else { " " };
                println!("    {marker} {}  {}", f.at.date_naive(), pct(f.prob));
                if let Some(b) = &f.because {
                    println!("        because: {b}");
                }
            }
            match &c.resolution {
                Some(r) => {
                    let truth = if r.outcome.happened() { "TRUE" } else { "FALSE" };
                    println!("  resolved {truth} on {}", r.at.date_naive());
                    if let Some(n) = &r.note {
                        println!("    post-mortem: {n}");
                    }
                    if let Some(s) = c.sample() {
                        println!("    Brier on final forecast: {:.3}", (s.prob - s.outcome).powi(2));
                    }
                }
                None => println!("  (open — reality has not yet spoken)"),
            }
        }

        Cmd::Report { tag, bins } => {
            print!("{}", report::render(&ledger, tag.as_deref(), bins));
        }
    }
    Ok(())
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
