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
    },
    /// Show the full history of one belief — the palimpsest of your mind
    Show { id: String },
    /// The calibration report: the real shape of your judgement
    Report {
        #[arg(long)]
        tag: Option<String>,
        #[arg(long, default_value_t = 10)]
        bins: usize,
    },
    /// Serve as a Model Context Protocol server over stdio — exposes
    /// predict/resolve/calibration/list as tools for any MCP-capable agent.
    Mcp,
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

/// The global agent ledger driven by `ana mcp` and the Claude Code plugin — the
/// cross-project calibration spine. `ANAMNESIS_AGENT_DATA` overrides it.
fn agent_ledger_path() -> PathBuf {
    if let Ok(p) = std::env::var("ANAMNESIS_AGENT_DATA") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".anamnesis").join("agent.json");
    }
    PathBuf::from("agent.json")
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
    let path = data_path(&cli);
    let mut ledger = store::load(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;

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

        Cmd::List {
            open,
            resolved,
            due,
            tag,
        } => {
            let today = Utc::now().date_naive();
            let tagf = tag.as_ref().map(|t| t.to_lowercase());
            let keep = |c: &Claim| {
                if let Some(t) = &tagf {
                    if !c.tags.iter().any(|x| x == t) {
                        return false;
                    }
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
                            println!("  resolved {truth} on {}", r.at.date_naive());
                            if let Some(s) = c.sample() {
                                println!(
                                    "    Brier on final forecast: {:.3}",
                                    (s.prob - s.outcome).powi(2)
                                );
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

        Cmd::Report { tag, bins } => {
            if cli.json {
                println!("{}", report::render_json(&ledger, tag.as_deref(), *bins));
            } else {
                print!("{}", report::render(&ledger, tag.as_deref(), *bins));
            }
        }

        Cmd::Mcp => {
            // The MCP surface defaults to the global AGENT ledger, not the human one.
            let ledger_path = cli.data.clone().unwrap_or_else(agent_ledger_path);
            return mcp::serve(ledger_path).map_err(|e| format!("mcp: {e}"));
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
