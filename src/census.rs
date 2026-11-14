//! Instance census: how many aggregates exist per machine version and state.
//!
//! The census is a read. It opens the store read-only, never creates, upgrades or
//! locks it for writing, and reports only what the `aggregates` table records. It does
//! not read fact values.

use crate::canonical::canonical_json;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: ik census <store.sqlite> [--json]";
const SCHEMA: &str = "ik.census.v1";
/// Store schema versions whose `aggregates` table this reader understands.
const READABLE: [&str; 2] = ["3", "4"];

/// Instances of one machine identity in one state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CensusRow {
    pub machine_id: String,
    pub machine_version: u64,
    pub machine_spec_hash: String,
    /// The state name, when the aggregate's state records one (declarative machines do).
    pub state: Option<String>,
    pub terminal: bool,
    pub count: u64,
}

/// Count instances per `(machine, version, spec hash, state, terminal)`.
pub fn read(path: &Path) -> Result<Vec<CensusRow>, String> {
    if !path.is_file() {
        return Err(format!("{} is not a store file", path.display()));
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    check_schema(&conn, path)?;
    let mut stmt = conn
        .prepare("SELECT machine_id, machine_version, machine_spec_hash, state, terminal FROM aggregates")
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut counts: Vec<CensusRow> = Vec::new();
    for row in rows {
        let (machine_id, version, hash, state, terminal) =
            row.map_err(|e| format!("read {}: {e}", path.display()))?;
        add(&mut counts, machine_id, version, hash, &state, terminal != 0);
    }
    counts.sort_by(|a, b| {
        (&a.machine_id, a.machine_version, &a.state, a.terminal)
            .cmp(&(&b.machine_id, b.machine_version, &b.state, b.terminal))
    });
    Ok(counts)
}

fn check_schema(conn: &Connection, path: &Path) -> Result<(), String> {
    let version: String = conn
        .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |row| row.get(0))
        .map_err(|e| format!("{} is not a Desmo store: {e}", path.display()))?;
    if READABLE.contains(&version.as_str()) {
        Ok(())
    } else {
        Err(format!("unknown store schema version `{version}`"))
    }
}

fn add(
    counts: &mut Vec<CensusRow>,
    machine_id: String,
    version: i64,
    hash: String,
    state_json: &str,
    terminal: bool,
) {
    let state = state_name(state_json);
    let version = u64::try_from(version).unwrap_or(0);
    let found = counts.iter_mut().find(|item| {
        item.machine_id == machine_id
            && item.machine_version == version
            && item.machine_spec_hash == hash
            && item.state == state
            && item.terminal == terminal
    });
    match found {
        Some(item) => item.count += 1,
        None => counts.push(CensusRow {
            machine_id,
            machine_version: version,
            machine_spec_hash: hash,
            state,
            terminal,
            count: 1,
        }),
    }
}

fn state_name(state_json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(state_json).ok()?;
    value.get("state")?.as_str().map(str::to_string)
}

/// The census as `ik.census.v1`.
pub fn to_value(store: &Path, rows: &[CensusRow]) -> Value {
    json!({
        "schema": SCHEMA,
        "store": store.display().to_string(),
        "instances": rows.iter().map(|row| json!({
            "machine_id": row.machine_id,
            "machine_version": row.machine_version,
            "machine_spec_hash": row.machine_spec_hash,
            "state": row.state,
            "terminal": row.terminal,
            "count": row.count,
        })).collect::<Vec<_>>(),
    })
}

/// CLI entry for `ik census`.
pub fn run(args: &[String]) -> i32 {
    let (path, json_out) = match parse(args) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(message) => {
            eprintln!("ik census: {message}\n{USAGE}");
            return 2;
        }
    };
    match read(&path) {
        Ok(rows) if json_out => {
            println!("{}", canonical_json(&to_value(&path, &rows)));
            0
        }
        Ok(rows) => {
            print!("{}", text(&path, &rows));
            0
        }
        Err(message) => {
            eprintln!("ik census: {message}");
            2
        }
    }
}

fn parse(args: &[String]) -> Result<Option<(PathBuf, bool)>, String> {
    let mut path: Option<PathBuf> = None;
    let mut json_out = false;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--json" => json_out = true,
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            other if path.is_none() => path = Some(PathBuf::from(other)),
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    Ok(Some((path.ok_or("a store path is required")?, json_out)))
}

fn text(path: &Path, rows: &[CensusRow]) -> String {
    let mut out = format!("store {}\n\n", path.display());
    if rows.is_empty() {
        out.push_str("no instances\n");
        return out;
    }
    let table: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            vec![
                row.machine_id.clone(),
                row.machine_version.to_string(),
                row.state.clone().unwrap_or_else(|| "(not named)".into()),
                if row.terminal { "finished" } else { "unfinished" }.to_string(),
                row.count.to_string(),
            ]
        })
        .collect();
    out.push_str(&crate::diagram::md_table(
        &["machine", "version", "state", "lifecycle", "instances"],
        &table,
    ));
    out
}
