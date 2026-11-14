//! `ik operations` lists what the store recorded for one aggregate, newest first, and
//! leaves no trace of requests that did not commit.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn ik(args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_ik")).args(args).output().expect("spawn ik");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// A loan with `checkout` and `recall` committed and a refused `extend` between them.
fn loan_store(name: &str) -> PathBuf {
    let out = std::env::temp_dir().join(format!("ik-ops-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    let (_, err, code) = ik(&[
        "scenario",
        "run",
        &repo("validation/dfy/journeys/fixtures/loan_v2.ik.toml").display().to_string(),
        &repo("validation/dfy/journeys/fixtures/scenarios/existing-loan.json").display().to_string(),
        "--out",
        &out.display().to_string(),
    ]);
    assert_eq!(code, Some(0), "{err}");
    out.join("scratch/store.sqlite")
}

fn listing(store: &PathBuf, extra: &[&str]) -> Value {
    let path = store.display().to_string();
    let mut args = vec!["operations", path.as_str(), "--aggregate", "loan-1", "--json"];
    args.extend_from_slice(extra);
    let (out, err, code) = ik(&args);
    assert_eq!(code, Some(0), "{err}");
    serde_json::from_str(&out).expect("json")
}

#[test]
fn operations_are_listed_newest_first_with_the_generation_each_reached() {
    let doc = listing(&loan_store("order"), &[]);
    assert_eq!(doc["snapshot"]["generation"], 2);
    assert_eq!(doc["window"]["complete"], true);
    let rows = doc["operations"].as_array().expect("rows");
    let ids: Vec<&str> = rows.iter().filter_map(|row| row["operation_id"].as_str()).collect();
    assert_eq!(ids, vec!["op-recall", "op-checkout"]);
    assert_eq!(rows[0]["generation"], 2);
    assert_eq!(rows[1]["generation"], 1);
}

#[test]
fn a_refused_request_leaves_no_row() {
    let doc = listing(&loan_store("refused"), &[]);
    let kinds = doc["operations"].as_array().expect("rows").len();
    assert_eq!(kinds, 2, "the refused extend must not appear: {doc}");
}

#[test]
fn the_limit_keeps_only_the_most_recent_operations() {
    let doc = listing(&loan_store("limit"), &["--limit", "1"]);
    let rows = doc["operations"].as_array().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["operation_id"], "op-recall");
}

#[test]
fn listing_does_not_change_the_store_and_unknown_aggregates_are_refused() {
    let store = loan_store("readonly");
    let before = fs::read(&store).expect("bytes");
    let _ = listing(&store, &[]);
    assert_eq!(fs::read(&store).expect("bytes"), before);
    let path = store.display().to_string();
    let (_, err, code) = ik(&["operations", &path, "--aggregate", "nope"]);
    assert_eq!(code, Some(2));
    assert_eq!(err.contains("no aggregate `nope` in the store"), true, "{err}");
    let (_, err, code) = ik(&["operations", &path]);
    assert_eq!(code, Some(2));
    assert_eq!(err.contains("--aggregate is required"), true, "{err}");
}

fn copy_store(name: &str) -> PathBuf {
    let source = loan_store(name);
    let copy = source.with_file_name("copy.sqlite");
    fs::copy(&source, &copy).expect("copy store");
    copy
}

fn add_operations(store: &PathBuf, count: u32) {
    let conn = rusqlite::Connection::open(store).expect("open");
    for n in 0..count {
        let sequence = 10 + n;
        conn.execute(
            "INSERT INTO operations (aggregate_id, operation_id, kind, fingerprint, first_sequence, last_sequence, generation, state_digest, event_hashes, receipt_ids) VALUES ('loan-1', ?1, 'command', 'f', ?2, ?2, ?2, 'd', '[]', '[]')",
            rusqlite::params![format!("bulk-{n}"), sequence],
        )
        .expect("insert");
    }
}

#[test]
fn a_window_smaller_than_the_history_says_older_operations_are_not_shown() {
    let store = copy_store("window");
    add_operations(&store, 25);
    let doc = listing(&store, &[]);
    assert_eq!(doc["window"]["limit"], 20);
    assert_eq!(doc["window"]["returned"], 20);
    assert_eq!(doc["window"]["complete"], false);
    let ids: Vec<&str> = doc["operations"]
        .as_array()
        .expect("rows")
        .iter()
        .filter_map(|row| row["operation_id"].as_str())
        .collect();
    assert_eq!(ids.contains(&"op-checkout"), false, "the oldest operation is outside the window");
    let path = store.display().to_string();
    let (text, _, _) = ik(&["operations", &path, "--aggregate", "loan-1"]);
    assert_eq!(text.contains("older operations exist and are not shown"), true, "{text}");
    assert_eq!(text.contains("did not commit"), false, "{text}");
    assert_eq!(text.contains("no established outcome here"), true, "{text}");
    let all = listing(&store, &["--limit", "100"]);
    assert_eq!(all["window"]["complete"], true);
    assert_eq!(all["window"]["returned"], 27);
}

#[test]
fn a_negative_stored_coordinate_is_a_damaged_store_not_zero() {
    let store = copy_store("damaged");
    let conn = rusqlite::Connection::open(&store).expect("open");
    conn.execute("UPDATE operations SET generation = -1 WHERE operation_id = 'op-recall'", [])
        .expect("damage");
    drop(conn);
    let path = store.display().to_string();
    let (out, err, code) = ik(&["operations", &path, "--aggregate", "loan-1"]);
    assert_eq!(code, Some(2), "{out}");
    assert_eq!(err.contains("store is damaged") && err.contains("negative generation"), true, "{err}");
    let conn = rusqlite::Connection::open(&store).expect("open");
    conn.execute("UPDATE operations SET generation = 2", []).expect("repair");
    conn.execute("UPDATE aggregates SET sequence = -5", []).expect("damage position");
    drop(conn);
    let (_, err, code) = ik(&["operations", &path, "--aggregate", "loan-1"]);
    assert_eq!(code, Some(2));
    assert_eq!(err.contains("negative sequence"), true, "{err}");
}

#[test]
fn a_concurrent_writer_never_makes_the_position_disagree_with_the_rows() {
    let store = copy_store("concurrent");
    let writer_store = store.clone();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stop.clone();
    let writer = std::thread::spawn(move || {
        let mut conn = rusqlite::Connection::open(&writer_store).expect("open");
        conn.busy_timeout(std::time::Duration::from_secs(5)).expect("timeout");
        let mut next: i64 = 100;
        while !flag.load(std::sync::atomic::Ordering::Relaxed) {
            let tx = conn.transaction().expect("tx");
            tx.execute(
                "INSERT INTO operations (aggregate_id, operation_id, kind, fingerprint, first_sequence, last_sequence, generation, state_digest, event_hashes, receipt_ids) VALUES ('loan-1', ?1, 'command', 'f', ?2, ?2, ?2, 'd', '[]', '[]')",
                rusqlite::params![format!("live-{next}"), next],
            )
            .expect("insert");
            tx.execute("UPDATE aggregates SET generation = ?1, sequence = ?1", rusqlite::params![next])
                .expect("advance");
            tx.commit().expect("commit");
            next += 1;
        }
    });
    let mut checked = 0;
    for _ in 0..200 {
        let read = ik::operations::read(&store, "loan-1", 5);
        if let Ok(read) = read {
            let newest = read.operations.first().map(|row| row.generation);
            if newest.is_some_and(|generation| generation >= 100) {
                assert_eq!(newest, Some(read.current_generation), "{read:?}");
                checked += 1;
            }
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    writer.join().expect("writer");
    assert_eq!(checked > 0, true, "the reader never overlapped the writer");
}
