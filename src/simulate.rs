//! `ik simulate`: render a zero-dependency, self-contained interactive HTML simulator
//! from a declarative `.ik.toml` machine.
//!
//! Advisory explorer derived from the compiled `.ik.toml` definition (states, facts,
//! guards, `then` updates, invariants, effects, and schema-2 regions). Guard previews
//! and fact updates mirror the declared rules for exploration; durable commits and
//! production authority are enforced exclusively by the Rust `ik` runtime.

use crate::canonical::canonical_json;
use crate::declarative::{load_def, Definition};
use crate::machine::Machine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: ik simulate <machine.ik.toml> [--out <file.html>] [--open]";

pub fn run(args: &[String]) -> i32 {
    match run_inner(args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("ik simulate: {msg}");
            eprintln!("{USAGE}");
            2
        }
    }
}

fn run_inner(args: &[String]) -> Result<i32, String> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{USAGE}");
        return Ok(0);
    }
    let mut machine: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut open_browser = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                let value = args.get(i).ok_or("--out requires a path")?;
                out = Some(PathBuf::from(value));
            }
            "--open" => open_browser = true,
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            path if machine.is_none() => machine = Some(PathBuf::from(path)),
            other => return Err(format!("unexpected argument `{other}`")),
        }
        i += 1;
    }
    let machine = machine.ok_or("machine definition path is required")?;
    let compiled = load_def(&machine).map_err(|errors| errors.join("; "))?;
    let def = compiled.definition();
    let mut schema = crate::declarative::schema::machine_schema(def, compiled.identity());
    enrich_schema_with_def(&mut schema, def, &machine);
    let html = render_simulator_html(&schema)?;

    let target_out = match (out, open_browser) {
        (Some(path), _) => Some(path),
        (None, true) => Some(std::env::temp_dir().join(format!(
            "{}-v{}-simulator.html",
            compiled.identity().machine_id,
            compiled.identity().version
        ))),
        (None, false) => None,
    };

    if let Some(out_path) = target_out {
        write_html_file(&out_path, &html)?;
        let identity = compiled.identity();
        println!(
            "wrote {} ({} v{}, {} states, {} commands)",
            out_path.display(),
            identity.machine_id,
            identity.version,
            def.states.len(),
            def.commands.len()
        );
        if open_browser {
            open_in_browser(&out_path)?;
        }
    } else {
        print!("{html}");
    }
    Ok(0)
}

fn open_in_browser(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let launcher = "open";
    #[cfg(target_os = "linux")]
    let launcher = "xdg-open";
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let launcher = "open";

    std::process::Command::new(launcher)
        .arg(path)
        .spawn()
        .map_err(|err| format!("failed to launch `{launcher}` for {}: {err}", path.display()))?;
    Ok(())
}

/// Attach full compiled rule metadata (initial facts, guards, `then` updates, invariants,
/// effects, and schema-2 regions) to the `ik schema` payload so the simulator can preview
/// real guards, live facts, pending-effect locks, and export `scenario.json`.
fn enrich_schema_with_def(schema: &mut Value, def: &Definition, source_path: &Path) {
    let Some(root) = schema.as_object_mut() else {
        return;
    };
    root.insert(
        "source_path".to_string(),
        Value::String(source_path.display().to_string()),
    );

    if let Some(facts_arr) = root.get_mut("facts").and_then(Value::as_array_mut) {
        for item in facts_arr.iter_mut() {
            if let Some(obj) = item.as_object_mut() {
                if let Some(name) = obj.get("name").and_then(Value::as_str) {
                    if let Some(fdef) = def.facts.get(name) {
                        if let Some(init) = &fdef.initial {
                            obj.insert("initial".to_string(), init.clone());
                        }
                    }
                }
            }
        }
    }

    if let Some(cmds_arr) = root.get_mut("commands").and_then(Value::as_array_mut) {
        for item in cmds_arr.iter_mut() {
            if let Some(obj) = item.as_object_mut() {
                let Some(name) = obj.get("name").and_then(Value::as_str).map(str::to_string) else {
                    continue;
                };
                let Some(cdef) = def.commands.get(&name) else {
                    continue;
                };
                let outcomes: Vec<Value> = if let Some(list) = &cdef.outcomes {
                    list.iter()
                        .map(|o| {
                            json!({
                                "to": o.to,
                                "when": o.when,
                                "then": o.then.clone().unwrap_or_default(),
                            })
                        })
                        .collect()
                } else if let Some(to) = &cdef.to {
                    vec![json!({
                        "to": to,
                        "when": cdef.when,
                        "then": cdef.then.clone().unwrap_or_default(),
                    })]
                } else {
                    Vec::new()
                };
                obj.insert("outcomes".to_string(), Value::Array(outcomes));
                if let Some(eff) = &cdef.effect {
                    obj.insert(
                        "effect".to_string(),
                        json!({
                            "kind": eff.kind,
                            "id": eff.id,
                        }),
                    );
                }
            }
        }
    }

    let inv_defs: Vec<Value> = def
        .invariants
        .iter()
        .map(|(name, inv)| {
            json!({
                "name": name,
                "when": inv.when,
                "assert": inv.assert,
            })
        })
        .collect();
    root.insert("invariant_defs".to_string(), Value::Array(inv_defs));

    if def.chart.hierarchical && !def.chart.regions.is_empty() {
        let regions: Vec<Value> = def
            .chart
            .regions
            .iter()
            .map(|(name, r)| {
                let states: Vec<&String> = r.states.keys().collect();
                json!({
                    "name": name,
                    "states": states,
                })
            })
            .collect();
        let coords: serde_json::Map<String, Value> = def
            .origin
            .products
            .iter()
            .map(|(flat, parts)| (flat.clone(), json!(parts)))
            .collect();
        root.insert("regions".to_string(), Value::Array(regions));
        root.insert("state_coords".to_string(), Value::Object(coords));
    }
}

fn write_html_file(path: &Path, html: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("create directory {}: {err}", parent.display()))?;
        }
    }
    std::fs::write(path, html).map_err(|err| format!("write {}: {err}", path.display()))
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Escape JSON safely for embedding inside a `<script>` tag (blocks `</script>` breakouts).
fn safe_inline_json(value: &Value) -> String {
    let raw = canonical_json(value);
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    out
}

/// Render a standalone interactive HTML structural simulator from an `ik schema` JSON value.
pub fn render_simulator_html(schema: &Value) -> Result<String, String> {
    let machine = schema
        .get("machine")
        .and_then(Value::as_object)
        .ok_or("schema missing `machine` object")?;
    let machine_id = machine
        .get("machine_id")
        .and_then(Value::as_str)
        .ok_or("schema missing `machine.machine_id`")?;
    let version = machine
        .get("version")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .ok_or("schema missing `machine.version`")?;
    let spec_hash = machine
        .get("spec_hash")
        .and_then(Value::as_str)
        .ok_or("schema missing `machine.spec_hash`")?;
    let states_len = schema
        .get("states")
        .and_then(Value::as_array)
        .map(Vec::len)
        .ok_or("schema missing `states` array")?;
    let commands_len = schema
        .get("commands")
        .and_then(Value::as_array)
        .map(Vec::len)
        .ok_or("schema missing `commands` array")?;

    let short_hash: String = spec_hash.chars().take(12).collect();
    let title = format!("{machine_id} v{version} — Desmo interactive simulator");
    let embedded = safe_inline_json(schema);

    Ok(TEMPLATE
        .replace("__TITLE__", &escape_html(&title))
        .replace("__MACHINE_ID__", &escape_html(machine_id))
        .replace("__VERSION__", &escape_html(&version))
        .replace("__SHORT_HASH__", &escape_html(&short_hash))
        .replace("__STATES_LEN__", &states_len.to_string())
        .replace("__COMMANDS_LEN__", &commands_len.to_string())
        .replace("__EMBEDDED_SCHEMA__", &embedded))
}

const TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>__TITLE__</title>
<style>
  :root {
    --paper: #FFFFFF;
    --ink: #111315;
    --sec: #465158;
    --surf: #F2F5F6;
    --surf2: #E5EBEE;
    --line: #CBD3D7;
    --accent: #FF671D;
    --accent-ink: #9B3400;
    --accent-soft: #FFF1EA;
    --ok: #136F41;
    --ok-soft: #ECF8F1;
    --warn: #8A5300;
    --warn-soft: #FFF8E6;
    --mono: ui-monospace, "IBM Plex Mono", SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    --sans: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
  }
  * { box-sizing: border-box; }
  body { margin: 0; background: var(--paper); color: var(--ink); font-family: var(--sans); line-height: 1.45; }
  .accent-strip { height: 5px; background: var(--accent); width: 100%; }
  .wrap { max-width: 1280px; margin: 0 auto; padding: 24px 28px 48px; }
  header {
    display: flex; flex-wrap: wrap; justify-content: space-between; align-items: flex-end;
    gap: 16px; padding-bottom: 18px; border-bottom: 1.5px solid var(--line);
  }
  .kicker {
    display: inline-flex; align-items: center; gap: 10px;
    font: 600 12px/1 var(--mono); letter-spacing: 0.06em; color: var(--accent-ink); text-transform: uppercase;
  }
  .kicker::before { content: ""; width: 18px; height: 3px; background: var(--accent); display: inline-block; }
  h1 { margin: 8px 0 4px; font: 600 30px/1.1 var(--sans); letter-spacing: -0.03em; }
  .meta { font: 500 13px/1.4 var(--mono); color: var(--sec); }
  .advisory {
    margin: 16px 0 22px; padding: 11px 16px; border-radius: 8px;
    background: var(--surf); border: 1px solid var(--line);
    font: 500 12.5px/1.5 var(--mono); color: var(--sec);
  }
  .advisory strong { color: var(--ink); }
  .grid { display: grid; grid-template-columns: minmax(0, 1.45fr) minmax(350px, 1fr); gap: 22px; align-items: start; }
  @media (max-width: 980px) { .grid { grid-template-columns: 1fr; } }
  .card { background: var(--paper); border: 1.5px solid var(--line); border-radius: 12px; overflow: hidden; }
  .card-head {
    display: flex; justify-content: space-between; align-items: center; gap: 12px;
    padding: 12px 16px; background: var(--surf); border-bottom: 1px solid var(--line);
    font: 600 12px/1 var(--mono); color: var(--sec); letter-spacing: 0.04em; text-transform: uppercase;
  }
  .card-body { padding: 16px; }
  .stage { background: var(--surf); padding: 12px; overflow-x: auto; }
  svg#graph { width: 100%; height: auto; display: block; min-height: 280px; }
  .regions-bar {
    display: none; flex-wrap: wrap; gap: 8px; padding: 10px 16px;
    border-top: 1px solid var(--line); background: var(--surf);
    font: 600 12px/1.3 var(--mono);
  }
  .regions-bar.show { display: flex; }
  .region-chip {
    padding: 4px 10px; border-radius: 6px; background: var(--paper); border: 1px solid var(--line); color: var(--ink);
  }
  .region-chip span { color: var(--sec); font-weight: 500; }
  .status-bar {
    display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px;
    padding: 14px 16px; border-top: 1px solid var(--line); background: var(--paper);
  }
  .stat-label { font: 600 11px/1 var(--mono); color: var(--sec); text-transform: uppercase; margin-bottom: 4px; }
  .stat-val { font: 600 15.5px/1.2 var(--mono); color: var(--ink); word-break: break-all; }
  .banner {
    margin: 14px 16px 0; padding: 12px 14px; border-radius: 8px; display: none;
    font: 600 13px/1.45 var(--mono);
  }
  .banner.show { display: block; }
  .banner.refused { background: var(--accent-soft); border: 1.5px solid var(--accent); color: var(--accent-ink); }
  .banner.committed { background: var(--ok-soft); border: 1.5px solid var(--ok); color: var(--ok); }
  .banner.pending { background: var(--warn-soft); border: 1.5px solid #D99B00; color: var(--warn); }
  .effect-bar {
    margin: 12px 16px 0; padding: 12px 14px; border-radius: 8px; display: none;
    background: var(--warn-soft); border: 1.5px solid #D99B00;
    font: 600 12.5px/1.4 var(--mono); color: var(--warn);
  }
  .effect-bar.show { display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 10px; }
  .section-title {
    font: 600 11.5px/1 var(--mono); color: var(--sec); text-transform: uppercase;
    letter-spacing: 0.05em; margin: 14px 0 8px;
  }
  .section-title:first-child { margin-top: 0; }
  .cmd-list { display: flex; flex-direction: column; gap: 8px; }
  .cmd-item { display: flex; flex-direction: column; gap: 6px; }
  .arg-row {
    display: flex; flex-wrap: wrap; gap: 6px; padding: 6px 10px;
    background: var(--surf); border: 1px solid var(--line); border-radius: 6px;
    font: 500 11.5px/1 var(--mono); align-items: center;
  }
  .arg-row label { display: inline-flex; align-items: center; gap: 5px; color: var(--sec); }
  .arg-row input {
    width: 118px; padding: 4px 6px; border: 1px solid var(--line); border-radius: 4px;
    font: 500 12px/1.2 var(--mono); background: var(--paper); color: var(--ink);
  }
  .arg-row select {
    padding: 4px 6px; border: 1px solid var(--line); border-radius: 4px;
    font: 500 12px/1.2 var(--mono); background: var(--paper); color: var(--ink);
  }
  button.cmd {
    width: 100%; text-align: left; padding: 10px 12px; border-radius: 8px;
    cursor: pointer; font: 600 13.5px/1.3 var(--mono);
    display: flex; justify-content: space-between; align-items: center; gap: 10px;
    transition: transform 0.08s ease, background 0.12s ease;
  }
  button.cmd:active { transform: scale(0.99); }
  button.cmd.legal { background: var(--ink); color: var(--paper); border: 1.5px solid var(--ink); }
  button.cmd.legal:hover { background: #23282D; }
  button.cmd.illegal { background: var(--paper); color: var(--accent-ink); border: 1.5px dashed var(--accent); }
  button.cmd.illegal:hover { background: var(--accent-soft); }
  .cmd-sub { font: 400 11.5px/1.2 var(--mono); opacity: 0.85; text-align: right; }
  .guard-tag { display: block; font: 400 11px/1.3 var(--mono); opacity: 0.78; margin-top: 2px; }
  .toolbar { display: flex; flex-wrap: wrap; gap: 8px; }
  button.tool {
    padding: 7px 11px; border-radius: 6px; border: 1.5px solid var(--ink);
    background: var(--paper); color: var(--ink); cursor: pointer;
    font: 600 12px/1 var(--mono);
  }
  button.tool:hover { background: var(--surf); }
  button.tool.stale { border-color: var(--accent); color: var(--accent-ink); }
  button.tool.stale:hover { background: var(--accent-soft); }
  button.tool:disabled { opacity: 0.4; cursor: not-allowed; }
  .log { max-height: 280px; overflow-y: auto; display: flex; flex-direction: column; gap: 6px; font: 500 12px/1.4 var(--mono); }
  .log-row {
    padding: 8px 10px; border-radius: 6px; border: 1px solid var(--line); background: var(--surf);
    display: flex; justify-content: space-between; gap: 10px; align-items: baseline;
  }
  .log-row.refused { background: var(--accent-soft); border-color: var(--accent); color: var(--accent-ink); }
  .log-row.committed { background: var(--paper); border-color: var(--line); color: var(--ink); }
  .pill { display: inline-block; padding: 2px 7px; border-radius: 4px; font: 700 10.5px/1.2 var(--mono); text-transform: uppercase; }
  .pill.ok { background: var(--ink); color: var(--paper); }
  .pill.bad { background: var(--accent); color: var(--paper); }
  .facts-table { width: 100%; border-collapse: collapse; font: 500 12.5px/1.4 var(--mono); }
  .facts-table th, .facts-table td { text-align: left; padding: 5px 8px; border-bottom: 1px solid var(--line); }
  .facts-table th { font-size: 11px; color: var(--sec); text-transform: uppercase; }
  .facts-table td.val { color: var(--ink); font-weight: 600; }
  .inv-list { margin-top: 10px; font: 500 12px/1.45 var(--mono); color: var(--sec); display: flex; flex-direction: column; gap: 4px; }
  .inv-list code { color: var(--ink); font-weight: 600; }
  .branch-pick {
    margin-top: 8px; padding: 10px; border-radius: 8px; background: var(--surf);
    border: 1px solid var(--line); display: none;
  }
  .branch-pick.show { display: block; }
  .branch-btns { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 8px; }
</style>
</head>
<body>
<div class="accent-strip"></div>
<div class="wrap">
  <header>
    <div>
      <div class="kicker">Desmo · Interactive Lifecycle Simulator</div>
      <h1 id="machine-title">__MACHINE_ID__</h1>
      <div class="meta">version __VERSION__ · spec_hash __SHORT_HASH__ · __STATES_LEN__ states · __COMMANDS_LEN__ commands</div>
    </div>
    <div class="toolbar">
      <button class="tool" id="btn-save-handle" type="button">Copy Capability Handle</button>
      <button class="tool stale" id="btn-fire-stale" type="button" disabled>Fire Stale Handle</button>
      <button class="tool" id="btn-export" type="button">Export scenario.json</button>
      <button class="tool" id="btn-reset" type="button">Reset</button>
    </div>
  </header>

  <div class="advisory">
    <strong>Advisory structural simulation</strong> derived from <code>ik schema</code> (<code>__SHORT_HASH__</code>).
    Legal transitions, capability generations, live facts, guard previews, and pending-effect locks mirror the declared <code>.ik.toml</code> rules; durable commits and authoritative verification run inside the Rust <code>ik</code> runtime.
  </div>

  <div class="grid">
    <div class="card">
      <div class="card-head">
        <span>State Graph · Positive-Drive Rails</span>
        <span id="stale-badge">No copied handle</span>
      </div>
      <div id="banner" class="banner" role="status"></div>
      <div id="effect-bar" class="effect-bar">
        <span id="effect-label"></span>
        <div class="toolbar">
          <button class="tool" id="btn-eff-ok" type="button">Record Succeeded</button>
          <button class="tool stale" id="btn-eff-fail" type="button">Record Failed</button>
        </div>
      </div>
      <div class="stage">
        <svg id="graph" viewBox="0 0 880 340" aria-label="Interactive state machine graph"></svg>
      </div>
      <div id="regions-bar" class="regions-bar"></div>
      <div class="status-bar">
        <div>
          <div class="stat-label">Current State</div>
          <div class="stat-val" id="stat-state"></div>
        </div>
        <div>
          <div class="stat-label">Capability</div>
          <div class="stat-val" id="stat-cap"></div>
        </div>
        <div>
          <div class="stat-label">Terminal / Effect Lock</div>
          <div class="stat-val" id="stat-term"></div>
        </div>
      </div>
    </div>

    <div style="display:flex; flex-direction:column; gap:18px;">
      <div class="card">
        <div class="card-head">
          <span>Propose Command</span>
          <span>Click legal or illegal</span>
        </div>
        <div class="card-body">
          <div class="section-title">Legal from current state</div>
          <div class="cmd-list" id="legal-cmds"></div>
          <div class="branch-pick" id="branch-pick">
            <div class="meta" id="branch-prompt"></div>
            <div class="branch-btns" id="branch-btns"></div>
          </div>
          <div class="section-title">Illegal from current state (click to test refusal)</div>
          <div class="cmd-list" id="illegal-cmds"></div>
        </div>
      </div>

      <div class="card">
        <div class="card-head">
          <span>Live Facts &amp; Invariants</span>
          <span id="inv-count"></span>
        </div>
        <div class="card-body" id="facts-box"></div>
      </div>

      <div class="card">
        <div class="card-head">
          <span>Execution Timeline</span>
          <span id="log-count">0 events</span>
        </div>
        <div class="card-body">
          <div class="log" id="event-log"></div>
        </div>
      </div>
    </div>
  </div>
</div>

<script>
(() => {
  const SCHEMA = __EMBEDDED_SCHEMA__;
  const SVG_NS = "http://www.w3.org/2000/svg";
  const initialObj = SCHEMA.states.find((s) => s.initial) || SCHEMA.states[0];
  const stateMap = new Map(SCHEMA.states.map((s) => [s.name, s]));
  const factDefs = new Map((SCHEMA.facts || []).map((f) => [f.name, f]));

  function initialFacts() {
    const out = {};
    for (const f of (SCHEMA.facts || [])) {
      if (f.initial !== undefined) out[f.name] = f.initial;
    }
    return out;
  }

  let current = initialObj ? initialObj.name : "";
  let facts = initialFacts();
  let generation = 0;
  let nonce = "n0-a1f4";
  let savedHandle = null;
  let pendingEffect = null;
  let recordedEffect = null;
  let lastRefusal = null;
  let pendingBranch = null;
  let events = [];
  let scenarioSteps = [];
  const argValues = new Map();

  function defaultArg(arg) {
    if (arg.type === "int") return "1";
    if (arg.type === "bool") return "true";
    return arg.name + "_1";
  }

  function getCmdArgs(cmd) {
    const out = {};
    for (const a of (cmd.args || [])) {
      const key = cmd.name + "." + a.name;
      const raw = argValues.has(key) ? argValues.get(key) : defaultArg(a);
      if (a.type === "int") {
        const n = Number(raw);
        if (!Number.isInteger(n)) return { error: "argument $" + a.name + " requires an integer" };
        out[a.name] = n;
      } else if (a.type === "bool") {
        out[a.name] = raw === "true" || raw === true;
      } else {
        out[a.name] = String(raw);
      }
    }
    return { values: out };
  }

  function nextNonce(g) {
    const tag = ((g * 2654435761) >>> 0).toString(16).slice(0, 4).padStart(4, "0");
    return "n" + g + "-" + tag;
  }

  // Advisory evaluator for Desmo guard & invariant expressions (returns true, false, or null if unsupported)
  function evalExpr(src, stateName, factState, args) {
    if (!src || !src.trim()) return true;
    try {
      const toks = tokenize(src);
      let idx = 0;
      const peek = () => toks[idx];
      const next = () => toks[idx++];
      function parseOr() {
        let v = parseAnd();
        while (peek() && peek().t === "op" && peek().v === "||") { next(); const r = parseAnd(); v = v || r; }
        return v;
      }
      function parseAnd() {
        let v = parseAtom();
        while (peek() && peek().t === "op" && peek().v === "&&") { next(); const r = parseAtom(); v = v && r; }
        return v;
      }
      function parseAtom() {
        const tk = peek();
        if (!tk) throw new Error("eof");
        if (tk.t === "(") { next(); const v = parseOr(); if (!next() || toks[idx - 1].t !== ")") throw new Error(")"); return v; }
        if (tk.t === "id" && (tk.v === "exists" || tk.v === "not_exists")) {
          const fn = next().v;
          if (next().t !== "(") throw new Error("(");
          const f = next();
          if (next().t !== ")") throw new Error(")");
          const has = Object.prototype.hasOwnProperty.call(factState, f.v) && factState[f.v] !== undefined && factState[f.v] !== null;
          return fn === "exists" ? has : !has;
        }
        if (tk.t === "id" && tk.v === "text_in") {
          next();
          if (next().t !== "(") throw new Error("(");
          const val = parseOperand();
          if (next().t !== ",") throw new Error(",");
          const setTok = next();
          if (next().t !== ",") throw new Error(",");
          const minN = next().v;
          if (next().t !== ",") throw new Error(",");
          const maxN = next().v;
          if (next().t !== ")") throw new Error(")");
          if (typeof val !== "string") return false;
          if (val.length < minN || val.length > maxN) return false;
          const re = new RegExp("^[" + setTok.v.replace(/[\\\]^]/g, "\\$&") + "]*$");
          return re.test(val);
        }
        const left = parseOperand();
        const op = next();
        if (!op || op.t !== "op") throw new Error("cmp");
        const right = parseOperand();
        if (left === undefined || right === undefined) return false;
        if (op.v === "==") return left === right;
        if (op.v === "!=") return left !== right;
        if (op.v === "<") return left < right;
        if (op.v === "<=") return left <= right;
        if (op.v === ">") return left > right;
        if (op.v === ">=") return left >= right;
        throw new Error("op");
      }
      function parseOperand() {
        let base = parsePrimary();
        while (peek() && peek().t === "op" && ["+", "-", "*", "/"].includes(peek().v)) {
          const op = next().v;
          const rhs = parsePrimary();
          if (typeof base !== "number" || typeof rhs !== "number") return undefined;
          if (op === "+") base = base + rhs;
          else if (op === "-") base = base - rhs;
          else if (op === "*") base = base * rhs;
          else if (op === "/") base = rhs === 0 ? undefined : Math.trunc(base / rhs);
        }
        return base;
      }
      function parsePrimary() {
        const tk = next();
        if (!tk) throw new Error("operand");
        if (tk.t === "int" || tk.t === "str" || tk.t === "bool") return tk.v;
        if (tk.t === "$") return args[tk.v];
        if (tk.t === "id") {
          if (tk.v === "state") return stateName;
          if (tk.v === "effect_status") return pendingEffect ? "pending" : (recordedEffect ? "recorded" : "none");
          if (tk.v === "effect_kind") return pendingEffect ? pendingEffect.kind : (recordedEffect ? recordedEffect.kind : undefined);
          if (tk.v === "effect_outcome") return recordedEffect ? recordedEffect.outcome : undefined;
          return factState[tk.v];
        }
        throw new Error("primary");
      }
      const res = parseOr();
      return idx === toks.length ? Boolean(res) : null;
    } catch {
      return null;
    }
  }

  function tokenize(s) {
    const out = [];
    let i = 0;
    while (i < s.length) {
      const c = s[i];
      if (/\s/.test(c)) { i++; continue; }
      if (c === "(" || c === ")" || c === ",") { out.push({ t: c }); i++; continue; }
      if (c === "$") {
        i++; let id = "";
        while (i < s.length && /[A-Za-z0-9_]/.test(s[i])) id += s[i++];
        out.push({ t: "$", v: id }); continue;
      }
      if (c === "'" || c === '"') {
        const q = c; i++; let str = "";
        while (i < s.length && s[i] !== q) str += s[i++];
        if (i >= s.length) throw new Error("unterminated");
        i++; out.push({ t: "str", v: str }); continue;
      }
      if (/[0-9]/.test(c)) {
        let num = "";
        while (i < s.length && /[0-9]/.test(s[i])) num += s[i++];
        out.push({ t: "int", v: Number(num) }); continue;
      }
      if (/[+\-*/]/.test(c)) { out.push({ t: "op", v: c }); i++; continue; }
      if (/[&|=!<>]/.test(c)) {
        let op = "";
        while (i < s.length && /[&|=!<>]/.test(s[i])) op += s[i++];
        out.push({ t: "op", v: op }); continue;
      }
      if (/[A-Za-z_]/.test(c)) {
        let id = "";
        while (i < s.length && /[A-Za-z0-9_]/.test(s[i])) id += s[i++];
        if (id === "true") out.push({ t: "bool", v: true });
        else if (id === "false") out.push({ t: "bool", v: false });
        else out.push({ t: "id", v: id });
        continue;
      }
      throw new Error("char");
    }
    return out;
  }

  function evalArithExpr(src, stateName, factState, args) {
    const toks = tokenize(src);
    let idx = 0;
    const peek = () => toks[idx];
    const next = () => toks[idx++];
    function primary() {
      const tk = next();
      if (!tk) throw new Error("operand");
      if (tk.t === "int" || tk.t === "str" || tk.t === "bool") return tk.v;
      if (tk.t === "$") return args[tk.v];
      if (tk.t === "id") return factState[tk.v];
      throw new Error("primary");
    }
    let base = primary();
    while (peek() && peek().t === "op" && ["+", "-", "*", "/"].includes(peek().v)) {
      const op = next().v;
      const rhs = primary();
      if (typeof base !== "number" || typeof rhs !== "number") return undefined;
      if (op === "+") base = base + rhs;
      else if (op === "-") base = base - rhs;
      else if (op === "*") base = base * rhs;
      else if (op === "/") base = rhs === 0 ? undefined : Math.trunc(base / rhs);
    }
    return idx === toks.length ? base : undefined;
  }

  function applyThen(factState, thenMap, args) {
    const next = { ...factState };
    for (const [k, raw] of Object.entries(thenMap || {})) {
      const expr = String(raw).trim();
      if (/^\$[A-Za-z0-9_]+$/.test(expr)) {
        next[k] = args[expr.slice(1)];
      } else if (/^\+[0-9]+$/.test(expr)) {
        const delta = Number(expr);
        next[k] = (typeof factState[k] === "number" ? factState[k] : 0) + delta;
      } else if (expr === "true" || expr === "false") {
        next[k] = expr === "true";
      } else if ((expr.startsWith('"') && expr.endsWith('"')) || (expr.startsWith("'") && expr.endsWith("'"))) {
        next[k] = expr.slice(1, -1);
      } else if (/^-?\d+$/.test(expr)) {
        next[k] = Number(expr);
      } else if (/[+\-*/]/.test(expr)) {
        const computed = evalArithExpr(expr, "", factState, args);
        next[k] = computed !== undefined ? computed : expr;
      } else {
        next[k] = expr;
      }
    }
    return next;
  }

  function checkInvariants(nextState, nextFacts) {
    for (const inv of (SCHEMA.invariant_defs || [])) {
      if (inv.when) {
        const gate = evalExpr(inv.when, nextState, nextFacts, {});
        if (gate === false) continue;
      }
      const ok = evalExpr(inv.assert, nextState, nextFacts, {});
      if (ok === false) return inv;
    }
    return null;
  }

  const maxNameLen = Math.max(8, ...SCHEMA.states.map((s) => s.name.length));
  const NW = Math.min(260, Math.max(122, Math.round(maxNameLen * 8.4 + 28)));
  const NH = 44;

  function computeLayout() {
    const depth = new Map();
    if (initialObj) {
      depth.set(initialObj.name, 0);
      const q = [initialObj.name];
      while (q.length > 0) {
        const u = q.shift();
        const d = depth.get(u);
        for (const cmd of SCHEMA.commands) {
          if (!cmd.from.includes(u)) continue;
          for (const v of cmd.successors) {
            if (!depth.has(v)) {
              depth.set(v, d + 1);
              q.push(v);
            }
          }
        }
      }
    }
    let maxD = 0;
    for (const d of depth.values()) maxD = Math.max(maxD, d);
    for (const st of SCHEMA.states) {
      if (!depth.has(st.name)) depth.set(st.name, maxD + 1);
    }

    const cols = new Map();
    for (const st of SCHEMA.states) {
      const d = depth.get(st.name);
      if (!cols.has(d)) cols.set(d, []);
      cols.get(d).push(st);
    }
    for (const list of cols.values()) {
      list.sort((a, b) => Number(a.terminal) - Number(b.terminal) || a.name.localeCompare(b.name));
    }

    const colKeys = Array.from(cols.keys()).sort((a, b) => a - b);
    const maxRows = Math.max(1, ...Array.from(cols.values()).map((c) => c.length));
    const colGap = Math.max(216, NW + 94);
    const rowGap = 136;
    const padX = Math.max(105, Math.round(NW / 2 + 44));
    const padY = 92;
    const width = Math.max(880, padX * 2 + Math.max(1, colKeys.length - 1) * colGap);
    const height = Math.max(290, padY * 2 + Math.max(1, maxRows - 1) * rowGap + 40);

    const pos = new Map();
    colKeys.forEach((d, ci) => {
      const list = cols.get(d);
      list.forEach((st, ri) => {
        pos.set(st.name, { x: padX + ci * colGap, y: padY + ri * rowGap });
      });
    });
    return { pos, width, height };
  }

  const LAYOUT = computeLayout();

  function el(tag, attrs = {}, text) {
    const node = document.createElementNS(SVG_NS, tag);
    for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, String(v));
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function renderGraph() {
    const svg = document.getElementById("graph");
    svg.setAttribute("viewBox", "0 0 " + LAYOUT.width + " " + LAYOUT.height);
    while (svg.firstChild) svg.removeChild(svg.firstChild);

    const defs = el("defs");
    const mInk = el("marker", { id: "arr-ink", viewBox: "0 0 10 10", refX: 9, refY: 5, markerWidth: 7, markerHeight: 7, orient: "auto-start-reverse" });
    mInk.appendChild(el("path", { d: "M 0 1 L 10 5 L 0 9 z", fill: "#111315" }));
    defs.appendChild(mInk);
    svg.appendChild(defs);

    const edgeCounts = new Map();
    function drawEdgeLabel(lx, ly, text, active) {
      const w = Math.max(30, text.length * 7.2 + 10);
      svg.appendChild(el("rect", { x: lx - w / 2, y: ly - 10, width: w, height: 15, rx: 3, fill: "#F2F5F6" }));
      svg.appendChild(el("text", { x: lx, y: ly + 1.5, "text-anchor": "middle", "font-family": "var(--mono)", "font-size": 11.5, "font-weight": 600, fill: active ? "#111315" : "#465158" }, text));
    }

    for (const cmd of SCHEMA.commands) {
      for (const src of cmd.from) {
        const p1 = LAYOUT.pos.get(src);
        if (!p1) continue;
        for (const dst of cmd.successors) {
          const p2 = LAYOUT.pos.get(dst);
          if (!p2) continue;
          const key = src + "->" + dst;
          const idx = edgeCounts.get(key) || 0;
          edgeCounts.set(key, idx + 1);
          const isFromCurrent = src === current;
          const stroke = isFromCurrent ? "#111315" : "#8A969E";

          if (src === dst) {
            const lx = p1.x, ly = p1.y - NH / 2;
            const d = "M " + (lx - 18) + " " + ly + " C " + (lx - 34) + " " + (ly - 54) + ", " + (lx + 34) + " " + (ly - 54) + ", " + (lx + 18) + " " + ly;
            svg.appendChild(el("path", { d, fill: "none", stroke, "stroke-width": isFromCurrent ? 2.4 : 1.7, "marker-end": "url(#arr-ink)" }));
            drawEdgeLabel(lx, ly - 44, cmd.name, isFromCurrent);
          } else {
            const dx = p2.x - p1.x, dy = p2.y - p1.y;
            const dist = Math.hypot(dx, dy) || 1;
            const ux = dx / dist, uy = dy / dist;
            const sx = p1.x + ux * (NW / 2 + 2);
            const sy = p1.y + uy * (NH / 2 + 2);
            const ex = p2.x - ux * (NW / 2 + 6);
            const ey = p2.y - uy * (NH / 2 + 6);
            const isBack = p2.x < p1.x;
            const curve = isBack ? 56 + idx * 24 : (idx > 0 ? 28 * idx : 0);
            const mx = (sx + ex) / 2 - uy * curve;
            const my = (sy + ey) / 2 + ux * curve;
            const d = curve === 0
              ? "M " + sx + " " + sy + " L " + ex + " " + ey
              : "M " + sx + " " + sy + " Q " + mx + " " + my + " " + ex + " " + ey;
            svg.appendChild(el("path", { d, fill: "none", stroke, "stroke-width": isFromCurrent ? 2.4 : 1.7, "marker-end": "url(#arr-ink)" }));
            const lx = curve === 0 ? (sx + ex) / 2 : 0.25 * sx + 0.5 * mx + 0.25 * ex;
            const isHoriz = Math.abs(dy) < 10;
            const ly = (curve === 0 ? (sy + ey) / 2 : 0.25 * sy + 0.5 * my + 0.25 * ey) + (isHoriz ? -12 : 0);
            drawEdgeLabel(lx, ly, cmd.name, isFromCurrent);
          }
        }
      }
    }

    if (lastRefusal && lastRefusal.targetState) {
      const p1 = LAYOUT.pos.get(current);
      const p2 = LAYOUT.pos.get(lastRefusal.targetState);
      if (p1 && p2) {
        const mx = (p1.x + p2.x) / 2;
        const my = Math.min(p1.y, p2.y) - 62;
        const tStop = 0.48;
        const qx = (1 - tStop) * (1 - tStop) * p1.x + 2 * (1 - tStop) * tStop * mx + tStop * tStop * p2.x;
        const qy = (1 - tStop) * (1 - tStop) * (p1.y - NH / 2) + 2 * (1 - tStop) * tStop * my + tStop * tStop * (p2.y - NH / 2);
        const d = "M " + p1.x + " " + (p1.y - NH / 2) + " Q " + ((p1.x + mx) / 2) + " " + my + " " + qx + " " + qy;
        svg.appendChild(el("path", { d, fill: "none", stroke: "#FF671D", "stroke-width": 2.6, "stroke-dasharray": "6 5" }));
        svg.appendChild(el("line", { x1: qx, y1: qy - 14, x2: qx, y2: qy + 14, stroke: "#FF671D", "stroke-width": 4.5, "stroke-linecap": "round" }));
      }
    }

    for (const st of SCHEMA.states) {
      const p = LAYOUT.pos.get(st.name);
      if (!p) continue;
      const isCur = st.name === current;
      const rx = p.x - NW / 2, ry = p.y - NH / 2;

      if (st.terminal) {
        svg.appendChild(el("rect", { x: rx - 5, y: ry - 5, width: NW + 10, height: NH + 10, rx: 11, fill: "none", stroke: "#111315", "stroke-width": 1.6 }));
      }
      svg.appendChild(el("rect", {
        x: rx, y: ry, width: NW, height: NH, rx: 8,
        fill: isCur ? "#111315" : "#FFFFFF",
        stroke: isCur ? "#111315" : "#465158",
        "stroke-width": isCur ? 2.4 : 1.8,
      }));
      svg.appendChild(el("text", {
        x: p.x, y: p.y + 4.5, "text-anchor": "middle",
        "font-family": "var(--mono)", "font-size": 13, "font-weight": 600,
        fill: isCur ? "#FFFFFF" : "#111315",
      }, st.name));

      if (st.initial) {
        svg.appendChild(el("circle", { cx: rx - 12, cy: p.y, r: 4.5, fill: "#111315" }));
      }
      if (st.terminal) {
        svg.appendChild(el("text", {
          x: p.x, y: ry + NH + 18, "text-anchor": "middle",
          "font-family": "var(--mono)", "font-size": 10, "font-weight": 700, fill: "#465158",
        }, "TERMINAL"));
      }
    }
  }

  function setBanner(kind, message) {
    const b = document.getElementById("banner");
    b.className = "banner show " + kind;
    b.textContent = message;
  }

  function clearBanner() {
    const b = document.getElementById("banner");
    b.className = "banner";
    b.textContent = "";
  }

  function commitOutcome(cmd, outcome, args) {
    const fromState = current;
    const nextState = outcome.to || current;
    const nextFacts = applyThen(facts, outcome.then || {}, args);
    const violated = checkInvariants(nextState, nextFacts);
    if (violated) {
      refuseCommand(cmd, "IK_INVARIANT_FAILED", "invariant `" + violated.name + "` (" + violated.assert + ") failed on target state `" + nextState + "`.", args);
      return;
    }
    current = nextState;
    facts = nextFacts;
    generation += 1;
    nonce = nextNonce(generation);
    lastRefusal = null;
    pendingBranch = null;
    if (cmd.plans_effect) {
      const effKind = (cmd.effect && cmd.effect.kind) || cmd.name;
      pendingEffect = { kind: effKind, generation };
    }
    scenarioSteps.push({
      kind: "command",
      label: cmd.name + "-" + generation,
      name: cmd.name,
      args,
      expect: "committed",
    });
    events.unshift({
      kind: "committed",
      label: cmd.name + ": " + fromState + " → " + nextState,
      detail: "gen " + generation + (cmd.plans_effect ? " · effect pending" : ""),
    });
    setBanner(
      cmd.plans_effect ? "pending" : "committed",
      "✓ COMMITTED: " + cmd.name + " (" + fromState + " → " + nextState + ") · capability advanced to gen " + generation + (cmd.plans_effect ? " · pending effect locks further commands until recorded." : "")
    );
    renderAll();
  }

  function refuseCommand(cmd, code, detail, args) {
    lastRefusal = { command: cmd.name, code, targetState: cmd.successors[0] || null };
    pendingBranch = null;
    scenarioSteps.push({
      kind: "command",
      label: cmd.name + "-refused-" + (scenarioSteps.length + 1),
      name: cmd.name,
      args: args || {},
      expect: "refused",
      code,
    });
    events.unshift({
      kind: "refused",
      label: cmd.name + " refused (" + code + ")",
      detail: "stays " + current + " · gen " + generation,
    });
    setBanner("refused", "✕ REFUSED (" + code + "): " + detail + " Canonical state stays " + current + " (gen " + generation + ").");
    renderAll();
  }

  function onClickCommand(cmd) {
    const parsedArgs = getCmdArgs(cmd);
    if (parsedArgs.error) {
      refuseCommand(cmd, "IK_COMMAND_INVALID", parsedArgs.error, {});
      return;
    }
    const args = parsedArgs.values;
    if (pendingEffect) {
      refuseCommand(cmd, "IK_EFFECT_PENDING", "effect `" + pendingEffect.kind + "` (gen " + pendingEffect.generation + ") is pending; record its outcome before proposing new commands.", args);
      return;
    }
    const curState = stateMap.get(current);
    if (curState && curState.terminal) {
      refuseCommand(cmd, "IK_ILLEGAL_STATE", "state `" + current + "` is terminal; no further commands may commit.", args);
      return;
    }
    if (!cmd.from.includes(current)) {
      refuseCommand(cmd, "IK_ILLEGAL_STATE", "command `" + cmd.name + "` is legal only from [" + cmd.from.join(", ") + "], not `" + current + "`.", args);
      return;
    }
    const outcomes = cmd.outcomes && cmd.outcomes.length
      ? cmd.outcomes
      : cmd.successors.map((to) => ({ to, when: null, then: {} }));

    let indeterminate = false;
    for (const o of outcomes) {
      if (!o.when) {
        if (outcomes.length > 1 && !cmd.outcomes) {
          pendingBranch = { cmd, outcomes, args };
          renderAll();
          return;
        }
        commitOutcome(cmd, o, args);
        return;
      }
      const matched = evalExpr(o.when, current, facts, args);
      if (matched === true) {
        commitOutcome(cmd, o, args);
        return;
      }
      if (matched === null) {
        indeterminate = true;
      }
    }
    if (indeterminate) {
      pendingBranch = { cmd, outcomes, args };
      renderAll();
      return;
    }
    const guardList = outcomes.map((o) => o.when).filter(Boolean).join(" | ");
    refuseCommand(cmd, "IK_INVARIANT_FAILED", "guard not satisfied for `" + cmd.name + "` (" + guardList + ").", args);
  }

  function renderCommands() {
    const legalEl = document.getElementById("legal-cmds");
    const illegalEl = document.getElementById("illegal-cmds");
    legalEl.textContent = "";
    illegalEl.textContent = "";

    const curState = stateMap.get(current);
    const isTerm = Boolean(curState && curState.terminal);

    for (const cmd of SCHEMA.commands) {
      const legal = !isTerm && cmd.from.includes(current);
      const wrap = document.createElement("div");
      wrap.className = "cmd-item";

      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "cmd " + (legal ? "legal" : "illegal");

      const left = document.createElement("span");
      left.textContent = cmd.name + (cmd.args.length ? "(" + cmd.args.map((a) => a.name).join(", ") + ")" : "()");
      const guards = (cmd.outcomes || []).map((o) => o.when).filter(Boolean);
      if (guards.length > 0) {
        const g = document.createElement("span");
        g.className = "guard-tag";
        g.textContent = "when: " + guards.join(" | ");
        left.appendChild(g);
      }

      const right = document.createElement("span");
      right.className = "cmd-sub";
      right.textContent = legal
        ? "→ " + cmd.successors.join(" | ") + (cmd.plans_effect ? " · effect" : "")
        : "from [" + cmd.from.join(", ") + "]";
      btn.appendChild(left);
      btn.appendChild(right);
      btn.addEventListener("click", () => onClickCommand(cmd));
      wrap.appendChild(btn);

      if (legal && cmd.args && cmd.args.length > 0) {
        const row = document.createElement("div");
        row.className = "arg-row";
        for (const a of cmd.args) {
          const key = cmd.name + "." + a.name;
          if (!argValues.has(key)) argValues.set(key, defaultArg(a));
          const lbl = document.createElement("label");
          lbl.textContent = "$" + a.name + ":";
          if (a.type === "bool") {
            const sel = document.createElement("select");
            for (const opt of ["true", "false"]) {
              const o = document.createElement("option");
              o.value = opt; o.textContent = opt;
              if (String(argValues.get(key)) === opt) o.selected = true;
              sel.appendChild(o);
            }
            sel.addEventListener("change", () => argValues.set(key, sel.value));
            lbl.appendChild(sel);
          } else {
            const inp = document.createElement("input");
            inp.type = a.type === "int" ? "number" : "text";
            inp.value = String(argValues.get(key));
            inp.addEventListener("input", () => argValues.set(key, inp.value));
            lbl.appendChild(inp);
          }
          row.appendChild(lbl);
        }
        wrap.appendChild(row);
      }

      (legal ? legalEl : illegalEl).appendChild(wrap);
    }

    if (!legalEl.firstChild) {
      const empty = document.createElement("div");
      empty.className = "meta";
      empty.textContent = isTerm ? "Terminal state reached — all further commands are refused." : "No outgoing commands declared.";
      legalEl.appendChild(empty);
    }

    const pick = document.getElementById("branch-pick");
    const btns = document.getElementById("branch-btns");
    btns.textContent = "";
    if (pendingBranch) {
      pick.className = "branch-pick show";
      document.getElementById("branch-prompt").textContent =
        "Select an outcome for " + pendingBranch.cmd.name + " (runtime ik evaluates guards authoritatively):";
      for (const o of pendingBranch.outcomes) {
        const b = document.createElement("button");
        b.type = "button";
        b.className = "tool";
        b.textContent = "→ " + o.to + (o.when ? " [" + o.when + "]" : "");
        b.addEventListener("click", () => commitOutcome(pendingBranch.cmd, o, pendingBranch.args));
        btns.appendChild(b);
      }
    } else {
      pick.className = "branch-pick";
    }
  }

  function renderMeta() {
    document.getElementById("stat-state").textContent = current;
    document.getElementById("stat-cap").textContent = "gen " + generation + " · " + nonce;
    const isTerm = Boolean(stateMap.get(current) && stateMap.get(current).terminal);
    document.getElementById("stat-term").textContent = isTerm
      ? "LOCKED (terminal)"
      : (pendingEffect ? "LOCKED (effect pending)" : "Open");

    const effBar = document.getElementById("effect-bar");
    if (pendingEffect) {
      effBar.className = "effect-bar show";
      document.getElementById("effect-label").textContent =
        "Pending effect `" + pendingEffect.kind + "` bound to gen " + pendingEffect.generation + " — record outcome to unlock:";
    } else {
      effBar.className = "effect-bar";
    }

    const regBar = document.getElementById("regions-bar");
    regBar.textContent = "";
    if (SCHEMA.regions && SCHEMA.state_coords && SCHEMA.state_coords[current]) {
      regBar.className = "regions-bar show";
      const coords = SCHEMA.state_coords[current];
      SCHEMA.regions.forEach((r, idx) => {
        const chip = document.createElement("div");
        chip.className = "region-chip";
        const sp = document.createElement("span");
        sp.textContent = r.name + ": ";
        chip.appendChild(sp);
        chip.appendChild(document.createTextNode(coords[idx] || "?"));
        regBar.appendChild(chip);
      });
    } else {
      regBar.className = "regions-bar";
    }

    const badge = document.getElementById("stale-badge");
    const fireBtn = document.getElementById("btn-fire-stale");
    if (!savedHandle) {
      badge.textContent = "No copied handle";
      fireBtn.disabled = true;
    } else if (savedHandle.generation === generation) {
      badge.textContent = "Copied handle: gen " + savedHandle.generation + " (still current — advance state first)";
      fireBtn.disabled = false;
    } else {
      badge.textContent = "Copied handle: gen " + savedHandle.generation + " (STALE vs gen " + generation + ")";
      fireBtn.disabled = false;
    }

    const factsBox = document.getElementById("facts-box");
    factsBox.textContent = "";
    if ((SCHEMA.facts || []).length > 0) {
      const tbl = document.createElement("table");
      tbl.className = "facts-table";
      const thead = document.createElement("thead");
      thead.innerHTML = "<tr><th>Fact</th><th>Type / Policy</th><th>Current</th></tr>";
      tbl.appendChild(thead);
      const tbody = document.createElement("tbody");
      for (const f of SCHEMA.facts) {
        const tr = document.createElement("tr");
        const td1 = document.createElement("td"); td1.textContent = f.name;
        const td2 = document.createElement("td"); td2.textContent = f.type + " (" + f.policy + ")";
        const td3 = document.createElement("td"); td3.className = "val";
        td3.textContent = Object.prototype.hasOwnProperty.call(facts, f.name) ? JSON.stringify(facts[f.name]) : "unset";
        tr.appendChild(td1); tr.appendChild(td2); tr.appendChild(td3);
        tbody.appendChild(tr);
      }
      tbl.appendChild(tbody);
      factsBox.appendChild(tbl);
    } else {
      const none = document.createElement("div");
      none.className = "meta";
      none.textContent = "No facts declared.";
      factsBox.appendChild(none);
    }

    const invList = document.createElement("div");
    invList.className = "inv-list";
    const invs = SCHEMA.invariant_defs || (SCHEMA.invariants || []).map((n) => ({ name: n, assert: "" }));
    if (invs.length > 0) {
      for (const inv of invs) {
        const line = document.createElement("div");
        const code = document.createElement("code");
        code.textContent = inv.name;
        line.appendChild(code);
        if (inv.assert) {
          line.appendChild(document.createTextNode(": " + (inv.when ? "when (" + inv.when + ") " : "") + inv.assert));
        }
        invList.appendChild(line);
      }
    } else {
      invList.textContent = "Invariants: structural transition rules only";
    }
    factsBox.appendChild(invList);
    document.getElementById("inv-count").textContent = invs.length + " invariant(s)";

    const logEl = document.getElementById("event-log");
    logEl.textContent = "";
    document.getElementById("log-count").textContent = events.length + " event(s)";
    if (events.length === 0) {
      const empty = document.createElement("div");
      empty.className = "meta";
      empty.textContent = "Click any legal or illegal command to test the lifecycle boundary.";
      logEl.appendChild(empty);
    } else {
      for (const ev of events) {
        const row = document.createElement("div");
        row.className = "log-row " + ev.kind;
        const left = document.createElement("span");
        const pill = document.createElement("span");
        pill.className = "pill " + (ev.kind === "committed" ? "ok" : "bad");
        pill.textContent = ev.kind === "committed" ? "COMMIT" : "REFUSED";
        left.appendChild(pill);
        left.appendChild(document.createTextNode(" " + ev.label));
        const right = document.createElement("span");
        right.textContent = ev.detail;
        row.appendChild(left);
        row.appendChild(right);
        logEl.appendChild(row);
      }
    }
  }

  function recordEffect(outcome) {
    if (!pendingEffect) return;
    const eff = pendingEffect;
    pendingEffect = null;
    recordedEffect = { kind: eff.kind, outcome };
    generation += 1;
    nonce = nextNonce(generation);
    events.unshift({
      kind: "committed",
      label: "record(" + eff.kind + " = " + outcome + ")",
      detail: "gen " + generation + " · unlocked",
    });
    setBanner("committed", "✓ RECORDED: effect `" + eff.kind + "` " + outcome + " · capability advanced to gen " + generation + ".");
    renderAll();
  }

  function exportScenario() {
    const doc = {
      scenario_version: 1,
      machine: SCHEMA.source_path || (SCHEMA.machine.machine_id + ".ik.toml"),
      aggregate: SCHEMA.machine.machine_id + "-sim-1",
      steps: scenarioSteps,
    };
    const json = JSON.stringify(doc, null, 2) + "\n";
    const blob = new Blob([json], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = SCHEMA.machine.machine_id + ".scenario.json";
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
    setBanner("committed", "✓ EXPORTED: " + a.download + " (" + scenarioSteps.length + " steps) — run with `ik scenario run`.");
  }

  function renderAll() {
    renderGraph();
    renderCommands();
    renderMeta();
  }

  document.getElementById("btn-save-handle").addEventListener("click", () => {
    savedHandle = { state: current, generation, nonce };
    renderMeta();
  });

  document.getElementById("btn-fire-stale").addEventListener("click", () => {
    if (!savedHandle) return;
    const candidate = SCHEMA.commands.find((c) => c.from.includes(savedHandle.state)) || SCHEMA.commands[0];
    if (!candidate) return;
    if (savedHandle.generation !== generation) {
      refuseCommand(
        candidate,
        "IK_CAPABILITY_STALE",
        "copied handle held gen " + savedHandle.generation + " (" + savedHandle.nonce + "), but current aggregate is at gen " + generation + ".",
        {}
      );
    } else {
      setBanner("committed", "Copied handle is still at current gen " + generation + ". Advance the state first, then fire the copied handle to observe IK_CAPABILITY_STALE.");
    }
  });

  document.getElementById("btn-eff-ok").addEventListener("click", () => recordEffect("succeeded"));
  document.getElementById("btn-eff-fail").addEventListener("click", () => recordEffect("failed"));
  document.getElementById("btn-export").addEventListener("click", exportScenario);

  document.getElementById("btn-reset").addEventListener("click", () => {
    current = initialObj ? initialObj.name : "";
    facts = initialFacts();
    generation = 0;
    nonce = nextNonce(0);
    savedHandle = null;
    pendingEffect = null;
    recordedEffect = null;
    lastRefusal = null;
    pendingBranch = null;
    events = [];
    scenarioSteps = [];
    clearBanner();
    renderAll();
  });

  renderAll();
})();
</script>
</body>
</html>
"##;
