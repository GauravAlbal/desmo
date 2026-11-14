//! Seeded property tests for the durable K1/K2.5 lifecycle.
//!
//! Proptest reports its replay seed and minimal failing case. These are generative
//! tests, not exhaustive proofs. Every case uses the real SQLite store and checks the
//! projection/journal/receipt seam after each operation.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::diagnostic::DiagnosticCode;
use ik::effect::{EffectOutcomeV1, EffectStatus};
use ik::identity::{AggregateId, Capability};
use ik::machine::Machine;
use ik::store::{CommandRequest, EffectOutcomeRequest, Store};
use proptest::prelude::*;
use serde_json::json;

fn capability(
    machine: &ik::identity::MachineIdentity,
    aggregate: &AggregateId,
    generation: u64,
) -> Capability {
    Capability {
        generation,
        nonce: ik::runtime::derive_nonce(machine, aggregate, generation),
    }
}

fn token() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_-]{1,24}"
}

/// Monotone counter strategy: arbitrary u64 with deterministic weight on the
/// ceiling, near-ceiling, and zero so every run exercises the boundary.
fn counter() -> impl Strategy<Value = u64> {
    prop_oneof![
        1 => Just(u64::MAX),
        1 => Just(u64::MAX - 1),
        1 => Just(0),
        7 => 0u64..=u64::MAX,
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        max_shrink_iters: 2_048,
        .. ProptestConfig::default()
    })]

    #[test]
    fn arbitrary_identities_preserve_effect_and_replay_laws(
        aggregate_raw in token(),
        effect_raw in token(),
        payload in token(),
        idempotency in token(),
        digest_pair in (token(), token()).prop_filter(
            "recorded and conflicting outcome digests must differ",
            |(recorded, conflicting)| recorded != conflicting,
        ),
    ) {
        let (outcome_digest, conflicting_digest) = digest_pair;
        let dir = tempfile::tempdir().expect("tempdir");
        let machine = load_machine("effect_probe");
        let identity = machine.identity().clone();
        let aggregate = AggregateId(aggregate_raw);
        let mut store = Store::create(&dir.path().join("store.db")).expect("create store");
        let created = store.create_aggregate(&machine, &aggregate, &identity).expect("create aggregate");
        prop_assert_eq!(created.generation, 0);
        prop_assert_eq!(&created.capability, &capability(&identity, &aggregate, 0));

        let planned = store.command(&machine, &CommandRequest {
            aggregate_id: aggregate.clone(), machine: identity.clone(), capability: created.capability,
            command: json!({"name":"plan","args":{"effect_id":effect_raw,"payload_digest":payload,"idempotency_key":idempotency}}),
            observation: None,
        }).expect("plan effect");
        prop_assert_eq!(planned.generation, 1);
        let EffectStatus::Pending(intent) = &planned.effect else {
            return Err(TestCaseError::fail("plan did not persist a pending effect"));
        };
        prop_assert_eq!(&intent.aggregate_id, &aggregate);
        prop_assert_eq!(intent.generation, 1);

        let binding = EffectOutcomeV1 {
            effect_id: intent.effect_id.clone(), aggregate_id: aggregate.clone(),
            planned_generation: intent.generation, payload_digest: intent.payload_digest.clone(),
            idempotency_key: intent.idempotency_key.clone(), outcome_digest: outcome_digest.clone(),
        };
        let recorded = store.record_effect(&EffectOutcomeRequest {
            aggregate_id: aggregate.clone(), machine: identity.clone(),
            capability: planned.capability.clone(), effect: binding.clone(),
        }).expect("record effect");
        prop_assert_eq!(recorded.generation, 2);
        let recorded_receipt = recorded.receipt.clone().expect("record receipt");

        let duplicate = store.record_effect(&EffectOutcomeRequest {
            aggregate_id: aggregate.clone(), machine: identity.clone(),
            capability: planned.capability.clone(), effect: binding.clone(),
        }).expect("exact duplicate replays");
        prop_assert!(!duplicate.mutation);
        prop_assert_eq!(duplicate.generation, 2);
        prop_assert_eq!(duplicate.receipt.expect("replay receipt").receipt_id, recorded_receipt.receipt_id);

        let before = store.show(&aggregate).expect("show before conflict");
        let mut conflict = binding;
        conflict.outcome_digest = conflicting_digest;
        let diagnostic = store.record_effect(&EffectOutcomeRequest {
            aggregate_id: aggregate.clone(), machine: identity.clone(),
            capability: before.capability.clone(), effect: conflict,
        }).expect_err("different outcome refuses");
        prop_assert_eq!(diagnostic.code, DiagnosticCode::IkEffectOutcomeConflict);
        let after = store.show(&aggregate).expect("show after conflict");
        prop_assert_eq!(after.generation, before.generation);
        prop_assert_eq!(after.sequence, before.sequence);
        prop_assert_eq!(after.effect, before.effect);
        prop_assert!(store.verify(&machine, Some(&aggregate)).expect("verify").verified);
        let replay = store.replay(&machine, Some(&aggregate)).expect("replay");
        prop_assert_eq!(replay.aggregates[0].generation, after.generation);
        prop_assert_eq!(&replay.aggregates[0].state, &after.state);
    }

    #[test]
    fn stale_capability_never_mutates_arbitrary_aggregate(
        aggregate_raw in token(), artifact in token(), baseline in token(),
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let machine = load_machine("verification_attempt");
        let identity = machine.identity().clone();
        let aggregate = AggregateId(aggregate_raw);
        let mut store = Store::create(&dir.path().join("store.db")).expect("create store");
        let created = store.create_aggregate(&machine, &aggregate, &identity).expect("create");
        let stale = created.capability.clone();
        let started = store.command(&machine, &CommandRequest {
            aggregate_id: aggregate.clone(), machine: identity.clone(), capability: created.capability,
            command: json!({"name":"start"}), observation: None,
        }).expect("start");
        prop_assert_eq!(started.generation, 1);
        let before = store.show(&aggregate).expect("show before stale command");
        let diagnostic = store.command(&machine, &CommandRequest {
            aggregate_id: aggregate.clone(), machine: identity.clone(), capability: stale,
            command: json!({"name":"submit","args":{"artifact_digest":artifact,"baseline_digest":baseline}}),
            observation: None,
        }).expect_err("stale capability refuses");
        prop_assert_eq!(diagnostic.code, DiagnosticCode::IkCapabilityStale);
        let after = store.show(&aggregate).expect("show after stale command");
        prop_assert_eq!(after.generation, before.generation);
        prop_assert_eq!(after.sequence, before.sequence);
        prop_assert_eq!(after.state, before.state);
        prop_assert_eq!(after.effect, before.effect);
        prop_assert!(store.verify(&machine, Some(&aggregate)).expect("verify").verified);
    }

    #[test]
    fn counter_ceiling_is_deterministic_refusal_across_arbitrary_state(
        generation in counter(),
        sequence in counter(),
    ) {
        // Genuine model of the monotone counter boundary: every (generation,
        // sequence) pair below the ceiling commits with exactly +1 on both
        // counters and on the runtime-stamped effect intent; every pair touching
        // the ceiling refuses deterministically with IK_COUNTER_OVERFLOW naming
        // the first failing counter. Never a debug panic or a release wrap.
        let machine = load_machine("effect_probe");
        let identity = machine.identity().clone();
        let aggregate = AggregateId("ceiling-probe".into());
        let nonce = ik::runtime::derive_nonce(&identity, &aggregate, generation);
        let snapshot = ik::runtime::AggregateSnapshot {
            aggregate_id: aggregate.clone(),
            machine: identity.clone(),
            state: machine.initial_state(),
            terminal: false,
            generation,
            nonce: nonce.clone(),
            effect: EffectStatus::None,
            sequence,
            last_event_hash: ik::runtime::GENESIS_PREV.to_string(),
            last_receipt_id: String::new(),
        };
        let capability = Capability { generation, nonce };
        let plan_command = json!({
            "name": "plan",
            "args": {"effect_id": "e1", "payload_digest": "p", "idempotency_key": "k"}
        });
        let result = ik::runtime::execute_command(
            &machine,
            &snapshot,
            None,
            &capability,
            &plan_command,
            None,
        );
        match (generation, sequence) {
            (g, s) if g < u64::MAX && s < u64::MAX => {
                let step = result.expect("below-ceiling transition must commit");
                let ik::runtime::Step::Commit(plan) = step else {
                    return Err(TestCaseError::fail("below-ceiling plan must commit"));
                };
                prop_assert_eq!(plan.event.generation, g + 1, "generation advances exactly once");
                prop_assert_eq!(plan.event.sequence, s + 1, "sequence advances exactly once");
                prop_assert_eq!(plan.event.prev_hash, snapshot.last_event_hash);
                prop_assert_eq!(plan.receipt.generation, g + 1, "receipt mirrors the event");
                prop_assert_eq!(plan.receipt.sequence, s + 1, "receipt mirrors the event");
                let ik::runtime::EffectDelta::Plan { intent } = &plan.event.effect_delta else {
                    return Err(TestCaseError::fail("plan delta expected"));
                };
                prop_assert_eq!(intent.generation, g + 1, "stamped intent generation");
                prop_assert_eq!(&intent.aggregate_id, &aggregate);
            }
            (u64::MAX, _) => {
                let diagnostic = result.expect_err("generation at the ceiling must refuse");
                prop_assert_eq!(diagnostic.code, DiagnosticCode::IkCounterOverflow);
                prop_assert_eq!(diagnostic.rule.as_deref(), Some("generation"));
            }
            (_, u64::MAX) => {
                let diagnostic = result.expect_err("sequence at the ceiling must refuse");
                prop_assert_eq!(diagnostic.code, DiagnosticCode::IkCounterOverflow);
                prop_assert_eq!(diagnostic.rule.as_deref(), Some("sequence"));
            }
            (g, _) => {
                // Unreachable: the guard arm covers every pair below the ceiling,
                // so any survivor has at least one counter at u64::MAX. Kept as a
                // fail arm so the match stays exhaustive without weakening the
                // assertions above.
                let _ = g;
                return Err(TestCaseError::fail(
                    "unreachable counter pair reached the ceiling arm",
                ));
            }
        }
    }
}
