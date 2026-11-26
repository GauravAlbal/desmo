//! Atomic create-with-initial-command: aggregate genesis + the initial command
//! event are committed by ONE public kernel operation in ONE SQLite transaction
//! (IK-LAW-004; contract "aggregate genesis + admission is one SQLite
//! transaction, one atomic public operation"). The companion's former
//! two-transaction create+command sequence is replaced by this operation.
//!
//! Normative laws exercised (the OUGHT, from the design constitution/pivot spec,
//! never the current implementation):
//! - IK-LAW-004: capability validation, reduction, journal append, projection
//!   update, generation/nonce rotation, effect identity, and receipt commit
//!   atomically — here for the GENESIS + INITIAL COMMAND pair; a refusal before
//!   any write leaves no partial aggregate and no partial projection
//!   (REQ-IK-006);
//! - PBT-005 analog (exact duplicate create+admit is idempotent): a retry that
//!   would reproduce exactly the committed genesis + initial event returns the
//!   typed exact-duplicate result with zero mutation, the original stored
//!   receipt, and zero journal growth;
//! - PBT-006 analog (conflicting duplicate refuses): a retry that would have
//!   written different events refuses with the stable `create.command_conflict`
//!   diagnostic and never accepts a divergent admission.
//!
//! Process-crash boundaries live in `crash_matrix.rs` (Family::Admit); SQL-level
//! injected failures live in `atomicity.rs`.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::diagnostic::DiagnosticCode;
use ik::store::{CreateWithCommandOutcome, Store};
use serde_json::{json, Value};
use std::path::Path;

fn va() -> Box<dyn ik::machine::Machine> {
    Box::new(load_machine("verification_attempt"))
}

/// The initial command under test: `start` emits one event (created -> running).
fn admit_command() -> Value {
    json!({"name": "start"})
}

/// A valid-but-divergent initial command: `cancel` would emit a different event
/// and land in a different state (created -> cancelled).
fn divergent_command() -> Value {
    json!({"name": "cancel"})
}

/// An invalid initial command: `submit` requires `artifact_digest`/`baseline_digest`
/// args, so the machine refuses the bare command with `IK_COMMAND_INVALID`.
fn refused_command() -> Value {
    json!({"name": "submit"})
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

/// IK-LAW-004 + contract: one public create/admit operation owns one transaction.
/// The committed aggregate carries BOTH the genesis event (sequence 0) and the
/// initial command event (sequence 1, generation 1) with matching receipts, and
/// verify/replay agree — the pair committed atomically, not as two transactions.
#[test]
fn atomic_create_admit_commits_in_one_operation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let machine = va();
    let identity = machine.identity().clone();
    let mut store = Store::create(&path).expect("create store");
    let agg = ik::identity::AggregateId("agg-1".into());

    let outcome = store
        .create_aggregate_with_command(machine.as_ref(), &agg, &identity, &admit_command(), None)
        .expect("atomic create+admit commits");
    let CreateWithCommandOutcome::Committed(admitted) = outcome else {
        panic!("a fresh create+admit must commit, got a duplicate");
    };
    assert!(admitted.mutation);
    assert_eq!(admitted.generation, 1, "genesis + admission advance to 1");
    assert_eq!(
        admitted.state.get("state").and_then(Value::as_str),
        Some("running"),
        "the initial command must have applied"
    );
    assert!(!admitted.terminal);
    assert!(
        admitted.receipt.is_some(),
        "the admission event must carry a receipt"
    );

    // Both events committed: genesis + admission, each with a receipt.
    assert_eq!(journal_rows(&path, &agg.0), 2, "genesis + admission events");
    assert_eq!(receipt_rows(&path, &agg.0), 2, "one receipt per event");

    // The store verifies: journal + projection agree, replay reproduces without
    // rebuilding — the pair is a single consistent committed state.
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after atomic create+admit");
    let replay = store
        .replay(machine.as_ref(), Some(&agg))
        .expect("replay after atomic create+admit");
    assert_eq!(replay.aggregates.len(), 1);
    assert_eq!(replay.aggregates[0].sequence, 1);
    assert_eq!(replay.aggregates[0].generation, 1);
    assert!(
        !replay.aggregates[0].rebuilt,
        "the projection must already equal the journal fold"
    );

    // The lifecycle continues from the returned capability (generation 1).
    let next = store
        .command(
            machine.as_ref(),
            &ik::store::CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: admitted.capability.clone(),
                command: json!({"name": "submit", "args": {"artifact_digest": "a1", "baseline_digest": "b1"}}),
                observation: None,
            },
        )
        .expect("the lifecycle continues from the admitted capability");
    assert_eq!(next.generation, 2);
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after continuing");
}

/// PBT-005 create analog: an exact retry is idempotent. Repeating the identical
/// create+admit returns the TYPED exact-duplicate result with zero mutation, the
/// original stored receipt, and zero journal growth — it never re-admits or
/// diverges.
#[test]
fn exact_retry_returns_typed_duplicate_without_mutation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let machine = va();
    let identity = machine.identity().clone();
    let mut store = Store::create(&path).expect("create store");
    let agg = ik::identity::AggregateId("agg-1".into());

    let CreateWithCommandOutcome::Committed(first) = store
        .create_aggregate_with_command(machine.as_ref(), &agg, &identity, &admit_command(), None)
        .expect("first create+admit commits")
    else {
        panic!("first attempt must commit");
    };

    let outcome = store
        .create_aggregate_with_command(machine.as_ref(), &agg, &identity, &admit_command(), None)
        .expect("an exact retry is a typed duplicate, not an error");
    let CreateWithCommandOutcome::ExactDuplicate(retry) = outcome else {
        panic!("an exact retry must return the typed duplicate result");
    };
    assert!(!retry.mutation, "an exact retry must not mutate the store");
    assert_eq!(
        retry.receipt.as_ref(),
        first.receipt.as_ref(),
        "the duplicate returns the ORIGINAL admission receipt"
    );
    assert_eq!(retry.state, first.state);
    assert_eq!(retry.generation, first.generation);
    assert_eq!(retry.capability, first.capability);
    assert_eq!(retry.effect, first.effect);
    assert_eq!(
        journal_rows(&path, &agg.0),
        2,
        "an exact retry must not append events"
    );
    assert_eq!(
        receipt_rows(&path, &agg.0),
        2,
        "an exact retry must not append receipts"
    );
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after the exact retry");
}

/// PBT-006 create analog: a conflicting retry refuses. The same aggregate with a
/// DIFFERENT initial command would have written different events; the retry must
/// refuse with the stable conflict diagnostic and never accept the divergent
/// admission, leaving the committed journal untouched.
#[test]
fn conflicting_retry_refuses_divergent_admission() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let machine = va();
    let identity = machine.identity().clone();
    let mut store = Store::create(&path).expect("create store");
    let agg = ik::identity::AggregateId("agg-1".into());

    store
        .create_aggregate_with_command(machine.as_ref(), &agg, &identity, &admit_command(), None)
        .expect("create+admit commits");

    // `cancel` would admit a different event into a different state.
    let err = store
        .create_aggregate_with_command(
            machine.as_ref(),
            &agg,
            &identity,
            &divergent_command(),
            None,
        )
        .expect_err("a divergent admission must refuse");
    assert_eq!(err.code, DiagnosticCode::IkIllegalState);
    assert_eq!(
        err.rule.as_deref(),
        Some("create.command_conflict"),
        "the conflicting retry must carry the stable rule"
    );
    assert_eq!(
        journal_rows(&path, &agg.0),
        2,
        "the refusal must not append anything"
    );
    assert_eq!(receipt_rows(&path, &agg.0), 2);

    // The committed admission is untouched: state is still `running`.
    let projection = store.show(&agg).expect("show after conflict");
    assert_eq!(
        projection.state.get("state").and_then(Value::as_str),
        Some("running")
    );
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after the refused conflict");
}

/// A create-with-command retry against an aggregate created by the PLAIN genesis
/// op is a conflict: the stored journal (genesis only) does not match what this
/// request would have written (genesis + initial event). Divergent admission is
/// refused and the genesis-only aggregate survives untouched.
#[test]
fn retry_after_plain_genesis_refuses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let machine = va();
    let identity = machine.identity().clone();
    let mut store = Store::create(&path).expect("create store");
    let agg = ik::identity::AggregateId("agg-1".into());

    store
        .create_aggregate(machine.as_ref(), &agg, &identity)
        .expect("plain genesis commits");

    let err = store
        .create_aggregate_with_command(machine.as_ref(), &agg, &identity, &admit_command(), None)
        .expect_err("a create-with-command retry against a genesis-only aggregate must refuse");
    assert_eq!(err.code, DiagnosticCode::IkIllegalState);
    assert_eq!(err.rule.as_deref(), Some("create.command_conflict"));
    assert_eq!(
        journal_rows(&path, &agg.0),
        1,
        "the genesis-only journal must survive untouched"
    );
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after the refused retry");
}

/// REQ-IK-006: a refusal BEFORE any write leaves no partial state. When the
/// initial command is a domain refusal, neither genesis nor projection may be
/// observable, and the identical request with a VALID command still commits
/// afterwards — the refused attempt consumed nothing.
#[test]
fn refused_initial_command_leaves_no_partial_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let machine = va();
    let identity = machine.identity().clone();
    let mut store = Store::create(&path).expect("create store");
    let agg = ik::identity::AggregateId("agg-1".into());

    let err = store
        .create_aggregate_with_command(machine.as_ref(), &agg, &identity, &refused_command(), None)
        .expect_err("the invalid initial command must refuse");
    assert_eq!(err.code, DiagnosticCode::IkCommandInvalid);

    let ghost = store
        .show(&agg)
        .expect_err("no aggregate may be observable after the refused create+admit");
    assert_eq!(ghost.code, DiagnosticCode::IkIllegalState);
    assert_eq!(journal_rows(&path, &agg.0), 0, "no genesis event");
    assert_eq!(receipt_rows(&path, &agg.0), 0, "no genesis receipt");

    // Positive control: the same aggregate id is still free.
    store
        .create_aggregate_with_command(machine.as_ref(), &agg, &identity, &admit_command(), None)
        .expect("a valid create+admit commits after the refused attempt");
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after the positive control");
}

/// Native consumers match `CreateWithCommandOutcome` exhaustively. Adding a variant is a source break
/// for them (a downstream native crate stopped compiling against the `0.7.0` candidate when `Historical` was
/// added); caller-operation history is reported through `OperationOutcome` instead. This match must
/// stay exhaustive with exactly these two arms.
#[test]
fn legacy_create_outcome_stays_exhaustively_matchable_by_native_consumers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let machine = va();
    let identity = machine.identity().clone();
    let mut store = Store::create(&dir.path().join("s.db")).expect("create store");
    let agg = ik::identity::AggregateId("agg-compat".into());
    let outcome = store
        .create_aggregate_with_command(machine.as_ref(), &agg, &identity, &admit_command(), None)
        .expect("admit");
    let committed = match outcome {
        CreateWithCommandOutcome::Committed(result) => result.mutation,
        CreateWithCommandOutcome::ExactDuplicate(_) => false,
    };
    assert!(committed);
}
