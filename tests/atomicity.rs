//! K1/K2.5 transaction-atomicity boundary and crash recovery (IK-LAW-004,
//! REQ-IK-006 "injected failure fixtures leave no partial supported state",
//! REQ-IK-008 replay, REQ-IK-011 receipts).
//!
//! The existing suite proves that refusals BEFORE any write leave no partial state
//! (`failed_commands_leave_no_partial_state`) and that the happy path commits
//! journal+projection+receipt together (`journal_and_projection_update_atomically_with_receipts`).
//! This file closes the remaining verification gap: failures INJECTED at every
//! fallible persist step inside the transaction, and an executable crash/reopen/
//! replay proof with a real child process dying mid-transaction.
//!
//! Injection points (each exercised on the domain path AND the K2.5 kernel `record`
//! path, on the genesis create path, and on the ATOMIC create-with-command path
//! where genesis + the initial command share one transaction):
//! - at the journal INSERT: a pre-seeded journal row at the next sequence collides
//!   with the append's primary key;
//! - AFTER the journal append: a pre-seeded receipts row at the next sequence makes
//!   the receipts INSERT fail after the journal INSERT already succeeded;
//! - AFTER the receipts INSERT: a BEFORE UPDATE trigger on the projection table
//!   raises ROLLBACK when the projection write begins;
//! - AFTER the projection write, BEFORE commit: an AFTER UPDATE trigger on the
//!   projection table raises ROLLBACK once the aggregates row has been rewritten.
//!
//! Every probe asserts, AFTER REOPEN, that journal+receipt row counts are exactly
//! the pre-failure rows plus any pre-seeded injection row; each test then directly
//! asserts that every aggregate projection field (state, terminal, generation,
//! nonce, effect status, sequence, chain head, last receipt) is unchanged. Each
//! probe then removes the injection and commits the identical transition as a
//! positive control, proving the assertions are not vacuous. A mutation that lets
//! the event append escape the transaction or the projection/receipt/generation
//! commit non-atomically leaves a torn write behind and fails these probes.
//!
//! Crash proof (recovery/replay evidence):
//! - `crash_mid_transaction_recovers_to_last_committed_state` spawns a REAL child
//!   process (the test binary re-entered via `crash_child_entry`) that computes the
//!   next transition through the public pure API, writes the full persist payload
//!   (journal + receipt + projection update) inside an uncommitted IMMEDIATE
//!   transaction, and dies with SIGABRT before COMMIT. The parent then reopens the
//!   store and must observe exactly the last committed state through the supported
//!   API and the public CLI: `verify` passes, `replay` reproduces the projection
//!   without rebuilding it, `show` matches the pre-crash snapshot, and the
//!   lifecycle continues from the recovered capability.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::diagnostic::DiagnosticCode;
use ik::effect::{EffectOutcomeV1, EffectStatus};
use ik::identity::{AggregateId, Capability, EffectId, MachineIdentity};
use ik::runtime::{AggregateSnapshot, Step};
use ik::store::{CommandRequest, EffectOutcomeRequest, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn va() -> Box<dyn ik::machine::Machine> {
    Box::new(load_machine("verification_attempt"))
}

fn probe() -> Box<dyn ik::machine::Machine> {
    Box::new(load_machine("effect_probe"))
}

fn capability(identity: &MachineIdentity, agg: &AggregateId, generation: u64) -> Capability {
    Capability {
        generation,
        nonce: ik::runtime::derive_nonce(identity, agg, generation),
    }
}

fn setup_va(dir: &tempfile::TempDir) -> (Store, AggregateId, Box<dyn ik::machine::Machine>) {
    let machine = va();
    let identity = machine.identity().clone();
    let mut store = Store::create(&dir.path().join("s.db")).expect("create store");
    let agg = AggregateId("agg-1".into());
    store
        .create_aggregate(machine.as_ref(), &agg, &identity)
        .expect("create aggregate");
    (store, agg, machine)
}

fn setup_probe(dir: &tempfile::TempDir) -> (Store, AggregateId, Box<dyn ik::machine::Machine>) {
    let machine = probe();
    let identity = machine.identity().clone();
    let mut store = Store::create(&dir.path().join("s.db")).expect("create store");
    let agg = AggregateId("agg-1".into());
    store
        .create_aggregate(machine.as_ref(), &agg, &identity)
        .expect("create aggregate");
    (store, agg, machine)
}

fn start_va(
    store: &mut Store,
    machine: &dyn ik::machine::Machine,
    agg: &AggregateId,
    cap: &Capability,
) -> ik::store::CommandResult {
    store
        .command(
            machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: machine.identity().clone(),
                capability: cap.clone(),
                command: json!({"name": "start"}),
                observation: None,
            },
        )
        .expect("start commits")
}

fn submit() -> Value {
    json!({"name": "submit", "args": {"artifact_digest": "a1", "baseline_digest": "b1"}})
}

fn plan_probe(
    store: &mut Store,
    machine: &dyn ik::machine::Machine,
    agg: &AggregateId,
    capability: &Capability,
) -> ik::store::CommandResult {
    store
        .command(
            machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: machine.identity().clone(),
                capability: capability.clone(),
                command: json!({"name": "plan", "args": {"effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1"}}),
                observation: None,
            },
        )
        .expect("plan commits")
}

fn probe_outcome(agg: &AggregateId) -> EffectOutcomeV1 {
    EffectOutcomeV1 {
        effect_id: EffectId("e1".into()),
        aggregate_id: agg.clone(),
        planned_generation: 1,
        payload_digest: "p1".into(),
        idempotency_key: "k1".into(),
        outcome_digest: "o1".into(),
    }
}

fn journal_rows(path: &Path, aggregate: &str) -> i64 {
    let conn = rusqlite::Connection::open(path).expect("open store");
    conn.query_row(
        "SELECT COUNT(*) FROM journal WHERE aggregate_id = ?1",
        [aggregate],
        |row| row.get(0),
    )
    .expect("count journal rows")
}

fn receipt_rows(path: &Path, aggregate: &str) -> i64 {
    let conn = rusqlite::Connection::open(path).expect("open store");
    conn.query_row(
        "SELECT COUNT(*) FROM receipts WHERE aggregate_id = ?1",
        [aggregate],
        |row| row.get(0),
    )
    .expect("count receipt rows")
}

/// Every observable projection field must be untouched by a failed transition.
fn assert_projection_unchanged(before: &ik::store::Projection, after: &ik::store::Projection) {
    assert_eq!(after.machine, before.machine);
    assert_eq!(after.state, before.state);
    assert_eq!(after.terminal, before.terminal);
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.capability, before.capability);
    assert_eq!(after.effect, before.effect);
    assert_eq!(after.sequence, before.sequence);
    assert_eq!(after.last_event_hash, before.last_event_hash);
    assert_eq!(after.last_receipt, before.last_receipt);
}

/// Run one injected-failure atomicity probe on the domain (command) path.
///
/// Setup: genesis + `start` (sequence 1, generation 1, state `running`). `inject`
/// installs a failure at one fallible persist step of the next `submit` transition
/// (seed row or trigger); `cleanup` removes it afterwards. The probe asserts that
/// after the refused command — measured on a REOPENED connection — the journal and
/// receipts tables contain exactly the committed rows plus any pre-seeded injection
/// row. It returns the projection observed before the injection and the projection
/// observed after reopen; each caller directly asserts every projection field is
/// unchanged. The positive control then commits the identical transition and
/// verifies the store.
fn domain_atomicity_probe(
    dir: &tempfile::TempDir,
    expected_journal_rows: i64,
    expected_receipt_rows: i64,
    inject: impl FnOnce(&Path),
    cleanup: impl FnOnce(&Path),
) -> (ik::store::Projection, ik::store::Projection) {
    let path = dir.path().join("s.db");
    let (mut store, agg, machine) = setup_va(dir);
    let identity = machine.identity().clone();
    let cap = capability(&identity, &agg, 0);
    start_va(&mut store, machine.as_ref(), &agg, &cap); // sequence 1, generation 1

    let before = store.show(&agg).expect("show before");
    inject(&path);
    let err = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: before.capability.clone(),
                command: submit(),
                observation: None,
            },
        )
        .expect_err("the injected failure must refuse the command");
    assert_eq!(err.code, DiagnosticCode::IkStoreCorrupt);
    drop(store);

    // Reopen: the rollback must be durable — nothing leaked to disk.
    let mut store = Store::open(&path).expect("reopen after injected failure");
    let after = store.show(&agg).expect("show after reopen");
    assert_eq!(
        journal_rows(&path, &agg.0),
        expected_journal_rows,
        "no appended event may survive the injected failure"
    );
    assert_eq!(
        receipt_rows(&path, &agg.0),
        expected_receipt_rows,
        "no receipt may survive the injected failure"
    );
    drop(store);

    // Positive control: with the injection removed, the unchanged capability
    // commits the same transition and the store verifies.
    cleanup(&path);
    let mut store = Store::open(&path).expect("reopen for the positive control");
    let out = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity,
                capability: before.capability.clone(),
                command: submit(),
                observation: None,
            },
        )
        .expect("commit after the injection is removed");
    assert_eq!(out.generation, 2);
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after the recovered commit");
    (before, after)
}

/// Run one injected-failure atomicity probe on the K2.5 kernel `record` path.
///
/// Setup: genesis + `plan` (sequence 1, generation 1, effect pending). `inject`
/// installs a failure at one fallible persist step of the next `record_effect`
/// transition; `cleanup` removes it afterwards. Asserts, after REOPEN, that the
/// journal/receipts tables hold exactly the committed rows plus any pre-seeded
/// injection row and that the effect is still Pending. It returns the projection
/// observed before the injection and the projection observed after reopen; each
/// caller directly asserts every projection field is unchanged. The positive
/// control then records the exact pending binding and verifies the store.
fn record_atomicity_probe(
    dir: &tempfile::TempDir,
    expected_journal_rows: i64,
    expected_receipt_rows: i64,
    inject: impl FnOnce(&Path),
    cleanup: impl FnOnce(&Path),
) -> (ik::store::Projection, ik::store::Projection) {
    let path = dir.path().join("s.db");
    let (mut store, agg, machine) = setup_probe(dir);
    let identity = machine.identity().clone();
    let cap0 = capability(&identity, &agg, 0);
    let planned = plan_probe(&mut store, machine.as_ref(), &agg, &cap0); // sequence 1, generation 1
    assert!(
        matches!(planned.effect, EffectStatus::Pending(_)),
        "the plan must leave an effect pending"
    );

    let before = store.show(&agg).expect("show before");
    inject(&path);
    let err = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: before.capability.clone(),
            effect: probe_outcome(&agg),
        })
        .expect_err("the injected failure must refuse the record");
    assert_eq!(err.code, DiagnosticCode::IkStoreCorrupt);
    drop(store);

    // Reopen: the rollback must be durable — nothing leaked to disk.
    let mut store = Store::open(&path).expect("reopen after injected failure");
    let after = store.show(&agg).expect("show after reopen");
    assert!(
        matches!(after.effect, EffectStatus::Pending(_)),
        "the pending intent must survive the failed record"
    );
    assert_eq!(
        journal_rows(&path, &agg.0),
        expected_journal_rows,
        "no appended event may survive the injected failure"
    );
    assert_eq!(
        receipt_rows(&path, &agg.0),
        expected_receipt_rows,
        "no receipt may survive the injected failure"
    );
    drop(store);

    // Positive control: with the injection removed, the unchanged capability
    // records the exact pending binding and the store verifies.
    cleanup(&path);
    let mut store = Store::open(&path).expect("reopen for the positive control");
    let recorded = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity,
            capability: before.capability.clone(),
            effect: probe_outcome(&agg),
        })
        .expect("record after the injection is removed");
    assert!(recorded.mutation);
    assert_eq!(recorded.generation, 2);
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after the recovered record");
    (before, after)
}

#[test]
fn journal_insert_cannot_escape_the_transaction() {
    // Failure injected AT the journal INSERT: a pre-seeded journal row at the next
    // sequence collides with the append's primary key. The refusal must leave every
    // projection field and both append tables exactly as they were.
    let (before, after) = domain_atomicity_probe(
        &tempfile::tempdir().expect("tempdir"),
        3, // genesis + start + the pre-seeded conflicting journal row
        2, // genesis + start
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for seeding");
            conn.execute(
                "INSERT INTO journal (aggregate_id, sequence, generation, hash, prev_hash, event_json) VALUES ('agg-1', 2, 2, 'seeded', 'seeded', '{}')",
                [],
            )
            .expect("seed conflicting journal row");
            drop(conn);
        },
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for cleanup");
            conn.execute(
                "DELETE FROM journal WHERE aggregate_id = 'agg-1' AND hash = 'seeded'",
                [],
            )
            .expect("clear seed");
            drop(conn);
        },
    );
    assert_projection_unchanged(&before, &after);
}

#[test]
fn journal_append_cannot_escape_the_transaction() {
    // Failure injected AFTER the journal append: a pre-seeded receipts row at the
    // next sequence collides with the receipts INSERT (PK aggregate_id+sequence)
    // that follows the journal INSERT inside `persist_plan`. The transaction must
    // roll back the already-appended journal event; if the append had escaped the
    // transaction (auto-committed before the receipts write), the event would
    // survive and this test would fail.
    let (before, after) = domain_atomicity_probe(
        &tempfile::tempdir().expect("tempdir"),
        2, // genesis + start
        3, // genesis + start + the pre-seeded conflicting receipt row
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for seeding");
            conn.execute(
                "INSERT INTO receipts (aggregate_id, sequence, receipt_id, receipt_json) VALUES ('agg-1', 2, 'seeded', '{}')",
                [],
            )
            .expect("seed conflicting receipt");
            drop(conn);
        },
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for cleanup");
            conn.execute(
                "DELETE FROM receipts WHERE aggregate_id = 'agg-1' AND receipt_id = 'seeded'",
                [],
            )
            .expect("clear seed");
            drop(conn);
        },
    );
    assert_projection_unchanged(&before, &after);
}

#[test]
fn receipt_insert_cannot_escape_the_transaction() {
    // Failure injected AFTER the receipts INSERT, BEFORE the projection write: a
    // BEFORE UPDATE trigger on the projection table raises ROLLBACK when the
    // aggregates UPDATE begins. The already-appended journal event and receipt must
    // roll back with it.
    let (before, after) = domain_atomicity_probe(
        &tempfile::tempdir().expect("tempdir"),
        2,
        2,
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for injection");
            conn.execute_batch(
                "CREATE TRIGGER ik_injected_abort BEFORE UPDATE ON aggregates BEGIN SELECT RAISE(ROLLBACK, 'injected failure before projection write'); END;",
            )
            .expect("install failure injection before the projection write");
            drop(conn);
        },
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for cleanup");
            conn.execute_batch("DROP TRIGGER ik_injected_abort;")
                .expect("remove injection");
            drop(conn);
        },
    );
    assert_projection_unchanged(&before, &after);
}

#[test]
fn projection_write_cannot_escape_the_transaction() {
    // Failure injected AFTER the projection write, BEFORE commit: an AFTER UPDATE
    // trigger on the projection table raises ROLLBACK once the aggregates row has
    // been rewritten. The projection state, generation, nonce, effect status,
    // sequence, chain head, and last receipt must roll back together with the
    // appended journal event and receipt; if any of them had committed
    // non-atomically, the torn write would survive and this test would fail.
    let (before, after) = domain_atomicity_probe(
        &tempfile::tempdir().expect("tempdir"),
        2,
        2,
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for injection");
            conn.execute_batch(
                "CREATE TRIGGER ik_injected_abort AFTER UPDATE ON aggregates BEGIN SELECT RAISE(ROLLBACK, 'injected failure after projection write'); END;",
            )
            .expect("install failure injection after the projection write");
            drop(conn);
        },
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for cleanup");
            conn.execute_batch("DROP TRIGGER ik_injected_abort;")
                .expect("remove injection");
            drop(conn);
        },
    );
    assert_projection_unchanged(&before, &after);
}

#[test]
fn create_aggregate_append_cannot_escape_the_transaction() {
    // The genesis transition must commit atomically too: a pre-seeded receipts row
    // at sequence 0 collides with the genesis receipt INSERT after the genesis
    // journal INSERT succeeded. The whole create (journal + receipts + aggregates
    // row) must roll back; no partial aggregate may become observable through the
    // supported API, on a reopened connection.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let machine = va();
    let identity = machine.identity().clone();
    let mut store = Store::create(&path).expect("create store");

    let conn = rusqlite::Connection::open(&path).expect("open store for seeding");
    conn.execute(
        "INSERT INTO receipts (aggregate_id, sequence, receipt_id, receipt_json) VALUES ('agg-1', 0, 'seeded', '{}')",
        [],
    )
    .expect("seed conflicting receipt");
    drop(conn);

    let err = store
        .create_aggregate(machine.as_ref(), &AggregateId("agg-1".into()), &identity)
        .expect_err("the injected receipt conflict must fail the create");
    assert_eq!(err.code, DiagnosticCode::IkStoreCorrupt);
    drop(store);

    // Reopen: no partial aggregate is observable, and no genesis row leaked.
    let mut store = Store::open(&path).expect("reopen after injected failure");
    let ghost = store
        .show(&AggregateId("agg-1".into()))
        .expect_err("no aggregate may be observable after the failed create");
    assert_eq!(ghost.code, DiagnosticCode::IkIllegalState);
    assert_eq!(
        journal_rows(&path, "agg-1"),
        0,
        "no genesis event may survive"
    );
    assert_eq!(
        receipt_rows(&path, "agg-1"),
        1,
        "only the pre-seeded row remains"
    );
    drop(store);

    // Positive control: clearing the injection lets the create commit.
    let conn = rusqlite::Connection::open(&path).expect("open store for cleanup");
    conn.execute(
        "DELETE FROM receipts WHERE aggregate_id = 'agg-1' AND receipt_id = 'seeded'",
        [],
    )
    .expect("clear seed");
    drop(conn);
    let mut store = Store::open(&path).expect("reopen for the positive control");
    store
        .create_aggregate(machine.as_ref(), &AggregateId("agg-1".into()), &identity)
        .expect("create commits after the injection is cleared");
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after the create");
}

#[test]
fn effect_record_journal_insert_cannot_escape_the_transaction() {
    // K2.5: failure injected AT the record's journal INSERT. The pending intent and
    // every projection field must survive the refused record.
    let (before, after) = record_atomicity_probe(
        &tempfile::tempdir().expect("tempdir"),
        3, // genesis + plan + the pre-seeded conflicting journal row
        2, // genesis + plan
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for seeding");
            conn.execute(
                "INSERT INTO journal (aggregate_id, sequence, generation, hash, prev_hash, event_json) VALUES ('agg-1', 2, 2, 'seeded', 'seeded', '{}')",
                [],
            )
            .expect("seed conflicting journal row");
            drop(conn);
        },
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for cleanup");
            conn.execute(
                "DELETE FROM journal WHERE aggregate_id = 'agg-1' AND hash = 'seeded'",
                [],
            )
            .expect("clear seed");
            drop(conn);
        },
    );
    assert_projection_unchanged(&before, &after);
}

#[test]
fn effect_record_journal_append_cannot_escape_the_transaction() {
    // K2.5: failure injected AFTER the record's journal append (receipts INSERT
    // conflict). The appended recording event must roll back; the effect stays
    // pending.
    let (before, after) = record_atomicity_probe(
        &tempfile::tempdir().expect("tempdir"),
        2, // genesis + plan
        3, // genesis + plan + the pre-seeded conflicting receipt row
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for seeding");
            conn.execute(
                "INSERT INTO receipts (aggregate_id, sequence, receipt_id, receipt_json) VALUES ('agg-1', 2, 'seeded', '{}')",
                [],
            )
            .expect("seed conflicting receipt");
            drop(conn);
        },
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for cleanup");
            conn.execute(
                "DELETE FROM receipts WHERE aggregate_id = 'agg-1' AND receipt_id = 'seeded'",
                [],
            )
            .expect("clear seed");
            drop(conn);
        },
    );
    assert_projection_unchanged(&before, &after);
}

#[test]
fn effect_record_receipt_insert_cannot_escape_the_transaction() {
    // K2.5: failure injected AFTER the record's receipts INSERT, BEFORE the
    // projection write (BEFORE UPDATE trigger). Journal event, receipt, and the
    // pending intent all roll back together.
    let (before, after) = record_atomicity_probe(
        &tempfile::tempdir().expect("tempdir"),
        2,
        2,
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for injection");
            conn.execute_batch(
                "CREATE TRIGGER ik_injected_abort BEFORE UPDATE ON aggregates BEGIN SELECT RAISE(ROLLBACK, 'injected failure before projection write'); END;",
            )
            .expect("install failure injection before the projection write");
            drop(conn);
        },
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for cleanup");
            conn.execute_batch("DROP TRIGGER ik_injected_abort;")
                .expect("remove injection");
            drop(conn);
        },
    );
    assert_projection_unchanged(&before, &after);
}

#[test]
fn effect_record_projection_write_cannot_escape_the_transaction() {
    // K2.5: failure injected AFTER the record's projection write, BEFORE commit
    // (AFTER UPDATE trigger). The recorded effect status, generation +1, nonce
    // rotation, and chain head must roll back together with the appended event and
    // receipt; the effect stays pending.
    let (before, after) = record_atomicity_probe(
        &tempfile::tempdir().expect("tempdir"),
        2,
        2,
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for injection");
            conn.execute_batch(
                "CREATE TRIGGER ik_injected_abort AFTER UPDATE ON aggregates BEGIN SELECT RAISE(ROLLBACK, 'injected failure after projection write'); END;",
            )
            .expect("install failure injection after the projection write");
            drop(conn);
        },
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for cleanup");
            conn.execute_batch("DROP TRIGGER ik_injected_abort;")
                .expect("remove injection");
            drop(conn);
        },
    );
    assert_projection_unchanged(&before, &after);
}

#[test]
fn crash_mid_transaction_recovers_to_last_committed_state() {
    // Executable crash/reopen/replay proof with a REAL child process. The child
    // computes the next transition through the public pure API, writes the full
    // persist payload (journal + receipt + projection update) inside an uncommitted
    // IMMEDIATE transaction, and dies with SIGABRT before COMMIT — leaving exactly
    // the WAL artifact of a process crash between event append and commit. The
    // supported API must then observe exactly the last committed state.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let agg = AggregateId("agg-1".into());
    {
        let (mut store, agg, machine) = setup_va(&dir);
        let identity = machine.identity().clone();
        let cap = capability(&identity, &agg, 0);
        start_va(&mut store, machine.as_ref(), &agg, &cap); // sequence 1, generation 1
        drop(store);
    }
    let machine = va();
    let mut store = Store::open(&path).expect("open before crash");
    let before = store.show(&agg).expect("committed state before crash");
    assert_eq!(before.generation, 1);
    assert_eq!(before.sequence, 1);
    drop(store);

    let marker = dir.path().join("crash.armed");
    let status = std::process::Command::new(std::env::current_exe().expect("test binary path"))
        .arg("crash_child_entry")
        .arg("--exact")
        .env("IK_CRASH_CHILD", "1")
        .env("IK_CRASH_STORE", &path)
        .env("IK_CRASH_AGG", &agg.0)
        .env("IK_CRASH_MARKER", &marker)
        .status()
        .expect("run crash child");
    assert!(
        !status.success(),
        "crash child must die abnormally (SIGABRT), got {status:?}"
    );
    let armed = std::fs::read_to_string(&marker)
        .unwrap_or_else(|e| panic!("crash child never reached the abort point: {e}"));
    assert_eq!(
        armed.trim(),
        "armed",
        "the torn transaction must be written before death"
    );

    // Reopen: SQLite crash recovery discards the torn transaction; the supported
    // API must observe exactly the last committed state.
    let mut store = Store::open(&path).expect("reopen after crash");
    let after = store.show(&agg).expect("show after crash");
    assert_eq!(
        after.generation, before.generation,
        "torn generation discarded"
    );
    assert_eq!(
        after.sequence, before.sequence,
        "torn journal event discarded"
    );
    assert_eq!(after.state, before.state);
    assert_eq!(after.capability, before.capability);
    assert_eq!(after.effect, before.effect);
    assert_eq!(after.last_event_hash, before.last_event_hash);
    assert_eq!(after.last_receipt, before.last_receipt);

    // Positive-control verification: journal + projection agree and replay
    // reproduces the recovered projection without rebuilding it.
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after crash recovery");
    let summary = store
        .replay(machine.as_ref(), None)
        .expect("replay after crash");
    assert_eq!(summary.aggregates.len(), 1);
    assert_eq!(summary.aggregates[0].sequence, before.sequence);
    assert_eq!(summary.aggregates[0].generation, before.generation);
    assert!(
        !summary.aggregates[0].rebuilt,
        "replay must not need a rebuild"
    );

    // Executable proof through the public CLI: `ik verify` on the recovered store.
    let machine_path = machine_def("verification_attempt");
    let request = json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": "0.6.0",
        "request_id": "verify-after-crash",
        "op": "verify",
    });
    let (stdout, exit_code) = run_cli(&path, Some(&machine_path), &request);
    assert_eq!(
        exit_code, 0,
        "ik verify must pass on the recovered store: {stdout}"
    );
    let response: Value = serde_json::from_str(&stdout).expect("verify envelope");
    assert_eq!(response["result"]["verified"], json!(true));

    // The lifecycle continues from the recovered capability.
    let out = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: machine.identity().clone(),
                capability: after.capability.clone(),
                command: submit(),
                observation: None,
            },
        )
        .expect("lifecycle continues after crash");
    assert_eq!(out.generation, before.generation + 1);
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after continuing");
}

#[test]
fn crash_child_entry() {
    // Re-entered as a real child process by
    // `crash_mid_transaction_recovers_to_last_committed_state`. Computes the next
    // transition through the public pure API, writes the full persist payload
    // (journal + receipt + projection update) inside an uncommitted IMMEDIATE
    // transaction, then dies with SIGABRT before COMMIT.
    if std::env::var("IK_CRASH_CHILD").is_err() {
        return; // normal parent-side invocation of the test binary: nothing to do
    }
    let store_path = PathBuf::from(std::env::var("IK_CRASH_STORE").expect("IK_CRASH_STORE"));
    let agg = AggregateId(std::env::var("IK_CRASH_AGG").expect("IK_CRASH_AGG"));
    let marker = PathBuf::from(std::env::var("IK_CRASH_MARKER").expect("IK_CRASH_MARKER"));

    let machine: Box<dyn ik::machine::Machine> = Box::new(load_machine("verification_attempt"));
    let identity = machine.identity().clone();
    let mut store = Store::open(&store_path).expect("child opens store");
    let projection = store.show(&agg).expect("child shows aggregate");
    drop(store); // release the store connection before the raw crash connection

    let snapshot = AggregateSnapshot {
        aggregate_id: agg.clone(),
        machine: projection.machine.clone(),
        state: projection.state.clone(),
        terminal: projection.terminal,
        generation: projection.generation,
        nonce: projection.capability.nonce.clone(),
        effect: projection.effect.clone(),
        sequence: projection.sequence,
        last_event_hash: projection.last_event_hash.clone(),
        last_receipt_id: projection
            .last_receipt
            .as_ref()
            .map(|receipt| receipt.receipt_id.clone())
            .unwrap_or_default(),
    };
    let step = ik::runtime::execute_command(
        machine.as_ref(),
        &snapshot,
        projection.last_receipt.as_ref(),
        &projection.capability,
        &json!({"name": "submit", "args": {"artifact_digest": "crash-a", "baseline_digest": "crash-b"}}),
        None,
    )
    .expect("child computes the transition");
    let Step::Commit(plan) = step else {
        panic!("crash child expected a commit plan, got {step:?}");
    };
    let nonce = ik::runtime::derive_nonce(&identity, &agg, plan.event.generation);

    // Mirror store::persist_plan inside an uncommitted IMMEDIATE transaction, then
    // die before COMMIT. Recovery on the next open must discard all of it.
    let mut conn = rusqlite::Connection::open(&store_path).expect("child opens raw connection");
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("child begins write transaction");
    tx.execute(
        "INSERT INTO journal (aggregate_id, sequence, generation, hash, prev_hash, event_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            agg.0,
            plan.event.sequence as i64,
            plan.event.generation as i64,
            plan.event.hash,
            plan.event.prev_hash,
            ik::canonical::canonical_json(&plan.event)
        ],
    )
    .expect("child journal write");
    tx.execute(
        "INSERT INTO receipts (aggregate_id, sequence, receipt_id, receipt_json) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            agg.0,
            plan.event.sequence as i64,
            plan.receipt.receipt_id,
            ik::canonical::canonical_json(&plan.receipt)
        ],
    )
    .expect("child receipt write");
    tx.execute(
        "UPDATE aggregates SET state = ?1, terminal = ?2, generation = ?3, nonce = ?4, effect_json = ?5, sequence = ?6, last_event_hash = ?7, last_receipt_id = ?8 WHERE aggregate_id = ?9",
        rusqlite::params![
            ik::canonical::canonical_json(&plan.state),
            plan.event.terminal as i64,
            plan.event.generation as i64,
            nonce,
            ik::canonical::canonical_json(&plan.effect_after),
            plan.event.sequence as i64,
            plan.event.hash,
            plan.receipt.receipt_id,
            agg.0
        ],
    )
    .expect("child projection write");

    std::fs::write(&marker, "armed").expect("write crash marker");
    // Die with the transaction open: never COMMIT, never ROLLBACK.
    std::process::abort();
}

/// Run one injected-failure atomicity probe on the ATOMIC create-with-command
/// path (`create_aggregate_with_command`: genesis + initial command in ONE
/// transaction; contract "aggregate genesis + admission is one SQLite
/// transaction").
///
/// Setup: a fresh store shell (no aggregate). `inject` installs a failure at one
/// fallible persist step of the next create+admit (seed row or trigger);
/// `cleanup` removes it afterwards. The probe asserts, after REOPEN, that no
/// aggregate is observable through the supported API and that the journal and
/// receipts tables contain exactly the committed rows plus any pre-seeded
/// injection row. The positive control then commits the identical create+admit
/// and verifies the store.
fn create_command_atomicity_probe(
    dir: &tempfile::TempDir,
    expected_journal_rows: i64,
    expected_receipt_rows: i64,
    inject: impl FnOnce(&Path),
    cleanup: impl FnOnce(&Path),
) {
    let path = dir.path().join("s.db");
    let machine = va();
    let identity = machine.identity().clone();
    let mut store = Store::create(&path).expect("create store");
    let agg = AggregateId("agg-1".into());

    inject(&path);
    let err = store
        .create_aggregate_with_command(
            machine.as_ref(),
            &agg,
            &identity,
            &json!({"name": "start"}),
            None,
        )
        .expect_err("the injected failure must refuse the atomic create+admit");
    assert_eq!(err.code, DiagnosticCode::IkStoreCorrupt);
    drop(store);

    // Reopen: the rollback must be durable — neither genesis nor admission leaked.
    let mut store = Store::open(&path).expect("reopen after injected failure");
    let ghost = store
        .show(&agg)
        .expect_err("no aggregate may be observable after the failed create+admit");
    assert_eq!(ghost.code, DiagnosticCode::IkIllegalState);
    assert_eq!(
        journal_rows(&path, &agg.0),
        expected_journal_rows,
        "no genesis/admission event may survive the injected failure"
    );
    assert_eq!(
        receipt_rows(&path, &agg.0),
        expected_receipt_rows,
        "no genesis/admission receipt may survive the injected failure"
    );
    drop(store);

    // Positive control: with the injection removed, the identical create+admit
    // commits and the store verifies.
    cleanup(&path);
    let mut store = Store::open(&path).expect("reopen for the positive control");
    store
        .create_aggregate_with_command(
            machine.as_ref(),
            &agg,
            &identity,
            &json!({"name": "start"}),
            None,
        )
        .expect("create+admit commits after the injection is removed");
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after the recovered commit");
}

#[test]
fn create_with_command_journal_insert_cannot_escape_the_transaction() {
    // Failure injected AT the genesis journal INSERT: a pre-seeded journal row at
    // sequence 0 collides with the genesis append's primary key. Neither genesis
    // nor the initial command may become observable.
    create_command_atomicity_probe(
        &tempfile::tempdir().expect("tempdir"),
        1, // the pre-seeded conflicting journal row
        0,
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for seeding");
            conn.execute(
                "INSERT INTO journal (aggregate_id, sequence, generation, hash, prev_hash, event_json) VALUES ('agg-1', 0, 0, 'seeded', 'seeded', '{}')",
                [],
            )
            .expect("seed conflicting journal row");
            drop(conn);
        },
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for cleanup");
            conn.execute(
                "DELETE FROM journal WHERE aggregate_id = 'agg-1' AND hash = 'seeded'",
                [],
            )
            .expect("clear seed");
            drop(conn);
        },
    );
}

#[test]
fn create_with_command_admit_append_cannot_escape_the_transaction() {
    // Failure injected AFTER the genesis journal/receipt/projection row were
    // written but INSIDE the same transaction: a pre-seeded receipts row at
    // sequence 1 collides with the ADMIT receipt INSERT. The already-appended
    // genesis must roll back together with the failed admission — no partial
    // aggregate may survive between the two events. If the genesis step had
    // escaped the transaction (two-transaction sequence), the torn aggregate
    // would survive and this test would fail.
    create_command_atomicity_probe(
        &tempfile::tempdir().expect("tempdir"),
        0,
        1, // the pre-seeded conflicting receipt row
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for seeding");
            conn.execute(
                "INSERT INTO receipts (aggregate_id, sequence, receipt_id, receipt_json) VALUES ('agg-1', 1, 'seeded', '{}')",
                [],
            )
            .expect("seed conflicting receipt");
            drop(conn);
        },
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for cleanup");
            conn.execute(
                "DELETE FROM receipts WHERE aggregate_id = 'agg-1' AND receipt_id = 'seeded'",
                [],
            )
            .expect("clear seed");
            drop(conn);
        },
    );
}

#[test]
fn create_with_command_projection_write_cannot_escape_the_transaction() {
    // Failure injected AFTER the admission's projection write, BEFORE commit: an
    // AFTER UPDATE trigger on the projection table raises ROLLBACK once the
    // aggregates row has been rewritten by the initial command. Genesis,
    // admission, receipts, and the projection must all roll back together.
    create_command_atomicity_probe(
        &tempfile::tempdir().expect("tempdir"),
        0,
        0,
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for injection");
            conn.execute_batch(
                "CREATE TRIGGER ik_injected_abort AFTER UPDATE ON aggregates BEGIN SELECT RAISE(ROLLBACK, 'injected failure after projection write'); END;",
            )
            .expect("install failure injection after the projection write");
            drop(conn);
        },
        |path| {
            let conn = rusqlite::Connection::open(path).expect("open store for cleanup");
            conn.execute_batch("DROP TRIGGER ik_injected_abort;")
                .expect("remove injection");
            drop(conn);
        },
    );
}
