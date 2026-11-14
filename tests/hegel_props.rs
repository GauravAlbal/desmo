//! VFY-09: matched Hegel lanes for existing Proptest semantic properties.
//!
//! Proptest remains until crash/corruption/linearizability coverage is shown
//! not to depend on it. Disposition: HEGEL_ADOPT_RETAIN_PROPTEST_TEMPORARILY.

#[path = "common/mod.rs"]
mod common;

use common::*;
use hegel::generators as gs;
use hegel::TestCase;
use ik::diagnostic::DiagnosticCode;
use ik::effect::{EffectOutcomeV1, EffectStatus};
use ik::identity::{AggregateId, Capability};
use ik::machine::Machine;
use ik::metamorphic::{check_duplicate_idempotence, check_replay_equivalence, stale_never_current};
use ik::runtime::derive_nonce;
use ik::store::{CommandRequest, EffectOutcomeRequest, Store};
use serde_json::json;

fn cap(machine: &ik::identity::MachineIdentity, agg: &AggregateId, generation: u64) -> Capability {
    Capability {
        generation,
        nonce: derive_nonce(machine, agg, generation),
    }
}

#[hegel::test(test_cases = 24)]
fn hegel_effect_and_replay_laws(tc: TestCase) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::create(&dir.path().join("s.db")).expect("store");
    let machine = load_machine("effect_probe");
    let identity = machine.identity().clone();
    let agg = AggregateId(tc.draw(gs::text().min_size(1).max_size(16)));
    let created = store
        .create_aggregate(&machine, &agg, &identity)
        .expect("create");
    assert_eq!(created.capability, cap(&identity, &agg, 0));
    let planned = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: created.capability,
                command: json!({"name":"plan","args":{
                    "effect_id": tc.draw(gs::text().min_size(1).max_size(12)),
                    "payload_digest": tc.draw(gs::text().min_size(1).max_size(12)),
                    "idempotency_key": tc.draw(gs::text().min_size(1).max_size(12))
                }}),
                observation: None,
            },
        )
        .expect("plan");
    let EffectStatus::Pending(intent) = &planned.effect else {
        panic!("pending");
    };
    let req = EffectOutcomeRequest {
        aggregate_id: agg.clone(),
        machine: identity,
        capability: planned.capability,
        effect: EffectOutcomeV1 {
            effect_id: intent.effect_id.clone(),
            aggregate_id: agg.clone(),
            planned_generation: intent.generation,
            payload_digest: intent.payload_digest.clone(),
            idempotency_key: intent.idempotency_key.clone(),
            outcome_digest: tc.draw(gs::text().min_size(1).max_size(12)),
        },
    };
    store.record_effect(&req).expect("record");
    assert!(check_duplicate_idempotence(&mut store, &req).expect("dup"));
    assert!(check_replay_equivalence(&mut store, &machine, &agg).expect("replay"));
}

#[hegel::test(test_cases = 24)]
fn hegel_stale_capability_never_mutates(tc: TestCase) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::create(&dir.path().join("s.db")).expect("store");
    let machine = load_machine("effect_probe");
    let identity = machine.identity().clone();
    let agg = AggregateId(tc.draw(gs::text().min_size(1).max_size(16)));
    let created = store
        .create_aggregate(&machine, &agg, &identity)
        .expect("create");
    let old = created.capability.clone();
    store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: created.capability,
                command: json!({"name":"plan","args":{
                    "effect_id":"e","payload_digest":"p","idempotency_key":"k"
                }}),
                observation: None,
            },
        )
        .expect("plan");
    let current = store.show(&agg).expect("show").capability;
    assert!(stale_never_current(&old, &current));
    let err = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg,
                machine: identity,
                capability: old,
                command: json!({"name":"plan","args":{
                    "effect_id":"e2","payload_digest":"p2","idempotency_key":"k2"
                }}),
                observation: None,
            },
        )
        .expect_err("stale");
    assert_eq!(err.code, DiagnosticCode::IkCapabilityStale);
}
