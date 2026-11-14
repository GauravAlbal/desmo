//! Overflow hardening at the persistence seam.
//!
//! The runtime monotone counters (generation, sequence) advance with checked
//! arithmetic and refuse with `IK_COUNTER_OVERFLOW` at the `u64` ceiling. The
//! `u64::MAX` matrix is a pure-runtime condition and lives in the `src/runtime.rs`
//! unit tests plus `tests/properties.rs` (SQLite INTEGER is signed i64, so the
//! store can never represent `u64::MAX`; negative coordinates are refused as
//! corruption). This file covers the store seam at the SQLite-representable
//! boundary (`i64::MAX`): the last legal transition commits with exact pre/post
//! persistence evidence, capability currentness stays intact after a near-ceiling
//! commit, and idempotent replay at the ceiling is zero-mutation.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::diagnostic::DiagnosticCode;
use ik::effect::{EffectOutcomeV1, EffectStatus};
use ik::identity::{AggregateId, Capability, MachineIdentity};
use ik::machine::Machine;
use ik::store::{CommandRequest, EffectOutcomeRequest, Store};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;

/// SQLite's signed ceiling: the largest coordinate the store can represent.
const STORE_MAX: i64 = i64::MAX;

fn capability(identity: &MachineIdentity, aggregate: &AggregateId, generation: u64) -> Capability {
    Capability {
        generation,
        nonce: ik::runtime::derive_nonce(identity, aggregate, generation),
    }
}

fn journal_rows(path: &Path, aggregate: &str) -> i64 {
    let conn = Connection::open(path).expect("open store");
    conn.query_row(
        "SELECT COUNT(*) FROM journal WHERE aggregate_id = ?1",
        [aggregate],
        |row| row.get(0),
    )
    .expect("count journal rows")
}

fn receipt_rows(path: &Path, aggregate: &str) -> i64 {
    let conn = Connection::open(path).expect("open store");
    conn.query_row(
        "SELECT COUNT(*) FROM receipts WHERE aggregate_id = ?1",
        [aggregate],
        |row| row.get(0),
    )
    .expect("count receipt rows")
}

/// (generation, sequence) of the projection row.
fn counters(path: &Path, aggregate: &str) -> (i64, i64) {
    let conn = Connection::open(path).expect("open store");
    conn.query_row(
        "SELECT generation, sequence FROM aggregates WHERE aggregate_id = ?1",
        [aggregate],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("read projection counters")
}

fn nonce(path: &Path, aggregate: &str) -> String {
    let conn = Connection::open(path).expect("open store");
    conn.query_row(
        "SELECT nonce FROM aggregates WHERE aggregate_id = ?1",
        [aggregate],
        |row| row.get(0),
    )
    .expect("read projection nonce")
}

/// event_json of the journal event with the highest sequence.
fn last_event(path: &Path, aggregate: &str) -> Value {
    let conn = Connection::open(path).expect("open store");
    let json: String = conn
        .query_row(
            "SELECT event_json FROM journal WHERE aggregate_id = ?1 ORDER BY sequence DESC LIMIT 1",
            [aggregate],
            |row| row.get(0),
        )
        .expect("read last journal event");
    serde_json::from_str(&json).expect("last event decodes")
}

/// Position the projection at `generation` (and the same sequence), mirroring the
/// runtime invariant generation == sequence. Only positive `i64` coordinates are
/// written; the journal/receipts tables are untouched.
fn position_projection(
    path: &Path,
    aggregate: &AggregateId,
    identity: &MachineIdentity,
    generation: u64,
) {
    let conn = Connection::open(path).expect("open store");
    let nonce = ik::runtime::derive_nonce(identity, aggregate, generation);
    conn.execute(
        "UPDATE aggregates SET generation = ?1, sequence = ?1, nonce = ?2 WHERE aggregate_id = ?3",
        rusqlite::params![generation as i64, nonce, aggregate.0],
    )
    .expect("position projection at boundary");
}

#[test]
fn near_max_store_transition_commits_with_exact_persistence_evidence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let machine = load_machine("verification_attempt");
    let identity = machine.identity().clone();
    let agg = AggregateId("agg-near-max".into());
    let mut store = Store::create(&path).expect("create store");
    store
        .create_aggregate(&machine, &agg, &identity)
        .expect("create aggregate");
    let started = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: capability(&identity, &agg, 0),
                command: json!({"name": "start"}),
                observation: None,
            },
        )
        .expect("start commits");
    assert_eq!(started.generation, 1);
    // Pre-evidence: genesis + start persisted, projection at (1, 1).
    assert_eq!(journal_rows(&path, &agg.0), 2);
    assert_eq!(receipt_rows(&path, &agg.0), 2);
    assert_eq!(counters(&path, &agg.0), (1, 1));

    // Position the projection one below the SQLite ceiling.
    position_projection(&path, &agg, &identity, STORE_MAX as u64 - 1);
    assert_eq!(counters(&path, &agg.0), (STORE_MAX - 1, STORE_MAX - 1));

    // The last legal transition commits exactly onto the ceiling values.
    let near_max = capability(&identity, &agg, STORE_MAX as u64 - 1);
    let submitted = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: near_max.clone(),
                command: json!({"name": "submit", "args": {"artifact_digest": "a", "baseline_digest": "b"}}),
                observation: None,
            },
        )
        .expect("near-max transition commits");
    assert!(submitted.mutation);
    assert_eq!(submitted.generation, STORE_MAX as u64);

    // Exact post-evidence: projection row, journal event, and receipt count all
    // advanced by exactly one onto the ceiling.
    assert_eq!(journal_rows(&path, &agg.0), 3);
    assert_eq!(receipt_rows(&path, &agg.0), 3);
    assert_eq!(counters(&path, &agg.0), (STORE_MAX, STORE_MAX));
    assert_eq!(
        nonce(&path, &agg.0),
        ik::runtime::derive_nonce(&identity, &agg, STORE_MAX as u64)
    );
    let last = last_event(&path, &agg.0);
    assert_eq!(last["sequence"].as_u64(), Some(STORE_MAX as u64));
    assert_eq!(last["generation"].as_u64(), Some(STORE_MAX as u64));
    assert_eq!(last["event_type"], "submit");
    let projection = store.show(&agg).expect("show after near-max commit");
    assert_eq!(projection.generation, STORE_MAX as u64);
    assert_eq!(projection.sequence, STORE_MAX as u64);
    assert_eq!(projection.state["state"], "submitted");

    // Capability currentness is intact at the ceiling: the pre-transition
    // capability refuses as stale (never overflow, never a second commit), with
    // zero writes, and the refusal is deterministic on retry.
    let err = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: near_max.clone(),
                command: json!({"name": "cancel"}),
                observation: None,
            },
        )
        .expect_err("stale capability must refuse after the near-max commit");
    assert_eq!(err.code, DiagnosticCode::IkCapabilityStale);
    assert_eq!(journal_rows(&path, &agg.0), 3);
    assert_eq!(receipt_rows(&path, &agg.0), 3);
    assert_eq!(counters(&path, &agg.0), (STORE_MAX, STORE_MAX));
    let retry = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: near_max,
                command: json!({"name": "cancel"}),
                observation: None,
            },
        )
        .expect_err("retry refuses identically");
    assert_eq!(retry, err);
}

#[test]
fn duplicate_replay_at_store_ceiling_is_zero_mutation_and_replay_recovers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let machine = load_machine("effect_probe");
    let identity = machine.identity().clone();
    let agg = AggregateId("agg-replay".into());
    let mut store = Store::create(&path).expect("create store");
    store
        .create_aggregate(&machine, &agg, &identity)
        .expect("create aggregate");
    let planned = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: capability(&identity, &agg, 0),
                command: json!({"name": "plan", "args": {"effect_id": "e1", "payload_digest": "p", "idempotency_key": "k"}}),
                observation: None,
            },
        )
        .expect("plan commits");
    assert_eq!(planned.generation, 1);
    let EffectStatus::Pending(intent) = &planned.effect else {
        panic!("plan must persist a pending effect");
    };
    let binding = EffectOutcomeV1 {
        effect_id: intent.effect_id.clone(),
        aggregate_id: agg.clone(),
        planned_generation: intent.generation,
        payload_digest: intent.payload_digest.clone(),
        idempotency_key: intent.idempotency_key.clone(),
        outcome_digest: "o".into(),
    };
    let recorded = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: planned.capability.clone(),
            effect: binding.clone(),
        })
        .expect("record commits");
    assert_eq!(recorded.generation, 2);
    assert_eq!(journal_rows(&path, &agg.0), 3);
    assert_eq!(receipt_rows(&path, &agg.0), 3);

    // Position the projection at the SQLite ceiling. The effect state and the
    // recorded receipt are untouched, so the exact duplicate must still replay.
    position_projection(&path, &agg, &identity, STORE_MAX as u64 - 1);

    // G-002: an exact duplicate replays before capability currentness; the
    // original generation-1 capability is stale against the positioned snapshot,
    // yet the duplicate replays with zero mutation and zero new rows.
    let duplicate = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: planned.capability.clone(),
            effect: binding.clone(),
        })
        .expect("exact duplicate replays at the ceiling");
    assert!(!duplicate.mutation);
    assert_eq!(
        duplicate.receipt.expect("replay receipt").receipt_id,
        recorded.receipt.expect("record receipt").receipt_id
    );
    assert_eq!(counters(&path, &agg.0), (STORE_MAX - 1, STORE_MAX - 1));
    assert_eq!(journal_rows(&path, &agg.0), 3);
    assert_eq!(receipt_rows(&path, &agg.0), 3);

    // Binding rules still work at the ceiling position: a different outcome on
    // the same identity refuses as a conflict (before any counter arithmetic)
    // with a current capability, and writes nothing.
    let mut conflict = binding.clone();
    conflict.outcome_digest = "other".into();
    let err = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: capability(&identity, &agg, STORE_MAX as u64 - 1),
            effect: conflict,
        })
        .expect_err("conflicting binding refuses");
    assert_eq!(err.code, DiagnosticCode::IkEffectOutcomeConflict);
    assert_eq!(journal_rows(&path, &agg.0), 3);
    assert_eq!(receipt_rows(&path, &agg.0), 3);
    assert_eq!(counters(&path, &agg.0), (STORE_MAX - 1, STORE_MAX - 1));

    // The journal was never touched by any of the above: replay rebuilds the
    // positioned projection back to the true journal state, proving the runtime
    // replay path is unaffected by ceiling positioning.
    let summary = store.replay(&machine, Some(&agg)).expect("replay");
    assert!(summary.aggregates[0].rebuilt);
    assert_eq!(summary.aggregates[0].generation, 2);
    assert_eq!(summary.aggregates[0].sequence, 2);
    let after = store.show(&agg).expect("show after replay");
    assert_eq!(after.generation, 2);
    assert_eq!(after.sequence, 2);
    assert_eq!(journal_rows(&path, &agg.0), 3);
    assert_eq!(receipt_rows(&path, &agg.0), 3);
}
