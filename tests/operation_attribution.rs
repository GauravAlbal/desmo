//! IK-03: durable, atomic caller-operation attribution (`docs/operation-identity.md`).
//!
//! A caller-stable `operation_id` is recorded in the same SQLite transaction as the
//! mutation it names. These tests drive the public store and CLI paths and assert,
//! against authoritative rows, what was and was not written.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::diagnostic::DiagnosticCode;
use ik::effect::EffectOutcomeV1;
use ik::identity::{AggregateId, Capability, EffectId, MachineIdentity};
use ik::machine::Machine;
use ik::operation::{Lookup, OperationId, OperationKind, OperationOutcome, OperationRequest};
use ik::protocol::TransactionStep;
use ik::store::{
    CommandRequest, CreateWithCommandOutcome, EffectOutcomeRequest, Store, TransactionRequest,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
    store: Store,
    machine: Box<dyn Machine>,
    agg: AggregateId,
}

fn fixture_for(machine_id: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut store = Store::create(&path).unwrap();
    let machine: Box<dyn Machine> = Box::new(load_machine(machine_id));
    let agg = AggregateId("agg-1".into());
    store
        .create_aggregate(machine.as_ref(), &agg, machine.identity())
        .unwrap();
    Fixture {
        _dir: dir,
        path,
        store,
        machine,
        agg,
    }
}

fn fixture() -> Fixture {
    fixture_for("verification_attempt")
}

fn cap(identity: &MachineIdentity, agg: &AggregateId, generation: u64) -> Capability {
    Capability {
        generation,
        nonce: ik::runtime::derive_nonce(identity, agg, generation),
    }
}

impl Fixture {
    fn identity(&self) -> MachineIdentity {
        self.machine.identity().clone()
    }

    fn request(&self, generation: u64, command: Value) -> CommandRequest {
        CommandRequest {
            aggregate_id: self.agg.clone(),
            machine: self.identity(),
            capability: cap(&self.machine.identity().clone(), &self.agg, generation),
            command,
            observation: None,
        }
    }

    fn command_op(
        &mut self,
        generation: u64,
        command: Value,
        op: &str,
    ) -> Result<OperationOutcome<ik::store::CommandResult>, ik::diagnostic::Diagnostic> {
        let request = self.request(generation, command);
        self.store.command_op(
            self.machine.as_ref(),
            &request,
            &OperationId::parse(op).unwrap(),
        )
    }

    fn rows(&self) -> Value {
        authoritative_rows(&self.path)
    }

    fn journal_hash(&self, sequence: u64) -> String {
        Connection::open(&self.path)
            .unwrap()
            .query_row(
                "SELECT hash FROM journal WHERE aggregate_id = ?1 AND sequence = ?2",
                rusqlite::params![self.agg.0, sequence as i64],
                |row| row.get(0),
            )
            .unwrap()
    }
}

fn start() -> Value {
    json!({"name": "start"})
}

fn cancel() -> Value {
    json!({"name": "cancel"})
}

/// Every authoritative row, rendered in stable order.
fn authoritative_rows(path: &Path) -> Value {
    let conn = Connection::open(path).expect("open store");
    let mut out = serde_json::Map::new();
    for (table, order) in [
        ("aggregates", "aggregate_id"),
        ("journal", "aggregate_id, sequence"),
        ("receipts", "aggregate_id, sequence"),
        ("operations", "aggregate_id, operation_id"),
    ] {
        let Ok(mut stmt) = conn.prepare(&format!("SELECT * FROM {table} ORDER BY {order}")) else {
            // Only a pre-migration (version 3) store lacks `operations`.
            assert_eq!(table, "operations");
            continue;
        };
        let width = stmt.column_count();
        let rows = stmt
            .query_map([], |row| {
                let mut cells = Vec::with_capacity(width);
                for i in 0..width {
                    let value: rusqlite::types::Value = row.get(i)?;
                    cells.push(format!("{value:?}"));
                }
                Ok(Value::from(cells))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        out.insert(table.to_string(), Value::from(rows));
    }
    Value::Object(out)
}

fn operation_count(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn lost_response_then_other_writer_is_reconciled_by_exact_repeat() {
    let mut f = fixture();
    // The reply to this committed start is lost: the caller never sees the result.
    let _lost = f.command_op(0, start(), "op-start").unwrap();
    // Another writer advances the aggregate past the original capability.
    let advanced = f
        .store
        .command(f.machine.as_ref(), &f.request(1, cancel()))
        .unwrap();
    assert_eq!(advanced.generation, 2);
    let before = f.rows();

    let repeat = f.command_op(0, start(), "op-start").unwrap();
    let OperationOutcome::Historical(history) = repeat else {
        panic!("an exact repeat of a committed operation must be historical");
    };
    assert_eq!(history.record.kind, OperationKind::Command);
    assert_eq!(history.record.first_sequence, 1);
    assert_eq!(history.record.last_sequence, 1);
    assert_eq!(history.record.generation, 1);
    assert_eq!(history.record.event_hashes, vec![f.journal_hash(1)]);
    // The historical result is not current: the aggregate has moved on.
    assert_eq!(history.current_generation, 2);
    assert_eq!(history.current_sequence, 2);
    assert_eq!(f.rows(), before, "an exact repeat must not mutate anything");
}

#[test]
fn stale_request_without_an_operation_id_still_refuses() {
    let mut f = fixture();
    f.command_op(0, start(), "op-start").unwrap();
    f.store
        .command(f.machine.as_ref(), &f.request(1, cancel()))
        .unwrap();
    let err = f
        .store
        .command(f.machine.as_ref(), &f.request(0, start()))
        .expect_err("a non-duplicate stale proposal refuses");
    assert_eq!(err.code, DiagnosticCode::IkCapabilityStale);
}

#[test]
fn current_state_alone_cannot_tell_who_committed_but_attribution_can() {
    // World 1: the caller's start (op-A) committed and its reply was lost.
    let mut world1 = fixture();
    world1.command_op(0, start(), "op-A").unwrap();
    // World 2: op-A never arrived; a different writer's start committed instead.
    let mut world2 = fixture();
    world2.command_op(0, start(), "op-B").unwrap();

    let shown1 = world1.store.show(&world1.agg).unwrap();
    let shown2 = world2.store.show(&world2.agg).unwrap();
    // A current-state read cannot distinguish the worlds.
    assert_eq!(shown1.state, shown2.state);
    assert_eq!(shown1.generation, shown2.generation);
    assert_eq!(shown1.sequence, shown2.sequence);
    assert_eq!(shown1.last_event_hash, shown2.last_event_hash);

    // Attribution does.
    let id = OperationId::parse("op-A").unwrap();
    let request = world1.request(0, start());
    let in_world1 = world1
        .store
        .lookup_operation(&id, &OperationRequest::Command(&request))
        .unwrap();
    let request = world2.request(0, start());
    let in_world2 = world2
        .store
        .lookup_operation(&id, &OperationRequest::Command(&request))
        .unwrap();
    assert!(
        matches!(in_world1, Lookup::CommittedExact(_)),
        "{in_world1:?}"
    );
    assert!(
        matches!(in_world2, Lookup::NotRecorded { .. }),
        "absence must be reported as not recorded, never as refused: {in_world2:?}"
    );
}

#[test]
fn reusing_an_id_for_different_semantics_or_a_refreshed_capability_conflicts() {
    let mut f = fixture();
    f.command_op(0, start(), "op-1").unwrap();
    let before = f.rows();

    let changed = f
        .command_op(0, cancel(), "op-1")
        .expect_err("changed command");
    assert_eq!(changed.code, DiagnosticCode::IkOperationConflict);

    // The same command under a refreshed capability is a different request.
    let refreshed = f
        .command_op(1, start(), "op-1")
        .expect_err("refreshed capability");
    assert_eq!(refreshed.code, DiagnosticCode::IkOperationConflict);

    let id = OperationId::parse("op-1").unwrap();
    let request = f.request(0, cancel());
    let lookup = f
        .store
        .lookup_operation(&id, &OperationRequest::Command(&request))
        .unwrap();
    let Lookup::Conflict {
        kind,
        first_sequence,
        ..
    } = lookup
    else {
        panic!("lookup of a different request under a used id must conflict: {lookup:?}");
    };
    assert_eq!(kind, OperationKind::Command);
    assert_eq!(first_sequence, 1);
    assert_eq!(f.rows(), before, "conflicting reuse must not write");
}

#[test]
fn refused_requests_leave_no_operation_row_and_lookup_is_not_recorded() {
    let mut f = fixture();
    let before = f.rows();
    // `submit` is illegal from the initial state: the kernel refuses.
    let refused = f
        .command_op(
            0,
            json!({"name": "submit", "args": {"artifact_digest": "a", "baseline_digest": "b"}}),
            "op-refused",
        )
        .expect_err("illegal state refuses");
    assert_eq!(refused.code, DiagnosticCode::IkIllegalState);
    assert_eq!(f.rows(), before, "a refusal appends nothing");
    assert_eq!(operation_count(&f.path), 0);

    let id = OperationId::parse("op-refused").unwrap();
    let request = f.request(
        0,
        json!({"name": "submit", "args": {"artifact_digest": "a", "baseline_digest": "b"}}),
    );
    let lookup = f
        .store
        .lookup_operation(&id, &OperationRequest::Command(&request))
        .unwrap();
    assert_eq!(
        lookup,
        Lookup::NotRecorded {
            current_generation: Some(0),
            current_sequence: Some(0)
        }
    );
}

#[test]
fn a_batch_is_one_operation_row_over_its_consecutive_events() {
    let mut f = fixture();
    let request = TransactionRequest {
        aggregate_id: f.agg.clone(),
        machine: f.identity(),
        capability: cap(&f.identity(), &f.agg, 0),
        steps: vec![
            TransactionStep {
                command: start(),
                observation: None,
            },
            TransactionStep {
                command: cancel(),
                observation: None,
            },
        ],
    };
    let id = OperationId::parse("op-batch").unwrap();
    let committed = f
        .store
        .transaction_op(f.machine.as_ref(), &request, &id)
        .unwrap();
    assert!(matches!(&committed, OperationOutcome::Committed(r) if r.len() == 2));
    let before = f.rows();

    let repeat = f
        .store
        .transaction_op(f.machine.as_ref(), &request, &id)
        .unwrap();
    let OperationOutcome::Historical(history) = repeat else {
        panic!("an exact repeat of a batch must be historical");
    };
    assert_eq!(history.record.kind, OperationKind::Transaction);
    assert_eq!(history.record.first_sequence, 1);
    assert_eq!(history.record.last_sequence, 2);
    assert_eq!(
        history.record.event_hashes,
        vec![f.journal_hash(1), f.journal_hash(2)]
    );
    assert_eq!(f.rows(), before);
    assert!(f.store.verify(f.machine.as_ref(), None).unwrap().verified);
}

#[test]
fn effect_outcome_attribution_is_independent_of_effect_deduplication() {
    let mut f = fixture_for("effect_probe");
    let planned = f
        .store
        .command(
            f.machine.as_ref(),
            &f.request(
                0,
                json!({"name": "plan", "args": {"effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1"}}),
            ),
        )
        .unwrap();
    let agg = f.agg.clone();
    let identity = f.identity();
    let outcome = |digest: &str| EffectOutcomeV1 {
        effect_id: EffectId("e1".into()),
        aggregate_id: agg.clone(),
        planned_generation: 1,
        payload_digest: "p1".into(),
        idempotency_key: "k1".into(),
        outcome_digest: digest.into(),
    };
    let request = |digest: &str| EffectOutcomeRequest {
        aggregate_id: agg.clone(),
        machine: identity.clone(),
        capability: planned.capability.clone(),
        effect: outcome(digest),
    };
    let id = OperationId::parse("op-record").unwrap();
    let recorded = f.store.record_effect_op(&request("d1"), &id).unwrap();
    assert!(matches!(&recorded, OperationOutcome::Committed(r) if r.mutation));
    let before = f.rows();

    let repeat = f.store.record_effect_op(&request("d1"), &id).unwrap();
    assert!(
        matches!(&repeat, OperationOutcome::Historical(h) if h.record.kind == OperationKind::Record),
        "{repeat:?}"
    );
    assert_eq!(f.rows(), before);

    // A different outcome under the same operation id is an operation conflict,
    // not an effect-outcome conflict.
    let conflict = f
        .store
        .record_effect_op(&request("d2"), &id)
        .expect_err("different request under the used id");
    assert_eq!(conflict.code, DiagnosticCode::IkOperationConflict);
}

#[test]
fn create_with_command_is_attributed_and_an_exact_repeat_skips_the_reducer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut store = Store::create(&path).unwrap();
    let machine: Box<dyn Machine> = Box::new(load_machine("verification_attempt"));
    let agg = AggregateId("born-1".into());
    let id = OperationId::parse("op-create").unwrap();

    let missing = store
        .lookup_operation(
            &id,
            &OperationRequest::CreateWithCommand {
                machine: machine.identity(),
                aggregate_id: &agg,
                command: &start(),
                observation: None,
            },
        )
        .unwrap();
    assert_eq!(
        missing,
        Lookup::NotRecorded {
            current_generation: None,
            current_sequence: None
        }
    );

    let first = store
        .create_aggregate_with_command_op(
            machine.as_ref(),
            &agg,
            machine.identity(),
            &start(),
            None,
            &id,
        )
        .unwrap();
    assert!(matches!(
        first,
        OperationOutcome::Committed(CreateWithCommandOutcome::Committed(_))
    ));
    let before = authoritative_rows(&path);

    let repeat = store
        .create_aggregate_with_command_op(
            machine.as_ref(),
            &agg,
            machine.identity(),
            &start(),
            None,
            &id,
        )
        .unwrap();
    let OperationOutcome::Historical(history) = repeat else {
        panic!("an exact repeat must be historical, got {repeat:?}");
    };
    assert_eq!(history.record.kind, OperationKind::CreateWithCommand);
    assert_eq!(history.record.first_sequence, 0);
    assert_eq!(history.record.last_sequence, 1);
    assert_eq!(authoritative_rows(&path), before);
}

#[test]
fn operation_ids_are_scoped_to_their_aggregate() {
    let mut f = fixture();
    let other = AggregateId("agg-2".into());
    f.store
        .create_aggregate(f.machine.as_ref(), &other, f.machine.identity())
        .unwrap();
    f.command_op(0, start(), "shared-id").unwrap();
    let request = CommandRequest {
        aggregate_id: other.clone(),
        machine: f.identity(),
        capability: cap(&f.identity(), &other, 0),
        command: start(),
        observation: None,
    };
    let outcome = f
        .store
        .command_op(
            f.machine.as_ref(),
            &request,
            &OperationId::parse("shared-id").unwrap(),
        )
        .unwrap();
    assert!(matches!(outcome, OperationOutcome::Committed(_)));
    assert_eq!(operation_count(&f.path), 2);
}

#[test]
fn a_wrong_machine_identity_is_refused_before_any_history_is_revealed() {
    let mut f = fixture();
    f.command_op(0, start(), "op-1").unwrap();
    let foreign: Box<dyn Machine> = Box::new(load_machine("effect_probe"));
    let mut request = f.request(0, start());
    request.machine = foreign.identity().clone();
    let err = f
        .store
        .command_op(
            f.machine.as_ref(),
            &request,
            &OperationId::parse("op-1").unwrap(),
        )
        .expect_err("identity mismatch");
    assert_eq!(err.code, DiagnosticCode::IkMachineMismatch);
}

#[test]
fn a_version_3_store_migrates_in_place_without_inventing_operation_ids() {
    let mut f = fixture();
    f.store
        .command(f.machine.as_ref(), &f.request(0, start()))
        .unwrap();
    let path = f.path.clone();
    let machine_id = f.machine.identity().clone();
    let Fixture { _dir, store, .. } = f;
    drop(store);
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE operations; UPDATE meta SET value = '3' WHERE key = 'schema_version';",
        )
        .unwrap();
    }
    let journal_before = {
        let rows = authoritative_rows(&path);
        (rows["journal"].clone(), rows["aggregates"].clone())
    };

    let mut store = Store::open(&path).expect("version 3 store migrates on open");
    let schema: String = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(schema, ik::store::SCHEMA_VERSION.to_string());
    assert_eq!(operation_count(&path), 0, "no historical ids are invented");
    let after = authoritative_rows(&path);
    assert_eq!(
        (after["journal"].clone(), after["aggregates"].clone()),
        journal_before
    );

    // The migrated store accepts attributed mutations.
    let agg = AggregateId("agg-1".into());
    let machine: Box<dyn Machine> = Box::new(load_machine("verification_attempt"));
    let request = CommandRequest {
        aggregate_id: agg.clone(),
        machine: machine_id.clone(),
        capability: cap(&machine_id, &agg, 1),
        command: cancel(),
        observation: None,
    };
    let outcome = store
        .command_op(
            machine.as_ref(),
            &request,
            &OperationId::parse("after-migration").unwrap(),
        )
        .unwrap();
    assert!(matches!(outcome, OperationOutcome::Committed(_)));
    assert_eq!(operation_count(&path), 1);
}

#[test]
fn a_store_from_a_newer_schema_is_refused() {
    let f = fixture();
    let path = f.path.clone();
    let Fixture { _dir, store, .. } = f;
    drop(store);
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE meta SET value = '5' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();
    let err = Store::open(&path)
        .err()
        .expect("newer schema must be refused");
    assert_eq!(err.code, DiagnosticCode::IkPersistedSchemaUnknown);
}

#[test]
fn verify_refuses_an_operation_bound_to_an_event_that_is_not_in_the_journal() {
    let mut f = fixture();
    f.command_op(0, start(), "op-1").unwrap();
    assert!(f.store.verify(f.machine.as_ref(), None).unwrap().verified);

    let conn = Connection::open(&f.path).unwrap();
    conn.execute("UPDATE operations SET event_hashes = '[\"deadbeef\"]'", [])
        .unwrap();
    let err = f
        .store
        .verify(f.machine.as_ref(), None)
        .expect_err("a binding without its commit is corruption");
    assert_eq!(err.code, DiagnosticCode::IkJournalCorrupt);
    assert_eq!(err.rule.as_deref(), Some("operations.binding"));

    // Positive control: restoring the true hash makes verify pass again.
    let real = f.journal_hash(1);
    conn.execute(
        "UPDATE operations SET event_hashes = ?1",
        rusqlite::params![format!("[\"{real}\"]")],
    )
    .unwrap();
    assert!(f.store.verify(f.machine.as_ref(), None).unwrap().verified);
}

#[test]
fn a_failed_attribution_write_rolls_back_the_mutation() {
    let mut f = fixture();
    let before = f.rows();
    let conn = Connection::open(&f.path).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_operation BEFORE INSERT ON operations \
         BEGIN SELECT RAISE(ABORT, 'injected attribution failure'); END;",
    )
    .unwrap();
    let err = f
        .command_op(0, start(), "op-1")
        .expect_err("the attribution insert fails");
    assert_eq!(err.code, DiagnosticCode::IkStoreCorrupt);
    assert_eq!(
        f.rows(),
        before,
        "a mutation must never commit without its attribution row"
    );

    // Positive control: with the injection removed the same operation commits.
    conn.execute_batch("DROP TRIGGER fail_operation;").unwrap();
    let outcome = f.command_op(0, start(), "op-1").unwrap();
    assert!(matches!(&outcome, OperationOutcome::Committed(r) if r.mutation));
    assert_eq!(operation_count(&f.path), 1);
}

#[test]
fn concurrent_identical_operations_commit_exactly_once() {
    let f = fixture();
    let path = f.path.clone();
    let agg = f.agg.clone();
    let Fixture { _dir, store, .. } = f;
    drop(store);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(6));
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let path = path.clone();
            let agg = agg.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let machine: Box<dyn Machine> = Box::new(load_machine("verification_attempt"));
                let identity = machine.identity().clone();
                let mut store = Store::open(&path).unwrap();
                let request = CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap(&identity, &agg, 0),
                    command: start(),
                    observation: None,
                };
                let id = OperationId::parse("race-op").unwrap();
                barrier.wait();
                store.command_op(machine.as_ref(), &request, &id)
            })
        })
        .collect();
    let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let committed = outcomes
        .iter()
        .filter(|o| matches!(o, Ok(OperationOutcome::Committed(_))))
        .count();
    let historical = outcomes
        .iter()
        .filter(|o| matches!(o, Ok(OperationOutcome::Historical(_))))
        .count();
    assert_eq!(committed, 1, "{outcomes:?}");
    assert_eq!(historical, 5, "{outcomes:?}");
    let rows = authoritative_rows(&path);
    assert_eq!(
        rows["journal"].as_array().unwrap().len(),
        2,
        "genesis + one start"
    );
    assert_eq!(rows["operations"].as_array().unwrap().len(), 1);
}

#[cfg(feature = "test-support")]
mod real_process_crash {
    use super::*;

    const CHILD_ENV: &str = "IK_OPERATION_CRASH_CHILD";

    /// Runs only when re-executed by `crash_before_commit_*`; otherwise a no-op.
    #[test]
    fn operation_crash_child_entry() {
        let spec = std::env::var(CHILD_ENV);
        if spec.is_err() {
            return;
        }
        let spec = spec.unwrap();
        let (path, boundary) = spec.split_once('|').expect("path|boundary");
        let armed = boundary.to_string();
        let hook: ik::store::CrashHook = Box::new(move |point: &str| {
            if point == armed {
                std::process::exit(86);
            }
            false
        });
        let mut store = Store::open_with_crash_hook(Path::new(path), hook).unwrap();
        let machine: Box<dyn Machine> = Box::new(load_machine("verification_attempt"));
        let identity = machine.identity().clone();
        let agg = AggregateId("agg-1".into());
        let request = CommandRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: cap(&identity, &agg, 0),
            command: start(),
            observation: None,
        };
        let outcome = store.command_op(
            machine.as_ref(),
            &request,
            &OperationId::parse("crash-op").unwrap(),
        );
        // The armed boundary exits 86 before this point; reaching it means it never fired.
        assert!(
            outcome.is_ok(),
            "command must succeed when no boundary fires: {outcome:?}"
        );
        std::process::exit(0);
    }

    fn run_child_crashing_at(path: &Path, boundary: &str) -> Option<i32> {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "real_process_crash::operation_crash_child_entry",
                "--exact",
                "--nocapture",
            ])
            .env(CHILD_ENV, format!("{}|{boundary}", path.display()))
            .status()
            .unwrap()
            .code()
    }

    struct CrashRun {
        code: Option<i32>,
        rows_unchanged: bool,
        committed_after_reopen: bool,
        repeat_is_historical: bool,
        operation_rows: i64,
        verified: bool,
    }

    /// Kill a child at `boundary`, then reopen and run the same operation again.
    fn crash_then_recover(boundary: &str) -> CrashRun {
        let f = fixture();
        let path = f.path.clone();
        let before = f.rows();
        let machine = f.machine;
        drop(f.store);

        let code = run_child_crashing_at(&path, boundary);
        let rows_unchanged = authoritative_rows(&path) == before;

        let mut store = Store::open(&path).unwrap();
        let identity = machine.identity().clone();
        let agg = AggregateId("agg-1".into());
        let request = CommandRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: cap(&identity, &agg, 0),
            command: start(),
            observation: None,
        };
        let id = OperationId::parse("crash-op").unwrap();
        let committed = store.command_op(machine.as_ref(), &request, &id).unwrap();
        let repeat = store.command_op(machine.as_ref(), &request, &id).unwrap();
        CrashRun {
            code,
            rows_unchanged,
            committed_after_reopen: matches!(&committed, OperationOutcome::Committed(r) if r.mutation),
            repeat_is_historical: matches!(repeat, OperationOutcome::Historical(_)),
            operation_rows: operation_count(&path),
            verified: store.verify(machine.as_ref(), None).unwrap().verified,
        }
    }

    #[test]
    fn crash_before_commit_at_each_attribution_boundary_leaves_no_binding() {
        for boundary in [
            "crash.command.after_operation",
            "crash.command.after_projection",
        ] {
            let run = crash_then_recover(boundary);
            assert_eq!(run.code, Some(86), "the child must die at `{boundary}`");
            assert!(
                run.rows_unchanged,
                "a crash at `{boundary}` leaves neither the mutation nor its binding"
            );
            assert!(
                run.committed_after_reopen,
                "`{boundary}`: operation commits after reopen"
            );
            assert!(
                run.repeat_is_historical,
                "`{boundary}`: second attempt is historical"
            );
            assert_eq!(run.operation_rows, 1, "`{boundary}`: exactly one binding");
            assert!(run.verified, "`{boundary}`: store verifies");
        }
    }
}

fn cli(store: &Path, machine: &Path, request: Value) -> Value {
    run_cli_json(store, Some(machine), &request)
}

fn envelope(op: &str, id: &str, identity: &MachineIdentity, extra: Value) -> Value {
    let mut base = json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": env!("CARGO_PKG_VERSION"),
        "request_id": id,
        "op": op,
        "aggregate_id": "a1",
        "machine": identity,
    });
    for (key, value) in extra.as_object().unwrap() {
        base[key] = value.clone();
    }
    base
}

#[test]
fn the_public_cli_reconciles_a_lost_reply_without_returning_a_capability() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("s.db");
    let machine_path = machine_def("verification_attempt");
    let identity = load_machine("verification_attempt").identity().clone();

    let created = cli(
        &store,
        &machine_path,
        envelope("create", "r1", &identity, json!({})),
    );
    assert_eq!(created["ok"], true, "{created}");
    let cap0 = created["result"]["capability"].clone();

    let started = cli(
        &store,
        &machine_path,
        envelope(
            "command",
            "r2",
            &identity,
            json!({"capability": cap0, "command": {"name": "start"}, "operation_id": "cli-op"}),
        ),
    );
    assert_eq!(started["ok"], true, "{started}");
    let cap1 = started["result"]["capability"].clone();

    // Another writer cancels; the caller's reply to `start` was "lost".
    let cancelled = cli(
        &store,
        &machine_path,
        envelope(
            "command",
            "r3",
            &identity,
            json!({"capability": cap1, "command": {"name": "cancel"}}),
        ),
    );
    assert_eq!(cancelled["ok"], true, "{cancelled}");

    // A fresh attempt (new request_id) with the original semantics is historical.
    let repeat = cli(
        &store,
        &machine_path,
        envelope(
            "command",
            "r4",
            &identity,
            json!({"capability": cap0, "command": {"name": "start"}, "operation_id": "cli-op"}),
        ),
    );
    assert_eq!(repeat["ok"], true, "{repeat}");
    assert_eq!(repeat["result"]["historical"], true);
    assert_eq!(repeat["result"]["mutation"], false);
    assert!(
        repeat["result"].get("capability").is_none(),
        "no authority in history"
    );
    assert!(repeat["result"].get("state").is_none());
    assert_eq!(repeat["result"]["operation"]["first_sequence"], 1);
    assert_eq!(repeat["result"]["current"]["generation"], 2);

    let looked_up = cli(
        &store,
        &machine_path,
        envelope(
            "lookup_operation",
            "r5",
            &identity,
            json!({"capability": cap0, "command": {"name": "start"}, "operation_id": "cli-op"}),
        ),
    );
    assert_eq!(
        looked_up["result"]["outcome"], "committed_exact",
        "{looked_up}"
    );

    let unknown = cli(
        &store,
        &machine_path,
        envelope(
            "lookup_operation",
            "r6",
            &identity,
            json!({"capability": cap0, "command": {"name": "start"}, "operation_id": "never-sent"}),
        ),
    );
    assert_eq!(unknown["result"]["outcome"], "not_recorded", "{unknown}");
}

#[test]
fn the_public_cli_rejects_malformed_or_misplaced_operation_ids() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("s.db");
    let machine_path = machine_def("verification_attempt");
    let identity = load_machine("verification_attempt").identity().clone();
    let created = cli(
        &store,
        &machine_path,
        envelope("create", "r1", &identity, json!({})),
    );
    let cap0 = created["result"]["capability"].clone();

    let malformed = cli(
        &store,
        &machine_path,
        envelope(
            "command",
            "r2",
            &identity,
            json!({"capability": cap0, "command": {"name": "start"}, "operation_id": "has space"}),
        ),
    );
    assert_eq!(malformed["diagnostic"]["code"], "IK_PROTOCOL_INVALID");

    let misplaced = cli(
        &store,
        &machine_path,
        envelope("show", "r3", &identity, json!({"operation_id": "ok-id"})),
    );
    assert_eq!(misplaced["diagnostic"]["code"], "IK_PROTOCOL_INVALID");
}

/// IK-06: each operation-attribution wall is deleted in turn. The honest build satisfies the
/// predicate; the mutated build must not, and the seam must actually have been reached
/// (`with_law_mutant` refuses to return otherwise).
#[cfg(feature = "test-support")]
mod wall_mutants {
    use super::*;
    use ik::law_mutant::{observe_law_mutant, with_law_mutant, LawMutant};

    fn lookup_start(f: &mut Fixture, op: &str) -> Lookup {
        let request = f.request(0, start());
        f.store
            .lookup_operation(
                &OperationId::parse(op).unwrap(),
                &OperationRequest::Command(&request),
            )
            .unwrap()
    }

    #[test]
    fn skipping_operation_binding_is_killed() {
        let mut honest = fixture();
        honest.command_op(0, start(), "op-1").unwrap();
        assert!(matches!(
            lookup_start(&mut honest, "op-1"),
            Lookup::CommittedExact(_)
        ));

        let mut mutated = fixture();
        with_law_mutant(LawMutant::SkipOperationBinding, || {
            mutated.command_op(0, start(), "op-1").unwrap();
        });
        assert_eq!(mutated.store.show(&mutated.agg).unwrap().generation, 1);
        assert_eq!(operation_count(&mutated.path), 0);
        assert!(
            !matches!(
                lookup_start(&mut mutated, "op-1"),
                Lookup::CommittedExact(_)
            ),
            "the committed mutation is unattributable: the honest assertion fails"
        );
    }

    #[test]
    fn skipping_the_duplicate_gate_is_killed() {
        fn lost_reply_then_repeat(
            f: &mut Fixture,
        ) -> Result<OperationOutcome<ik::store::CommandResult>, ik::diagnostic::Diagnostic>
        {
            f.command_op(0, start(), "op-1").unwrap();
            f.store
                .command(f.machine.as_ref(), &f.request(1, cancel()))
                .unwrap();
            f.command_op(0, start(), "op-1")
        }
        let mut honest = fixture();
        assert!(matches!(
            lost_reply_then_repeat(&mut honest),
            Ok(OperationOutcome::Historical(_))
        ));

        let mut mutated = fixture();
        let repeat = with_law_mutant(LawMutant::SkipDuplicateGate, || {
            lost_reply_then_repeat(&mut mutated)
        });
        let refused = repeat.expect_err("without the gate the exact repeat is judged stale");
        assert_eq!(refused.code, DiagnosticCode::IkCapabilityStale);
    }

    #[test]
    fn omitting_the_capability_from_the_fingerprint_is_killed() {
        fn reuse_under_refreshed_capability(
            f: &mut Fixture,
        ) -> Result<OperationOutcome<ik::store::CommandResult>, ik::diagnostic::Diagnostic>
        {
            f.command_op(0, start(), "op-1").unwrap();
            f.command_op(1, start(), "op-1")
        }
        let mut honest = fixture();
        let conflict = reuse_under_refreshed_capability(&mut honest).expect_err("conflict");
        assert_eq!(conflict.code, DiagnosticCode::IkOperationConflict);

        let mut mutated = fixture();
        let repeat = with_law_mutant(LawMutant::OmitCapabilityFromFingerprint, || {
            reuse_under_refreshed_capability(&mut mutated)
        });
        assert!(
            matches!(repeat, Ok(OperationOutcome::Historical(_))),
            "the mutant mistakes a different request for an exact repeat"
        );
    }

    #[test]
    fn a_mutant_whose_seam_is_not_reached_reports_zero_firings() {
        // Plain `command` (no operation id) never reaches the operation-binding seam.
        let mut f = fixture();
        let (_, fired) = observe_law_mutant(LawMutant::SkipOperationBinding, || {
            f.store
                .command(f.machine.as_ref(), &f.request(0, start()))
                .unwrap()
        });
        assert_eq!(fired, 0, "evidence must not claim a seam that did not fire");
    }

    fn transaction_request(f: &Fixture, generation: u64) -> TransactionRequest {
        TransactionRequest {
            aggregate_id: f.agg.clone(),
            machine: f.identity(),
            capability: cap(&f.identity(), &f.agg, generation),
            steps: vec![TransactionStep {
                command: start(),
                observation: None,
            }],
        }
    }

    #[test]
    fn skipping_the_duplicate_gate_is_killed_for_transactions() {
        fn lost_reply_then_repeat(
            f: &mut Fixture,
        ) -> Result<OperationOutcome<Vec<ik::store::CommandResult>>, ik::diagnostic::Diagnostic>
        {
            let id = OperationId::parse("op-batch").unwrap();
            let request = transaction_request(f, 0);
            f.store
                .transaction_op(f.machine.as_ref(), &request, &id)
                .unwrap();
            f.store
                .command(f.machine.as_ref(), &f.request(1, cancel()))
                .unwrap();
            f.store.transaction_op(f.machine.as_ref(), &request, &id)
        }
        let mut honest = fixture();
        assert!(matches!(
            lost_reply_then_repeat(&mut honest),
            Ok(OperationOutcome::Historical(_))
        ));
        let mut mutated = fixture();
        let repeat = with_law_mutant(LawMutant::SkipDuplicateGate, || {
            lost_reply_then_repeat(&mut mutated)
        });
        assert_eq!(
            repeat.expect_err("exact batch repeat judged stale").code,
            DiagnosticCode::IkCapabilityStale
        );
    }

    #[test]
    fn omitting_the_capability_from_the_fingerprint_is_killed_for_transactions() {
        fn reuse_under_refreshed_capability(
            f: &mut Fixture,
        ) -> Result<OperationOutcome<Vec<ik::store::CommandResult>>, ik::diagnostic::Diagnostic>
        {
            let id = OperationId::parse("op-batch").unwrap();
            f.store
                .transaction_op(f.machine.as_ref(), &transaction_request(f, 0), &id)
                .unwrap();
            f.store
                .transaction_op(f.machine.as_ref(), &transaction_request(f, 1), &id)
        }
        let mut honest = fixture();
        assert_eq!(
            reuse_under_refreshed_capability(&mut honest)
                .expect_err("conflict")
                .code,
            DiagnosticCode::IkOperationConflict
        );
        let mut mutated = fixture();
        let repeat = with_law_mutant(LawMutant::OmitCapabilityFromFingerprint, || {
            reuse_under_refreshed_capability(&mut mutated)
        });
        assert!(matches!(repeat, Ok(OperationOutcome::Historical(_))));
    }

    /// One planned effect on the effect probe, ready to record its outcome.
    fn planned_effect() -> (Fixture, EffectOutcomeRequest) {
        let mut f = fixture_for("effect_probe");
        let planned = f
            .store
            .command(
                f.machine.as_ref(),
                &f.request(
                    0,
                    json!({"name": "plan", "args": {"effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1"}}),
                ),
            )
            .unwrap();
        let request = EffectOutcomeRequest {
            aggregate_id: f.agg.clone(),
            machine: f.identity(),
            capability: planned.capability.clone(),
            effect: EffectOutcomeV1 {
                effect_id: EffectId("e1".into()),
                aggregate_id: f.agg.clone(),
                planned_generation: 1,
                payload_digest: "p1".into(),
                idempotency_key: "k1".into(),
                outcome_digest: "d1".into(),
            },
        };
        (f, request)
    }

    #[test]
    fn skipping_the_duplicate_gate_is_killed_for_effect_records() {
        let (mut honest, request) = planned_effect();
        let id = OperationId::parse("op-record").unwrap();
        honest.store.record_effect_op(&request, &id).unwrap();
        assert!(matches!(
            honest.store.record_effect_op(&request, &id),
            Ok(OperationOutcome::Historical(_))
        ));
        let (mut mutated, request) = planned_effect();
        mutated.store.record_effect_op(&request, &id).unwrap();
        let repeat = with_law_mutant(LawMutant::SkipDuplicateGate, || {
            mutated.store.record_effect_op(&request, &id)
        });
        assert!(
            !matches!(repeat, Ok(OperationOutcome::Historical(_))),
            "without the gate an exact record repeat is not answered from history"
        );
    }

    #[test]
    fn omitting_the_capability_from_the_fingerprint_is_killed_for_effect_records() {
        fn reuse_under_other_capability(
            f: &mut Fixture,
            request: &EffectOutcomeRequest,
        ) -> Result<OperationOutcome<ik::store::CommandResult>, ik::diagnostic::Diagnostic>
        {
            let id = OperationId::parse("op-record").unwrap();
            f.store.record_effect_op(request, &id).unwrap();
            let refreshed = EffectOutcomeRequest {
                capability: cap(&f.identity(), &f.agg, 2),
                ..request.clone()
            };
            f.store.record_effect_op(&refreshed, &id)
        }
        let (mut honest, request) = planned_effect();
        assert_eq!(
            reuse_under_other_capability(&mut honest, &request)
                .expect_err("conflict")
                .code,
            DiagnosticCode::IkOperationConflict
        );
        let (mut mutated, request) = planned_effect();
        let repeat = with_law_mutant(LawMutant::OmitCapabilityFromFingerprint, || {
            reuse_under_other_capability(&mut mutated, &request)
        });
        assert!(matches!(repeat, Ok(OperationOutcome::Historical(_))));
    }

    #[test]
    fn skipping_the_duplicate_gate_is_killed_for_create_with_command() {
        fn create_twice(
            f: &mut Fixture,
        ) -> Result<OperationOutcome<CreateWithCommandOutcome>, ik::diagnostic::Diagnostic>
        {
            let id = OperationId::parse("op-create").unwrap();
            let agg = AggregateId("agg-2".into());
            let mut create = |store: &mut Store| {
                store.create_aggregate_with_command_op(
                    f.machine.as_ref(),
                    &agg,
                    f.machine.identity(),
                    &start(),
                    None,
                    &id,
                )
            };
            create(&mut f.store).unwrap();
            create(&mut f.store)
        }
        let mut honest = fixture();
        assert!(matches!(
            create_twice(&mut honest),
            Ok(OperationOutcome::Historical(_))
        ));
        let mut mutated = fixture();
        let repeat = with_law_mutant(LawMutant::SkipDuplicateGate, || create_twice(&mut mutated));
        assert!(
            !matches!(repeat, Ok(OperationOutcome::Historical(_))),
            "without the gate the repeat is not answered from the operation's recorded history"
        );
    }
}
