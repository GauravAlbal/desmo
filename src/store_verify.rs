//! Offline, read-only whole-store integrity verification (`ik store verify`).

use crate::canonical::canonical_json;
use crate::machine::Machine;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: ik store verify <store.sqlite> [--machine <m.ik.toml>]... [--json]";
const SCHEMA: &str = "ik.store_verify.v1";

struct VerifyArgs {
    store_path: PathBuf,
    machine_paths: Vec<PathBuf>,
    json: bool,
}

fn parse_args(args: &[String]) -> Result<Option<VerifyArgs>, String> {
    if args.is_empty() || matches!(args[0].as_str(), "-h" | "--help") {
        return Ok(None);
    }
    if args[0] != "verify" {
        return Err(format!("unknown `ik store` subcommand `{}`\n{USAGE}", args[0]));
    }
    let rest = &args[1..];
    if rest.iter().any(|a| a == "-h" || a == "--help") {
        return Ok(None);
    }
    let mut store_path: Option<PathBuf> = None;
    let mut machine_paths = Vec::new();
    let mut json = false;
    let mut i = 0;
    while i < rest.len() {
        let arg = &rest[i];
        if arg == "--json" {
            json = true;
        } else if arg == "--machine" {
            i += 1;
            let val = rest
                .get(i)
                .ok_or_else(|| format!("--machine requires a path\n{USAGE}"))?;
            machine_paths.push(PathBuf::from(val));
        } else if let Some(val) = arg.strip_prefix("--machine=") {
            if val.is_empty() {
                return Err(format!("--machine requires a path\n{USAGE}"));
            }
            machine_paths.push(PathBuf::from(val));
        } else if arg.starts_with('-') {
            return Err(format!("unknown option `{arg}`\n{USAGE}"));
        } else if store_path.is_none() {
            store_path = Some(PathBuf::from(arg));
        } else {
            return Err(format!("unexpected argument `{arg}`\n{USAGE}"));
        }
        i += 1;
    }
    let store_path = store_path.ok_or_else(|| format!("missing <store.sqlite>\n{USAGE}"))?;
    Ok(Some(VerifyArgs {
        store_path,
        machine_paths,
        json,
    }))
}

fn load_machine(path: &Path) -> Result<Box<dyn Machine>, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let source = crate::declarative::region::parse_source_with_limits(
        &src,
        crate::declarative::region::RegionLimits::default(),
    )
    .map_err(|e| format!("parse {}: {}", path.display(), e.join("; ")))?;
    let compiled = crate::declarative::compile::compile(source.definition)
        .map_err(|e| format!("compile {}: {}", path.display(), e.join("; ")))?;
    Ok(Box::new(compiled))
}

fn emit_json(value: &Value) {
    println!("{}", canonical_json(value));
}

pub fn run(args: &[String]) -> i32 {
    let parsed = match parse_args(args) {
        Ok(Some(v)) => v,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(msg) => {
            eprintln!("{msg}");
            return 2;
        }
    };

    if !parsed.store_path.is_file() {
        let msg = format!("{} is not a store file", parsed.store_path.display());
        if parsed.json {
            emit_json(&json!({
                "schema": SCHEMA,
                "verified": false,
                "error": { "code": "IK_STORE_CORRUPT", "message": msg },
            }));
        } else {
            eprintln!("error: {msg}");
        }
        return 1;
    }

    let mut loaded_machines = Vec::with_capacity(parsed.machine_paths.len());
    for mpath in &parsed.machine_paths {
        match load_machine(mpath) {
            Ok(m) => loaded_machines.push(m),
            Err(msg) => {
                if parsed.json {
                    emit_json(&json!({
                        "schema": SCHEMA,
                        "verified": false,
                        "error": { "code": "IK_INVALID_SPEC", "message": msg },
                    }));
                } else {
                    eprintln!("error: {msg}");
                }
                return 1;
            }
        }
    }

    let conn = match Connection::open_with_flags(&parsed.store_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
    {
        Ok(c) => c,
        Err(err) => {
            let msg = format!("open {}: {err}", parsed.store_path.display());
            if parsed.json {
                emit_json(&json!({
                    "schema": SCHEMA,
                    "verified": false,
                    "error": { "code": "IK_STORE_CORRUPT", "message": msg },
                }));
            } else {
                eprintln!("error: {msg}");
            }
            return 1;
        }
    };

    let machines_opt = if loaded_machines.is_empty() {
        None
    } else {
        Some(loaded_machines.as_slice())
    };

    match crate::store::verify_connection_readonly(&conn, machines_opt) {
        Ok(report) => {
            if parsed.json {
                emit_json(&json!({
                    "schema": SCHEMA,
                    "verified": true,
                    "mode": report.mode,
                    "store_id": report.store_id,
                    "aggregates": report.aggregates,
                    "events": report.events,
                    "operations": report.operations,
                }));
            } else {
                println!(
                    "verified {} store={} aggregates={} events={} operations={} ({})",
                    parsed.store_path.display(),
                    report.store_id,
                    report.aggregates,
                    report.events,
                    report.operations,
                    report.mode,
                );
            }
            0
        }
        Err(diag) => {
            if parsed.json {
                emit_json(&json!({
                    "schema": SCHEMA,
                    "verified": false,
                    "error": {
                        "code": diag.code.as_str(),
                        "rule": diag.rule,
                        "message": diag.message,
                    },
                }));
            } else {
                eprintln!("error [{}]: {}", diag.code.as_str(), diag.message);
            }
            1
        }
    }
}
