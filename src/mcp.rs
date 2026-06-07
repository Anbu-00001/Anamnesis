//! `ana mcp` — a minimal Model Context Protocol server over stdio.
//!
//! Exposes Anamnesis as MCP **tools** (`predict`, `resolve`, `calibration`,
//! `list`) so that *any* MCP-capable agent — Claude, Cursor, Cline, Windsurf,
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
use crate::scoring::{self, NumericSample, Sample};
use crate::{report, store};

/// The protocol revision this server implements.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Serve MCP over stdio against `ledger`, until the input stream closes.
pub fn serve(ledger: PathBuf) -> io::Result<()> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut out = io::stdout().lock();
    let mut line = String::new();

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

        let resp = match method {
            "initialize" => initialize(&req, id),
            "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
            "tools/list" => {
                json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tool_schemas() } })
            }
            "tools/call" => tools_call(&req, id, &ledger),
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

fn initialize(req: &Value, id: Value) -> Value {
    // Echo the client's protocol version when present (version negotiation).
    let ver = req
        .pointer("/params/protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(PROTOCOL_VERSION);
    json!({ "jsonrpc": "2.0", "id": id, "result": {
        "protocolVersion": ver,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "anamnesis", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Log falsifiable predictions BEFORE acting (predict), resolve them the moment reality answers (resolve), and read your standing over/under-confidence (calibration). The engine is no-LLM and cannot flatter you — honesty is the optimal strategy."
    }})
}

fn tools_call(req: &Value, id: Value, ledger: &Path) -> Value {
    let name = req
        .pointer("/params/name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let args = req
        .pointer("/params/arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let outcome = match name {
        "predict" => tool_predict(&args, ledger),
        "resolve" => tool_resolve(&args, ledger),
        "calibration" => tool_calibration(&args, ledger),
        "recalibrate" => tool_recalibrate(&args, ledger),
        "list" => tool_list(&args, ledger),
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

fn tool_predict(args: &Value, ledger: &Path) -> ToolResult {
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

    let mut tags: Vec<String> = vec!["who:claude".into()];
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
        tags: normalize_tags(&tags),
        kind,
        stake,
        forecasts: vec![forecast],
        resolution: None,
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
            let prob = led.claims[idx].current_prob().unwrap_or(0.5);
            led.claims[idx].resolution = Some(Resolution {
                at: now,
                outcome: Some(if happened {
                    Outcome::True
                } else {
                    Outcome::False
                }),
                value: None,
                note,
            });
            store::save(ledger, &led).map_err(|e| e.to_string())?;
            let brier = (prob - if happened { 1.0 } else { 0.0 }).powi(2);
            Ok((
                format!(
                    "[{cid}] resolved {} — you said {:.0}% → Brier {brier:.3}",
                    if happened { "TRUE" } else { "FALSE" },
                    prob * 100.0
                ),
                Some(json!({ "id": cid, "outcome": happened, "prob": prob, "brier": brier })),
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

fn tool_calibration(args: &Value, ledger: &Path) -> ToolResult {
    let tag = args.get("tag").and_then(Value::as_str);
    let bins = args.get("bins").and_then(Value::as_u64).unwrap_or(10) as usize;
    let led = store::load(ledger).map_err(|e| e.to_string())?;
    let text = report::render(&led, tag, bins);
    let structured: Value =
        serde_json::from_str(&report::render_json(&led, tag, bins)).unwrap_or(Value::Null);
    Ok((text, Some(structured)))
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
    let led = store::load(ledger).map_err(|e| e.to_string())?;

    // Resolved binary samples matching the tag, in the order they were learned.
    let mut claims: Vec<&Claim> = led
        .claims
        .iter()
        .filter(|c| c.is_resolved())
        .filter(|c| tag.as_ref().is_none_or(|t| c.tags.iter().any(|x| x == t)))
        .collect();
    claims.sort_by_key(|c| c.resolution.as_ref().map(|r| r.at));
    let samples: Vec<Sample> = claims.iter().filter_map(|c| c.sample()).collect();
    let n = samples.len();

    let e = scoring::calibration_eprocess(&samples);
    let recal = scoring::fit_recalibration(&samples, report::RECAL_RIDGE);
    // Apply the map only once there is real (≥ suggestive) evidence of miscalibration
    // and enough samples — otherwise hand back the stated number untouched.
    let has_evidence = e.is_some_and(|ev| ev >= report::RECAL_MIN_E) && n >= report::RECAL_MIN_N;
    let (corrected, applied) = match (&recal, has_evidence) {
        (Some(r), true) => (r.apply(p), true),
        _ => (p, false),
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
            "description": "Log a falsifiable prediction BEFORE acting. For best calibration: (1) take the OUTSIDE VIEW first — name a `reference_class` of similar past cases and its base rate; (2) make your `prob`, then a `second_prob` that assumes your first is wrong (give yourself two reasons it could be) — the tool logs their average (dialectical bootstrapping, the wisdom of your own crowd); (3) tag a `kind` to learn calibration per type of call. Use `prob` for yes/no or `interval` for a quantity.",
            "inputSchema": { "type": "object", "properties": {
                "statement": { "type": "string", "description": "the falsifiable claim" },
                "prob": { "type": "number", "description": "probability it is true, 0..1 (binary)" },
                "second_prob": { "type": "number", "description": "a SECOND, consider-the-opposite estimate, 0..1; logged prob becomes the average of the two" },
                "reference_class": { "type": "string", "description": "the outside view: similar past cases and their base rate" },
                "interval": { "type": "string", "description": "credible interval \"LOW..HIGH\" (numeric)" },
                "level": { "type": "number", "description": "interval confidence level, default 0.8" },
                "kind": { "type": "string", "description": "estimate | tests-pass | bug-hypothesis | approach | compat" },
                "stake": { "type": "number", "description": "how much this call matters (≥ 0, default 1) — weights the Brier toward consequential calls" },
                "project": { "type": "string", "description": "project/repo slug" },
                "by": { "type": "string", "description": "expected resolution date, YYYY-MM-DD" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "extra tags" }
            }, "required": ["statement"] }
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
        assert_eq!(arr.len(), 5);
        for tool in arr {
            assert!(tool["name"].is_string());
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn parse_interval_roundtrips() {
        assert_eq!(parse_interval("2..6").unwrap(), (2.0, 6.0));
        assert!(parse_interval("6..2").is_err());
        assert!(parse_interval("nope").is_err());
    }
}
