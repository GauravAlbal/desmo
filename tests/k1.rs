//! K1 — Durable positive-control runtime.
//!
//! Defends: durable store/aggregate identity; generation +1 exactly once per
//! committed mutation; nonce rotation; stale and copied-stale capability rejection;
//! one same-generation race commit (BEGIN IMMEDIATE serialization); atomic
//! journal+projection+receipt; reopen preserving state; durable receipts; replay;
//! projection/journal corruption detection; failed commands leave no partial state.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::diagnostic::DiagnosticCode;
use ik::identity::{AggregateId, Capability};
use ik::store::{CommandRequest, Store};
use serde_json::json;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

fn va() -> Box<dyn ik::machine::Machine> {
    Box::new(load_machine("verification_attempt"))
}

fn setup(dir: &tempfile::TempDir) -> (Store, AggregateId, Box<dyn ik::machine::Machine>) {
    let machine = va();
    let identity = machine.identity().clone();
    let mut store = Store::create(&dir.path().join("s.db")).expect("create store");
    let agg = AggregateId("agg-1".into());
    store
        .create_aggregate(machine.as_ref(), &agg, &identity)
        .expect("create aggregate");
    (store, agg, machine)
}

fn capability(
    identity: &ik::identity::MachineIdentity,
    agg: &AggregateId,
    generation: u64,
) -> Capability {
    Capability {
        generation,
        nonce: ik::runtime::derive_nonce(identity, agg, generation),
    }
}

fn start(
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

#[test]
fn store_identity_is_durable_across_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let store = Store::create(&path).expect("create");
    let id = store.store_id().clone();
    drop(store);
    let reopened = Store::open(&path).expect("reopen");
    assert_eq!(reopened.store_id(), &id);
}

#[test]
fn aggregate_identity_and_state_survive_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    {
        let (mut store, agg, machine) = setup(&dir);
        let identity = machine.identity().clone();
        let cap = capability(&identity, &agg, 0);
        let out = start(&mut store, machine.as_ref(), &agg, &cap);
        assert_eq!(out.generation, 1);
        drop(store);
    }
    let mut store = Store::open(&path).expect("reopen");
    let machine = va();
    let projection = store.show(&AggregateId("agg-1".into())).expect("show");
    assert_eq!(projection.state["state"], "running");
    assert_eq!(projection.generation, 1);
    assert!(machine.identity() == &projection.machine);
}

#[test]
fn journal_is_compact_and_projection_owns_the_only_complete_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let expected = {
        let (mut store, agg, machine) = setup(&dir);
        let cap = capability(machine.identity(), &agg, 0);
        start(&mut store, machine.as_ref(), &agg, &cap).state
    };
    let conn = rusqlite::Connection::open(&path).expect("open sqlite store");
    let mut stmt = conn
        .prepare("SELECT event_json FROM journal WHERE aggregate_id = 'agg-1' ORDER BY sequence")
        .expect("prepare journal query");
    let events = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query journal")
        .collect::<Result<Vec<_>, _>>()
        .expect("read journal");
    assert_eq!(events.len(), 2);
    for encoded in events {
        let event: serde_json::Value = serde_json::from_str(&encoded).expect("decode event");
        assert!(
            event.get("state").is_none(),
            "journal event must not persist complete post-state"
        );
        assert!(
            event
                .get("state_digest")
                .and_then(serde_json::Value::as_str)
                .is_some(),
            "journal event must bind the expected post-state digest"
        );
    }
    let projection: String = conn
        .query_row(
            "SELECT state FROM aggregates WHERE aggregate_id = 'agg-1'",
            [],
            |row| row.get(0),
        )
        .expect("read projection state");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&projection).expect("decode projection"),
        expected
    );
}

#[test]
fn altered_domain_state_digest_refuses_replay() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    {
        let (mut store, agg, machine) = setup(&dir);
        let cap = capability(machine.identity(), &agg, 0);
        start(&mut store, machine.as_ref(), &agg, &cap);
    }
    rewrite_journal_event(&path, 1, |event| {
        assert_eq!(event.event_kind, ik::runtime::EventKind::DomainCommand);
        event.state_digest = ik::canonical::digest_json(&json!({"state": "forged"}));
    });
    let mut store = Store::open(&path).expect("reopen");
    let err = store
        .replay(va().as_ref(), Some(&AggregateId("agg-1".into())))
        .expect_err("digest-only domain corruption must refuse replay");
    assert_eq!(err.code, DiagnosticCode::IkReplayMismatch);
    assert_eq!(err.rule.as_deref(), Some("replay.state"));
}

#[test]
fn generation_increments_exactly_once_per_committed_mutation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut store, agg, machine) = setup(&dir);
    let identity = machine.identity().clone();
    let mut gen = 0u64;
    for expected in [1u64, 2, 3] {
        let cap = capability(&identity, &agg, gen);
        let command = match expected {
            1 => json!({"name": "start"}),
            2 => {
                json!({"name": "submit", "args": {"artifact_digest": "a1", "baseline_digest": "b1"}})
            }
            _ => json!({"name": "reject"}),
        };
        let out = store
            .command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap,
                    command,
                    observation: None,
                },
            )
            .expect("command commits");
        assert_eq!(out.generation, expected);
        gen = expected;
    }
}

#[test]
fn nonce_rotates_and_prior_capabilities_reject() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut store, agg, machine) = setup(&dir);
    let identity = machine.identity().clone();
    let cap0 = capability(&identity, &agg, 0);
    let out0 = start(&mut store, machine.as_ref(), &agg, &cap0);
    // Nonce rotated.
    assert_ne!(out0.capability.nonce, cap0.nonce);
    // The prior capability now refuses as stale.
    let err = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap0,
                command: json!({"name": "submit", "args": {"artifact_digest": "a", "baseline_digest": "b"}}),
                observation: None,
            },
        )
        .expect_err("stale capability must refuse");
    assert_eq!(err.code, DiagnosticCode::IkCapabilityStale);
}

#[test]
fn copied_stale_capability_cannot_commit_twice() {
    // REQ-IK-005: copying capability bytes must not produce two committed successors.
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut store, agg, machine) = setup(&dir);
    let identity = machine.identity().clone();
    let cap = capability(&identity, &agg, 0);
    // First copy commits.
    let out1 = start(&mut store, machine.as_ref(), &agg, &cap);
    assert_eq!(out1.generation, 1);
    // Second copy (same bytes) refuses.
    let err = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap.clone(),
                command: json!({"name": "submit", "args": {"artifact_digest": "a", "baseline_digest": "b"}}),
                observation: None,
            },
        )
        .expect_err("copied capability must refuse");
    assert_eq!(err.code, DiagnosticCode::IkCapabilityStale);
}

#[test]
fn modified_nonce_refuses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut store, agg, machine) = setup(&dir);
    let identity = machine.identity().clone();
    let bad = Capability {
        generation: 0,
        nonce: "0000000000000000000000000000000000000000000000000000000000000000".into(),
    };
    let err = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity,
                capability: bad,
                command: json!({"name": "start"}),
                observation: None,
            },
        )
        .expect_err("modified nonce must refuse");
    assert_eq!(err.code, DiagnosticCode::IkNonceInvalid);
}

#[test]
fn same_generation_race_commits_exactly_once() {
    // REQ-IK-007: preserve the real contention seam (two SQLite connections) while
    // bounding liveness. A sequential copied-capability test covers the cheap law;
    // this test covers the lock/currentness race archetype.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    {
        let (store, _agg, _machine) = setup(&dir);
        drop(store);
    }
    let machine: Arc<dyn ik::machine::Machine> = Arc::from(va());
    let identity = machine.identity().clone();
    let agg = AggregateId("agg-1".into());
    let cap = capability(&identity, &agg, 0);

    let (ready_send, ready_receive) = mpsc::channel();
    let (result_send, result_receive) = mpsc::channel();
    let mut start_senders = Vec::new();
    let handles = (0..2)
        .map(|_| {
            let machine = machine.clone();
            let identity = identity.clone();
            let agg = agg.clone();
            let cap = cap.clone();
            let path = path.clone();
            let ready_send = ready_send.clone();
            let result_send = result_send.clone();
            let (start_send, start_receive) = mpsc::channel();
            start_senders.push(start_send);
            thread::spawn(move || {
                let mut store = Store::open(&path).expect("open store");
                ready_send.send(()).expect("signal worker readiness");
                start_receive
                    .recv_timeout(Duration::from_secs(2))
                    .expect("start signal exceeded the two-second liveness bound");
                let result = match store.command(
                    machine.as_ref(),
                    &CommandRequest {
                        aggregate_id: agg,
                        machine: identity,
                        capability: cap,
                        command: json!({"name": "start"}),
                        observation: None,
                    },
                ) {
                    Ok(out) => DiagnosticCodeOrOk::Ok(out.generation),
                    Err(d) => DiagnosticCodeOrOk::Err(d.code),
                };
                result_send.send(result).expect("send race result");
            })
        })
        .collect::<Vec<_>>();
    drop(ready_send);
    drop(result_send);

    for _ in 0..2 {
        ready_receive
            .recv_timeout(Duration::from_secs(2))
            .expect("worker setup exceeded the two-second liveness bound");
    }
    for start in start_senders {
        start.send(()).expect("release prepared race worker");
    }
    let results = (0..2)
        .map(|_| {
            result_receive
                .recv_timeout(Duration::from_secs(2))
                .expect("race worker exceeded the two-second liveness bound")
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().expect("race worker panicked");
    }

    let committed_generations = results
        .iter()
        .filter_map(|result| match result {
            DiagnosticCodeOrOk::Ok(generation) => Some(*generation),
            DiagnosticCodeOrOk::Err(_) => None,
        })
        .collect::<Vec<_>>();
    let stale_count = results
        .iter()
        .filter(|result| {
            matches!(
                result,
                DiagnosticCodeOrOk::Err(DiagnosticCode::IkCapabilityStale)
            )
        })
        .count();
    assert_eq!(
        committed_generations,
        [1],
        "exactly one generation-1 commit"
    );
    assert_eq!(stale_count, 1, "exactly one stale refusal, got {results:?}");

    let mut store = Store::open(&path).expect("open");
    let projection = store.show(&agg).expect("show");
    assert_eq!(projection.generation, 1);
    assert_eq!(
        projection.sequence, 1,
        "exactly one journal event committed"
    );
    assert_eq!(projection.state["state"], "running");
}

#[derive(Debug)]
enum DiagnosticCodeOrOk {
    Ok(u64),
    Err(DiagnosticCode),
}

#[test]
fn journal_and_projection_update_atomically_with_receipts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut store, agg, machine) = setup(&dir);
    let identity = machine.identity().clone();
    let cap = capability(&identity, &agg, 0);
    let out = start(&mut store, machine.as_ref(), &agg, &cap);
    let receipt = out.receipt.expect("committed transitions return receipts");

    // Receipt fields equal committed event identity/state/generation fields.
    assert_eq!(receipt.aggregate_id, agg);
    assert_eq!(receipt.generation, 1);
    assert_eq!(receipt.sequence, 1);
    assert_eq!(receipt.receipt_id, receipt.event_hash);
    assert_eq!(receipt.machine_id, identity.machine_id);

    let projection = store.show(&agg).expect("show");
    assert_eq!(projection.sequence, 1);
    assert_eq!(projection.generation, 1);
    assert_eq!(
        projection
            .last_receipt
            .as_ref()
            .map(|r| r.receipt_id.clone()),
        Some(receipt.receipt_id.clone())
    );
}

#[test]
fn failed_commands_leave_no_partial_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut store, agg, machine) = setup(&dir);
    let identity = machine.identity().clone();
    let cap = capability(&identity, &agg, 0);

    // A refused command (wrong nonce) must not change anything.
    let bad = Capability {
        generation: 0,
        nonce: "x".into(),
    };
    assert!(store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: bad,
                command: json!({"name": "start"}),
                observation: None,
            },
        )
        .is_err());

    let projection = store.show(&agg).expect("show");
    assert_eq!(projection.generation, 0);
    assert_eq!(projection.sequence, 0);
    assert_eq!(projection.state["state"], "created");

    // A machine-refused command (illegal state) also leaves no partial state.
    let err = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap.clone(),
                command: json!({"name": "submit", "args": {"artifact_digest": "a", "baseline_digest": "b"}}),
                observation: None,
            },
        )
        .expect_err("illegal state command refuses");
    assert_eq!(err.code, DiagnosticCode::IkIllegalState);
    let projection = store.show(&agg).expect("show");
    assert_eq!(projection.generation, 0);
    assert_eq!(projection.sequence, 0);
}

#[test]
fn replay_reconstructs_projection_and_verify_passes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    {
        let (mut store, agg, machine) = setup(&dir);
        let identity = machine.identity().clone();
        let mut cap = capability(&identity, &agg, 0);
        for command in [
            json!({"name": "start"}),
            json!({"name": "submit", "args": {"artifact_digest": "a1", "baseline_digest": "b1"}}),
            json!({"name": "verify", "args": {"receipt_digest": "r1", "receipt_artifact_digest": "a1", "receipt_baseline_digest": "b1"}}),
        ] {
            let out = store
                .command(
                    machine.as_ref(),
                    &CommandRequest {
                        aggregate_id: agg.clone(),
                        machine: identity.clone(),
                        capability: cap,
                        command,
                        observation: None,
                    },
                )
                .expect("commit");
            cap = out.capability;
        }
        store.verify(machine.as_ref(), None).expect("verify passes");
        drop(store);
    }
    // Reopen + replay reproduces the same projection.
    let mut store = Store::open(&path).expect("reopen");
    let machine = va();
    let summary = store.replay(machine.as_ref(), None).expect("replay");
    assert_eq!(summary.aggregates.len(), 1);
    let entry = &summary.aggregates[0];
    assert_eq!(entry.generation, 3);
    assert!(!entry.rebuilt);
    assert_eq!(entry.state["state"], "verified");
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after replay");
}

#[test]
fn projection_corruption_is_detected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    {
        let (mut store, agg, machine) = setup(&dir);
        let identity = machine.identity().clone();
        let cap = capability(&identity, &agg, 0);
        start(&mut store, machine.as_ref(), &agg, &cap);
        drop(store);
    }
    // Corrupt the projection directly in SQLite.
    let conn = rusqlite::Connection::open(&path).expect("open");
    conn.execute_batch(
        "UPDATE aggregates SET state = '{\"state\":\"cancelled\"}' WHERE aggregate_id = 'agg-1'",
    )
    .expect("mutate projection");
    drop(conn);

    let mut store = Store::open(&path).expect("reopen");
    let machine = va();
    let err = store
        .verify(machine.as_ref(), None)
        .expect_err("verify refuses");
    assert_eq!(err.code, DiagnosticCode::IkProjectionCorrupt);

    // Replay repairs the projection; verify then passes.
    let summary = store
        .replay(machine.as_ref(), None)
        .expect("replay repairs");
    assert!(summary.aggregates[0].rebuilt);
    store
        .verify(machine.as_ref(), None)
        .expect("verify passes after repair");
}

#[test]
fn journal_corruption_is_detected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    {
        let (mut store, agg, machine) = setup(&dir);
        let identity = machine.identity().clone();
        let cap = capability(&identity, &agg, 0);
        start(&mut store, machine.as_ref(), &agg, &cap);
        drop(store);
    }
    let conn = rusqlite::Connection::open(&path).expect("open");
    conn.execute_batch(
        "UPDATE journal SET hash = 'deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef' WHERE aggregate_id = 'agg-1' AND sequence = 1",
    )
    .expect("mutate journal");
    drop(conn);

    let mut store = Store::open(&path).expect("reopen");
    let machine = va();
    let err = store
        .verify(machine.as_ref(), None)
        .expect_err("verify refuses");
    assert_eq!(err.code, DiagnosticCode::IkJournalCorrupt);
}

fn rewrite_journal_event(
    path: &std::path::Path,
    sequence: u64,
    mutate: impl FnOnce(&mut ik::runtime::JournalEvent),
) {
    let conn = rusqlite::Connection::open(path).expect("open");
    let tail: u64 = conn
        .query_row(
            "SELECT MAX(sequence) FROM journal WHERE aggregate_id = 'agg-1'",
            [],
            |row| row.get(0),
        )
        .expect("load journal tail");
    assert_eq!(
        sequence, tail,
        "semantic corruption helper only rewrites the tail; rewriting ancestors must rehash descendants"
    );
    let encoded: String = conn
        .query_row(
            "SELECT event_json FROM journal WHERE aggregate_id = 'agg-1' AND sequence = ?1",
            [sequence as i64],
            |row| row.get(0),
        )
        .expect("load journal event");
    let mut event: ik::runtime::JournalEvent =
        serde_json::from_str(&encoded).expect("decode journal event");
    mutate(&mut event);
    event.hash = ik::runtime::recompute_event_hash(&event);
    let encoded = ik::canonical::canonical_json(&event);
    conn.execute(
        "UPDATE journal SET hash = ?1, event_json = ?2 WHERE aggregate_id = 'agg-1' AND sequence = ?3",
        rusqlite::params![event.hash, encoded, sequence as i64],
    )
    .expect("rewrite journal event");
    conn.execute(
        "UPDATE aggregates SET last_event_hash = ?1 WHERE aggregate_id = 'agg-1'",
        [event.hash],
    )
    .expect("align projection chain head");
}

#[test]
fn authenticated_genesis_origin_corruption_fails_replay() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    {
        let (store, _, _) = setup(&dir);
        drop(store);
    }
    rewrite_journal_event(&path, 0, |event| {
        event.event_kind = ik::runtime::EventKind::DomainCommand;
    });

    let mut store = Store::open(&path).expect("reopen");
    let err = store
        .replay(va().as_ref(), None)
        .expect_err("replay refuses");
    assert_eq!(err.code, DiagnosticCode::IkReplayMismatch);
}

#[test]
fn authenticated_genesis_payload_corruption_fails_replay() {
    for case in [
        "effect",
        "observation",
        "note",
        "terminal",
        "operation",
        "aggregate",
        "machine",
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("s.db");
        {
            let (store, _, _) = setup(&dir);
            drop(store);
        }
        rewrite_journal_event(&path, 0, |event| match case {
            "effect" => {
                event.effect_delta = ik::runtime::EffectDelta::Plan {
                    intent: ik::effect::EffectIntentV1 {
                        effect_id: ik::identity::EffectId("forged".into()),
                        aggregate_id: event.aggregate_id.clone(),
                        generation: 0,
                        effect_kind: "forged".into(),
                        payload_digest: "forged".into(),
                        idempotency_key: "forged".into(),
                        payload: None,
                    },
                }
            }
            "observation" => event.observation = Some(json!({"forged": true})),
            "note" => event.note = Some(json!({"forged": true})),
            "terminal" => event.terminal = true,
            "operation" => event.command["op"] = json!("forged"),
            "aggregate" => event.command["aggregate_id"] = json!("forged"),
            "machine" => event.command["machine"] = json!({"forged": true}),
            _ => unreachable!(),
        });
        let mut store = Store::open(&path).expect("reopen");
        let err = match store.replay(va().as_ref(), None) {
            Err(err) => err,
            Ok(ok) => panic!("case {case} unexpectedly replayed: {ok:?}"),
        };
        assert_eq!(err.code, DiagnosticCode::IkReplayMismatch, "case {case}");
    }
}

#[test]
fn authenticated_genesis_digest_and_contract_corruption_fails_replay() {
    // Each case corrupts one hash-authenticated genesis field. The state digest
    // remains the sole journaled check on the deterministic initial state.
    for case in ["state_digest", "event_type", "event_version", "payload"] {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("s.db");
        {
            let (store, _, _) = setup(&dir);
            drop(store);
        }
        rewrite_journal_event(&path, 0, |event| match case {
            "state_digest" => {
                event.state_digest = ik::canonical::digest_json(&json!({"forged": true}));
            }
            "event_type" => event.event_type = "forged".into(),
            "event_version" => event.event_version = 0,
            "payload" => event.payload = json!({"forged": true}),
            _ => unreachable!(),
        });
        let mut store = Store::open(&path).expect("reopen");
        let err = match store.replay(va().as_ref(), None) {
            Err(err) => err,
            Ok(ok) => panic!("case {case} unexpectedly replayed: {ok:?}"),
        };
        assert_eq!(err.code, DiagnosticCode::IkReplayMismatch, "case {case}");
    }
}

#[test]
fn authenticated_domain_origin_corruption_fails_replay() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    {
        let (mut store, agg, machine) = setup(&dir);
        let cap = capability(machine.identity(), &agg, 0);
        start(&mut store, machine.as_ref(), &agg, &cap);
        drop(store);
    }
    rewrite_journal_event(&path, 1, |event| {
        event.event_kind = ik::runtime::EventKind::KernelRecord;
    });

    let mut store = Store::open(&path).expect("reopen");
    let err = store
        .replay(va().as_ref(), None)
        .expect_err("replay refuses");
    assert_eq!(err.code, DiagnosticCode::IkReplayMismatch);
}

#[test]
fn authenticated_domain_payload_corruption_fails_replay() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    {
        let (mut store, agg, machine) = setup(&dir);
        let cap = capability(machine.identity(), &agg, 0);
        start(&mut store, machine.as_ref(), &agg, &cap);
        drop(store);
    }
    rewrite_journal_event(&path, 1, |event| {
        event.payload = json!({"outcome": 999, "args": {}});
    });

    let mut store = Store::open(&path).expect("reopen");
    let err = store
        .replay(va().as_ref(), None)
        .expect_err("replay refuses");
    assert_eq!(err.code, DiagnosticCode::IkReplayMismatch);
}

#[test]
fn authenticated_declarative_source_corruption_fails_replay() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    {
        let (mut store, agg, machine) = setup(&dir);
        let cap = capability(machine.identity(), &agg, 0);
        start(&mut store, machine.as_ref(), &agg, &cap);
        drop(store);
    }
    rewrite_journal_event(&path, 1, |event| {
        event.payload["source"] = json!("running");
    });

    let mut store = Store::open(&path).expect("reopen");
    let err = store
        .replay(va().as_ref(), None)
        .expect_err("replay refuses");
    assert_eq!(err.code, DiagnosticCode::IkReplayMismatch);
}

#[test]
fn authenticated_domain_machine_identity_corruption_fails_replay() {
    // The journal event binds the authoring machine identity (id, version, spec
    // hash); replay must refuse a domain tail event whose machine identity no longer
    // matches the presented machine, even though the hash chain re-authenticates it.
    for case in ["machine_id", "machine_version", "machine_spec_hash"] {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("s.db");
        {
            let (mut store, agg, machine) = setup(&dir);
            let cap = capability(machine.identity(), &agg, 0);
            start(&mut store, machine.as_ref(), &agg, &cap);
            drop(store);
        }
        rewrite_journal_event(&path, 1, |event| match case {
            "machine_id" => event.machine_id = "forged".into(),
            "machine_version" => event.machine_version = 999,
            "machine_spec_hash" => event.machine_spec_hash = "forged".into(),
            _ => unreachable!(),
        });
        let mut store = Store::open(&path).expect("reopen");
        let err = match store.replay(va().as_ref(), None) {
            Err(err) => err,
            Ok(ok) => panic!("case {case} unexpectedly replayed: {ok:?}"),
        };
        assert_eq!(err.code, DiagnosticCode::IkReplayMismatch, "case {case}");
    }
}

#[test]
fn authenticated_domain_unknown_event_schema_fails_replay() {
    // An unknown domain event type/version must surface the machine's schema refusal
    // (IK_EVENT_SCHEMA_UNKNOWN), distinct from a malformed payload, which folds into
    // IK_REPLAY_MISMATCH.
    for case in ["event_type", "event_version"] {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("s.db");
        {
            let (mut store, agg, machine) = setup(&dir);
            let cap = capability(machine.identity(), &agg, 0);
            start(&mut store, machine.as_ref(), &agg, &cap);
            drop(store);
        }
        rewrite_journal_event(&path, 1, |event| match case {
            "event_type" => event.event_type = "forged.command".into(),
            "event_version" => event.event_version = 999,
            _ => unreachable!(),
        });
        let mut store = Store::open(&path).expect("reopen");
        let err = match store.replay(va().as_ref(), None) {
            Err(err) => err,
            Ok(ok) => panic!("case {case} unexpectedly replayed: {ok:?}"),
        };
        assert_eq!(
            err.code,
            DiagnosticCode::IkEventSchemaUnknown,
            "case {case}"
        );
    }
}

#[test]
fn unknown_persisted_schema_version_fails_closed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    Store::create(&path).expect("create");
    let conn = rusqlite::Connection::open(&path).expect("open");
    conn.execute_batch("UPDATE meta SET value = '999' WHERE key = 'schema_version'")
        .expect("mutate schema version");
    drop(conn);
    let err = Store::open(&path);
    let code = match err {
        Err(d) => d.code,
        Ok(_) => panic!("open must refuse an unknown schema version"),
    };
    assert_eq!(code, DiagnosticCode::IkPersistedSchemaUnknown);
}

#[test]
fn duplicate_aggregate_create_refuses_and_missing_aggregate_refuses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut store, agg, machine) = setup(&dir);
    let identity = machine.identity().clone();
    let err = store
        .create_aggregate(machine.as_ref(), &agg, &identity)
        .expect_err("duplicate create refuses");
    assert_eq!(err.code, DiagnosticCode::IkIllegalState);

    let ghost = AggregateId("ghost".into());
    let err = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: ghost,
                machine: identity,
                capability: Capability {
                    generation: 0,
                    nonce: "x".into(),
                },
                command: json!({"name": "start"}),
                observation: None,
            },
        )
        .expect_err("missing aggregate refuses");
    assert_eq!(err.code, DiagnosticCode::IkIllegalState);
}

#[test]
fn create_fails_closed_on_machine_identity_mismatch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::create(&dir.path().join("s.db")).expect("create");
    let machine = va();
    let wrong = ik::machine::native_identity("other", 1, "hash");
    assert_eq!(
        store
            .create_aggregate(machine.as_ref(), &AggregateId("a".into()), &wrong)
            .expect_err("unknown machine id refuses")
            .code,
        DiagnosticCode::IkMachineUnknown
    );
    let wrong = ik::machine::native_identity("verification_attempt", 99, "hash");
    assert_eq!(
        store
            .create_aggregate(machine.as_ref(), &AggregateId("a".into()), &wrong)
            .expect_err("unknown version refuses")
            .code,
        DiagnosticCode::IkMachineVersionUnknown
    );
    let wrong = ik::machine::native_identity("verification_attempt", 1, "hash");
    assert_eq!(
        store
            .create_aggregate(machine.as_ref(), &AggregateId("a".into()), &wrong)
            .expect_err("hash mismatch refuses")
            .code,
        DiagnosticCode::IkMachineHashMismatch
    );
}

#[test]
fn capability_nonce_is_bound_to_aggregate_and_rejects_cross_aggregate_use() {
    let dir = tempfile::tempdir().expect("tempdir");
    let machine = va();
    let identity = machine.identity().clone();
    let mut store = Store::create(&dir.path().join("s.db")).expect("create store");
    let first = AggregateId("agg-1".into());
    let second = AggregateId("agg-2".into());
    let first_created = store
        .create_aggregate(machine.as_ref(), &first, &identity)
        .expect("create first aggregate");
    let second_created = store
        .create_aggregate(machine.as_ref(), &second, &identity)
        .expect("create second aggregate");
    assert_ne!(
        first_created.capability.nonce, second_created.capability.nonce,
        "same-generation capabilities must remain aggregate-specific"
    );

    let err = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: second.clone(),
                machine: identity,
                capability: first_created.capability,
                command: json!({"name": "start"}),
                observation: None,
            },
        )
        .expect_err("a capability from another aggregate must refuse live");
    assert_eq!(err.code, DiagnosticCode::IkNonceInvalid);
    let projection = store.show(&second).expect("show second aggregate");
    assert_eq!(projection.generation, 0);
    assert_eq!(projection.sequence, 0);
    assert_eq!(projection.state["state"], "created");
}

#[test]
fn consistently_reanchored_generation_tamper_fails_verify_chain() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    {
        let (mut store, agg, machine) = setup(&dir);
        let identity = machine.identity().clone();
        start(
            &mut store,
            machine.as_ref(),
            &agg,
            &capability(&identity, &agg, 0),
        );
        drop(store);
    }

    let conn = rusqlite::Connection::open(&path).expect("open database");
    let encoded: String = conn
        .query_row(
            "SELECT event_json FROM journal WHERE aggregate_id = 'agg-1' AND sequence = 1",
            [],
            |row| row.get(0),
        )
        .expect("load event");
    let mut event: ik::runtime::JournalEvent =
        serde_json::from_str(&encoded).expect("decode event");
    event.generation = 2;
    event.hash = ik::runtime::recompute_event_hash(&event);
    let encoded = ik::canonical::canonical_json(&event);
    conn.execute(
        "UPDATE journal SET generation = ?1, hash = ?2, event_json = ?3 WHERE aggregate_id = 'agg-1' AND sequence = 1",
        rusqlite::params![event.generation as i64, event.hash, encoded],
    )
    .expect("rewrite journal generation and hash");
    let receipt_json: String = conn
        .query_row(
            "SELECT receipt_json FROM receipts WHERE aggregate_id = 'agg-1' AND sequence = 1",
            [],
            |row| row.get(0),
        )
        .expect("load receipt");
    let mut receipt: ik::runtime::Receipt =
        serde_json::from_str(&receipt_json).expect("decode receipt");
    receipt.generation = 2;
    conn.execute(
        "UPDATE receipts SET receipt_json = ?1 WHERE aggregate_id = 'agg-1' AND sequence = 1",
        [ik::canonical::canonical_json(&receipt)],
    )
    .expect("rewrite receipt generation");
    conn.execute(
        "UPDATE aggregates SET generation = 2, nonce = ?1, last_event_hash = ?2 WHERE aggregate_id = 'agg-1'",
        rusqlite::params![
            ik::runtime::derive_nonce(&identity_of("verification_attempt"), &AggregateId("agg-1".into()), 2),
            event.hash,
        ],
    )
    .expect("reanchor projection generation");
    drop(conn);

    let mut store = Store::open(&path).expect("reopen");
    let err = store
        .verify(va().as_ref(), None)
        .expect_err("generation must remain tied to journal sequence");
    assert_eq!(err.code, DiagnosticCode::IkJournalCorrupt);
    assert_eq!(err.rule.as_deref(), Some("journal.generation"));
}

#[test]
fn journal_generation_and_prev_hash_column_mismatches_refuse() {
    for column in ["generation", "prev_hash"] {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("s.db");
        {
            let (mut store, agg, machine) = setup(&dir);
            let identity = machine.identity().clone();
            start(
                &mut store,
                machine.as_ref(),
                &agg,
                &capability(&identity, &agg, 0),
            );
            drop(store);
        }
        let conn = rusqlite::Connection::open(&path).expect("open database");
        let sql = match column {
            "generation" => {
                "UPDATE journal SET generation = 999 WHERE aggregate_id = 'agg-1' AND sequence = 1"
            }
            "prev_hash" => {
                "UPDATE journal SET prev_hash = 'forged' WHERE aggregate_id = 'agg-1' AND sequence = 1"
            }
            _ => unreachable!(),
        };
        conn.execute_batch(sql).expect("corrupt journal column");
        drop(conn);

        let mut store = Store::open(&path).expect("reopen");
        let err = store
            .verify(va().as_ref(), None)
            .expect_err("column-only journal corruption must refuse");
        assert_eq!(
            err.code,
            DiagnosticCode::IkJournalCorrupt,
            "column={column}"
        );
        assert_eq!(
            err.rule.as_deref(),
            Some("journal.columns"),
            "column={column}"
        );
    }
}
