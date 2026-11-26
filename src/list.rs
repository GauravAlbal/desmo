//! Read-only aggregate query (`ik list`) filtered by machine, state, or effect status.
//!
//! Opens the SQLite store strictly read-only inside one snapshot transaction and queries the
//! materialized `aggregates` table without creating, upgrading, or mutating the store.

use crate::canonical::canonical_json;
use crate::effect::EffectStatus;
use rusqlite::{types::Value as SqlValue, Connection, OpenFlags};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: ik list <store.sqlite> [--machine <id>] [--state <name>] [--effect none|pending|recorded] [--terminal true|false] [--after <aggregate_id>] [--limit <n>] [--json]";
const SCHEMA: &str = "ik.list.v1";
const READABLE: [&str; 2] = ["3", "4"];
const DEFAULT_LIMIT: u64 = 100;
const MAX_LIMIT: u64 = 10_000;

/// Filter parameters for [`read`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListQuery {
    pub machine_id: Option<String>,
    pub state: Option<String>,
    pub effect: Option<String>,
    pub terminal: Option<bool>,
    pub after: Option<String>,
    pub limit: Option<u64>,
}

/// Summary row for one aggregate returned by `ik list`.
#[derive(Clone, Debug, PartialEq)]
pub struct AggregateSummary {
    pub aggregate_id: String,
    pub machine_id: String,
    pub machine_version: u64,
    pub machine_spec_hash: String,
    pub state: Value,
    pub state_name: Option<String>,
    pub terminal: bool,
    pub generation: u64,
    pub sequence: u64,
    pub effect: EffectStatus,
    pub effect_status: &'static str,
}

/// Paginated aggregate list returned by [`read`].
#[derive(Clone, Debug, PartialEq)]
pub struct AggregateList {
    pub next_after: Option<String>,
    pub has_more: bool,
    pub aggregates: Vec<AggregateSummary>,
}

fn effect_status_str(effect: &EffectStatus) -> &'static str {
    match effect {
        EffectStatus::None => "none",
        EffectStatus::Pending(_) => "pending",
        EffectStatus::Recorded(_, _, _) => "recorded",
    }
}

/// List aggregates matching `query` inside one read-only snapshot transaction.
pub fn read(path: &Path, query: &ListQuery) -> Result<AggregateList, String> {
    if !path.is_file() {
        return Err(format!("{} is not a store file", path.display()));
    }
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT);
    if limit == 0 || limit > MAX_LIMIT {
        return Err(format!("--limit must be between 1 and {MAX_LIMIT}"));
    }
    if let Some(eff) = query.effect.as_deref() {
        if !matches!(eff, "none" | "pending" | "recorded") {
            return Err("--effect must be `none`, `pending`, or `recorded`".into());
        }
    }

    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    check_schema(&tx, path)?;

    let mut clauses = Vec::new();
    let mut sql_params: Vec<SqlValue> = Vec::new();
    if let Some(machine_id) = &query.machine_id {
        sql_params.push(SqlValue::Text(machine_id.clone()));
        clauses.push(format!("machine_id = ?{}", sql_params.len()));
    }
    if let Some(terminal) = query.terminal {
        sql_params.push(SqlValue::Integer(if terminal { 1 } else { 0 }));
        clauses.push(format!("terminal = ?{}", sql_params.len()));
    }
    if let Some(after) = &query.after {
        sql_params.push(SqlValue::Text(after.clone()));
        clauses.push(format!("aggregate_id > ?{}", sql_params.len()));
    }

    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    let sql = format!(
        "SELECT aggregate_id, machine_id, machine_version, machine_spec_hash, state, terminal, generation, effect_json, sequence \
         FROM aggregates{where_sql} ORDER BY aggregate_id ASC"
    );

    let mut stmt = tx
        .prepare(&sql)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut rows = stmt
        .query(rusqlite::params_from_iter(sql_params))
        .map_err(|e| format!("read {}: {e}", path.display()))?;

    let want = limit as usize;
    let mut aggregates = Vec::with_capacity(want.min(128));
    let mut has_more = false;

    while let Some(row) = rows
        .next()
        .map_err(|e| format!("read {}: {e}", path.display()))?
    {
        let aggregate_id: String = row.get(0).map_err(|e| format!("read {}: {e}", path.display()))?;
        let machine_id: String = row.get(1).map_err(|e| format!("read {}: {e}", path.display()))?;
        let raw_version: i64 = row.get(2).map_err(|e| format!("read {}: {e}", path.display()))?;
        let machine_spec_hash: String =
            row.get(3).map_err(|e| format!("read {}: {e}", path.display()))?;
        let state_json: String = row.get(4).map_err(|e| format!("read {}: {e}", path.display()))?;
        let raw_terminal: i64 = row.get(5).map_err(|e| format!("read {}: {e}", path.display()))?;
        let raw_gen: i64 = row.get(6).map_err(|e| format!("read {}: {e}", path.display()))?;
        let effect_json: String = row.get(7).map_err(|e| format!("read {}: {e}", path.display()))?;
        let raw_seq: i64 = row.get(8).map_err(|e| format!("read {}: {e}", path.display()))?;

        let machine_version = u64::try_from(raw_version).map_err(|_| {
            format!("damaged store: negative machine_version {raw_version} for `{aggregate_id}`")
        })?;
        let generation = u64::try_from(raw_gen).map_err(|_| {
            format!("damaged store: negative generation {raw_gen} for `{aggregate_id}`")
        })?;
        let sequence = u64::try_from(raw_seq).map_err(|_| {
            format!("damaged store: negative sequence {raw_seq} for `{aggregate_id}`")
        })?;

        let state: Value = serde_json::from_str(&state_json).map_err(|e| {
            format!("damaged projection state for aggregate `{aggregate_id}`: {e}")
        })?;
        let state_name = state
            .get("state")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(wanted_state) = &query.state {
            if state_name.as_deref() != Some(wanted_state.as_str()) {
                continue;
            }
        }

        let effect: EffectStatus = serde_json::from_str(&effect_json).map_err(|e| {
            format!("damaged effect status for aggregate `{aggregate_id}`: {e}")
        })?;
        let effect_status = effect_status_str(&effect);
        if let Some(wanted_effect) = &query.effect {
            if effect_status != wanted_effect {
                continue;
            }
        }

        if aggregates.len() == want {
            has_more = true;
            break;
        }
        aggregates.push(AggregateSummary {
            aggregate_id,
            machine_id,
            machine_version,
            machine_spec_hash,
            state,
            state_name,
            terminal: raw_terminal != 0,
            generation,
            sequence,
            effect,
            effect_status,
        });
    }

    let next_after = aggregates.last().map(|r| r.aggregate_id.clone());
    Ok(AggregateList {
        next_after,
        has_more,
        aggregates,
    })
}

fn check_schema(conn: &Connection, path: &Path) -> Result<(), String> {
    let version: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("{} is not a Desmo store: {e}", path.display()))?;
    if READABLE.contains(&version.as_str()) {
        Ok(())
    } else {
        Err(format!("unknown store schema version `{version}`"))
    }
}

/// Convert an [`AggregateList`] into `ik.list.v1` JSON.
pub fn to_value(store: &Path, list: &AggregateList) -> Value {
    json!({
        "schema": SCHEMA,
        "store": store.display().to_string(),
        "next_after": list.next_after,
        "has_more": list.has_more,
        "aggregates": list.aggregates.iter().map(|row| json!({
            "aggregate_id": row.aggregate_id,
            "machine_id": row.machine_id,
            "machine_version": row.machine_version,
            "machine_spec_hash": row.machine_spec_hash,
            "state": row.state,
            "state_name": row.state_name,
            "terminal": row.terminal,
            "generation": row.generation,
            "sequence": row.sequence,
            "effect": row.effect,
            "effect_status": row.effect_status,
        })).collect::<Vec<_>>(),
    })
}

struct Args {
    path: PathBuf,
    query: ListQuery,
    json: bool,
}

/// CLI entry for `ik list`.
pub fn run(args: &[String]) -> i32 {
    let parsed = match parse(args) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(msg) => {
            eprintln!("ik list: {msg}\n{USAGE}");
            return 2;
        }
    };
    match read(&parsed.path, &parsed.query) {
        Ok(list) if parsed.json => {
            println!("{}", canonical_json(&to_value(&parsed.path, &list)));
            0
        }
        Ok(list) => {
            print!("{}", text(&parsed.path, &list));
            0
        }
        Err(msg) => {
            eprintln!("ik list: {msg}");
            2
        }
    }
}

fn parse(args: &[String]) -> Result<Option<Args>, String> {
    let mut path: Option<PathBuf> = None;
    let mut query = ListQuery::default();
    let mut json_out = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => return Ok(None),
            "--json" => json_out = true,
            "--machine" => {
                i += 1;
                query.machine_id = Some(args.get(i).ok_or("--machine requires a machine id")?.clone());
            }
            "--state" => {
                i += 1;
                query.state = Some(args.get(i).ok_or("--state requires a state name")?.clone());
            }
            "--effect" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or("--effect requires `none`, `pending`, or `recorded`")?;
                if !matches!(val.as_str(), "none" | "pending" | "recorded") {
                    return Err("--effect must be `none`, `pending`, or `recorded`".into());
                }
                query.effect = Some(val.clone());
            }
            "--terminal" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or("--terminal requires `true` or `false`")?;
                let parsed = match val.as_str() {
                    "true" => true,
                    "false" => false,
                    _ => return Err("--terminal must be `true` or `false`".into()),
                };
                query.terminal = Some(parsed);
            }
            "--after" => {
                i += 1;
                query.after = Some(args.get(i).ok_or("--after requires an aggregate id")?.clone());
            }
            "--limit" => {
                i += 1;
                let raw = args.get(i).ok_or("--limit requires a positive integer")?;
                let n = raw
                    .parse::<u64>()
                    .map_err(|e| format!("--limit must be a positive integer ({e})"))?;
                query.limit = Some(n);
            }
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            other if path.is_none() => path = Some(PathBuf::from(other)),
            other => return Err(format!("unexpected argument `{other}`")),
        }
        i += 1;
    }
    Ok(Some(Args {
        path: path.ok_or("a store path is required")?,
        query,
        json: json_out,
    }))
}

fn text(store: &Path, list: &AggregateList) -> String {
    let mut out = format!(
        "store {} · {} aggregate(s) (has_more={})\n\n",
        store.display(),
        list.aggregates.len(),
        list.has_more
    );
    if list.aggregates.is_empty() {
        out.push_str("no matching aggregates\n");
        return out;
    }
    let rows: Vec<Vec<String>> = list
        .aggregates
        .iter()
        .map(|row| {
            vec![
                row.aggregate_id.clone(),
                format!("{} v{}", row.machine_id, row.machine_version),
                row.state_name.clone().unwrap_or_else(|| "(custom)".into()),
                row.generation.to_string(),
                row.effect_status.to_string(),
                if row.terminal { "terminal" } else { "active" }.to_string(),
            ]
        })
        .collect();
    out.push_str(&crate::diagram::md_table(
        &["aggregate", "machine", "state", "gen", "effect", "lifecycle"],
        &rows,
    ));
    if list.has_more {
        if let Some(cursor) = &list.next_after {
            out.push_str(&format!("\nNext page: --after {cursor}\n"));
        }
    }
    out
}
