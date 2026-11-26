//! Read-only ordered event cursor (`ik events`) for brownfield read-model projections.
//!
//! Opens the SQLite store strictly read-only inside one snapshot transaction, queries
//! journal rows by monotonic `rowid` cursor, and joins the aggregate's current projection
//! so external read models can execute idempotent generation-fenced upserts.

use crate::canonical::canonical_json;
use crate::runtime::{EventKind, JournalEvent};
use rusqlite::{params, Connection, OpenFlags};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const USAGE: &str =
    "usage: ik events <store.sqlite> [--since <cursor>] [--aggregate <id>] [--limit <n>] [--json]";
const SCHEMA: &str = "ik.events.v1";
const READABLE: [&str; 2] = ["3", "4"];
const DEFAULT_LIMIT: u64 = 100;
const MAX_LIMIT: u64 = 10_000;

/// One ordered journal event plus the aggregate's latest projection snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct EventCursorRow {
    pub cursor: u64,
    pub event: JournalEvent,
    pub is_latest_for_aggregate: bool,
    pub current_state: Value,
    pub current_sequence: u64,
    pub current_generation: u64,
    pub current_terminal: bool,
}

/// Paginated cursor feed returned by [`read`].
#[derive(Clone, Debug, PartialEq)]
pub struct EventFeed {
    pub since: u64,
    pub next_cursor: u64,
    pub has_more: bool,
    pub events: Vec<EventCursorRow>,
}

/// Read ordered events with `rowid > since` inside a single read-only snapshot.
pub fn read(
    path: &Path,
    since: u64,
    aggregate: Option<&str>,
    limit: u64,
) -> Result<EventFeed, String> {
    if !path.is_file() {
        return Err(format!("{} is not a store file", path.display()));
    }
    if limit == 0 || limit > MAX_LIMIT {
        return Err(format!("--limit must be between 1 and {MAX_LIMIT}"));
    }
    let since_i64 =
        i64::try_from(since).map_err(|_| format!("--since `{since}` exceeds SQLite i64 range"))?;
    let fetch_limit = i64::try_from(limit + 1).map_err(|_| "--limit is too large".to_string())?;

    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    check_schema(&tx, path)?;

    let sql = if aggregate.is_some() {
        "SELECT j.rowid, j.event_json, a.state, a.sequence, a.generation, a.terminal \
         FROM journal j JOIN aggregates a ON a.aggregate_id = j.aggregate_id \
         WHERE j.rowid > ?1 AND j.aggregate_id = ?2 \
         ORDER BY j.rowid ASC LIMIT ?3"
    } else {
        "SELECT j.rowid, j.event_json, a.state, a.sequence, a.generation, a.terminal \
         FROM journal j JOIN aggregates a ON a.aggregate_id = j.aggregate_id \
         WHERE j.rowid > ?1 \
         ORDER BY j.rowid ASC LIMIT ?2"
    };

    let mut stmt = tx
        .prepare(sql)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(i64, String, String, i64, i64, i64)> {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        ))
    };
    let raw_rows: Vec<(i64, String, String, i64, i64, i64)> = if let Some(agg) = aggregate {
        stmt.query_map(params![since_i64, agg, fetch_limit], map_row)
            .map_err(|e| format!("read {}: {e}", path.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read {}: {e}", path.display()))?
    } else {
        stmt.query_map(params![since_i64, fetch_limit], map_row)
            .map_err(|e| format!("read {}: {e}", path.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read {}: {e}", path.display()))?
    };

    let has_more = raw_rows.len() > limit as usize;
    let mut events = Vec::with_capacity(raw_rows.len().min(limit as usize));
    for (rowid, event_json, state_json, cur_seq, cur_gen, cur_term) in
        raw_rows.into_iter().take(limit as usize)
    {
        let cursor = u64::try_from(rowid)
            .map_err(|_| format!("damaged store: negative journal rowid {rowid}"))?;
        let current_sequence = u64::try_from(cur_seq)
            .map_err(|_| format!("damaged store: negative sequence {cur_seq}"))?;
        let current_generation = u64::try_from(cur_gen)
            .map_err(|_| format!("damaged store: negative generation {cur_gen}"))?;
        let event: JournalEvent = serde_json::from_str(&event_json)
            .map_err(|e| format!("damaged journal event at cursor {cursor}: {e}"))?;
        let current_state: Value = serde_json::from_str(&state_json)
            .map_err(|e| format!("damaged projection state at cursor {cursor}: {e}"))?;
        let is_latest_for_aggregate = event.sequence == current_sequence;
        events.push(EventCursorRow {
            cursor,
            event,
            is_latest_for_aggregate,
            current_state,
            current_sequence,
            current_generation,
            current_terminal: cur_term != 0,
        });
    }
    let next_cursor = events.last().map(|r| r.cursor).unwrap_or(since);
    Ok(EventFeed {
        since,
        next_cursor,
        has_more,
        events,
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

fn kind_str(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Genesis => "genesis",
        EventKind::DomainCommand => "domain_command",
        EventKind::KernelRecord => "kernel_record",
    }
}

/// Convert an [`EventFeed`] into `ik.events.v1` JSON.
pub fn to_value(store: &Path, feed: &EventFeed) -> Value {
    json!({
        "schema": SCHEMA,
        "store": store.display().to_string(),
        "since": feed.since,
        "next_cursor": feed.next_cursor,
        "has_more": feed.has_more,
        "events": feed.events.iter().map(|row| json!({
            "cursor": row.cursor,
            "aggregate_id": row.event.aggregate_id.0,
            "sequence": row.event.sequence,
            "generation": row.event.generation,
            "machine_id": row.event.machine_id,
            "machine_version": row.event.machine_version,
            "machine_spec_hash": row.event.machine_spec_hash,
            "event_kind": kind_str(row.event.event_kind),
            "event_type": row.event.event_type,
            "event_version": row.event.event_version,
            "command": row.event.command,
            "state_digest": row.event.state_digest,
            "terminal": row.event.terminal,
            "effect_delta": row.event.effect_delta,
            "event_hash": row.event.hash,
            "prev_event_hash": row.event.prev_hash,
            "is_latest_for_aggregate": row.is_latest_for_aggregate,
            "current_projection": {
                "state": row.current_state,
                "sequence": row.current_sequence,
                "generation": row.current_generation,
                "terminal": row.current_terminal,
            }
        })).collect::<Vec<_>>(),
    })
}

struct Args {
    path: PathBuf,
    since: u64,
    aggregate: Option<String>,
    limit: u64,
    json: bool,
}

/// CLI entry for `ik events`.
pub fn run(args: &[String]) -> i32 {
    let parsed = match parse(args) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(msg) => {
            eprintln!("ik events: {msg}\n{USAGE}");
            return 2;
        }
    };
    match read(&parsed.path, parsed.since, parsed.aggregate.as_deref(), parsed.limit) {
        Ok(feed) if parsed.json => {
            println!("{}", canonical_json(&to_value(&parsed.path, &feed)));
            0
        }
        Ok(feed) => {
            print!("{}", text(&parsed.path, &feed));
            0
        }
        Err(msg) => {
            eprintln!("ik events: {msg}");
            2
        }
    }
}

fn parse(args: &[String]) -> Result<Option<Args>, String> {
    let mut path: Option<PathBuf> = None;
    let mut since = 0u64;
    let mut aggregate: Option<String> = None;
    let mut limit = DEFAULT_LIMIT;
    let mut json_out = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => return Ok(None),
            "--json" => json_out = true,
            "--since" => {
                i += 1;
                let raw = args.get(i).ok_or("--since requires a cursor integer")?;
                since = raw
                    .parse::<u64>()
                    .map_err(|e| format!("--since must be a non-negative integer ({e})"))?;
            }
            "--aggregate" => {
                i += 1;
                aggregate = Some(args.get(i).ok_or("--aggregate requires an id")?.clone());
            }
            "--limit" => {
                i += 1;
                let raw = args.get(i).ok_or("--limit requires a positive integer")?;
                limit = raw
                    .parse::<u64>()
                    .map_err(|e| format!("--limit must be a positive integer ({e})"))?;
            }
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            other if path.is_none() => path = Some(PathBuf::from(other)),
            other => return Err(format!("unexpected argument `{other}`")),
        }
        i += 1;
    }
    Ok(Some(Args {
        path: path.ok_or("a store path is required")?,
        since,
        aggregate,
        limit,
        json: json_out,
    }))
}

fn text(store: &Path, feed: &EventFeed) -> String {
    let mut out = format!(
        "store {} · cursor {} -> {} (returned {}, has_more={})\n\n",
        store.display(),
        feed.since,
        feed.next_cursor,
        feed.events.len(),
        feed.has_more
    );
    if feed.events.is_empty() {
        out.push_str("no events after cursor\n");
        return out;
    }
    let rows: Vec<Vec<String>> = feed
        .events
        .iter()
        .map(|row| {
            vec![
                row.cursor.to_string(),
                row.event.aggregate_id.0.clone(),
                row.event.sequence.to_string(),
                row.event.generation.to_string(),
                kind_str(row.event.event_kind).to_string(),
                row.event.event_type.clone(),
                if row.is_latest_for_aggregate {
                    "yes"
                } else {
                    "no"
                }
                .to_string(),
            ]
        })
        .collect();
    out.push_str(&crate::diagram::md_table(
        &["cursor", "aggregate", "seq", "gen", "kind", "event", "latest"],
        &rows,
    ));
    out
}
