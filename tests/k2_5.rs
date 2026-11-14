//! K2.5 — Minimal external effect intent.
//!
//! Defends: pending effect identity is durable across reopen; wrong-effect outcome
//! refuses (IK_EFFECT_MISMATCH); stale outcome refuses (IK_EFFECT_STALE); conflicting
//! outcome refuses (IK_EFFECT_OUTCOME_CONFLICT); exact duplicate recording REPLAYS
//! the original receipt with zero mutation and zero error, recognized BEFORE
//! stale-capability refusal (amendment 1); new effect while pending refuses; the
//! runtime adds no scheduler/retry/timer semantics (a pending effect survives
//! reopen and replay byte-identically — nothing auto-advances); the domain path
//! (native machine event fold carrying an outcome) binds through the same rules;
//! authenticated corruption of any semantic field of a committed effect-record
//! refuses replay (IK_REPLAY_MISMATCH); the native domain event fold refuses
//! forged plan intents, forged record outcomes, delta-variant swaps, and
//! chain-invalid records (no prior pending intent) or plans (a second pending
//! intent). Host neutrality (REQ-IK-002: no consumer/package/work-tracking/approval
//! semantics in the reusable runtime) is enforced on the wire by the strict
//! closed protocol suite in `tests/k2.rs` and exercised here by driving the
//! runtime through plain neutral machines.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::diagnostic::{Diagnostic, DiagnosticCode};
use ik::effect::{EffectIntentV1, EffectOutcomeV1, EffectStatus};
use ik::identity::{AggregateId, Capability, EffectId, MachineIdentity};
use ik::machine::{
    ApplyOutcome, Decision, DomainEvent, EventCodec, Machine, NativeApply, NativeDecision,
    NativeMachine,
};
use ik::store::{CommandRequest, EffectOutcomeRequest, Store};
use serde_json::{json, Value};

fn probe() -> Box<dyn ik::machine::Machine> {
    Box::new(load_machine("effect_probe"))
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

fn cap(identity: &ik::identity::MachineIdentity, agg: &AggregateId, generation: u64) -> Capability {
    Capability {
        generation,
        nonce: ik::runtime::derive_nonce(identity, agg, generation),
    }
}

fn plan(
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

fn outcome(agg: &AggregateId, effect_id: &str, generation: u64, digest: &str) -> EffectOutcomeV1 {
    EffectOutcomeV1 {
        effect_id: EffectId(effect_id.to_string()),
        aggregate_id: agg.clone(),
        planned_generation: generation,
        payload_digest: "p1".to_string(),
        idempotency_key: "k1".to_string(),
        outcome_digest: digest.to_string(),
    }
}

#[test]
fn pending_effect_identity_is_durable_across_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    {
        let (mut store, agg, machine) = setup_probe(&dir);
        let identity = machine.identity().clone();
        let cap = cap(&identity, &agg, 0);
        let out = plan(&mut store, machine.as_ref(), &agg, &cap);
        let EffectStatus::Pending(intent) = out.effect else {
            panic!("expected a pending effect, got {:?}", out.effect);
        };
        assert_eq!(intent.effect_id.0, "e1");
        assert_eq!(intent.generation, 1); // runtime-stamped planned generation
        assert_eq!(intent.aggregate_id, agg);
        drop(store);
    }
    // Reopen: the pending intent survives with the exact same identity.
    let mut store = Store::open(&path).expect("reopen");
    let projection = store.show(&AggregateId("agg-1".into())).expect("show");
    let EffectStatus::Pending(intent) = projection.effect else {
        panic!("expected a pending effect after reopen");
    };
    assert_eq!(intent.effect_id.0, "e1");
    assert_eq!(intent.generation, 1);
    assert_eq!(intent.payload_digest, "p1");
    assert_eq!(intent.idempotency_key, "k1");
}

#[test]
fn wrong_and_stale_outcomes_refuse() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut store, agg, machine) = setup_probe(&dir);
    let identity = machine.identity().clone();

    // Outcome with nothing pending -> stale.
    let cap0 = cap(&identity, &agg, 0);
    let err = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: cap0.clone(),
            effect: outcome(&agg, "e1", 0, "o1"),
        })
        .expect_err("stale outcome must refuse");
    assert_eq!(err.code, DiagnosticCode::IkEffectStale);

    // Plan, then bind with a foreign effect id -> mismatch.
    let out1 = plan(&mut store, machine.as_ref(), &agg, &cap0);
    let cap1 = out1.capability;
    let err = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: cap1.clone(),
            effect: outcome(&agg, "e2", 1, "o1"),
        })
        .expect_err("foreign effect must refuse");
    assert_eq!(err.code, DiagnosticCode::IkEffectMismatch);

    // Same identity, different binding (planned generation) -> conflict.
    let err = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: cap1.clone(),
            effect: outcome(&agg, "e1", 0, "o1"),
        })
        .expect_err("conflicting binding must refuse");
    assert_eq!(err.code, DiagnosticCode::IkEffectOutcomeConflict);
}

#[test]
fn exact_duplicate_replays_before_stale_capability_with_zero_mutation() {
    // Amendment 1: a post-commit retry presents the pre-outcome capability (stale),
    // yet the exact duplicate must replay the original receipt with zero mutation.
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut store, agg, machine) = setup_probe(&dir);
    let identity = machine.identity().clone();

    let cap0 = cap(&identity, &agg, 0);
    let out1 = plan(&mut store, machine.as_ref(), &agg, &cap0);
    let cap1 = out1.capability;

    // Record the outcome (mutation: generation 1 -> 2).
    let binding = outcome(&agg, "e1", 1, "o1");
    let recorded = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: cap1.clone(),
            effect: binding.clone(),
        })
        .expect("record commits");
    assert!(recorded.mutation);
    assert_eq!(recorded.generation, 2);
    let original_receipt = recorded.receipt.expect("receipt");

    // Retry with the STALE capability: exact duplicate replays, zero mutation.
    let replayed = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: cap1, // stale: generation 1, current is 2
            effect: binding,
        })
        .expect("duplicate replay must not error");
    assert!(!replayed.mutation, "duplicate replay is zero-mutation");
    assert_eq!(replayed.generation, 2, "no generation increment on replay");
    let receipt = replayed.receipt.expect("replayed receipt");
    assert_eq!(receipt.receipt_id, original_receipt.receipt_id);

    // The store still shows the original recording: no new journal event.
    let projection = store.show(&agg).expect("show");
    assert_eq!(
        projection.sequence, 2,
        "replay must not append a journal event"
    );
    assert_eq!(projection.generation, 2);
}

#[test]
fn conflicting_duplicate_outcome_refuses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut store, agg, machine) = setup_probe(&dir);
    let identity = machine.identity().clone();
    let cap0 = cap(&identity, &agg, 0);
    let out1 = plan(&mut store, machine.as_ref(), &agg, &cap0);
    let cap1 = out1.capability;
    store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: cap1.clone(),
            effect: outcome(&agg, "e1", 1, "o1"),
        })
        .expect("record");
    // With the original (now stale) capability, currentness still dominates a
    // nonduplicate conflict; G-002's precedence exception is exact-replay only.
    let err = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: cap1,
            effect: outcome(&agg, "e1", 1, "o2"),
        })
        .expect_err("stale conflicting outcome must refuse as stale capability");
    assert_eq!(err.code, DiagnosticCode::IkCapabilityStale);

    // With current generation-2 authority, the same identity plus a different outcome
    // reaches the binding rule and refuses as the required conflict, with no mutation.
    let before = store.show(&agg).expect("show before current conflict");
    let err = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: before.capability.clone(),
            effect: outcome(&agg, "e1", 1, "o2"),
        })
        .expect_err("current conflicting outcome must refuse as conflict");
    assert_eq!(err.code, DiagnosticCode::IkEffectOutcomeConflict);
    let after = store.show(&agg).expect("show after current conflict");
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.sequence, before.sequence);
    assert_eq!(after.effect, before.effect);
}

#[test]
fn recorded_effect_rejects_foreign_outcome_as_mismatch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut store, agg, machine) = setup_probe(&dir);
    let identity = machine.identity().clone();
    let planned = plan(&mut store, machine.as_ref(), &agg, &cap(&identity, &agg, 0));
    let recorded = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: planned.capability,
            effect: outcome(&agg, "e1", 1, "o1"),
        })
        .expect("record outcome");
    let err = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity,
            capability: recorded.capability,
            effect: outcome(&agg, "foreign", 2, "o2"),
        })
        .expect_err("foreign outcome against recorded effect must mismatch");
    assert_eq!(err.code, DiagnosticCode::IkEffectMismatch);
}

#[test]
fn new_effect_while_pending_refuses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (mut store, agg, machine) = setup_probe(&dir);
    let identity = machine.identity().clone();
    let cap0 = cap(&identity, &agg, 0);
    let out1 = plan(&mut store, machine.as_ref(), &agg, &cap0);
    let cap1 = out1.capability;
    // The domain reducer owns this refusal and must prevent a second pending effect.
    let before = store.show(&agg).expect("show before refusal");
    let err = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap1,
                command: json!({"name": "plan", "args": {"effect_id": "e2", "payload_digest": "p2", "idempotency_key": "k2"}}),
                observation: None,
            },
        )
        .expect_err("plan while pending must refuse");
    assert_eq!(err.code, DiagnosticCode::IkIllegalState);
    let after = store.show(&agg).expect("show after refusal");
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.sequence, before.sequence);
    assert_eq!(after.state, before.state);
    assert_eq!(after.effect, before.effect);
}

/// Rewrite the tail kernel effect-record (sequence 2) with a mutated semantic field
/// and re-anchor the chain hash, so only the replay semantic checks can refuse.
/// Modeled on k1's `rewrite_journal_event`; this helper is tail-only because the
/// recorded effect is the last transition of the probe flow.
fn rewrite_effect_record(
    path: &std::path::Path,
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
        tail, 2,
        "corruption helper only rewrites the effect-record tail"
    );
    let encoded: String = conn
        .query_row(
            "SELECT event_json FROM journal WHERE aggregate_id = 'agg-1' AND sequence = 2",
            [],
            |row| row.get(0),
        )
        .expect("load journal event");
    let mut event: ik::runtime::JournalEvent =
        serde_json::from_str(&encoded).expect("decode journal event");
    mutate(&mut event);
    event.hash = ik::runtime::recompute_event_hash(&event);
    let encoded = ik::canonical::canonical_json(&event);
    conn.execute(
        "UPDATE journal SET hash = ?1, event_json = ?2 WHERE aggregate_id = 'agg-1' AND sequence = 2",
        rusqlite::params![event.hash, encoded],
    )
    .expect("rewrite journal event");
    conn.execute(
        "UPDATE aggregates SET last_event_hash = ?1 WHERE aggregate_id = 'agg-1'",
        [event.hash],
    )
    .expect("align projection chain head");
}

#[test]
fn authenticated_effect_record_corruption_fails_replay() {
    // Each case plans one effect and records its outcome (journal: genesis at 0,
    // plan at 1, kernel.effect_recorded at 2), then corrupts exactly one semantic
    // field of the sequence-2 kernel record and re-anchors the chain hash. Replay
    // must refuse every corruption as IK_REPLAY_MISMATCH.
    for case in [
        "event_type",
        "event_version",
        "payload",
        "payload_undecodable",
        "effect_delta",
        "binding",
        "observation",
        "state_digest",
        "terminal",
        "delta_variant",
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("s.db");
        {
            let (mut store, agg, machine) = setup_probe(&dir);
            let identity = machine.identity().clone();
            let cap0 = cap(&identity, &agg, 0);
            let out1 = plan(&mut store, machine.as_ref(), &agg, &cap0);
            let cap1 = out1.capability;
            store
                .record_effect(&EffectOutcomeRequest {
                    aggregate_id: agg.clone(),
                    machine: identity,
                    capability: cap1,
                    effect: outcome(&agg, "e1", 1, "o1"),
                })
                .expect("record commits");
            drop(store);
        }
        rewrite_effect_record(&path, |event| match case {
            "event_type" => event.event_type = "forged".into(),
            "event_version" => event.event_version = 2,
            "payload" => event.payload["outcome_digest"] = json!("forged"),
            "payload_undecodable" => event.payload = json!({"forged": true}),
            "effect_delta" => {
                let ik::runtime::EffectDelta::Record { outcome } = &mut event.effect_delta else {
                    panic!("kernel record must carry a record delta");
                };
                outcome.outcome_digest = "forged".into();
            }
            "binding" => {
                event.payload["payload_digest"] = json!("forged");
                let ik::runtime::EffectDelta::Record { outcome } = &mut event.effect_delta else {
                    panic!("kernel record must carry a record delta");
                };
                outcome.payload_digest = "forged".into();
            }
            "observation" => event.observation = Some(json!({"forged": true})),
            "state_digest" => event.state_digest = "forged".into(),
            "terminal" => event.terminal = true,
            "delta_variant" => event.effect_delta = ik::runtime::EffectDelta::None,
            _ => unreachable!(),
        });
        let mut store = Store::open(&path).expect("reopen");
        let err = match store.replay(probe().as_ref(), None) {
            Err(err) => err,
            Ok(ok) => panic!("case {case} unexpectedly replayed: {ok:?}"),
        };
        assert_eq!(err.code, DiagnosticCode::IkReplayMismatch, "case {case}");
    }
}

#[test]
fn domain_path_outcome_binds_through_the_runtime() {
    // A native machine may produce an outcome through its event fold (G-005
    // RECORD_EFFECT_OUTCOME flows through decide/apply); the runtime binds the exact
    // pending identity atomically.
    use ik::machine::NativeAdapter;
    let dir = tempfile::tempdir().expect("tempdir");
    let machine: Box<dyn ik::machine::Machine> =
        Box::new(NativeAdapter::new(crate::NativeOutcomeMachine::new()));
    let identity = machine.identity().clone();
    let mut store = Store::create(&dir.path().join("s.db")).expect("create store");
    let agg = AggregateId("dom".into());
    store
        .create_aggregate(machine.as_ref(), &agg, &identity)
        .expect("create");

    let cap0 = cap(&identity, &agg, 0);
    let out = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap0,
                command: json!({"name": "plan"}),
                observation: None,
            },
        )
        .expect("plan via native machine");
    let cap1 = out.capability;

    let bound = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap1.clone(),
                command: json!({"name": "resolve"}),
                observation: None,
            },
        )
        .expect("resolve via native machine");
    let EffectStatus::Recorded(intent, digest, _) = bound.effect else {
        panic!("expected a recorded effect, got {:?}", bound.effect);
    };
    assert_eq!(intent.effect_id.0, "dom-effect");
    assert_eq!(digest, "dom-outcome");
    store
        .verify(machine.as_ref(), Some(&agg))
        .expect("domain outcome journal replays exactly");

    // Exact duplicate through the domain path also replays, even with a stale cap.
    let replayed = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap1, // stale
                command: json!({"name": "resolve"}),
                observation: None,
            },
        )
        .expect("duplicate resolve replays");
    assert!(!replayed.mutation);
}

/// A native machine whose `resolve` command binds an outcome through the event fold.
struct NativeOutcomeMachine {
    identity: MachineIdentity,
}

impl NativeOutcomeMachine {
    fn new() -> Self {
        Self {
            identity: ik::machine::native_identity("native_outcome", 1, "native-outcome-spec-v1"),
        }
    }
}

/// Versioned domain events for the outcome machine. `Resolve` freezes the pending
/// intent identity (read from the effect status at decide time) so that `apply` can
/// rebuild the exact binding without consulting runtime state.
#[derive(Clone, Debug, PartialEq)]
enum NativeOutcomeEvent {
    Plan,
    Resolve {
        effect_id: String,
        planned_generation: u64,
        payload_digest: String,
        idempotency_key: String,
    },
}

impl EventCodec for NativeOutcomeEvent {
    fn event_type(&self) -> &'static str {
        match self {
            NativeOutcomeEvent::Plan => "plan",
            NativeOutcomeEvent::Resolve { .. } => "resolve",
        }
    }

    fn event_version(&self) -> u64 {
        1
    }

    fn payload(&self) -> Value {
        match self {
            NativeOutcomeEvent::Plan => json!({}),
            NativeOutcomeEvent::Resolve {
                effect_id,
                planned_generation,
                payload_digest,
                idempotency_key,
            } => json!({
                "effect_id": effect_id,
                "planned_generation": planned_generation,
                "payload_digest": payload_digest,
                "idempotency_key": idempotency_key,
            }),
        }
    }

    fn decode(event: &DomainEvent) -> Result<Option<Self>, Diagnostic> {
        // Unknown event type or version is not this machine's event schema.
        if event.event_version != 1 {
            return Ok(None);
        }
        match event.event_type.as_str() {
            "plan" => Ok(Some(NativeOutcomeEvent::Plan)),
            "resolve" => {
                let obj = event.payload.as_object().ok_or_else(|| {
                    Diagnostic::internal("native outcome resolve event payload is not an object")
                })?;
                let effect_id = obj
                    .get("effect_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        Diagnostic::internal("native outcome resolve event payload lacks effect_id")
                    })?;
                let planned_generation = obj
                    .get("planned_generation")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        Diagnostic::internal(
                            "native outcome resolve event payload lacks planned_generation",
                        )
                    })?;
                let payload_digest = obj
                    .get("payload_digest")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        Diagnostic::internal(
                            "native outcome resolve event payload lacks payload_digest",
                        )
                    })?;
                let idempotency_key = obj
                    .get("idempotency_key")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        Diagnostic::internal(
                            "native outcome resolve event payload lacks idempotency_key",
                        )
                    })?;
                Ok(Some(NativeOutcomeEvent::Resolve {
                    effect_id: effect_id.to_string(),
                    planned_generation,
                    payload_digest: payload_digest.to_string(),
                    idempotency_key: idempotency_key.to_string(),
                }))
            }
            _ => Ok(None),
        }
    }
}

impl NativeMachine for NativeOutcomeMachine {
    type State = serde_json::Value;
    type Command = serde_json::Value;
    type Observation = serde_json::Value;
    type Event = NativeOutcomeEvent;
    type Diagnostic = Diagnostic;

    fn identity(&self) -> &MachineIdentity {
        &self.identity
    }

    fn initial_state(&self) -> Self::State {
        json!({"state": "idle"})
    }

    fn decide(
        &self,
        _state: &Self::State,
        _observation: Option<&Self::Observation>,
        effect: &EffectStatus,
        command: &Self::Command,
    ) -> Result<NativeDecision<Self::Event>, Self::Diagnostic> {
        match command["name"].as_str() {
            Some("plan") => {
                // G-005 parity: only a PENDING effect blocks a new plan. A recorded
                // outcome leaves the intent completed, so planning again is legal.
                if matches!(effect, EffectStatus::Pending(_)) {
                    return Err(Diagnostic::new(
                        DiagnosticCode::IkEffectStale,
                        Some("effect.pending"),
                        "pending",
                    ));
                }
                Ok(NativeDecision::emit(NativeOutcomeEvent::Plan))
            }
            Some("resolve") => {
                let (EffectStatus::Pending(intent) | EffectStatus::Recorded(intent, _, _)) = effect
                else {
                    return Err(Diagnostic::new(
                        DiagnosticCode::IkEffectStale,
                        Some("effect.none"),
                        "no effect to resolve",
                    ));
                };
                Ok(NativeDecision::emit(NativeOutcomeEvent::Resolve {
                    effect_id: intent.effect_id.0.clone(),
                    planned_generation: intent.generation,
                    payload_digest: intent.payload_digest.clone(),
                    idempotency_key: intent.idempotency_key.clone(),
                }))
            }
            _ => Err(Diagnostic::new(
                DiagnosticCode::IkCommandInvalid,
                Some("command.unknown"),
                "unknown",
            )),
        }
    }

    fn apply(
        &self,
        state: &Self::State,
        event: &Self::Event,
    ) -> Result<NativeApply<Self::State>, Self::Diagnostic> {
        match event {
            NativeOutcomeEvent::Plan => {
                let intent = EffectIntentV1 {
                    effect_id: EffectId("dom-effect".to_string()),
                    aggregate_id: AggregateId(String::new()),
                    generation: 0,
                    effect_kind: "DOM".to_string(),
                    payload_digest: "dom-payload".to_string(),
                    idempotency_key: "dom-key".to_string(),
                    payload: None,
                };
                Ok(NativeApply::new(state.clone()).with_effect(intent))
            }
            NativeOutcomeEvent::Resolve {
                effect_id,
                planned_generation,
                payload_digest,
                idempotency_key,
            } => {
                let outcome = EffectOutcomeV1 {
                    effect_id: EffectId(effect_id.clone()),
                    aggregate_id: AggregateId(String::new()),
                    planned_generation: *planned_generation,
                    payload_digest: payload_digest.clone(),
                    idempotency_key: idempotency_key.clone(),
                    outcome_digest: "dom-outcome".to_string(),
                };
                Ok(NativeApply::new(state.clone()).with_outcome(outcome))
            }
        }
    }
}

/// Build a genesis + plan + resolve journal for the native outcome machine and drop
/// the store, returning the store path and aggregate id for authenticated corruption.
fn native_journal(dir: &tempfile::TempDir) -> (std::path::PathBuf, AggregateId) {
    let path = dir.path().join("n.db");
    let machine: Box<dyn ik::machine::Machine> = Box::new(ik::machine::NativeAdapter::new(
        crate::NativeOutcomeMachine::new(),
    ));
    let identity = machine.identity().clone();
    let mut store = Store::create(&path).expect("create store");
    let agg = AggregateId("dom".into());
    store
        .create_aggregate(machine.as_ref(), &agg, &identity)
        .expect("create aggregate");
    let cap0 = cap(&identity, &agg, 0);
    let planned = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap0,
                command: json!({"name": "plan"}),
                observation: None,
            },
        )
        .expect("plan via native machine");
    store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: planned.capability,
                command: json!({"name": "resolve"}),
                observation: None,
            },
        )
        .expect("resolve via native machine");
    drop(store);
    (path, agg)
}

/// Rewrite a journal event at `sequence` in place (authenticated semantic
/// corruption), recomputing the hash chain from that event through the tail and
/// aligning the projection chain head. The mutated event must stay structurally
/// valid so only the replay fold — never the hash-chain check — can refuse it.
fn rewrite_journal_event(
    path: &std::path::Path,
    aggregate_id: &AggregateId,
    sequence: u64,
    mutate: impl FnOnce(&mut ik::runtime::JournalEvent),
) {
    let conn = rusqlite::Connection::open(path).expect("open store for mutation");
    let tail: u64 = conn
        .query_row(
            "SELECT MAX(sequence) FROM journal WHERE aggregate_id = ?1",
            [aggregate_id.0.as_str()],
            |row| row.get(0),
        )
        .expect("load journal tail");
    assert!(
        sequence <= tail,
        "corruption sequence {sequence} is beyond the journal tail {tail}"
    );
    let mut stmt = conn
        .prepare(
            "SELECT event_json FROM journal WHERE aggregate_id = ?1 AND sequence >= ?2 ORDER BY sequence",
        )
        .expect("prepare journal read");
    let encoded: Vec<String> = stmt
        .query_map(
            rusqlite::params![aggregate_id.0.as_str(), sequence as i64],
            |row| row.get(0),
        )
        .expect("read journal events")
        .collect::<Result<_, _>>()
        .expect("collect journal events");
    drop(stmt);
    assert_eq!(
        encoded.len() as u64,
        tail - sequence + 1,
        "journal read must cover the mutated event through the tail"
    );
    let mut events: Vec<ik::runtime::JournalEvent> = encoded
        .iter()
        .map(|encoded| serde_json::from_str(encoded).expect("decode journal event"))
        .collect();
    mutate(&mut events[0]);
    for index in 0..events.len() {
        if index > 0 {
            events[index].prev_hash = events[index - 1].hash.clone();
        }
        events[index].hash = ik::runtime::recompute_event_hash(&events[index]);
        let encoded = ik::canonical::canonical_json(&events[index]);
        conn.execute(
            "UPDATE journal SET hash = ?1, prev_hash = ?2, event_json = ?3 WHERE aggregate_id = ?4 AND sequence = ?5",
            rusqlite::params![
                events[index].hash,
                events[index].prev_hash,
                encoded,
                aggregate_id.0.as_str(),
                events[index].sequence as i64,
            ],
        )
        .expect("rewrite journal event");
    }
    conn.execute(
        "UPDATE aggregates SET last_event_hash = ?1 WHERE aggregate_id = ?2",
        rusqlite::params![
            events.last().expect("non-empty tail").hash,
            aggregate_id.0.as_str()
        ],
    )
    .expect("align projection chain head");
}

#[test]
fn authenticated_plan_intent_corruption_fails_replay() {
    // The persisted Plan delta's intent must equal the intent the event fold emits;
    // an otherwise-valid forged intent (same shape, different effect identity) must
    // refuse replay at the effect-delta match guard.
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, agg) = native_journal(&dir);
    rewrite_journal_event(&path, &agg, 1, |event| {
        let ik::runtime::EffectDelta::Plan { intent } = &mut event.effect_delta else {
            panic!("plan event must carry a Plan delta");
        };
        intent.effect_id = EffectId("forged-effect".into());
    });
    let mut store = Store::open(&path).expect("reopen");
    let machine: Box<dyn ik::machine::Machine> = Box::new(ik::machine::NativeAdapter::new(
        crate::NativeOutcomeMachine::new(),
    ));
    let err = store
        .replay(machine.as_ref(), Some(&agg))
        .expect_err("forged plan intent must refuse replay");
    assert_eq!(err.code, DiagnosticCode::IkReplayMismatch);
}

#[test]
fn authenticated_record_outcome_corruption_fails_replay() {
    // The persisted Record delta's outcome must equal the outcome the fold binds; a
    // forged outcome digest on an otherwise-valid binding must refuse replay.
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, agg) = native_journal(&dir);
    rewrite_journal_event(&path, &agg, 2, |event| {
        let ik::runtime::EffectDelta::Record { outcome } = &mut event.effect_delta else {
            panic!("resolve event must carry a Record delta");
        };
        outcome.outcome_digest = "forged-outcome".into();
    });
    let mut store = Store::open(&path).expect("reopen");
    let machine: Box<dyn ik::machine::Machine> = Box::new(ik::machine::NativeAdapter::new(
        crate::NativeOutcomeMachine::new(),
    ));
    let err = store
        .replay(machine.as_ref(), Some(&agg))
        .expect_err("forged outcome must refuse replay");
    assert_eq!(err.code, DiagnosticCode::IkReplayMismatch);
}

#[test]
fn authenticated_delta_variant_corruption_fails_replay() {
    // The persisted delta variant must agree with what the event fold emits: swapping
    // Plan/Record for None (or one another) while the fold still emits the other
    // shape must refuse replay on the same closed match.
    for (event_sequence, case) in [
        (1u64, "none"),
        (1u64, "record"),
        (2u64, "none"),
        (2u64, "plan"),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let (path, agg) = native_journal(&dir);
        rewrite_journal_event(&path, &agg, event_sequence, |event| match case {
            "none" => event.effect_delta = ik::runtime::EffectDelta::None,
            "record" => {
                event.effect_delta = ik::runtime::EffectDelta::Record {
                    outcome: EffectOutcomeV1 {
                        effect_id: EffectId("dom-effect".into()),
                        aggregate_id: event.aggregate_id.clone(),
                        planned_generation: 1,
                        payload_digest: "dom-payload".into(),
                        idempotency_key: "dom-key".into(),
                        outcome_digest: "dom-outcome".into(),
                    },
                }
            }
            "plan" => {
                event.effect_delta = ik::runtime::EffectDelta::Plan {
                    intent: EffectIntentV1 {
                        effect_id: EffectId("dom-effect".into()),
                        aggregate_id: event.aggregate_id.clone(),
                        generation: 1,
                        effect_kind: "DOM".into(),
                        payload_digest: "dom-payload".into(),
                        idempotency_key: "dom-key".into(),
                        payload: None,
                    },
                }
            }
            _ => unreachable!(),
        });
        let mut store = Store::open(&path).expect("reopen");
        let machine: Box<dyn ik::machine::Machine> = Box::new(ik::machine::NativeAdapter::new(
            crate::NativeOutcomeMachine::new(),
        ));
        let err = store
            .replay(machine.as_ref(), Some(&agg))
            .expect_err("variant-mismatched delta must refuse replay");
        assert_eq!(
            err.code,
            DiagnosticCode::IkReplayMismatch,
            "case {case} at sequence {event_sequence}"
        );
    }
}

#[test]
fn authenticated_domain_record_without_pending_fails_replay() {
    // G-005 defect guard: replay must refuse a domain Record delta when the
    // accumulated chain holds no pending intent. The live path refuses via the
    // binding classifier (IK_EFFECT_STALE); replay must not let apply_effect_delta
    // fabricate a binding from the outcome alone. The first domain event is rewritten
    // into a resolve (the fold emits an outcome and the Record delta passes its own
    // guard) with no prior plan in the chain.
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, agg) = native_journal(&dir);
    rewrite_journal_event(&path, &agg, 1, |event| {
        event.event_type = "resolve".into();
        event.command = json!({"name": "resolve"});
        event.payload = json!({
            "effect_id": "dom-effect",
            "planned_generation": 1,
            "payload_digest": "dom-payload",
            "idempotency_key": "dom-key",
        });
        event.effect_delta = ik::runtime::EffectDelta::Record {
            outcome: EffectOutcomeV1 {
                effect_id: EffectId("dom-effect".into()),
                aggregate_id: event.aggregate_id.clone(),
                planned_generation: 1,
                payload_digest: "dom-payload".into(),
                idempotency_key: "dom-key".into(),
                outcome_digest: "dom-outcome".into(),
            },
        };
    });
    let mut store = Store::open(&path).expect("reopen");
    let machine: Box<dyn ik::machine::Machine> = Box::new(ik::machine::NativeAdapter::new(
        crate::NativeOutcomeMachine::new(),
    ));
    let err = store
        .replay(machine.as_ref(), Some(&agg))
        .expect_err("record without a pending intent must refuse replay");
    assert_eq!(err.code, DiagnosticCode::IkReplayMismatch);
}

#[test]
fn authenticated_domain_plan_while_pending_fails_replay() {
    // G-005 defect guard: replay must refuse a second Plan delta while one is already
    // pending in the accumulated chain. The live path refuses via the domain reducer
    // (IK_EFFECT_STALE); replay must not let apply_effect_delta overwrite the first
    // intent. The resolve event is rewritten into a second plan.
    let dir = tempfile::tempdir().expect("tempdir");
    let (path, agg) = native_journal(&dir);
    rewrite_journal_event(&path, &agg, 2, |event| {
        event.event_type = "plan".into();
        event.command = json!({"name": "plan"});
        event.payload = json!({});
        event.effect_delta = ik::runtime::EffectDelta::Plan {
            intent: EffectIntentV1 {
                effect_id: EffectId("dom-effect".into()),
                aggregate_id: event.aggregate_id.clone(),
                generation: 1,
                effect_kind: "DOM".into(),
                payload_digest: "dom-payload".into(),
                idempotency_key: "dom-key".into(),
                payload: None,
            },
        };
    });
    let mut store = Store::open(&path).expect("reopen");
    let machine: Box<dyn ik::machine::Machine> = Box::new(ik::machine::NativeAdapter::new(
        crate::NativeOutcomeMachine::new(),
    ));
    let err = store
        .replay(machine.as_ref(), Some(&agg))
        .expect_err("a second plan while pending must refuse replay");
    assert_eq!(err.code, DiagnosticCode::IkReplayMismatch);
}

#[test]
fn consecutive_effect_cycles_replay_exactly() {
    // G-005 live/replay parity guard: the live path allows a NEW plan once the
    // prior effect is recorded (only a PENDING effect blocks planning, in
    // runtime::derive_effect_delta); replay must reproduce that decision for the
    // SECOND plan/record cycle. Regression for the live capstone defect where the
    // replay fold required EffectStatus::None before a Plan delta, so a journal
    // with two consecutive plan/record cycles committed live but refused replay
    // with IK_REPLAY_MISMATCH replay.effect_delta at the second plan.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("n.db");
    let machine: Box<dyn ik::machine::Machine> = Box::new(ik::machine::NativeAdapter::new(
        crate::NativeOutcomeMachine::new(),
    ));
    let identity = machine.identity().clone();
    let mut store = Store::create(&path).expect("create store");
    let agg = AggregateId("dom".into());
    store
        .create_aggregate(machine.as_ref(), &agg, &identity)
        .expect("create aggregate");

    // Capstone path: domain plan -> kernel record -> domain plan -> kernel record.
    // The first record binds the exact stamped pending identity
    // (dom-effect, planned_generation 1, dom-payload, dom-key); the second domain
    // plan is legal once the prior effect is RECORDED (only PENDING blocks
    // planning, G-005), and the second record binds generation 3.
    let cap0 = cap(&identity, &agg, 0);
    let planned = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap0,
                command: json!({"name": "plan"}),
                observation: None,
            },
        )
        .expect("first plan");
    let EffectStatus::Pending(intent) = &planned.effect else {
        panic!("expected pending after the first plan");
    };
    assert_eq!(intent.generation, 1);

    let resolved = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: planned.capability.clone(),
            effect: EffectOutcomeV1 {
                effect_id: EffectId("dom-effect".into()),
                aggregate_id: agg.clone(),
                planned_generation: 1,
                payload_digest: "dom-payload".into(),
                idempotency_key: "dom-key".into(),
                outcome_digest: "outcome1".into(),
            },
        })
        .expect("first kernel record");
    let EffectStatus::Recorded(intent, digest, _) = &resolved.effect else {
        panic!("expected recorded after the first record");
    };
    assert_eq!(intent.effect_id.0, "dom-effect");
    assert_eq!(intent.generation, 1);
    assert_eq!(digest, "outcome1");

    let replanned = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: resolved.capability.clone(),
                command: json!({"name": "plan"}),
                observation: None,
            },
        )
        .expect("second plan after a recorded effect");
    let EffectStatus::Pending(intent) = &replanned.effect else {
        panic!("expected pending after the second plan");
    };
    assert_eq!(intent.generation, 3);
    let rerecorded = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: replanned.capability.clone(),
            effect: EffectOutcomeV1 {
                effect_id: EffectId("dom-effect".into()),
                aggregate_id: agg.clone(),
                planned_generation: 3,
                payload_digest: "dom-payload".into(),
                idempotency_key: "dom-key".into(),
                outcome_digest: "outcome2".into(),
            },
        })
        .expect("second kernel record");
    assert_eq!(rerecorded.generation, 4);
    let EffectStatus::Recorded(intent, digest, _) = &rerecorded.effect else {
        panic!("expected recorded after the second record");
    };
    assert_eq!(intent.generation, 3);
    assert_eq!(digest, "outcome2");

    // Journal: genesis(0), plan(1), kernel_record(2), plan(3), kernel_record(4).
    store
        .verify(machine.as_ref(), Some(&agg))
        .expect("verify passes on the live journal");
    drop(store);

    // Reopen + replay must reproduce the projection exactly — including the second
    // planned effect — and verify must pass.
    let machine: Box<dyn ik::machine::Machine> = Box::new(ik::machine::NativeAdapter::new(
        crate::NativeOutcomeMachine::new(),
    ));
    let mut store = Store::open(&path).expect("reopen");
    let summary = store
        .replay(machine.as_ref(), Some(&agg))
        .expect("replay with two consecutive effect cycles");
    assert_eq!(summary.aggregates[0].sequence, 4);
    assert_eq!(summary.aggregates[0].generation, 4);
    assert!(
        !summary.aggregates[0].rebuilt,
        "replay must match the projection"
    );
    store
        .verify(machine.as_ref(), Some(&agg))
        .expect("verify passes after replay");
}
struct RecklessPlanner {
    identity: MachineIdentity,
}

impl RecklessPlanner {
    fn new() -> Self {
        Self {
            identity: ik::machine::native_identity("reckless_planner", 1, "reckless-planner-v1"),
        }
    }
}

impl Machine for RecklessPlanner {
    fn identity(&self) -> &MachineIdentity {
        &self.identity
    }

    fn initial_state(&self) -> Value {
        json!({"state": "idle"})
    }

    fn decide(
        &self,
        _state: &Value,
        _observation: Option<&Value>,
        _effect: &EffectStatus,
        _command: &Value,
    ) -> Result<Decision, Diagnostic> {
        Ok(Decision::emit(DomainEvent::new("plan", 1, json!({}))))
    }

    fn apply(&self, state: &Value, event: &DomainEvent) -> Result<ApplyOutcome, Diagnostic> {
        if event.event_type != "plan" || event.event_version != 1 {
            return Err(Diagnostic::new(
                DiagnosticCode::IkEventSchemaUnknown,
                Some("event.schema"),
                "unknown event",
            ));
        }
        Ok(
            ApplyOutcome::new(state.clone()).with_effect(EffectIntentV1 {
                effect_id: EffectId("reckless-effect".into()),
                aggregate_id: AggregateId(String::new()),
                generation: 0,
                effect_kind: "TEST".into(),
                payload_digest: "payload".into(),
                idempotency_key: "key".into(),
                payload: None,
            }),
        )
    }
}

#[test]
fn live_runtime_rejects_plan_while_pending_even_if_machine_emits_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("reckless.db");
    let machine = RecklessPlanner::new();
    let identity = machine.identity().clone();
    let agg = AggregateId("reckless".into());
    let mut store = Store::create(&path).expect("create store");
    store
        .create_aggregate(&machine, &agg, &identity)
        .expect("create aggregate");
    let first = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap(&identity, &agg, 0),
                command: json!({"name": "plan"}),
                observation: None,
            },
        )
        .expect("first plan");
    assert!(matches!(first.effect, EffectStatus::Pending(_)));
    let before = store.show(&agg).expect("show before second plan");
    let err = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity,
                capability: before.capability.clone(),
                command: json!({"name": "plan"}),
                observation: None,
            },
        )
        .expect_err("runtime must reject a second pending effect");
    assert_eq!(err.code, DiagnosticCode::IkEffectStale);
    let after = store.show(&agg).expect("show after second plan");
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.sequence, before.sequence);
    assert_eq!(after.effect, before.effect);
}
