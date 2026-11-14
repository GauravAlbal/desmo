//! Recorded operations of one aggregate: discovery for a caller that lost track of what
//! it sent. This is a listing, not a lookup.
//!
//! It opens the store read-only and reads the aggregate's position and its operation rows
//! inside one read transaction, so both describe the same store snapshot. It never creates,
//! upgrades or writes. The listing is a window: the most recent `limit` rows. An operation
//! missing from the window has no established outcome here. It may be older than the window,
//! still running, never delivered, refused, or a no-op (`docs/operation-identity.md`). Only an
//! exact lookup by operation id, with the original request, can answer "did this commit".

use crate::canonical::canonical_json;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: ik operations <store.sqlite> --aggregate <id> [--limit <n>] [--json]";
const SCHEMA: &str = "ik.operations.v1";
const DEFAULT_LIMIT: u32 = 20;

/// One recorded operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationRow {
    pub operation_id: String,
    pub kind: String,
    pub first_sequence: u64,
    pub last_sequence: u64,
    /// The generation the aggregate reached when this operation committed.
    pub generation: u64,
}

/// The aggregate's position and a window of its most recent operations, newest first, all
/// observed at one store snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub current_generation: u64,
    pub current_sequence: u64,
    pub operations: Vec<OperationRow>,
    /// The window size that was asked for.
    pub limit: u32,
    /// True when no recorded operation is older than the window.
    pub complete: bool,
}

/// Read the most recent `limit` operations of `aggregate_id` at one snapshot.
pub fn read(path: &Path, aggregate_id: &str, limit: u32) -> Result<Listing, String> {
    if !path.is_file() {
        return Err(format!("{} is not a store file", path.display()));
    }
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    check_schema(&tx, path)?;
    let (generation, sequence) = position(&tx, aggregate_id)?;
    let mut operations = rows(&tx, aggregate_id, limit, path)?;
    let complete = operations.len() <= limit as usize;
    operations.truncate(limit as usize);
    Ok(Listing {
        current_generation: generation,
        current_sequence: sequence,
        operations,
        limit,
        complete,
    })
}

/// Up to `limit + 1` rows, so the caller can tell whether older rows exist.
fn rows(
    conn: &Connection,
    aggregate_id: &str,
    limit: u32,
    path: &Path,
) -> Result<Vec<OperationRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT operation_id, kind, first_sequence, last_sequence, generation FROM operations WHERE aggregate_id = ?1 ORDER BY last_sequence DESC LIMIT ?2",
        )
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let found = stmt
        .query_map(rusqlite::params![aggregate_id, i64::from(limit) + 1], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut operations = Vec::new();
    for row in found {
        let (operation_id, kind, first, last, generation) =
            row.map_err(|e| format!("read {}: {e}", path.display()))?;
        operations.push(OperationRow {
            first_sequence: coordinate("first_sequence", &operation_id, first)?,
            last_sequence: coordinate("last_sequence", &operation_id, last)?,
            generation: coordinate("generation", &operation_id, generation)?,
            operation_id,
            kind,
        });
    }
    Ok(operations)
}

/// A stored coordinate must be non-negative. A negative value is a damaged store, never a
/// legitimate initial position, so it is an error and not zero.
fn coordinate(field: &str, subject: &str, value: i64) -> Result<u64, String> {
    u64::try_from(value).map_err(|e| {
        format!("store is damaged: `{subject}` has a negative {field} ({value}): {e}")
    })
}

fn check_schema(conn: &Connection, path: &Path) -> Result<(), String> {
    let version: String = conn
        .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |row| row.get(0))
        .map_err(|e| format!("{} is not a Desmo store: {e}", path.display()))?;
    if version == "4" {
        Ok(())
    } else {
        Err(format!(
            "store schema version `{version}` has no operations table (schema 4 is required)"
        ))
    }
}

fn position(conn: &Connection, aggregate_id: &str) -> Result<(u64, u64), String> {
    let (generation, sequence) = conn
        .query_row(
            "SELECT generation, sequence FROM aggregates WHERE aggregate_id = ?1",
            rusqlite::params![aggregate_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                format!("no aggregate `{aggregate_id}` in the store")
            }
            other => format!("read aggregate `{aggregate_id}`: {other}"),
        })?;
    Ok((
        coordinate("generation", aggregate_id, generation)?,
        coordinate("sequence", aggregate_id, sequence)?,
    ))
}

/// The listing as `ik.operations.v1`.
pub fn to_value(store: &Path, aggregate_id: &str, listing: &Listing) -> Value {
    json!({
        "schema": SCHEMA,
        "store": store.display().to_string(),
        "aggregate_id": aggregate_id,
        "snapshot": {
            "generation": listing.current_generation,
            "sequence": listing.current_sequence,
        },
        "window": {
            "limit": listing.limit,
            "returned": listing.operations.len(),
            "complete": listing.complete,
        },
        "operations": listing.operations.iter().map(|row| json!({
            "operation_id": row.operation_id,
            "kind": row.kind,
            "first_sequence": row.first_sequence,
            "last_sequence": row.last_sequence,
            "generation": row.generation,
        })).collect::<Vec<_>>(),
    })
}

struct Args {
    path: PathBuf,
    aggregate: String,
    limit: u32,
    json: bool,
}

/// CLI entry for `ik operations`.
pub fn run(args: &[String]) -> i32 {
    let parsed = match parse(args) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(message) => {
            eprintln!("ik operations: {message}\n{USAGE}");
            return 2;
        }
    };
    match read(&parsed.path, &parsed.aggregate, parsed.limit) {
        Ok(listing) if parsed.json => {
            println!(
                "{}",
                canonical_json(&to_value(&parsed.path, &parsed.aggregate, &listing))
            );
            0
        }
        Ok(listing) => {
            print!("{}", text(&parsed.aggregate, &listing));
            0
        }
        Err(message) => {
            eprintln!("ik operations: {message}");
            2
        }
    }
}

fn parse(args: &[String]) -> Result<Option<Args>, String> {
    let mut path: Option<PathBuf> = None;
    let mut aggregate: Option<String> = None;
    let mut limit = DEFAULT_LIMIT;
    let mut json_out = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => return Ok(None),
            "--json" => json_out = true,
            "--aggregate" => {
                i += 1;
                aggregate = Some(args.get(i).ok_or("--aggregate requires an id")?.clone());
            }
            "--limit" => {
                i += 1;
                limit = args
                    .get(i)
                    .ok_or("--limit requires a number")?
                    .parse()
                    .map_err(|e| format!("--limit must be a non-negative integer ({e})"))?;
            }
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            other if path.is_none() => path = Some(PathBuf::from(other)),
            other => return Err(format!("unexpected argument `{other}`")),
        }
        i += 1;
    }
    Ok(Some(Args {
        path: path.ok_or("a store path is required")?,
        aggregate: aggregate.ok_or("--aggregate is required")?,
        limit,
        json: json_out,
    }))
}

fn text(aggregate_id: &str, listing: &Listing) -> String {
    let mut out = format!(
        "aggregate {aggregate_id} at this store snapshot: generation {}, sequence {}\n",
        listing.current_generation, listing.current_sequence
    );
    out.push_str(&format!(
        "showing the {} most recent recorded operations ({} returned; {})\n\n",
        listing.limit,
        listing.operations.len(),
        if listing.complete {
            "nothing older is recorded"
        } else {
            "older operations exist and are not shown"
        }
    ));
    if listing.operations.is_empty() {
        out.push_str("no recorded operations\n");
    } else {
        let rows: Vec<Vec<String>> = listing
            .operations
            .iter()
            .map(|row| {
                vec![
                    row.operation_id.clone(),
                    row.kind.clone(),
                    row.generation.to_string(),
                    format!("{}-{}", row.first_sequence, row.last_sequence),
                ]
            })
            .collect();
        out.push_str(&crate::diagram::md_table(
            &["operation id", "kind", "generation after", "sequence"],
            &rows,
        ));
    }
    out.push_str(
        "\nAn operation missing from this list has no established outcome here: it may be older than the window, still running, never delivered, refused, or a no-op. Only an exact lookup by operation id with the original request answers whether it committed. This list does not make any operation a current handle.\n",
    );
    out
}
