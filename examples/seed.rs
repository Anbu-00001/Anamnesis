//! Generate a realistic, backdated demo ledger.
//!
//!     cargo run --example seed -- seed.json
//!
//! The data encodes a very human pattern — competent discrimination wrapped in
//! systematic overconfidence — so that `ana --data seed.json report` has
//! something honest to reflect back. None of these are real predictions; they
//! are a fictional forecaster's year, chosen to make the mirror legible.

use std::path::Path;

use anamnesis::model::{
    gen_id, normalize_tags, Claim, ClaimKind, Forecast, Ledger, NumericForecast, Outcome,
    Resolution,
};
use anamnesis::store;
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};

/// (day-offset-from-base, probability, reasoning)
type Fc<'a> = (i64, f64, &'a str);
/// (day-offset, happened, post-mortem note)
type Res<'a> = (i64, bool, &'a str);

fn tags_of(tags: &[&str]) -> Vec<String> {
    normalize_tags(&tags.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}

fn claim(
    salt: u64,
    base: DateTime<Utc>,
    stmt: &str,
    tags: &[&str],
    forecasts: &[Fc],
    resolution: Option<Res>,
    resolve_by: Option<NaiveDate>,
) -> Claim {
    let fc: Vec<Forecast> = forecasts
        .iter()
        .map(|&(day, prob, because)| Forecast {
            at: base + Duration::days(day),
            prob: Some(prob),
            interval: None,
            because: (!because.is_empty()).then(|| because.to_string()),
        })
        .collect();
    let created_at = fc.first().map(|f| f.at).unwrap_or(base);
    // Demo flavour: market/personal calls carry real consequences → higher stake,
    // so the stake-weighted Brier visibly diverges from the flat one.
    let stake = if tags.iter().any(|&t| t == "markets" || t == "personal") {
        3.0
    } else {
        1.0
    };
    Claim {
        id: gen_id(stmt, salt),
        statement: stmt.to_string(),
        created_at,
        resolve_by,
        tags: tags_of(tags),
        kind: ClaimKind::Binary,
        stake,
        forecasts: fc,
        resolution: resolution.map(|(day, happened, note)| Resolution {
            at: base + Duration::days(day),
            outcome: Some(if happened {
                Outcome::True
            } else {
                Outcome::False
            }),
            value: None,
            note: (!note.is_empty()).then(|| note.to_string()),
        }),
    }
}

/// A numeric claim: a single credible interval [low, high] at `level`, resolved
/// to `value`. Encodes the same human flaw as the binary set — intervals drawn
/// too tight.
// Demo scaffolding: a flat positional signature keeps the 5 call sites readable
// as one-liners; folding these into a params struct would only add noise here.
#[allow(clippy::too_many_arguments)]
fn numeric_claim(
    salt: u64,
    base: DateTime<Utc>,
    day: i64,
    stmt: &str,
    tags: &[&str],
    low: f64,
    high: f64,
    level: f64,
    because: &str,
    resolve_day: i64,
    value: f64,
) -> Claim {
    Claim {
        id: gen_id(stmt, salt),
        statement: stmt.to_string(),
        created_at: base + Duration::days(day),
        resolve_by: None,
        tags: tags_of(tags),
        kind: ClaimKind::Numeric,
        stake: 1.0,
        forecasts: vec![Forecast {
            at: base + Duration::days(day),
            prob: None,
            interval: Some(NumericForecast { low, high, level }),
            because: (!because.is_empty()).then(|| because.to_string()),
        }],
        resolution: Some(Resolution {
            at: base + Duration::days(resolve_day),
            outcome: None,
            value: Some(value),
            note: None,
        }),
    }
}

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

// The claims are pushed one-by-one, grouped under section comments, because the
// `next()` id-salt generator is interleaved with each entry and the grouping
// reads far better than a single 41-element `vec![]` literal would. This is demo
// scaffolding, not a hot path, so the clarity wins.
#[allow(clippy::vec_init_then_push)]
fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "seed.json".to_string());
    let base = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();

    let mut salt = 1000u64;
    let mut next = || {
        salt += 1;
        salt
    };
    let mut c: Vec<Claim> = Vec::new();

    // ── High confidence (0.90–0.95): boldest calls, and where overconfidence
    //    bites hardest — several of these were wrong. ──────────────────────────
    c.push(claim(
        next(),
        base,
        "The US Federal Reserve cuts rates at least once before end of 2025",
        &["geopolitics", "markets"],
        &[(
            4,
            0.90,
            "inflation clearly cooling; Fed already guiding toward cuts",
        )],
        Some((320, true, "")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "All three of OpenAI, Google and Anthropic ship a new flagship model in 2025",
        &["tech", "ai"],
        &[(6, 0.95, "release cadence makes this nearly certain")],
        Some((300, true, "")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "The S&P 500 ends 2025 above its 2024 close",
        &["markets"],
        &[(8, 0.95, "the trend is my friend and earnings look fine")],
        Some((350, false, "a Q4 drawdown I waved away as noise")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "Apple ships a fully LLM-revamped Siri by end of 2025",
        &["tech", "ai"],
        &[(20, 0.90, "WWDC promised it; surely they execute")],
        Some((330, false, "I trusted a roadmap demo over shipping history")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "A consumer AR-glasses product sells over one million units in 2025",
        &["tech"],
        &[(30, 0.90, "everyone says this is the year")],
        Some((340, false, "I mistook hype volume for demand")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "A new GLP-1 weight-loss drug wins major FDA approval in 2025",
        &["science", "health"],
        &[(40, 0.90, "deep pipeline, strong trials")],
        Some((290, true, "")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "I attend the wedding I already RSVP'd yes to",
        &["personal"],
        &[(60, 0.95, "it's on the calendar and I want to go")],
        Some((250, true, "")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "I keep my daily writing streak unbroken for 90 days",
        &["personal"],
        &[(15, 0.90, "motivation is sky-high right now")],
        Some((140, false, "motivation is not a plan; I missed day 47")),
        None,
    ));

    // ── 0.80 ───────────────────────────────────────────────────────────────
    c.push(claim(
        next(),
        base,
        "Bitcoin trades above $100k at some point in 2025",
        &["markets", "crypto"],
        &[(10, 0.80, "halving dynamics plus ETF inflows")],
        Some((280, true, "")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "An open-weights model tops a major public leaderboard outright in 2025",
        &["tech", "ai"],
        &[(45, 0.80, "open models are closing the gap fast")],
        Some((360, false, "closed frontier pulled ahead again")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "Tesla delivers fewer vehicles in 2025 than in 2024",
        &["markets"],
        &[(50, 0.80, "demand softening across the lineup")],
        Some((360, true, "")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "I finish the side project before my own deadline",
        &["personal"],
        &[(25, 0.80, "I always think I'm faster than I am")],
        Some((200, false, "planning fallacy, exactly as warned")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "A new pandemic triggers a WHO emergency declaration in 2025",
        &["geopolitics", "health"],
        &[(
            12,
            0.20,
            "low base rate; I was rightly skeptical of the noise",
        )],
        Some((350, false, "")),
        None,
    ));

    // ── 0.70 ───────────────────────────────────────────────────────────────
    c.push(claim(
        next(),
        base,
        "OPEC+ extends its production cuts through the end of 2025",
        &["geopolitics", "markets"],
        &[(14, 0.70, "fiscal break-evens demand higher prices")],
        Some((300, true, "")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "Nvidia remains the most valuable semiconductor company all year",
        &["tech", "markets"],
        &[(16, 0.70, "no credible challenger this cycle")],
        Some((360, true, "")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "Bitcoin ends 2025 above $150k",
        &["markets", "crypto"],
        &[(11, 0.70, "supercycle thesis")],
        Some((360, false, "the thesis was a wish wearing a number")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "The reigning champion defends their Wimbledon title in 2025",
        &["sports"],
        &[(160, 0.70, "looks dominant on grass")],
        Some((190, false, "an upset I gave too little weight")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "I read at least 12 books this year",
        &["personal"],
        &[(5, 0.70, "one a month is the floor for me")],
        Some((360, true, "")),
        None,
    ));

    // ── 0.60 ───────────────────────────────────────────────────────────────
    c.push(claim(
        next(),
        base,
        "Gold sets a new all-time high in 2025",
        &["markets"],
        &[(18, 0.60, "central-bank buying plus real-rate path")],
        Some((300, true, "")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "An outsider (outside the pre-season top four) wins a major football trophy",
        &["sports"],
        &[(120, 0.60, "the field looks unusually open")],
        Some((300, false, "the favourites held after all")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "I run a sub-2:00 half marathon this year",
        &["personal", "health"],
        &[(35, 0.60, "training is on track if I stay healthy")],
        Some((280, false, "a calf strain in week 9")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "India wins more than five golds at the season's flagship athletics meet",
        &["sports"],
        &[(130, 0.60, "strong recent form")],
        Some((300, true, "")),
        None,
    ));

    // ── 0.50 (honest coin-flips) ────────────────────────────────────────────
    c.push(claim(
        next(),
        base,
        "A major AI lab faces a serious public safety controversy in 2025",
        &["tech", "ai"],
        &[(22, 0.50, "broad claim, genuinely uncertain")],
        Some((330, true, "")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "Oil (Brent) closes 2025 lower than it opened",
        &["markets"],
        &[(20, 0.50, "no edge; truly a toss-up")],
        Some((360, false, "")),
        None,
    ));

    // ── 0.30 (leaning 'no') ─────────────────────────────────────────────────
    c.push(claim(
        next(),
        base,
        "A major European government collapses into a snap election in H2 2025",
        &["geopolitics"],
        &[(24, 0.30, "fragile coalitions but inertia is strong")],
        Some((330, false, "")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "A speculative retail mania (meme-stock scale) returns in 2025",
        &["markets"],
        &[(28, 0.30, "liquidity is tighter than 2021")],
        Some((340, false, "")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "A major US bank fails or needs a public rescue in 2025",
        &["markets"],
        &[(26, 0.30, "stress is real but backstops are ready")],
        Some((340, false, "")),
        None,
    ));

    // ── 0.20 (confidently 'no' — and sometimes wrong) ───────────────────────
    c.push(claim(
        next(),
        base,
        "Brent crude spikes above $100 at some point in 2025",
        &["markets", "geopolitics"],
        &[(20, 0.20, "I doubted any supply shock would land")],
        Some((210, true, "a regional flare-up I underweighted")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "A room-temperature superconductor is independently validated in 2025",
        &["science"],
        &[(48, 0.20, "extraordinary claims; replication usually fails")],
        Some((300, false, "")),
        None,
    ));

    // ── 0.10 (very confident 'no' — correctly) ──────────────────────────────
    c.push(claim(
        next(),
        base,
        "A crewed mission lands on the Moon in 2025",
        &["science"],
        &[(50, 0.10, "schedules have slipped for years")],
        Some((350, false, "")),
        None,
    ));
    c.push(claim(
        next(),
        base,
        "An FDA-approved outright cure for a major cancer is announced in 2025",
        &["science", "health"],
        &[(52, 0.10, "that is not how oncology progresses")],
        Some((350, false, "")),
        None,
    ));

    // ── Beliefs you actually REVISED (the palimpsest / mind-changing) ───────
    // M1 — updated toward truth: started bullish, cooled off, resolved false.
    c.push(claim(
        next(),
        base,
        "My country's central bank hikes rates again in 2025",
        &["geopolitics", "markets"],
        &[
            (5, 0.70, "early data looked hot"),
            (180, 0.30, "data turned; hikes look off the table"),
        ],
        Some((330, false, "")),
        None,
    ));
    // M2 — updated toward truth: warmed up correctly, resolved true.
    c.push(claim(
        next(),
        base,
        "A serious AI-regulation bill becomes law in a major economy in 2025",
        &["geopolitics", "ai"],
        &[
            (8, 0.55, "lots of talk, little movement"),
            (210, 0.80, "a concrete bill cleared committee"),
        ],
        Some((345, true, "")),
        None,
    ));
    // M3 — updated AWAY from truth: talked myself into confidence, resolved false.
    c.push(claim(
        next(),
        base,
        "Our team's flagship feature ships to general availability in 2025",
        &["personal", "tech"],
        &[
            (9, 0.40, "scope looked too big"),
            (160, 0.80, "leadership promised resources"),
        ],
        Some((
            330,
            false,
            "I let an org promise override my own estimate — beware that",
        )),
        None,
    ));
    // M4 — a near-wash revision.
    c.push(claim(
        next(),
        base,
        "Global EV sales grow more than 15% year over year in 2025",
        &["markets", "tech"],
        &[
            (7, 0.60, "China strong, West cooling"),
            (150, 0.70, "subsidy news tipped me up"),
        ],
        Some((350, true, "")),
        None,
    ));

    // ── NUMERIC forecasts (credible intervals, mostly drawn too tight) ──────
    c.push(numeric_claim(
        next(),
        base,
        6,
        "Number of US Fed rate cuts in 2025",
        &["markets", "geopolitics"],
        1.0,
        3.0,
        0.80,
        "a cut or two looks likely",
        320,
        2.0,
    )); // inside
    c.push(numeric_claim(
        next(),
        base,
        8,
        "S&P 500 year-end 2025 close (hundreds of points)",
        &["markets"],
        58.0,
        64.0,
        0.80,
        "grind higher from here",
        350,
        56.0,
    )); // outside (below)
    c.push(numeric_claim(
        next(),
        base,
        11,
        "Bitcoin year-end 2025 price (thousands of $)",
        &["markets", "crypto"],
        120.0,
        180.0,
        0.80,
        "supercycle math",
        360,
        95.0,
    )); // outside (below)
    c.push(numeric_claim(
        next(),
        base,
        5,
        "Books I finish reading in 2025",
        &["personal"],
        12.0,
        18.0,
        0.80,
        "roughly one a month, give or take",
        360,
        14.0,
    )); // inside
    c.push(numeric_claim(
        next(),
        base,
        7,
        "Global EV sales year-over-year growth in 2025 (%)",
        &["markets", "tech"],
        10.0,
        20.0,
        0.80,
        "strong but decelerating",
        350,
        23.0,
    )); // outside (above)

    // ── Still OPEN (so list/due/report have live items) ─────────────────────
    c.push(claim(
        next(),
        base,
        "Bitcoin closes above $200k at some point in 2026",
        &["markets", "crypto"],
        &[(380, 0.45, "no edge yet; halving tailwind vs macro")],
        None,
        Some(ymd(2026, 12, 31)),
    ));
    c.push(claim(
        next(),
        base,
        "A frontier lab releases weights of a GPT-4-class model in 2026",
        &["tech", "ai"],
        &[(385, 0.35, "competitive pressure rising")],
        None,
        Some(ymd(2026, 12, 31)),
    ));
    c.push(claim(
        next(),
        base,
        "I ship Anamnesis v1.0 with a TUI by the end of 2026",
        &["personal"],
        &[(
            500,
            0.55,
            "the core is done; UI is the easy part — said every engineer ever",
        )],
        None,
        Some(ymd(2026, 12, 31)),
    ));
    c.push(claim(
        next(),
        base,
        "A general AI-safety treaty is signed by 3+ major powers in 2026",
        &["geopolitics", "ai"],
        &[(390, 0.15, "diplomacy is slow; this is aspirational")],
        None,
        Some(ymd(2026, 12, 31)),
    ));
    // Two already PAST their resolve-by date but unresolved → 'due'.
    c.push(claim(
        next(),
        base,
        "My open-source library passes 1,000 GitHub stars",
        &["personal", "tech"],
        &[(300, 0.50, "depends entirely on one good launch")],
        None,
        Some(ymd(2026, 5, 1)),
    ));
    c.push(claim(
        next(),
        base,
        "Inflation in my country falls below 3% (annualised) by spring 2026",
        &["markets"],
        &[(360, 0.65, "trend is clearly down")],
        None,
        Some(ymd(2026, 4, 30)),
    ));

    let ledger = Ledger { claims: c };
    store::save(Path::new(&path), &ledger).expect("write seed");
    let resolved = ledger.claims.iter().filter(|c| c.is_resolved()).count();
    let open = ledger.claims.len() - resolved;
    eprintln!(
        "wrote {} claims ({resolved} resolved, {open} open) to {path}",
        ledger.claims.len()
    );
}
