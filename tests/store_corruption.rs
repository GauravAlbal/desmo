//! Focused store corruption hardening (P0): persisted authority coordinates
//! (machine_version, generation, sequence) must never decode signed-to-unsigned
//! unchecked, `terminal` must be exactly boolean 0/1, and the stored receipt set
//! must equal the canonical journal event set. Every negative control fails
//! closed with a stable IK_* diagnostic and no journal/receipt/projection
//! mutation.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::diagnostic::DiagnosticCode;
use ik::identity::{AggregateId, Capability};
use ik::store::{CommandRequest, Store};
use proptest::prelude::*;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::path::Path;

fn machine() -> Box<dyn ik::machine::Machine> {
    Box::new(load_machine("verification_attempt"))
}

fn cap(identity: &ik::identity::MachineIdentity, agg: &AggregateId, generation: u64) -> Capability {
    Capability {
        generation,
        nonce: ik::runtime::derive_nonce(identity, agg, generation),
    }
}

/// Store with `agg-1` advanced to `running`: journal genesis at 0, start at 1,
/// receipts mirroring both events. Returns the store path.
fn seeded_store(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("s.db");
    {
        let mut store = Store::create(&path).expect("create store");
        let machine = machine();
        let identity = machine.identity().clone();
        let agg = AggregateId("agg-1".into());
        store
            .create_aggregate(machine.as_ref(), &agg, &identity)
            .expect("create aggregate");
        store
            .command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap(&identity, &agg, 0),
                    command: json!({"name": "start"}),
                    observation: None,
                },
            )
            .expect("start commits");
    }
    path
}

fn open_conn(path: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(path).expect("open sqlite")
}

/// (journal rows, receipts rows, aggregates rows) — the projection surface a
/// refused operation must never mutate.
fn row_counts(path: &Path) -> (i64, i64, i64) {
    let conn = open_conn(path);
    let journal: i64 = conn
        .query_row("SELECT COUNT(*) FROM journal", [], |r| r.get(0))
        .expect("journal count");
    let receipts: i64 = conn
        .query_row("SELECT COUNT(*) FROM receipts", [], |r| r.get(0))
        .expect("receipt count");
    let aggregates: i64 = conn
        .query_row("SELECT COUNT(*) FROM aggregates", [], |r| r.get(0))
        .expect("aggregate count");
    (journal, receipts, aggregates)
}

fn authority_column(path: &Path, column: &str) -> i64 {
    let conn = open_conn(path);
    conn.query_row(
        &format!("SELECT {column} FROM aggregates WHERE aggregate_id = 'agg-1'"),
        [],
        |r| r.get(0),
    )
    .expect("read authority column")
}

/// Negative persisted authority coordinates must refuse show/command/replay/verify
/// with the exact rule, and the refusal must not mutate any row.
#[test]
fn negative_authority_coordinates_refuse_before_command_and_replay() {
    for (column, rule) in [
        ("machine_version", "store.negative_machine_version"),
        ("generation", "store.negative_generation"),
        ("sequence", "store.negative_sequence"),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = seeded_store(&dir);
        let conn = open_conn(&path);
        conn.execute(
            &format!("UPDATE aggregates SET {column} = -1 WHERE aggregate_id = 'agg-1'"),
            [],
        )
        .expect("corrupt authority coordinate");
        drop(conn);
        let (journal_before, receipts_before, aggregates_before) = row_counts(&path);

        let mut store = Store::open(&path).expect("reopen");
        let agg = AggregateId("agg-1".into());
        let machine = machine();
        let identity = machine.identity().clone();

        let err = store
            .show(&agg)
            .expect_err("negative coordinate must refuse show");
        assert_eq!(err.code, DiagnosticCode::IkStoreCorrupt, "case {column}");
        assert_eq!(err.rule.as_deref(), Some(rule), "case {column}");

        let err = store
            .command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap(&identity, &agg, 0),
                    command: json!({"name": "cancel"}),
                    observation: None,
                },
            )
            .expect_err("negative coordinate must refuse command");
        assert_eq!(err.code, DiagnosticCode::IkStoreCorrupt, "case {column}");
        assert_eq!(err.rule.as_deref(), Some(rule), "case {column}");

        let err = store
            .replay(machine.as_ref(), None)
            .expect_err("negative coordinate must refuse replay");
        assert_eq!(err.code, DiagnosticCode::IkStoreCorrupt, "case {column}");
        assert_eq!(err.rule.as_deref(), Some(rule), "case {column}");

        let err = store
            .verify(machine.as_ref(), None)
            .expect_err("negative coordinate must refuse verify");
        assert_eq!(err.code, DiagnosticCode::IkStoreCorrupt, "case {column}");
        assert_eq!(err.rule.as_deref(), Some(rule), "case {column}");

        let (journal_after, receipts_after, aggregates_after) = row_counts(&path);
        assert_eq!(
            (journal_before, receipts_before, aggregates_before),
            (journal_after, receipts_after, aggregates_after),
            "refusal must not mutate journal/receipts/aggregates (case {column})"
        );
    }
}

/// `terminal` must be exactly boolean 0/1; any other value is storage corruption.
#[test]
fn non_boolean_terminal_refuses_with_exact_diagnostic() {
    for terminal in [-1i64, 2i64, i64::MAX, i64::MIN] {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = seeded_store(&dir);
        let conn = open_conn(&path);
        conn.execute(
            "UPDATE aggregates SET terminal = ?1 WHERE aggregate_id = 'agg-1'",
            rusqlite::params![terminal],
        )
        .expect("corrupt terminal");
        drop(conn);

        let mut store = Store::open(&path).expect("reopen");
        let machine = machine();
        let err = store
            .show(&AggregateId("agg-1".into()))
            .expect_err("non-boolean terminal must refuse show");
        assert_eq!(
            err.code,
            DiagnosticCode::IkStoreCorrupt,
            "terminal={terminal}"
        );
        assert_eq!(
            err.rule.as_deref(),
            Some("store.terminal"),
            "terminal={terminal}"
        );
        let err = store
            .replay(machine.as_ref(), None)
            .expect_err("non-boolean terminal must refuse replay");
        assert_eq!(
            err.code,
            DiagnosticCode::IkStoreCorrupt,
            "terminal={terminal}"
        );
        assert_eq!(
            err.rule.as_deref(),
            Some("store.terminal"),
            "terminal={terminal}"
        );
    }
    // Both legal booleans still decode.
    for terminal in [0i64, 1i64] {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = seeded_store(&dir);
        let conn = open_conn(&path);
        conn.execute(
            "UPDATE aggregates SET terminal = ?1 WHERE aggregate_id = 'agg-1'",
            rusqlite::params![terminal],
        )
        .expect("set terminal");
        drop(conn);
        let mut store = Store::open(&path).expect("reopen");
        let projection = store.show(&AggregateId("agg-1".into())).expect("show");
        assert_eq!(projection.terminal, terminal == 1, "terminal={terminal}");
    }
}

/// Negative journal sequence/generation columns must refuse replay and verify
/// with the exact rule, before any repair could run.
#[test]
fn negative_journal_coordinates_refuse_replay_and_verify() {
    for (column, rule) in [
        ("sequence", "journal.negative_sequence"),
        ("generation", "journal.negative_generation"),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = seeded_store(&dir);
        let conn = open_conn(&path);
        conn.execute(
            &format!(
                "UPDATE journal SET {column} = -1 WHERE aggregate_id = 'agg-1' AND sequence = 1"
            ),
            [],
        )
        .expect("corrupt journal coordinate");
        drop(conn);

        let mut store = Store::open(&path).expect("reopen");
        let machine = machine();
        let err = store
            .replay(machine.as_ref(), None)
            .expect_err("negative journal coordinate must refuse replay");
        assert_eq!(err.code, DiagnosticCode::IkJournalCorrupt, "case {column}");
        assert_eq!(err.rule.as_deref(), Some(rule), "case {column}");
        let err = store
            .verify(machine.as_ref(), None)
            .expect_err("negative journal coordinate must refuse verify");
        assert_eq!(err.code, DiagnosticCode::IkJournalCorrupt, "case {column}");
        assert_eq!(err.rule.as_deref(), Some(rule), "case {column}");
    }
}

fn load_receipt_json_opt(conn: &rusqlite::Connection, sequence: i64) -> Option<Value> {
    // rusqlite has no FromSql for serde_json::Value; read the TEXT column and
    // decode explicitly.
    let json: Option<String> = conn
        .query_row(
            "SELECT receipt_json FROM receipts WHERE aggregate_id = 'agg-1' AND sequence = ?1",
            [sequence],
            |row| row.get(0),
        )
        .optional()
        .expect("load receipt json");
    json.map(|raw| serde_json::from_str(&raw).expect("decode receipt json"))
}

fn load_receipt_json(conn: &rusqlite::Connection, sequence: i64) -> Value {
    load_receipt_json_opt(conn, sequence).expect("receipt row exists")
}

fn verify_err(path: &Path) -> ik::diagnostic::Diagnostic {
    let mut store = Store::open(path).expect("reopen");
    store
        .verify(machine().as_ref(), None)
        .expect_err("corrupted receipts must refuse verify")
}

/// A deleted receipt row changes the receipt set cardinality.
#[test]
fn missing_receipt_row_refuses_verify() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = seeded_store(&dir);
    let conn = open_conn(&path);
    conn.execute(
        "DELETE FROM receipts WHERE aggregate_id = 'agg-1' AND sequence = 1",
        [],
    )
    .expect("delete receipt");
    drop(conn);
    let err = verify_err(&path);
    assert_eq!(err.code, DiagnosticCode::IkProjectionCorrupt);
    assert_eq!(err.rule.as_deref(), Some("receipts.cardinality"));
}

/// An extra, well-formed receipt row (valid payload, columns consistent) changes
/// the cardinality and must refuse.
#[test]
fn extra_receipt_row_refuses_verify() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = seeded_store(&dir);
    let conn = open_conn(&path);
    let mut extra = load_receipt_json(&conn, 1);
    extra["receipt_id"] = json!("extra-999");
    extra["sequence"] = json!(999u64);
    conn.execute(
        "INSERT INTO receipts (aggregate_id, sequence, receipt_id, receipt_json) VALUES ('agg-1', 999, 'extra-999', ?1)",
        [ik::canonical::canonical_json(&extra)],
    )
    .expect("insert extra receipt");
    drop(conn);
    let err = verify_err(&path);
    assert_eq!(err.code, DiagnosticCode::IkProjectionCorrupt);
    assert_eq!(err.rule.as_deref(), Some("receipts.cardinality"));
}

/// A duplicate receipt row (unique constraints absent in a corrupt store) changes
/// the cardinality and must refuse.
#[test]
fn duplicate_receipt_row_refuses_verify() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = seeded_store(&dir);
    let conn = open_conn(&path);
    conn.execute_batch(
        "ALTER TABLE receipts RENAME TO receipts_orig;
         CREATE TABLE receipts (
           aggregate_id TEXT NOT NULL,
           sequence INTEGER NOT NULL,
           receipt_id TEXT NOT NULL,
           receipt_json TEXT NOT NULL
         );
         INSERT INTO receipts SELECT * FROM receipts_orig;
         DROP TABLE receipts_orig;
         INSERT INTO receipts SELECT * FROM receipts WHERE aggregate_id = 'agg-1' AND sequence = 1;",
    )
    .expect("drop constraints and duplicate a receipt row");
    drop(conn);
    let err = verify_err(&path);
    assert_eq!(err.code, DiagnosticCode::IkProjectionCorrupt);
    assert_eq!(err.rule.as_deref(), Some("receipts.cardinality"));
}

/// A receipt row whose position says sequence 1 but whose content is the genesis
/// receipt (columns forced consistent) is out of sequence and must refuse.
#[test]
fn out_of_sequence_receipt_content_refuses_verify() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = seeded_store(&dir);
    let conn = open_conn(&path);
    let tail_id: String = conn
        .query_row(
            "SELECT receipt_id FROM receipts WHERE aggregate_id = 'agg-1' AND sequence = 1",
            [],
            |row| row.get(0),
        )
        .expect("load tail receipt id");
    let mut forged = load_receipt_json(&conn, 0);
    forged["receipt_id"] = json!(tail_id);
    forged["sequence"] = json!(1u64);
    conn.execute(
        "UPDATE receipts SET receipt_json = ?1 WHERE aggregate_id = 'agg-1' AND sequence = 1",
        [ik::canonical::canonical_json(&forged)],
    )
    .expect("forge out-of-sequence receipt content");
    drop(conn);
    let err = verify_err(&path);
    assert_eq!(err.code, DiagnosticCode::IkProjectionCorrupt);
    assert_eq!(err.rule.as_deref(), Some("receipts.mismatch"));
}

/// A receipt row in `agg-1`'s table carrying a foreign aggregate's receipt is an
/// unrelated-aggregate row and must refuse.
#[test]
fn unrelated_aggregate_receipt_row_refuses_verify() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = seeded_store(&dir);
    let conn = open_conn(&path);
    let mut forged = load_receipt_json(&conn, 1);
    forged["aggregate_id"] = json!("other-agg");
    forged["receipt_id"] = json!("forged-foreign");
    conn.execute(
        "UPDATE receipts SET receipt_id = 'forged-foreign', receipt_json = ?1 WHERE aggregate_id = 'agg-1' AND sequence = 1",
        [ik::canonical::canonical_json(&forged)],
    )
    .expect("forge foreign-aggregate receipt");
    drop(conn);
    let err = verify_err(&path);
    assert_eq!(err.code, DiagnosticCode::IkProjectionCorrupt);
    assert_eq!(err.rule.as_deref(), Some("receipts.mismatch"));
}

/// A receipt row keyed to an aggregate that does not exist is invisible to
/// per-aggregate verification; the global orphan check must refuse it.
#[test]
fn orphan_receipt_row_for_unknown_aggregate_refuses_verify() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = seeded_store(&dir);
    let conn = open_conn(&path);
    conn.execute(
        "INSERT INTO receipts (aggregate_id, sequence, receipt_id, receipt_json) VALUES ('ghost', 0, 'ghost-0', '{}')",
        [],
    )
    .expect("insert orphan receipt");
    drop(conn);
    let err = verify_err(&path);
    assert_eq!(err.code, DiagnosticCode::IkProjectionCorrupt);
    assert_eq!(err.rule.as_deref(), Some("receipts.orphan_aggregate"));
}

/// A negative receipt sequence column must refuse with the exact rule.
#[test]
fn negative_receipt_sequence_refuses_verify() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = seeded_store(&dir);
    let conn = open_conn(&path);
    conn.execute(
        "UPDATE receipts SET sequence = -1 WHERE aggregate_id = 'agg-1' AND sequence = 1",
        [],
    )
    .expect("corrupt receipt sequence");
    drop(conn);
    let err = verify_err(&path);
    assert_eq!(err.code, DiagnosticCode::IkProjectionCorrupt);
    assert_eq!(err.rule.as_deref(), Some("receipts.negative_sequence"));
}

/// A receipt row whose stored columns disagree with its payload must refuse.
#[test]
fn receipt_column_payload_disagreement_refuses_verify() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = seeded_store(&dir);
    let conn = open_conn(&path);
    conn.execute(
        "UPDATE receipts SET receipt_id = 'forged' WHERE aggregate_id = 'agg-1' AND sequence = 1",
        [],
    )
    .expect("forge receipt id column");
    drop(conn);
    let err = verify_err(&path);
    assert_eq!(err.code, DiagnosticCode::IkProjectionCorrupt);
    assert_eq!(err.rule.as_deref(), Some("receipts.columns"));
}

/// A commit whose next sequence crosses the SQLite INTEGER ceiling (i64::MAX + 1
/// fits in u64, so the runtime u64 check passes) must refuse before any write:
/// the write-side checked cast is the last line of defense.
#[test]
fn sequence_at_sqlite_integer_ceiling_refuses_before_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = seeded_store(&dir);
    let conn = open_conn(&path);
    conn.execute(
        "UPDATE aggregates SET sequence = 9223372036854775807 WHERE aggregate_id = 'agg-1'",
        [],
    )
    .expect("set sequence to i64::MAX");
    drop(conn);
    let (journal_before, receipts_before, aggregates_before) = row_counts(&path);

    let mut store = Store::open(&path).expect("reopen");
    let agg = AggregateId("agg-1".into());
    let machine = machine();
    let identity = machine.identity().clone();
    let err = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap(&identity, &agg, 1),
                command: json!({"name": "cancel"}),
                observation: None,
            },
        )
        .expect_err("a commit crossing the SQLite INTEGER range must refuse");
    assert_eq!(err.code, DiagnosticCode::IkCounterOverflow);
    assert_eq!(err.rule.as_deref(), Some("sequence"));

    let (journal_after, receipts_after, aggregates_after) = row_counts(&path);
    assert_eq!(
        (journal_before, receipts_before, aggregates_before),
        (journal_after, receipts_after, aggregates_after),
        "the refused commit must not mutate journal/receipts/aggregates"
    );
    assert_eq!(
        authority_column(&path, "sequence"),
        i64::MAX,
        "the projection sequence must be untouched"
    );
}

/// The last valid persisted sequence is i64::MAX: a commit at the near-ceiling
/// (i64::MAX - 1 -> i64::MAX) succeeds, and the next transition (2^63 fits u64
/// but not the SQLite INTEGER column) refuses with the counter-overflow
/// diagnostic before any journal/receipt/projection write.
#[test]
fn near_ceiling_commit_then_next_transition_refuses_without_mutation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = seeded_store(&dir);
    let identity = identity_of("verification_attempt");
    let agg = AggregateId("agg-1".into());
    let near_max = i64::MAX as u64 - 1;
    let conn = open_conn(&path);
    conn.execute(
        "UPDATE aggregates SET generation = ?1, sequence = ?2, nonce = ?3 WHERE aggregate_id = 'agg-1'",
        rusqlite::params![
            near_max as i64,
            near_max as i64,
            ik::runtime::derive_nonce(&identity, &agg, near_max),
        ],
    )
    .expect("advance projection to the near-ceiling state");
    drop(conn);

    let mut store = Store::open(&path).expect("reopen");
    let machine = machine();
    // The near-ceiling commit is legal: i64::MAX fits the SQLite INTEGER column
    // and both counters advance together.
    let out = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap(&identity, &agg, near_max),
                command: json!({"name": "submit", "args": {"artifact_digest": "a1", "baseline_digest": "b1"}}),
                observation: None,
            },
        )
        .expect("near-ceiling commit must succeed");
    assert_eq!(out.generation, i64::MAX as u64);
    assert!(out.mutation);

    let (journal_before, receipts_before, aggregates_before) = row_counts(&path);
    // The next transition crosses the SQLite INTEGER range; the write-side checked
    // cast must refuse before any write.
    let err = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap(&identity, &agg, i64::MAX as u64),
                command: json!({"name": "reject"}),
                observation: None,
            },
        )
        .expect_err("a transition past the SQLite INTEGER range must refuse");
    assert_eq!(err.code, DiagnosticCode::IkCounterOverflow);
    assert_eq!(err.rule.as_deref(), Some("sequence"));

    let (journal_after, receipts_after, aggregates_after) = row_counts(&path);
    assert_eq!(
        (journal_before, receipts_before, aggregates_before),
        (journal_after, receipts_after, aggregates_after),
        "the refused transition must not mutate journal/receipts/aggregates"
    );
    assert_eq!(authority_column(&path, "sequence"), i64::MAX);
    assert_eq!(authority_column(&path, "generation"), i64::MAX);
}

fn boundary_i64() -> impl Strategy<Value = i64> {
    prop_oneof![
        Just(i64::MIN),
        Just(-1_000_000_000),
        Just(-1),
        Just(0),
        Just(1),
        Just(1_000_000_000),
        Just(i64::MAX),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        max_shrink_iters: 2_048,
        .. ProptestConfig::default()
    })]

    /// Integer-boundary property: negative persisted authority coordinates refuse
    /// with the exact rule; non-negative values (including the i64 extremes) are
    /// legal persisted integers and decode without a corruption refusal.
    #[test]
    fn authority_coordinate_boundaries_fail_closed(
        column in prop_oneof![
            Just("machine_version"),
            Just("generation"),
            Just("sequence"),
        ],
        value in boundary_i64(),
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = seeded_store(&dir);
        let conn = open_conn(&path);
        conn.execute(
            &format!("UPDATE aggregates SET {column} = ?1 WHERE aggregate_id = 'agg-1'"),
            rusqlite::params![value],
        )
        .expect("corrupt authority coordinate");
        drop(conn);

        let mut store = Store::open(&path).expect("reopen");
        let agg = AggregateId("agg-1".into());
        let machine = machine();
        if value < 0 {
            let rule = match column {
                "machine_version" => "store.negative_machine_version",
                "generation" => "store.negative_generation",
                _ => "store.negative_sequence",
            };
            let err = store.show(&agg).expect_err("negative coordinate must refuse");
            prop_assert_eq!(
                err.code,
                DiagnosticCode::IkStoreCorrupt,
                "column={} value={}",
                column,
                value
            );
            prop_assert_eq!(
                err.rule.as_deref(),
                Some(rule),
                "column={} value={}",
                column,
                value
            );
            let err = store
                .replay(machine.as_ref(), None)
                .expect_err("negative coordinate must refuse replay");
            prop_assert_eq!(
                err.code,
                DiagnosticCode::IkStoreCorrupt,
                "column={} value={}",
                column,
                value
            );
            prop_assert_eq!(
                err.rule.as_deref(),
                Some(rule),
                "column={} value={}",
                column,
                value
            );
        } else {
            store.show(&agg).expect("non-negative coordinate decodes");
        }
    }

    /// Terminal strictness property over the full i64 range: exactly 0/1 decode,
    /// anything else refuses with `store.terminal`.
    #[test]
    fn terminal_is_strictly_boolean(
        terminal in prop_oneof![boundary_i64(), any::<i64>()],
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = seeded_store(&dir);
        let conn = open_conn(&path);
        conn.execute(
            "UPDATE aggregates SET terminal = ?1 WHERE aggregate_id = 'agg-1'",
            rusqlite::params![terminal],
        )
        .expect("set terminal");
        drop(conn);
        let mut store = Store::open(&path).expect("reopen");
        match store.show(&AggregateId("agg-1".into())) {
            Ok(projection) => {
                prop_assert!(
                    terminal == 0 || terminal == 1,
                    "terminal {} must not decode",
                    terminal
                );
                prop_assert_eq!(projection.terminal, terminal == 1);
            }
            Err(err) => {
                prop_assert!(
                    terminal != 0 && terminal != 1,
                    "legal terminal {} must decode, got {}",
                    terminal,
                    err
                );
                prop_assert_eq!(err.code, DiagnosticCode::IkStoreCorrupt);
                prop_assert_eq!(err.rule.as_deref(), Some("store.terminal"));
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ReceiptOp {
    Noop,
    DeleteGenesis,
    DeleteTail,
    InsertExtra,
    SwapPayloads,
    ForgeIdColumn,
}

fn receipt_op() -> impl Strategy<Value = ReceiptOp> {
    prop_oneof![
        Just(ReceiptOp::Noop),
        Just(ReceiptOp::DeleteGenesis),
        Just(ReceiptOp::DeleteTail),
        Just(ReceiptOp::InsertExtra),
        Just(ReceiptOp::SwapPayloads),
        Just(ReceiptOp::ForgeIdColumn),
    ]
}

fn apply_receipt_ops(path: &Path, ops: &[ReceiptOp]) {
    let conn = open_conn(path);
    for op in ops {
        match op {
            ReceiptOp::Noop => {}
            ReceiptOp::DeleteGenesis => {
                conn.execute(
                    "DELETE FROM receipts WHERE aggregate_id = 'agg-1' AND sequence = 0",
                    [],
                )
                .expect("delete genesis receipt");
            }
            ReceiptOp::DeleteTail => {
                conn.execute(
                    "DELETE FROM receipts WHERE aggregate_id = 'agg-1' AND sequence = 1",
                    [],
                )
                .expect("delete tail receipt");
            }
            ReceiptOp::InsertExtra => {
                // A prior DeleteTail already corrupted the set; skip when the
                // template row is gone rather than panicking on the read.
                if let Some(template) = load_receipt_json_opt(&conn, 1) {
                    let mut extra = template.clone();
                    extra["receipt_id"] = json!("extra-999");
                    extra["sequence"] = json!(999u64);
                    conn.execute(
                        "INSERT INTO receipts (aggregate_id, sequence, receipt_id, receipt_json) VALUES ('agg-1', 999, 'extra-999', ?1)",
                        [ik::canonical::canonical_json(&extra)],
                    )
                    .expect("insert extra receipt");
                }
            }
            ReceiptOp::SwapPayloads => {
                // Prior deletions already corrupted the set; only swap when both
                // rows still exist so the swap itself stays well-defined.
                if let (Some(genesis), Some(tail)) = (
                    load_receipt_json_opt(&conn, 0),
                    load_receipt_json_opt(&conn, 1),
                ) {
                    conn.execute(
                        "UPDATE receipts SET receipt_json = ?1 WHERE aggregate_id = 'agg-1' AND sequence = 0",
                        [ik::canonical::canonical_json(&tail)],
                    )
                    .expect("swap genesis receipt");
                    conn.execute(
                        "UPDATE receipts SET receipt_json = ?1 WHERE aggregate_id = 'agg-1' AND sequence = 1",
                        [ik::canonical::canonical_json(&genesis)],
                    )
                    .expect("swap tail receipt");
                }
            }
            ReceiptOp::ForgeIdColumn => {
                conn.execute(
                    "UPDATE receipts SET receipt_id = 'forged' WHERE aggregate_id = 'agg-1' AND sequence = 1",
                    [],
                )
                .expect("forge receipt id column");
            }
        }
    }
    drop(conn);
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        max_shrink_iters: 2_048,
        .. ProptestConfig::default()
    })]

    /// Cardinality/order property: `verify` succeeds iff the stored receipt set
    /// equals the canonical journal-derived set. Any extra, missing, swapped, or
    /// column-forged receipt row (distinct ops only, so no op can cancel another)
    /// must refuse with IK_PROJECTION_CORRUPT; a pristine set must pass.
    #[test]
    fn receipt_set_equals_canonical_journal_set(
        ops in prop::collection::hash_set(receipt_op(), 0..3),
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = seeded_store(&dir);
        let mut ops: Vec<ReceiptOp> = ops.into_iter().collect();
        // Deterministic application order (hash_set order is arbitrary).
        ops.sort_by_key(|op| format!("{op:?}"));
        apply_receipt_ops(&path, &ops);

        let mut store = Store::open(&path).expect("reopen");
        let machine = machine();
        let corrupted = ops.iter().any(|op| *op != ReceiptOp::Noop);
        match store.verify(machine.as_ref(), None) {
            Ok(summary) => {
                prop_assert!(
                    !corrupted,
                    "a corrupted receipt set must not verify (ops={:?})",
                    ops
                );
                prop_assert!(summary.verified);
            }
            Err(err) => {
                prop_assert!(
                    corrupted,
                    "a pristine receipt set must verify (ops={:?})",
                    ops
                );
                prop_assert_eq!(
                    err.code,
                    DiagnosticCode::IkProjectionCorrupt,
                    "ops={:?}",
                    ops
                );
            }
        }
    }
}
