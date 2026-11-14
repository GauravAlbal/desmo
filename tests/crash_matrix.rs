#![cfg(feature = "test-support")]
//! Real process-crash matrix across every persistence boundary (IK-LAW-004,
//! REQ-IK-006 "injected failure fixtures leave no partial supported state",
//! REQ-IK-008 replay, REQ-IK-011 receipts).
//!
//! `atomicity.rs` proves SQL-level failure atomicity with triggers/seed rows and a
//! single crash proof whose child mirrors the persist payload by hand. This file
//! extends that to a full matrix: a REAL child process runs the store's actual
//! `create_aggregate` / `command` / `record_effect` code paths with an explicitly
//! injected, single-operation, RAII-cleared crash hook (`Store::open_with_crash_hook`,
//! never armed by production code and never reachable from ambient process state)
//! that terminates the child (SIGABRT or exit 86) at a deterministic named
//! persistence boundary:
//!
//! - genesis create: `crash.genesis.after_journal`, `crash.genesis.after_receipts`,
//!   `crash.genesis.after_projection`, `crash.genesis.after_aggregates_insert`
//!   (full payload written, immediately before COMMIT);
//! - ATOMIC create-with-command (`create_aggregate_with_command`, genesis + the
//!   initial command in ONE transaction): the same four genesis boundaries PLUS
//!   `crash.admit.after_journal`, `crash.admit.after_receipts`,
//!   `crash.admit.after_projection` — the boundaries INSIDE the transaction
//!   BETWEEN the genesis and the admission persist. A crash at any of them must
//!   leave the aggregate WHOLLY ABSENT, and the exact retry must commit exactly
//!   once (no multi-WAL partial state, no torn genesis);
//! - domain command: `crash.command.after_journal`, `crash.command.after_receipts`,
//!   `crash.command.after_projection` (immediately before COMMIT);
//! - kernel record: `crash.record.after_journal`, `crash.record.after_receipts`,
//!   `crash.record.after_projection` (immediately before COMMIT).
//!
//! Synchronization is sleep-free: the child writes a `prepared` marker naming the
//! armed failpoint, dies at it, and would write `done` only if the operation
//! completed; the parent blocks on process exit and then reads the markers.
//!
//! Every row asserts the same acceptance after reopen:
//! - genesis boundaries are WHOLLY ABSENT (no aggregate observable, zero journal and
//!   receipt rows, CLI `show` refuses);
//! - command/record boundaries are EXACTLY the last committed state (every projection
//!   field unchanged, identical journal/receipt row counts) — no torn
//!   journal/projection/receipt survives, `PRAGMA integrity_check` is clean;
//! - `verify` passes, `replay` reproduces the projection WITHOUT rebuilding it and
//!   equals the live view (replay/live equality after restart);
//! - the lifecycle continues from the recovered capability: the identical transition
//!   commits EXACTLY ONCE (genesis create, the same command, the same outcome
//!   binding), a duplicate create/command refuses, and a re-recorded outcome (with
//!   both the fresh and the stale pre-crash capability) replays the original receipt
//!   with zero mutation and zero journal growth — no duplicate effect outcome.
//! - the public `ik` CLI observes the same: `verify` passes, `show` matches the
//!   recovered projection (or refuses for genesis), `replay` does not rebuild.
//!
//! The entire file (and the crash-arming seam it drives) is gated behind the
//! NON-DEFAULT `test-support` feature: `cargo test --all-features` compiles and
//! runs this matrix; a normal `cargo build`/`cargo test` exposes no crash-arming
//! API. `tests/crash_surface_contract.rs` pins that gate in both builds.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::diagnostic::DiagnosticCode;
use ik::effect::{EffectOutcomeV1, EffectStatus};
use ik::identity::{AggregateId, Capability, MachineIdentity};
use ik::store::{CommandRequest, CrashHook, EffectOutcomeRequest, Store};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Persistence boundary family exercised by the matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Family {
    Genesis,
    /// The ATOMIC create-with-command operation: genesis + the initial command in
    /// ONE transaction (contract "aggregate genesis + admission is one SQLite
    /// transaction"). Crashes at every boundary — including the boundaries INSIDE
    /// the transaction BETWEEN the genesis and the admission persist — must leave
    /// the aggregate wholly absent, and the exact retry must commit exactly once.
    Admit,
    Command,
    Record,
}

impl Family {
    fn as_str(self) -> &'static str {
        match self {
            Family::Genesis => "genesis",
            Family::Admit => "admit",
            Family::Command => "command",
            Family::Record => "record",
        }
    }
}

/// How the child dies when the armed failpoint fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Die {
    Abort,
    Exit,
}

impl Die {
    fn as_str(self) -> &'static str {
        match self {
            Die::Abort => "abort",
            Die::Exit => "exit",
        }
    }
}

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

/// The transition under test on the domain path. The child and the parent's
/// exactly-once continue use the identical command.
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

/// The pending outcome bound on the record path. The child and the parent's
/// exactly-once continue use the identical outcome.
fn probe_outcome(agg: &AggregateId) -> EffectOutcomeV1 {
    EffectOutcomeV1 {
        effect_id: ik::identity::EffectId("e1".into()),
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

fn integrity_check(path: &Path) -> String {
    let conn = rusqlite::Connection::open(path).expect("open store");
    conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("integrity check")
}

/// Every observable projection field must be untouched by a crashed transition.
fn assert_projection_unchanged(before: &ik::store::Projection, after: &ik::store::Projection) {
    assert_eq!(after.aggregate_id, before.aggregate_id);
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

/// The child must die exactly the way the armed failpoint dictates: SIGABRT for
/// `Die::Abort`, exit code 86 for `Die::Exit`.
#[cfg(unix)]
fn assert_child_death(status: std::process::ExitStatus, die: Die, failpoint: &str) {
    use std::os::unix::process::ExitStatusExt;
    match die {
        Die::Abort => assert_eq!(
            status.signal(),
            Some(6),
            "child must die with SIGABRT at {failpoint}, got {status:?}"
        ),
        Die::Exit => {
            assert_eq!(
                status.signal(),
                None,
                "exit-mode child must not die by signal at {failpoint}: {status:?}"
            );
            assert_eq!(
                status.code(),
                Some(86),
                "exit-mode child must exit 86 at {failpoint}: {status:?}"
            );
        }
    }
}

#[cfg(not(unix))]
fn assert_child_death(status: std::process::ExitStatus, die: Die, failpoint: &str) {
    let _ = die;
    assert!(
        !status.success(),
        "child must die abnormally at {failpoint}: {status:?}"
    );
}

/// Build a minimal strict request envelope for the `ik` CLI.
fn cli_request(op: &str, request_id: &str, extra: &Value) -> Value {
    let mut request = json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": "0.5.0",
        "request_id": request_id,
        "op": op,
    });
    if let Some(fields) = extra.as_object() {
        for (key, value) in fields {
            request
                .as_object_mut()
                .expect("request is an object")
                .insert(key.clone(), value.clone());
        }
    }
    request
}

/// Run one crash-matrix row: seed the pre-crash committed state, spawn a real child
/// that dies at the named failpoint, then reopen and assert the full acceptance
/// (wholly absent or exactly once committed, verify, replay/live equality, continue,
/// no duplicate effect outcome, CLI parity). `failpoint` is the FULL literal the
/// store emits (e.g. `crash.genesis.after_journal` for the genesis persist inside
/// the atomic create-with-command, `crash.admit.after_journal` for the admission
/// persist), so one family can arm boundaries from several persists.
fn run_crash_row(family: Family, failpoint: &'static str, die: Die) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let agg = AggregateId("agg-1".into());

    // Seed the pre-crash committed state through the supported API.
    let (identity, machine, baseline_journal, baseline_receipts, before) = match family {
        Family::Genesis | Family::Admit => {
            let machine = va();
            let identity = machine.identity().clone();
            Store::create(&path).expect("create store shell");
            (identity, machine, 0i64, 0i64, None)
        }
        Family::Command => {
            let (mut store, agg, machine) = setup_va(&dir);
            let identity = machine.identity().clone();
            let cap = capability(&identity, &agg, 0);
            start_va(&mut store, machine.as_ref(), &agg, &cap); // sequence 1, generation 1
            let before = store.show(&agg).expect("show before crash");
            (identity, machine, 2i64, 2i64, Some(before))
        }
        Family::Record => {
            let (mut store, agg, machine) = setup_probe(&dir);
            let identity = machine.identity().clone();
            let cap0 = capability(&identity, &agg, 0);
            let planned = plan_probe(&mut store, machine.as_ref(), &agg, &cap0); // sequence 1
            assert!(
                matches!(planned.effect, EffectStatus::Pending(_)),
                "the plan must leave an effect pending"
            );
            let before = store.show(&agg).expect("show before crash");
            (identity, machine, 2i64, 2i64, Some(before))
        }
    };

    // Spawn the real child, which re-enters this binary as `crash_matrix_child_entry`,
    // runs the real store operation with the injected hook, and dies at the failpoint.
    // `status()` blocks on process exit — no sleeps, no polling.
    let prepared = dir.path().join("prepared.marker");
    let done = dir.path().join("done.marker");
    let status = std::process::Command::new(std::env::current_exe().expect("test binary path"))
        .arg("crash_matrix_child_entry")
        .arg("--exact")
        .env("IK_CRASH_CHILD", "1")
        .env("IK_CRASH_STORE", &path)
        .env("IK_CRASH_AGG", &agg.0)
        .env("IK_CRASH_FAMILY", family.as_str())
        .env("IK_CRASH_POINT", failpoint)
        .env("IK_CRASH_DIE", die.as_str())
        .env("IK_CRASH_PREPARED", &prepared)
        .env("IK_CRASH_DONE", &done)
        .status()
        .expect("run crash child");
    assert_child_death(status, die, failpoint);
    let armed = std::fs::read_to_string(&prepared)
        .unwrap_or_else(|e| panic!("crash child never reached the operation: {e}"));
    assert_eq!(
        armed.trim(),
        failpoint,
        "the child must arm exactly the failpoint under test"
    );
    assert!(
        !done.exists(),
        "the child must die before completing the operation at {failpoint}"
    );

    // Reopen: SQLite crash recovery discards the torn transaction. Every boundary is
    // either wholly absent (genesis/admit) or exactly the last committed state.
    let mut store = Store::open(&path).expect("reopen after crash");
    match family {
        Family::Genesis | Family::Admit => {
            let err = store
                .show(&agg)
                .expect_err("no aggregate may be observable after the crashed genesis/admit");
            assert_eq!(err.code, DiagnosticCode::IkIllegalState);
            assert_eq!(
                journal_rows(&path, &agg.0),
                0,
                "no torn genesis journal row"
            );
            assert_eq!(receipt_rows(&path, &agg.0), 0, "no torn genesis receipt");
        }
        Family::Command | Family::Record => {
            let after = store.show(&agg).expect("show after crash");
            assert_projection_unchanged(before.as_ref().expect("pre-crash projection"), &after);
            assert_eq!(
                journal_rows(&path, &agg.0),
                baseline_journal,
                "no torn journal append may survive the crash"
            );
            assert_eq!(
                receipt_rows(&path, &agg.0),
                baseline_receipts,
                "no torn receipt may survive the crash"
            );
        }
    }
    assert_eq!(
        integrity_check(&path),
        "ok",
        "SQLite integrity must hold after crash recovery"
    );

    // Verify passes and replay reproduces the projection without rebuilding it;
    // replay/live equality holds after restart.
    let summary = store
        .verify(machine.as_ref(), None)
        .expect("verify passes after crash recovery");
    assert!(summary.verified);
    match family {
        Family::Genesis | Family::Admit => {
            let replay = store
                .replay(machine.as_ref(), None)
                .expect("replay on the recovered store");
            assert!(
                replay.aggregates.is_empty(),
                "the recovered store has no aggregates to replay"
            );
        }
        Family::Command | Family::Record => {
            let replay = store
                .replay(machine.as_ref(), Some(&agg))
                .expect("replay after crash");
            assert_eq!(replay.aggregates.len(), 1);
            let entry = &replay.aggregates[0];
            assert!(
                !entry.rebuilt,
                "replay must reproduce the projection without rebuilding it"
            );
            let live = store.show(&agg).expect("live view after restart");
            assert_eq!(
                entry.sequence, live.sequence,
                "replay/live sequence equality"
            );
            assert_eq!(
                entry.generation, live.generation,
                "replay/live generation equality"
            );
            assert_eq!(entry.state, live.state, "replay/live state equality");
        }
    }

    // Public CLI parity on the recovered store.
    let machine_path = machine_def(if matches!(family, Family::Record) {
        "effect_probe"
    } else {
        "verification_attempt"
    });
    let (stdout, code) = run_cli(
        &path,
        Some(&machine_path),
        &cli_request("verify", "verify-after-crash", &json!({})),
    );
    assert_eq!(
        code, 0,
        "ik verify must pass on the recovered store at {failpoint}: {stdout}"
    );
    let response: Value = serde_json::from_str(&stdout).expect("verify envelope");
    assert_eq!(response["result"]["verified"], json!(true));
    let (stdout, code) = run_cli(
        &path,
        Some(&machine_path),
        &cli_request("show", "show-after-crash", &json!({"aggregate_id": agg.0})),
    );
    let response: Value = serde_json::from_str(&stdout).expect("show envelope");
    match family {
        Family::Genesis | Family::Admit => {
            assert_eq!(
                code, 1,
                "ik show must refuse the wholly absent aggregate at {failpoint}: {stdout}"
            );
            assert_eq!(response["ok"], json!(false));
        }
        Family::Command | Family::Record => {
            assert_eq!(
                code, 0,
                "ik show must observe the recovered projection at {failpoint}: {stdout}"
            );
            let before = before.as_ref().expect("pre-crash projection");
            assert_eq!(response["result"]["generation"], json!(before.generation));
            assert_eq!(response["result"]["sequence"], json!(before.sequence));
            assert_eq!(response["result"]["state"], before.state);
        }
    }
    if !matches!(family, Family::Genesis | Family::Admit) {
        let (stdout, code) = run_cli(
            &path,
            Some(&machine_path),
            &cli_request(
                "replay",
                "replay-after-crash",
                &json!({"aggregate_id": agg.0}),
            ),
        );
        assert_eq!(
            code, 0,
            "ik replay must pass on the recovered store at {failpoint}: {stdout}"
        );
        let response: Value = serde_json::from_str(&stdout).expect("replay envelope");
        let entry = &response["result"]["aggregates"][0];
        assert_eq!(entry["rebuilt"], json!(false));
        let before = before.as_ref().expect("pre-crash projection");
        assert_eq!(entry["sequence"], json!(before.sequence));
        assert_eq!(entry["generation"], json!(before.generation));
        assert_eq!(entry["state"], before.state);
    }

    // The lifecycle continues from the recovered capability: the identical
    // transition commits EXACTLY ONCE after restart.
    match family {
        Family::Genesis => {
            let created = store
                .create_aggregate(machine.as_ref(), &agg, &identity)
                .expect("create commits exactly once after crash recovery");
            assert!(created.mutation);
            assert_eq!(created.generation, 0);
            let dup = store
                .create_aggregate(machine.as_ref(), &agg, &identity)
                .expect_err("a duplicate create must refuse");
            assert_eq!(dup.code, DiagnosticCode::IkIllegalState);
            assert_eq!(journal_rows(&path, &agg.0), 1, "exactly one genesis event");
            assert_eq!(
                receipt_rows(&path, &agg.0),
                1,
                "exactly one genesis receipt"
            );
        }
        Family::Admit => {
            // The identical atomic create+admit commits EXACTLY ONCE after the
            // crash — genesis + the initial command in one transaction.
            let start = json!({"name": "start"});
            let created = store
                .create_aggregate_with_command(machine.as_ref(), &agg, &identity, &start, None)
                .expect("the identical create+admit commits exactly once after crash recovery");
            let ik::store::CreateWithCommandOutcome::Committed(created) = created else {
                panic!("a fresh post-crash create+admit must commit");
            };
            assert!(created.mutation);
            assert_eq!(created.generation, 1, "genesis + admission advance to 1");
            assert_eq!(
                journal_rows(&path, &agg.0),
                2,
                "exactly one genesis + one admission event"
            );
            assert_eq!(
                receipt_rows(&path, &agg.0),
                2,
                "exactly one genesis + one admission receipt"
            );
            // The exact retry is IDEMPOTENT: typed duplicate, zero mutation, zero
            // journal growth (PBT-005 create analog).
            let dup = store
                .create_aggregate_with_command(machine.as_ref(), &agg, &identity, &start, None)
                .expect("an exact retry is a typed duplicate after crash recovery");
            let ik::store::CreateWithCommandOutcome::ExactDuplicate(dup) = dup else {
                panic!("an exact retry must return the typed duplicate result");
            };
            assert!(!dup.mutation, "an exact retry must not mutate");
            assert_eq!(
                dup.receipt.as_ref(),
                created.receipt.as_ref(),
                "the duplicate returns the original admission receipt"
            );
            assert_eq!(dup.state, created.state);
            assert_eq!(journal_rows(&path, &agg.0), 2, "no journal growth");
            assert_eq!(receipt_rows(&path, &agg.0), 2, "no receipt growth");
            // A DIVERGENT admission refuses (PBT-006 create analog): a different
            // initial command would have written different events.
            let conflict = store
                .create_aggregate_with_command(
                    machine.as_ref(),
                    &agg,
                    &identity,
                    &json!({"name": "cancel"}),
                    None,
                )
                .expect_err("a divergent admission must refuse after crash recovery");
            assert_eq!(conflict.code, DiagnosticCode::IkIllegalState);
            assert_eq!(
                conflict.rule.as_deref(),
                Some("create.command_conflict"),
                "the conflicting retry must carry the stable rule"
            );
            assert_eq!(journal_rows(&path, &agg.0), 2, "no journal growth");
            assert_eq!(receipt_rows(&path, &agg.0), 2, "no receipt growth");
        }
        Family::Command => {
            let committed = store
                .command(
                    machine.as_ref(),
                    &CommandRequest {
                        aggregate_id: agg.clone(),
                        machine: identity.clone(),
                        capability: before
                            .as_ref()
                            .expect("pre-crash projection")
                            .capability
                            .clone(),
                        command: submit(),
                        observation: None,
                    },
                )
                .expect("the identical command commits exactly once after restart");
            assert!(committed.mutation);
            assert_eq!(
                committed.generation,
                before.as_ref().expect("pre-crash projection").generation + 1
            );
            assert_eq!(
                journal_rows(&path, &agg.0),
                baseline_journal + 1,
                "exactly one new journal event"
            );
            assert_eq!(
                receipt_rows(&path, &agg.0),
                baseline_receipts + 1,
                "exactly one new receipt"
            );
            // The identical command cannot commit a second time: the domain state
            // advanced with the recovered commit, so the retry is a domain refusal,
            // not a duplicate mutation.
            let refused = store
                .command(
                    machine.as_ref(),
                    &CommandRequest {
                        aggregate_id: agg.clone(),
                        machine: identity.clone(),
                        capability: committed.capability.clone(),
                        command: submit(),
                        observation: None,
                    },
                )
                .expect_err("the identical command cannot commit twice");
            assert_ne!(
                refused.code,
                DiagnosticCode::IkStoreCorrupt,
                "the second attempt must be a domain refusal, not corruption"
            );
            assert_eq!(journal_rows(&path, &agg.0), baseline_journal + 1);
            assert_eq!(receipt_rows(&path, &agg.0), baseline_receipts + 1);
        }
        Family::Record => {
            let recorded = store
                .record_effect(&EffectOutcomeRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: before
                        .as_ref()
                        .expect("pre-crash projection")
                        .capability
                        .clone(),
                    effect: probe_outcome(&agg),
                })
                .expect("the pending outcome binds exactly once after restart");
            assert!(recorded.mutation);
            assert_eq!(
                recorded.generation,
                before.as_ref().expect("pre-crash projection").generation + 1
            );
            assert!(matches!(recorded.effect, EffectStatus::Recorded(..)));
            assert_eq!(
                journal_rows(&path, &agg.0),
                baseline_journal + 1,
                "exactly one recording event"
            );
            assert_eq!(
                receipt_rows(&path, &agg.0),
                baseline_receipts + 1,
                "exactly one recording receipt"
            );
            // No duplicate effect outcome: re-recording the identical outcome — with
            // the fresh capability AND with the stale pre-crash capability (G-002
            // precedence exception) — replays the original receipt with zero mutation
            // and zero journal growth.
            let fresh = recorded.capability.clone();
            let stale = before
                .as_ref()
                .expect("pre-crash projection")
                .capability
                .clone();
            for (label, capability) in [("fresh", fresh), ("stale", stale)] {
                let dup = store
                    .record_effect(&EffectOutcomeRequest {
                        aggregate_id: agg.clone(),
                        machine: identity.clone(),
                        capability,
                        effect: probe_outcome(&agg),
                    })
                    .expect("a duplicate outcome replays instead of refusing");
                assert!(!dup.mutation, "{label} duplicate must not mutate");
                assert_eq!(
                    dup.receipt.as_ref(),
                    recorded.receipt.as_ref(),
                    "{label} duplicate returns the original receipt"
                );
                assert_eq!(dup.generation, recorded.generation);
                assert_eq!(
                    journal_rows(&path, &agg.0),
                    baseline_journal + 1,
                    "{label} duplicate must not append a journal event"
                );
                assert_eq!(
                    receipt_rows(&path, &agg.0),
                    baseline_receipts + 1,
                    "{label} duplicate must not append a receipt"
                );
            }
        }
    }
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after continuing");
}

#[test]
fn crash_matrix_child_entry() {
    // Re-entered as a real child process by `run_crash_row`. Opens the store with an
    // explicitly injected crash hook armed for exactly one deterministic failpoint
    // name, writes the `prepared` marker, and runs the REAL store operation
    // (`create_aggregate` / `create_aggregate_with_command` / `command` /
    // `record_effect`). The hook terminates the process (SIGABRT or exit 86) the
    // moment the armed persistence boundary fires; the `done` marker is written only
    // if the operation completes, which the parent treats as a failed arming.
    if std::env::var("IK_CRASH_CHILD").is_err() {
        return; // normal parent-side invocation of the test binary: nothing to do
    }
    let store_path = PathBuf::from(std::env::var("IK_CRASH_STORE").expect("IK_CRASH_STORE"));
    let agg = AggregateId(std::env::var("IK_CRASH_AGG").expect("IK_CRASH_AGG"));
    let family = match std::env::var("IK_CRASH_FAMILY")
        .expect("IK_CRASH_FAMILY")
        .as_str()
    {
        "genesis" => Family::Genesis,
        "admit" => Family::Admit,
        "command" => Family::Command,
        "record" => Family::Record,
        other => panic!("unknown IK_CRASH_FAMILY `{other}`"),
    };
    let failpoint = std::env::var("IK_CRASH_POINT").expect("IK_CRASH_POINT");
    let die = match std::env::var("IK_CRASH_DIE")
        .expect("IK_CRASH_DIE")
        .as_str()
    {
        "abort" => Die::Abort,
        "exit" => Die::Exit,
        other => panic!("unknown IK_CRASH_DIE `{other}`"),
    };
    let prepared = PathBuf::from(std::env::var("IK_CRASH_PREPARED").expect("IK_CRASH_PREPARED"));
    let done = PathBuf::from(std::env::var("IK_CRASH_DONE").expect("IK_CRASH_DONE"));
    // The full literal the store emits, e.g. `crash.genesis.after_journal` (the
    // genesis persist inside the atomic create-with-command) or
    // `crash.admit.after_projection` (the admission persist).
    let armed = failpoint.clone();

    let (machine, identity) = match family {
        Family::Genesis | Family::Admit | Family::Command => {
            let machine = va();
            let identity = machine.identity().clone();
            (machine, identity)
        }
        Family::Record => {
            let machine = probe();
            let identity = machine.identity().clone();
            (machine, identity)
        }
    };

    let hook: CrashHook = Box::new(move |name: &str| {
        if name == armed.as_str() {
            match die {
                Die::Abort => std::process::abort(),
                Die::Exit => std::process::exit(86),
            }
        }
        // Any other boundary leaves the hook armed for the selected point.
        false
    });
    let mut store =
        Store::open_with_crash_hook(&store_path, hook).expect("child opens store with hook");

    std::fs::write(&prepared, &failpoint).expect("write prepared marker");

    match family {
        Family::Genesis => {
            let _ = store.create_aggregate(machine.as_ref(), &agg, &identity);
        }
        Family::Admit => {
            // The REAL atomic create+admit operation: genesis + the initial
            // command in ONE transaction, dying at the armed boundary.
            let _ = store.create_aggregate_with_command(
                machine.as_ref(),
                &agg,
                &identity,
                &json!({"name": "start"}),
                None,
            );
        }
        Family::Command => {
            let projection = store.show(&agg).expect("child shows aggregate");
            let _ = store.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: projection.capability.clone(),
                    command: submit(),
                    observation: None,
                },
            );
        }
        Family::Record => {
            let projection = store.show(&agg).expect("child shows aggregate");
            let _ = store.record_effect(&EffectOutcomeRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: projection.capability.clone(),
                effect: probe_outcome(&agg),
            });
        }
    }

    // The armed failpoint must have terminated the process before this line.
    std::fs::write(&done, "done").expect("write done marker");
    panic!("crash child completed `{failpoint}` without dying");
}

macro_rules! crash_matrix {
    ($($name:ident: $family:ident/$failpoint:literal/$die:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                run_crash_row(Family::$family, $failpoint, Die::$die);
            }
        )*
    };
}

// Every deterministic failpoint literal the store can emit, in both death modes.
crash_matrix! {
    genesis_after_journal_abort: Genesis/"crash.genesis.after_journal"/Abort,
    genesis_after_journal_exit: Genesis/"crash.genesis.after_journal"/Exit,
    genesis_after_receipts_abort: Genesis/"crash.genesis.after_receipts"/Abort,
    genesis_after_receipts_exit: Genesis/"crash.genesis.after_receipts"/Exit,
    genesis_after_projection_abort: Genesis/"crash.genesis.after_projection"/Abort,
    genesis_after_projection_exit: Genesis/"crash.genesis.after_projection"/Exit,
    genesis_after_aggregates_insert_abort: Genesis/"crash.genesis.after_aggregates_insert"/Abort,
    genesis_after_aggregates_insert_exit: Genesis/"crash.genesis.after_aggregates_insert"/Exit,
    // Atomic create-with-command: the same four genesis boundaries PLUS the
    // boundaries INSIDE the transaction between the genesis and the admission
    // persist. A crash at ANY of them must leave the aggregate wholly absent.
    admit_genesis_after_journal_abort: Admit/"crash.genesis.after_journal"/Abort,
    admit_genesis_after_journal_exit: Admit/"crash.genesis.after_journal"/Exit,
    admit_genesis_after_receipts_abort: Admit/"crash.genesis.after_receipts"/Abort,
    admit_genesis_after_receipts_exit: Admit/"crash.genesis.after_receipts"/Exit,
    admit_genesis_after_projection_abort: Admit/"crash.genesis.after_projection"/Abort,
    admit_genesis_after_projection_exit: Admit/"crash.genesis.after_projection"/Exit,
    admit_genesis_after_aggregates_insert_abort: Admit/"crash.genesis.after_aggregates_insert"/Abort,
    admit_genesis_after_aggregates_insert_exit: Admit/"crash.genesis.after_aggregates_insert"/Exit,
    admit_after_journal_abort: Admit/"crash.admit.after_journal"/Abort,
    admit_after_journal_exit: Admit/"crash.admit.after_journal"/Exit,
    admit_after_receipts_abort: Admit/"crash.admit.after_receipts"/Abort,
    admit_after_receipts_exit: Admit/"crash.admit.after_receipts"/Exit,
    admit_after_projection_abort: Admit/"crash.admit.after_projection"/Abort,
    admit_after_projection_exit: Admit/"crash.admit.after_projection"/Exit,
    command_after_journal_abort: Command/"crash.command.after_journal"/Abort,
    command_after_journal_exit: Command/"crash.command.after_journal"/Exit,
    command_after_receipts_abort: Command/"crash.command.after_receipts"/Abort,
    command_after_receipts_exit: Command/"crash.command.after_receipts"/Exit,
    command_after_projection_abort: Command/"crash.command.after_projection"/Abort,
    command_after_projection_exit: Command/"crash.command.after_projection"/Exit,
    record_after_journal_abort: Record/"crash.record.after_journal"/Abort,
    record_after_journal_exit: Record/"crash.record.after_journal"/Exit,
    record_after_receipts_abort: Record/"crash.record.after_receipts"/Abort,
    record_after_receipts_exit: Record/"crash.record.after_receipts"/Exit,
    record_after_projection_abort: Record/"crash.record.after_projection"/Abort,
    record_after_projection_exit: Record/"crash.record.after_projection"/Exit,
}

#[test]
fn crash_hook_is_inert_without_explicit_arming() {
    // Negative control for the test-only seam: a store opened through the normal
    // constructor never carries a crash hook, so a full command lifecycle commits
    // normally and no failpoint can ever fire. Guards against the seam ever
    // becoming ambient (env/config-triggered or default-armed).
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut store, agg, machine) = setup_va(&dir);
    let identity = machine.identity().clone();
    let cap = capability(&identity, &agg, 0);
    start_va(&mut store, machine.as_ref(), &agg, &cap);
    let current = store.show(&agg).expect("show").capability.clone();
    let committed = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: current,
                command: submit(),
                observation: None,
            },
        )
        .expect("the command commits normally with no hook armed");
    assert!(committed.mutation);
    assert_eq!(committed.generation, 2);
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes on the hook-free store");
}

#[test]
fn injected_hook_fires_exactly_once_in_process() {
    // In-process seam semantics (no child needed): an injected hook fires exactly
    // once — at the FIRST named boundary, where it is consumed (single-operation) —
    // and a non-aborting hook does not disturb the surrounding transaction, which
    // still commits. This pins the hook contract; the process-death matrix above
    // pins the crash behavior itself.
    use std::sync::{Arc, Mutex};

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let (mut store, agg, machine) = setup_va(&dir);
    let identity = machine.identity().clone();
    let cap = capability(&identity, &agg, 0);
    start_va(&mut store, machine.as_ref(), &agg, &cap);
    drop(store);

    let fired: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let fired_in_hook = fired.clone();
    let hook: CrashHook = Box::new(move |name: &str| {
        fired_in_hook.lock().expect("lock").push(name.to_string());
        true
    });
    let mut store = Store::open_with_crash_hook(&path, hook).expect("open with hook");
    let projection = store.show(&agg).expect("show");
    let out = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: projection.capability.clone(),
                command: submit(),
                observation: None,
            },
        )
        .expect("the command commits with a non-aborting hook");
    assert!(out.mutation);
    assert_eq!(
        fired.lock().expect("lock").as_slice(),
        &["crash.command.after_journal".to_string()],
        "the hook must fire exactly once, at the first named boundary, and then be consumed"
    );
    drop(store);

    // The commit is fully durable despite the hook having run mid-transaction.
    let mut store = Store::open(&path).expect("reopen");
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after the hook-fired commit");
}
