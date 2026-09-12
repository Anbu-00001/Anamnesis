//! `ana mcp` — a minimal Model Context Protocol server over stdio.
//!
//! Exposes Anamnesis as MCP **tools** (`predict`, `resolve`, `calibration`,
//! `recalibrate`, `decide`, `list`) so that *any* MCP-capable agent — Claude, Cursor, Cline, Windsurf,
//! and the growing list of hosts that speak the protocol — can keep a
//! calibration ledger, not just the Claude Code plugin. This is the reach
//! surface: one server, every agent.
//!
//! It is a hand-rolled JSON-RPC 2.0 server over newline-delimited stdio. No
//! extra dependencies, no async runtime, the same instant cold-start as the rest
//! of `ana`. The scoring core stays the single source of truth: every tool just
//! loads the ledger, calls into [`crate::scoring`]/[`crate::report`], and saves.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, Utc};
use serde_json::{json, Value};

use crate::model::{
    compose_reasoning, gen_id, normalize_tags, Claim, ClaimKind, Forecast, NumericForecast,
    Outcome, Resolution,
};
use crate::scoring::{self, NumericSample};
use crate::{report, store};

/// The **modern** (stateless, per-request `_meta`) revision this server speaks.
/// Since 2026-07-28 there is no `initialize` handshake: every request carries its
/// own protocol version, and `server/discover` replaces the negotiation round trip.
const MODERN_VERSION: &str = "2026-07-28";

/// The **legacy** (`initialize`-handshake) revisions this server speaks, newest
/// first. Only revisions whose tool behaviour has actually been checked belong
/// here — claiming support is a promise, not a wish.
const SUPPORTED_LEGACY: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Every revision this server speaks, modern first. Reported by `server/discover`
/// and by the `UnsupportedProtocolVersionError` payload.
fn supported_versions() -> Vec<&'static str> {
    let mut v = vec![MODERN_VERSION];
    v.extend_from_slice(SUPPORTED_LEGACY);
    v
}

/// JSON-RPC error code for `UnsupportedProtocolVersionError` (MCP 2026-07-28).
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// Pick the legacy revision to answer an `initialize` with.
///
/// Echoing whatever the client asked for — which this used to do — meant a client
/// requesting `"1999-01-01"` was solemnly told the server spoke `"1999-01-01"`.
/// The spec says to reply with the newest revision the server actually supports
/// when the requested one is not among them, and let the client decide.
fn negotiate_legacy(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|v| SUPPORTED_LEGACY.iter().copied().find(|s| *s == v))
        .unwrap_or(SUPPORTED_LEGACY[0])
}

/// The protocol version a modern request declares in its `_meta`.
fn meta_protocol_version(req: &Value) -> Option<&str> {
    req.pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
        .or_else(|| {
            req.get("params")
                .and_then(|p| p.get("_meta"))
                .and_then(|m| m.get("io.modelcontextprotocol/protocolVersion"))
        })
        .and_then(Value::as_str)
}

/// The client's self-reported name, from modern `_meta` or legacy `clientInfo`.
fn client_name(req: &Value) -> Option<String> {
    let v = req
        .get("params")
        .and_then(|p| p.get("_meta"))
        .and_then(|m| m.get("io.modelcontextprotocol/clientInfo"))
        .and_then(|c| c.get("name"))
        .or_else(|| req.pointer("/params/clientInfo/name"))
        .and_then(Value::as_str)?;
    Some(sanitize_who(v))
}

/// Reduce a client name to a tag-safe `[a-z0-9-]` slug.
pub fn sanitize_who(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "unknown".into()
    } else {
        s
    }
}

/// Serve MCP over stdio against `ledger`, until the input stream closes.
pub fn serve(ledger: PathBuf) -> io::Result<()> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut out = io::stdout().lock();
    let mut line = String::new();
    // Who we are talking to, learned from `initialize` (legacy) or from the
    // `_meta` of any modern request. `ANAMNESIS_WHO` overrides both. Defaults to
    // `unknown` rather than `claude`: tagging a Cursor client's predictions
    // `who:claude` silently corrupts every per-client comparison in the ledger.
    let mut who: Option<String> = std::env::var("ANAMNESIS_WHO")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| sanitize_who(&s));
    let who_pinned = who.is_some();

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break; // EOF: client closed the pipe.
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                send(
                    &mut out,
                    &rpc_error(Value::Null, -32700, &format!("parse error: {e}")),
                )?;
                continue;
            }
        };

        // Requests carry an `id`; notifications do not and get no response.
        let Some(id) = req.get("id").cloned() else {
            continue;
        };
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");

        if !who_pinned {
            if let Some(n) = client_name(&req) {
                who = Some(n);
            }
        }

        // A modern request declares its version in `_meta`; if we do not speak it,
        // the spec requires `UnsupportedProtocolVersionError` listing what we do.
        if let Some(v) = meta_protocol_version(&req) {
            if !supported_versions().contains(&v) {
                send(&mut out, &unsupported_version(id, v))?;
                continue;
            }
        }

        let resp = match method {
            // Modern era: no handshake, one discovery call. Servers MUST implement it.
            "server/discover" => discover(id),
            "initialize" => initialize(&req, id),
            "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
            "tools/list" => {
                json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tool_schemas() } })
            }
            "tools/call" => tools_call(&req, id, &ledger, who.as_deref()),
            other => rpc_error(id, -32601, &format!("method not found: {other}")),
        };
        send(&mut out, &resp)?;
    }
    Ok(())
}

fn send(out: &mut impl Write, v: &Value) -> io::Result<()> {
    out.write_all(v.to_string().as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// `server/discover` — the modern replacement for the `initialize` handshake.
/// A dual-era client probes with this first; a recognised modern reply tells it
/// the server is modern, and anything else sends it back to `initialize`.
fn discover(id: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": {
        "resultType": "complete",
        "supportedVersions": supported_versions(),
        "capabilities": { "tools": {} },
        "_meta": {
            "io.modelcontextprotocol/serverInfo": {
                "name": "anamnesis",
                "version": env!("CARGO_PKG_VERSION")
            }
        },
        "instructions": SERVER_INSTRUCTIONS
    }})
}

fn unsupported_version(id: Value, requested: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": {
        "code": UNSUPPORTED_PROTOCOL_VERSION,
        "message": "Unsupported protocol version",
        "data": { "supported": supported_versions(), "requested": requested }
    }})
}

const SERVER_INSTRUCTIONS: &str = "Log falsifiable predictions BEFORE acting (predict), resolve them the moment reality answers (resolve), and read your standing over/under-confidence (calibration). Always pass `resolve_by`: the sequential evidence test orders claims by that date, and a prediction without one is scored but cannot count as evidence. The engine is no-LLM and cannot flatter you — honesty is the optimal strategy.";

fn initialize(req: &Value, id: Value) -> Value {
    // Reply with a revision we actually support, not with whatever was asked for.
    let ver = negotiate_legacy(
        req.pointer("/params/protocolVersion")
            .and_then(Value::as_str),
    );
    json!({ "jsonrpc": "2.0", "id": id, "result": {
        "protocolVersion": ver,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "anamnesis", "version": env!("CARGO_PKG_VERSION") },
        "instructions": SERVER_INSTRUCTIONS
    }})
}

fn tools_call(req: &Value, id: Value, ledger: &Path, who: Option<&str>) -> Value {
    let name = req
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let args = req
        .pointer("/params/arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let outcome = match name {
        "predict" => tool_predict(&args, ledger, who),
        "update" => tool_update(&args, ledger),
        "resolve" => tool_resolve(&args, ledger),
        "calibration" => tool_calibration(&args, ledger),
        "recalibrate" => tool_recalibrate(&args, ledger),
        "decide" => tool_decide(&args, ledger),
        "list" => tool_list(&args, ledger),
        "void" => tool_void(&args, ledger),
        "amend" => tool_amend(&args, ledger),
        other => Err(format!("unknown tool: {other}")),
    };
    match outcome {
        Ok((text, structured)) => {
            let mut result =
                json!({ "content": [{ "type": "text", "text": text }], "isError": false });
            if let Some(s) = structured {
                result["structuredContent"] = s;
            }
            json!({ "jsonrpc": "2.0", "id": id, "result": result })
        }
        // Tool-execution errors are reported in-band (isError) so the agent sees them.
        Err(msg) => json!({ "jsonrpc": "2.0", "id": id, "result": {
            "content": [{ "type": "text", "text": format!("error: {msg}") }],
            "isError": true
        }}),
    }
}

type ToolResult = Result<(String, Option<Value>), String>;

fn tool_predict(args: &Value, ledger: &Path, who: Option<&str>) -> ToolResult {
    let statement = args
        .get("statement")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if statement.is_empty() {
        return Err("statement is required".into());
    }
    let now = Utc::now();
    let because = args.get("because").and_then(Value::as_str);
    let reference_class = args.get("reference_class").and_then(Value::as_str);

    let (kind, forecast) = if let Some(p) = args.get("prob").and_then(Value::as_f64) {
        if !(0.0..=1.0).contains(&p) {
            return Err(format!("prob must be between 0 and 1, got {p}"));
        }
        // Dialectical bootstrapping: average in a second, "consider the opposite"
        // estimate when one is supplied.
        let (p_eff, estimates) = match args.get("second_prob").and_then(Value::as_f64) {
            Some(sp) => {
                if !(0.0..=1.0).contains(&sp) {
                    return Err(format!("second_prob must be between 0 and 1, got {sp}"));
                }
                (scoring::dialectical_mean(p, sp), Some((p, sp)))
            }
            None => (p, None),
        };
        (
            ClaimKind::Binary,
            Forecast {
                at: now,
                prob: Some(p_eff),
                interval: None,
                because: compose_reasoning(because, reference_class, estimates),
            },
        )
    } else if let Some(iv) = args.get("interval").and_then(Value::as_str) {
        let (low, high) = parse_interval(iv)?;
        let level = args.get("level").and_then(Value::as_f64).unwrap_or(0.80);
        if !(level > 0.0 && level < 1.0) {
            return Err(format!(
                "level must be strictly between 0 and 1, got {level}"
            ));
        }
        (
            ClaimKind::Numeric,
            Forecast {
                at: now,
                prob: None,
                interval: Some(NumericForecast { low, high, level }),
                because: compose_reasoning(because, reference_class, None),
            },
        )
    } else {
        return Err("give either `prob` (binary) or `interval` \"LOW..HIGH\" (numeric)".into());
    };

    // Precedence: an explicit `who` argument, then the client's own name from the
    // handshake or `_meta`, then `unknown`. Never a hardcoded "claude": a Cursor
    // client's predictions tagged `who:claude` quietly ruin every per-client
    // comparison the ledger can make.
    let who = args
        .get("who")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(sanitize_who)
        .or_else(|| who.map(String::from))
        .unwrap_or_else(|| "unknown".into());
    let mut tags: Vec<String> = vec![format!("who:{who}")];
    if let Some(p) = args
        .get("project")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        tags.push(format!("project:{p}"));
    }
    if let Some(k) = args
        .get("kind")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        tags.push(format!("kind:{k}"));
    }
    if let Some(s) = args
        .get("session")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        tags.push(format!("session:{s}"));
    }
    if let Some(extra) = args.get("tags").and_then(Value::as_array) {
        tags.extend(extra.iter().filter_map(Value::as_str).map(String::from));
    }
    let resolve_by = args
        .get("by")
        .and_then(Value::as_str)
        .map(parse_date)
        .transpose()?;
    let stake = args.get("stake").and_then(Value::as_f64).unwrap_or(1.0);
    if !(stake.is_finite() && stake >= 0.0) {
        return Err(format!("stake must be a finite number ≥ 0, got {stake}"));
    }

    // Hold the exclusive lock across load and save: an MCP server and a human
    // running `ana add` in a terminal are two writers on the same file.
    let _guard = store::lock(ledger).map_err(|e| e.to_string())?;
    let mut led = store::load(ledger).map_err(|e| e.to_string())?;
    let mut salt = now.timestamp_nanos_opt().unwrap_or(0) as u64;
    let id = loop {
        let candidate = gen_id(statement, salt);
        if !led.has_id(&candidate) {
            break candidate;
        }
        salt = salt.wrapping_add(1);
    };
    led.claims.push(Claim {
        id: id.clone(),
        statement: statement.to_string(),
        created_at: now,
        resolve_by,
        // Fixed at creation and written to the file, so the evidence order is
        // auditable and cannot shift later when a default changes.
        horizon_days: Some(crate::evidence::horizon_for(&normalize_tags(&tags))),
        tags: normalize_tags(&tags),
        kind,
        stake,
        forecasts: vec![forecast],
        resolution: None,
        void: None,
        amendments: Vec::new(),
    });
    store::save(ledger, &led).map_err(|e| e.to_string())?;
    Ok((
        format!("logged [{id}] \"{statement}\""),
        Some(json!({ "id": id, "kind": kind })),
    ))
}

fn tool_resolve(args: &Value, ledger: &Path) -> ToolResult {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or("id is required")?;
    let note = args.get("note").and_then(Value::as_str).map(String::from);
    let _guard = store::lock(ledger).map_err(|e| e.to_string())?;
    let mut led = store::load(ledger).map_err(|e| e.to_string())?;
    let idx = led.index_of(id)?;
    if led.claims[idx].is_resolved() {
        return Err(format!("[{}] is already resolved", led.claims[idx].id));
    }
    let kind = led.claims[idx].kind;
    let now = Utc::now();
    let cid = led.claims[idx].id.clone();

    match kind {
        ClaimKind::Binary => {
            let happened = match args.get("outcome") {
                Some(Value::Bool(b)) => *b,
                Some(Value::String(s)) => {
                    matches!(s.to_lowercase().as_str(), "yes" | "true" | "y" | "t" | "1")
                }
                _ => return Err("binary claim: pass `outcome` as yes/no (or true/false)".into()),
            };
            // Grade the FIRST forecast (invariant #6). Until `update` was
            // exposed over MCP an agent could only ever have one, so this read the
            // right number by accident; with revision reachable it would have told
            // a reviser their score was the one they reached after the evidence.
            let prob = led.claims[idx].first_prob().unwrap_or(0.5);
            let final_prob = led.claims[idx].current_prob();
            let revisions = led.claims[idx].forecasts.len();
            led.claims[idx].resolution = Some(Resolution {
                at: now,
                outcome: Some(if happened {
                    Outcome::True
                } else {
                    Outcome::False
                }),
                value: None,
                note,
                resolved_by: None,
            });
            store::save(ledger, &led).map_err(|e| e.to_string())?;
            let truth_f = if happened { 1.0 } else { 0.0 };
            let brier = (prob - truth_f).powi(2);
            let final_brier = final_prob.map(|fp| (fp - truth_f).powi(2));
            let revised = match (revisions > 1, final_prob, final_brier) {
                (true, Some(fp), Some(fb)) => format!(
                    " Graded on your FIRST forecast; you later revised to {:.0}% ({fb:.3} — shown, not graded).",
                    fp * 100.0
                ),
                _ => String::new(),
            };
            Ok((
                format!(
                    "[{cid}] resolved {} — you said {:.0}% → Brier {brier:.3}.{revised}",
                    if happened { "TRUE" } else { "FALSE" },
                    prob * 100.0
                ),
                Some(json!({
                    "id": cid,
                    "outcome": happened,
                    "prob": prob,
                    "brier": brier,
                    "score_basis": "first",
                    "final_prob": final_prob,
                    "final_brier": final_brier,
                })),
            ))
        }
        ClaimKind::Numeric => {
            let v = args
                .get("value")
                .and_then(Value::as_f64)
                .ok_or("numeric claim: pass `value` N")?;
            let iv = led.claims[idx]
                .current_interval()
                .ok_or("numeric claim has no interval forecast")?;
            led.claims[idx].resolution = Some(Resolution {
                at: now,
                outcome: None,
                value: Some(v),
                note,
                resolved_by: None,
            });
            store::save(ledger, &led).map_err(|e| e.to_string())?;
            let ns = NumericSample {
                low: iv.low,
                high: iv.high,
                level: iv.level,
                value: v,
            };
            let w = scoring::winkler(&ns);
            Ok((
                format!(
                    "[{cid}] resolved value {v} — interval [{}, {}] {} → Winkler {w:.3}",
                    iv.low,
                    iv.high,
                    if ns.contains() { "caught it" } else { "MISSED" }
                ),
                Some(json!({ "id": cid, "value": v, "inside": ns.contains(), "winkler": w })),
            ))
        }
    }
}

/// Revise an open forecast, appending rather than overwriting.
///
/// This verb existed only on the CLI until 0.4.0, which made the advertised loop
/// (predict → update → resolve → calibrate) unreachable for the agent that is the
/// tool's primary user: 426 claims logged over three months contained exactly zero
/// revisions, because there was no way to make one. That measured "nobody ever
/// changes their mind" as a fact about agents when it was a fact about the API.
///
/// It is safe to expose only because the headline score grades the FIRST forecast
/// (`Claim::sample`). Shipping `update` while the score read the last one would
/// have handed every agent a one-call route to a perfect record.
fn tool_update(args: &Value, ledger: &Path) -> ToolResult {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or("id is required")?;
    let because = args.get("because").and_then(Value::as_str);
    let _guard = store::lock(ledger).map_err(|e| e.to_string())?;
    let mut led = store::load(ledger).map_err(|e| e.to_string())?;
    let idx = led.index_of(id)?;
    if led.claims[idx].is_void() {
        return Err(format!("[{}] is void", led.claims[idx].id));
    }
    if led.claims[idx].is_resolved() {
        return Err(format!(
            "[{}] is already resolved; its history is final",
            led.claims[idx].id
        ));
    }
    let kind = led.claims[idx].kind;
    let now = Utc::now();

    let (forecast, human, detail) = match kind {
        ClaimKind::Binary => {
            let p = args
                .get("prob")
                .and_then(Value::as_f64)
                .ok_or("this is a binary claim; revise it with `prob` (0..1)")?;
            if !(0.0..=1.0).contains(&p) {
                return Err(format!("prob must be between 0 and 1, got {p}"));
            }
            let old = led.claims[idx].current_prob().unwrap_or(p);
            (
                Forecast {
                    at: now,
                    prob: Some(p),
                    interval: None,
                    because: because.map(str::to_string),
                },
                format!("{:.0}% → {:.0}%", old * 100.0, p * 100.0),
                json!({ "old_prob": old, "prob": p }),
            )
        }
        ClaimKind::Numeric => {
            let iv = args
                .get("interval")
                .and_then(Value::as_str)
                .ok_or("this is a numeric claim; revise it with `interval` \"LOW..HIGH\"")?;
            let (low, high) = parse_interval(iv)?;
            let lvl = args
                .get("level")
                .and_then(Value::as_f64)
                .or_else(|| led.claims[idx].current_interval().map(|i| i.level))
                .unwrap_or(0.80);
            if !(0.0..1.0).contains(&lvl) {
                return Err(format!("level must be between 0 and 1, got {lvl}"));
            }
            (
                Forecast {
                    at: now,
                    prob: None,
                    interval: Some(NumericForecast {
                        low,
                        high,
                        level: lvl,
                    }),
                    because: because.map(str::to_string),
                },
                format!("[{low}, {high}] @ {:.0}%", lvl * 100.0),
                json!({ "low": low, "high": high, "level": lvl }),
            )
        }
    };

    led.claims[idx].forecasts.push(forecast);
    let cid = led.claims[idx].id.clone();
    let rev = led.claims[idx].forecasts.len();
    store::save(ledger, &led).map_err(|e| e.to_string())?;

    let mut payload = json!({ "id": cid, "kind": kind, "revision": rev });
    if let (Some(o), Some(d)) = (payload.as_object_mut(), detail.as_object()) {
        for (k, v) in d {
            o.insert(k.clone(), v.clone());
        }
    }
    Ok((
        format!(
            "[{cid}] {human} (revision #{rev}). Your FIRST forecast is still what the headline score grades — revising is free and does not launder the record."
        ),
        Some(payload),
    ))
}

fn tool_void(args: &Value, ledger: &Path) -> ToolResult {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or("id is required")?;
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("a void needs a reason — it is the whole record of why")?;
    let _guard = store::lock(ledger).map_err(|e| e.to_string())?;
    let mut led = store::load(ledger).map_err(|e| e.to_string())?;
    let idx = led.index_of(id)?;
    if led.claims[idx].is_void() {
        return Err(format!("[{}] is already void", led.claims[idx].id));
    }
    led.claims[idx].void = Some(crate::model::Void {
        at: Utc::now(),
        reason: reason.to_string(),
    });
    let cid = led.claims[idx].id.clone();
    store::save(ledger, &led).map_err(|e| e.to_string())?;
    Ok((
        format!("[{cid}] voided — excluded from every score, kept in history. reason: {reason}"),
        Some(json!({ "id": cid, "void": true, "reason": reason })),
    ))
}

fn tool_amend(args: &Value, ledger: &Path) -> ToolResult {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or("id is required")?;
    let statement = args
        .get("statement")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let tags: Option<Vec<String>> = args.get("tags").and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect()
    });
    if statement.is_none() && tags.is_none() {
        return Err("nothing to amend — give `statement` and/or `tags`".into());
    }
    let _guard = store::lock(ledger).map_err(|e| e.to_string())?;
    let mut led = store::load(ledger).map_err(|e| e.to_string())?;
    let idx = led.index_of(id)?;
    if led.claims[idx].is_resolved() {
        return Err(format!(
            "[{}] is already resolved — a resolved claim is the record, and the record does not change",
            led.claims[idx].id
        ));
    }
    let c = &mut led.claims[idx];
    let mut am = crate::model::Amendment {
        at: Utc::now(),
        old_statement: None,
        new_statement: None,
        old_tags: None,
        new_tags: None,
    };
    if let Some(new) = statement {
        am.old_statement = Some(c.statement.clone());
        am.new_statement = Some(new.to_string());
        c.statement = new.to_string();
    }
    if let Some(new) = tags {
        am.old_tags = Some(c.tags.clone());
        am.new_tags = Some(new.clone());
        c.tags = new;
    }
    c.amendments.push(am);
    let (cid, stmt, tg) = (c.id.clone(), c.statement.clone(), c.tags.clone());
    store::save(ledger, &led).map_err(|e| e.to_string())?;
    Ok((
        format!("[{cid}] amended — \"{stmt}\""),
        Some(json!({ "id": cid, "statement": stmt, "tags": tg })),
    ))
}

fn tool_calibration(args: &Value, ledger: &Path) -> ToolResult {
    let tag = args.get("tag").and_then(Value::as_str);
    let bins = args.get("bins").and_then(Value::as_u64).unwrap_or(10) as usize;
    let led = store::load(ledger).map_err(|e| e.to_string())?;
    let today = Utc::now().date_naive();
    let text = report::render(&led, tag, bins, today);
    let structured: Value =
        serde_json::from_str(&report::render_json(&led, tag, bins, today)).unwrap_or(Value::Null);
    Ok((text, Some(structured)))
}

/// Load the ledger and ask [`report::earned_recalibration`] — the single source of
/// truth for the evidence gate — for the (tag-scoped) map and whether it is earned.
fn fit_and_gate(
    ledger: &Path,
    tag: Option<&str>,
) -> Result<(Option<scoring::Recalibration>, bool, usize, Option<f64>), String> {
    let led = store::load(ledger).map_err(|e| e.to_string())?;
    Ok(report::earned_recalibration(
        &led,
        tag,
        chrono::Utc::now().date_naive(),
    ))
}

fn tool_recalibrate(args: &Value, ledger: &Path) -> ToolResult {
    let p = args
        .get("prob")
        .and_then(Value::as_f64)
        .ok_or("prob (0..1) is required")?;
    if !(0.0..=1.0).contains(&p) {
        return Err(format!("prob must be between 0 and 1, got {p}"));
    }
    let tag = args
        .get("tag")
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    // Apply the map only once there is real evidence of miscalibration (and enough
    // samples) — otherwise hand back the stated number untouched.
    let (recal, applied, n, e) = fit_and_gate(ledger, tag.as_deref())?;
    let corrected = match (&recal, applied) {
        (Some(r), true) => r.apply(p),
        _ => p,
    };

    let text = if applied {
        format!(
            "{:.0}% → {:.0}%   (corrected from {n} resolved calls; e-value {:.1}, slope b={:.2})",
            p * 100.0,
            corrected * 100.0,
            e.unwrap_or(0.0),
            recal.as_ref().map(|r| r.b).unwrap_or(1.0)
        )
    } else {
        format!(
            "{:.0}% → {:.0}%   (unchanged — not enough evidence to correct yet: n={n}, e-value {:.1})",
            p * 100.0,
            corrected * 100.0,
            e.unwrap_or(1.0)
        )
    };
    Ok((
        text,
        Some(json!({
            "stated": p,
            "recalibrated": corrected,
            "applied": applied,
            "n": n,
            "eprocess": e,
            "a": recal.as_ref().map(|r| r.a),
            "b": recal.as_ref().map(|r| r.b),
        })),
    ))
}

fn tool_decide(args: &Value, ledger: &Path) -> ToolResult {
    let p = args
        .get("prob")
        .and_then(Value::as_f64)
        .ok_or("prob (0..1) is required")?;
    if !(0.0..=1.0).contains(&p) {
        return Err(format!("prob must be between 0 and 1, got {p}"));
    }
    let stake = args.get("stake").and_then(Value::as_f64).unwrap_or(1.0);
    let verify_cost = args
        .get("verify_cost")
        .and_then(Value::as_f64)
        .unwrap_or(0.2);
    let tag = args
        .get("tag")
        .and_then(Value::as_str)
        .map(str::to_lowercase);

    // Correct the stated probability through the earned map (if any), then apply
    // Chow's stake-aware threshold — the same evidence gate as `recalibrate`.
    let (recal, earned, n, e) = fit_and_gate(ledger, tag.as_deref())?;
    let map = if earned { recal } else { None };
    let d = scoring::decide(p, map, stake, verify_cost);

    let (verb, gloss) = match d.act {
        scoring::Act::Proceed => ("PROCEED", "confidence clears the bar for the stakes"),
        scoring::Act::Verify => ("VERIFY", "in the doubt zone — check before you commit"),
        scoring::Act::Abstain => (
            "ABSTAIN",
            "more likely to fail than succeed — replan or escalate",
        ),
    };
    let corr = if earned {
        format!(
            " (corrected {:.0}%→{:.0}% from {n} resolved calls)",
            p * 100.0,
            d.adjusted_p * 100.0
        )
    } else {
        String::new()
    };
    // An agent acting on a collapsed map would otherwise see the same verdict for
    // every probability it passes and have no way to know why.
    let note = if d.map_kind == scoring::MapKind::Constant {
        format!(" NOTE: your stated confidence has not tracked outcomes over {n} calls, so the number you gave was replaced with your base rate — every prob returns this same answer until that changes.")
    } else {
        String::new()
    };
    let text = format!(
        "{verb} — {gloss}. Need ≥{:.0}% at stake {stake:.1}; you have {:.0}%{corr}.{note}",
        d.proceed_threshold * 100.0,
        d.adjusted_p * 100.0,
    );
    Ok((
        text,
        Some(json!({
            "act": verb.to_lowercase(),
            "stated": p,
            "adjusted": d.adjusted_p,
            "proceed_threshold": d.proceed_threshold,
            "margin": d.margin,
            "stake": stake,
            "verify_cost": verify_cost,
            "map_kind": d.map_kind.as_str(),
            "used_recalibration": earned,
            "n": n,
            "eprocess": e,
        })),
    ))
}

fn tool_list(args: &Value, ledger: &Path) -> ToolResult {
    let filter = args.get("filter").and_then(Value::as_str).unwrap_or("all");
    let tag = args
        .get("tag")
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    let today = Utc::now().date_naive();
    let led = store::load(ledger).map_err(|e| e.to_string())?;
    let items: Vec<&Claim> = led
        .claims
        .iter()
        .filter(|c| {
            if let Some(t) = &tag {
                if !c.tags.iter().any(|x| x == t) {
                    return false;
                }
            }
            match filter {
                "open" => c.is_open(),
                "resolved" => c.is_resolved(),
                "due" => c.is_due(today),
                _ => true,
            }
        })
        .collect();
    let preds: Vec<Value> = items
        .iter()
        .map(|c| {
            json!({
                "id": c.id, "kind": c.kind, "statement": c.statement,
                "prob": c.current_prob(), "interval": c.current_interval(),
                "tags": c.tags, "resolve_by": c.resolve_by, "resolved": c.is_resolved(),
            })
        })
        .collect();
    let text = if preds.is_empty() {
        "(no matching predictions)".to_string()
    } else {
        items
            .iter()
            .map(|c| format!("[{}] {}", c.id, c.statement))
            .collect::<Vec<_>>()
            .join("\n")
    };
    Ok((
        text,
        Some(json!({ "count": preds.len(), "predictions": preds })),
    ))
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

fn tool_schemas() -> Value {
    json!([
        {
            "name": "predict",
            "description": "Log a falsifiable prediction BEFORE acting. ALWAYS pass `by`: the sequential evidence test orders claims by that date because it is fixed before the outcome is known, so a prediction without one is scored but can never count as evidence that you are (or are not) miscalibrated. For best calibration: (1) take the OUTSIDE VIEW first — name a `reference_class` of similar past cases and its base rate; (2) make your `prob`, then a `second_prob` that assumes your first is wrong (give yourself two reasons it could be) — the tool logs their average (dialectical bootstrapping, the wisdom of your own crowd); (3) tag a `kind` to learn calibration per type of call. Use `prob` for yes/no or `interval` for a quantity.",
            "inputSchema": { "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object", "properties": {
                "statement": { "type": "string", "description": "the falsifiable claim" },
                "prob": { "type": "number", "description": "probability it is true, 0..1 (binary)" },
                "second_prob": { "type": "number", "description": "a SECOND, consider-the-opposite estimate, 0..1; logged prob becomes the average of the two" },
                "reference_class": { "type": "string", "description": "the outside view: similar past cases and their base rate" },
                "interval": { "type": "string", "description": "credible interval \"LOW..HIGH\" (numeric)" },
                "level": { "type": "number", "description": "interval confidence level, default 0.8" },
                "kind": { "type": "string", "description": "estimate | tests-pass | bug-hypothesis | approach | compat" },
                "stake": { "type": "number", "description": "how much this call matters (≥ 0, default 1) — weights the Brier toward consequential calls" },
                "project": { "type": "string", "description": "project/repo slug" },
                "by": { "type": "string", "description": "the date you expect to know the answer, YYYY-MM-DD. Pass this on every prediction: without it the claim cannot enter the anytime-valid evidence test." },
                "who": { "type": "string", "description": "who is predicting; defaults to the MCP client's own name, so leave it unset unless you are logging on someone else's behalf" },
                "session": { "type": "string", "description": "session identifier, tagged as session:<value>" },
                "model": { "type": "string", "description": "the model making the call, tagged as model:<value>. Pooling calibration across model versions makes the numbers uninterpretable, so record it." },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "extra tags" }
            }, "required": ["statement", "by"] }
        },
        {
            "name": "update",
            "description": "Revise an OPEN prediction as evidence arrives. The old forecast is kept, never overwritten — the claim is a palimpsest. Revising is free and cannot launder your record: the headline score always grades your FIRST forecast, so an update is read as 'you learned something', not as 'you were right all along'. Use it the moment your belief actually moves; leaving a stale number logged is the thing that costs you.",
            "inputSchema": { "type": "object", "properties": {
                "id": { "type": "string", "description": "claim id (any unique prefix)" },
                "prob": { "type": "number", "description": "binary: the revised probability, 0..1" },
                "interval": { "type": "string", "description": "numeric: the revised interval \"LOW..HIGH\"" },
                "level": { "type": "number", "description": "numeric: confidence level; defaults to the claim's previous level" },
                "because": { "type": "string", "description": "what changed your mind — the reason is the part worth re-reading later" }
            }, "required": ["id"] }
        },
        {
            "name": "resolve",
            "description": "Resolve a prediction the moment reality answers; returns its Brier (binary) or Winkler (numeric) score.",
            "inputSchema": { "type": "object", "properties": {
                "id": { "type": "string", "description": "claim id (any unique prefix)" },
                "outcome": { "type": ["boolean", "string"], "description": "binary: yes/no or true/false" },
                "value": { "type": "number", "description": "numeric: the value that occurred" },
                "note": { "type": "string", "description": "post-mortem: what you misjudged" }
            }, "required": ["id"] }
        },
        {
            "name": "calibration",
            "description": "Your standing calibration report: over/under-confidence gap, per-kind breakdown, base-rate confidence interval, reliability diagram.",
            "inputSchema": { "type": "object", "properties": {
                "tag": { "type": "string", "description": "filter, e.g. who:claude or kind:estimate" },
                "bins": { "type": "integer", "description": "reliability-diagram bins, default 10" }
            } }
        },
        {
            "name": "recalibrate",
            "description": "Correct a stated probability through your learned recalibration map (p ↦ σ(a + b·logit p)) fit from your resolved calls. Hands the number back UNCHANGED until there is real evidence you are miscalibrated — it will not 'correct' on noise. Optionally scope to a `tag` (e.g. kind:estimate, who:claude).",
            "inputSchema": { "type": "object", "properties": {
                "prob": { "type": "number", "description": "your stated probability, 0..1" },
                "tag": { "type": "string", "description": "scope the map to claims with this tag" }
            }, "required": ["prob"] }
        },
        {
            "name": "decide",
            "description": "Turn a stated probability into an ACTION under stakes — the operational end of calibration, for when you're about to do something and want to know whether to just do it. Corrects your number through the earned recalibration map (verbalized confidence is unreliable), then applies Chow's stake-aware threshold: returns PROCEED, VERIFY (check first), or ABSTAIN (replan — more likely to fail than succeed). Raise `stake` for consequential or irreversible actions; the bar to proceed climbs with it. Use this instead of acting on a gut number.",
            "inputSchema": { "type": "object", "properties": {
                "prob": { "type": "number", "description": "your stated success probability, 0..1" },
                "stake": { "type": "number", "description": "cost of a wrong action relative to one verification; 1 = ordinary, raise for irreversible calls (default 1)" },
                "verify_cost": { "type": "number", "description": "cost of a verification step in the same unit (default 0.2)" },
                "tag": { "type": "string", "description": "scope the correction map to claims with this tag, e.g. kind:estimate" }
            }, "required": ["prob"] }
        },
        {
            "name": "void",
            "description": "Annul an ambiguous or unanswerable question. It keeps its place in the history but is excluded from every score, the way a forecasting platform annuls a question rather than grading it. Use this instead of leaving a bad question to rot unresolved — an unresolved overdue claim pauses the evidence test.",
            "inputSchema": { "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object", "properties": {
                "id": { "type": "string", "description": "claim id (any unique prefix)" },
                "reason": { "type": "string", "description": "why this question cannot fairly be graded" }
            }, "required": ["id", "reason"] }
        },
        {
            "name": "amend",
            "description": "Fix a typo in a claim's statement, or correct its tags. Pre-resolution only, and never the probability or the timestamps — those are the record. The previous wording is kept.",
            "inputSchema": { "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object", "properties": {
                "id": { "type": "string", "description": "claim id (any unique prefix)" },
                "statement": { "type": "string", "description": "replacement statement" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "replacement tag set" }
            }, "required": ["id"] }
        },
        {
            "name": "list",
            "description": "List predictions, optionally filtered by status and tag.",
            "inputSchema": { "type": "object", "properties": {
                "filter": { "type": "string", "enum": ["all", "open", "resolved", "due"], "description": "default all" },
                "tag": { "type": "string", "description": "only claims carrying this tag" }
            } }
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_are_well_formed() {
        let t = tool_schemas();
        let arr = t.as_array().unwrap();
        let names: Vec<&str> = arr.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(
            names,
            [
                "predict",
                "update",
                "resolve",
                "calibration",
                "recalibrate",
                "decide",
                "void",
                "amend",
                "list"
            ]
        );
        for tool in arr {
            assert!(tool["name"].is_string());
            assert!(tool["description"].is_string());
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(tool["inputSchema"]["properties"].is_object());
        }
        // A prediction with no resolve-by date cannot enter the evidence test, so
        // the schema asks for one rather than leaving it to the agent's judgement.
        let predict = arr.iter().find(|t| t["name"] == "predict").unwrap();
        let required: Vec<&str> = predict["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(required.contains(&"by"), "predict must require `by`");
    }

    #[test]
    fn legacy_negotiation_never_echoes_an_unknown_version() {
        // The server used to reply "1999-01-01" to a client that asked for it.
        assert_eq!(negotiate_legacy(Some("1999-01-01")), SUPPORTED_LEGACY[0]);
        assert_eq!(negotiate_legacy(None), SUPPORTED_LEGACY[0]);
        assert_eq!(negotiate_legacy(Some("2025-06-18")), "2025-06-18");
        for v in SUPPORTED_LEGACY {
            assert_eq!(negotiate_legacy(Some(v)), *v);
        }
    }

    #[test]
    fn client_names_become_tag_safe_slugs() {
        assert_eq!(sanitize_who("Cursor IDE"), "cursor-ide");
        assert_eq!(sanitize_who("Claude Code"), "claude-code");
        assert_eq!(sanitize_who("  "), "unknown");
        assert_eq!(sanitize_who("a/b:c"), "a-b-c");
    }

    #[test]
    fn parse_interval_roundtrips() {
        assert_eq!(parse_interval("2..6").unwrap(), (2.0, 6.0));
        assert!(parse_interval("6..2").is_err());
        assert!(parse_interval("nope").is_err());
    }
}
