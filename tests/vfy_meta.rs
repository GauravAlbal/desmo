//! VFY-06: generic metamorphic laws over authoritative kernel APIs.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::effect::{EffectOutcomeV1, EffectStatus};
use ik::identity::AggregateId;
use ik::machine::{Decision, Machine};
use ik::metamorphic::{
    check_duplicate_idempotence, check_refusal_immutability, check_replay_equivalence,
    deterministic_fold, durable_truth, frozen_event_stable, stale_never_current,
};
use ik::store::{CommandRequest, EffectOutcomeRequest, Store};
use serde_json::json;

#[test]
fn refusal_leaves_truth_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::create(&dir.path().join("s.db")).expect("store");
    let machine = load_machine("effect_probe");
    let identity = machine.identity().clone();
    let agg = AggregateId("meta-refuse".into());
    let created = store
        .create_aggregate(&machine, &agg, &identity)
        .expect("create");
    store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: created.capability.clone(),
                command: json!({"name":"plan","args":{
                    "effect_id":"e1","payload_digest":"p1","idempotency_key":"k1"
                }}),
                observation: None,
            },
        )
        .expect("plan");
    assert!(check_refusal_immutability(
        &mut store,
        &machine,
        &CommandRequest {
            aggregate_id: agg,
            machine: identity,
            capability: created.capability,
            command: json!({"name":"plan","args":{
                "effect_id":"e2","payload_digest":"p2","idempotency_key":"k2"
            }}),
            observation: None,
        },
    )
    .expect("check"));
}

#[test]
fn replay_matches_live_projection() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::create(&dir.path().join("s.db")).expect("store");
    let machine = load_machine("effect_probe");
    let identity = machine.identity().clone();
    let agg = AggregateId("meta-replay".into());
    let created = store
        .create_aggregate(&machine, &agg, &identity)
        .expect("create");
    let planned = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: created.capability,
                command: json!({"name":"plan","args":{
                    "effect_id":"e1","payload_digest":"p1","idempotency_key":"k1"
                }}),
                observation: None,
            },
        )
        .expect("plan");
    let EffectStatus::Pending(intent) = &planned.effect else {
        panic!("pending");
    };
    store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity,
            capability: planned.capability,
            effect: EffectOutcomeV1 {
                effect_id: intent.effect_id.clone(),
                aggregate_id: agg.clone(),
                planned_generation: intent.generation,
                payload_digest: intent.payload_digest.clone(),
                idempotency_key: intent.idempotency_key.clone(),
                outcome_digest: "out".into(),
            },
        })
        .expect("record");
    assert!(check_replay_equivalence(&mut store, &machine, &agg).expect("check"));
}

#[test]
fn duplicate_record_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::create(&dir.path().join("s.db")).expect("store");
    let machine = load_machine("effect_probe");
    let identity = machine.identity().clone();
    let agg = AggregateId("meta-dup".into());
    let created = store
        .create_aggregate(&machine, &agg, &identity)
        .expect("create");
    let planned = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: created.capability,
                command: json!({"name":"plan","args":{
                    "effect_id":"e1","payload_digest":"p1","idempotency_key":"k1"
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
            outcome_digest: "out".into(),
        },
    };
    store.record_effect(&req).expect("first record");
    assert!(check_duplicate_idempotence(&mut store, &req).expect("check"));
    let _ = durable_truth(&mut store, &agg).expect("truth");
}

#[test]
fn frozen_reject_event_ignores_live_guard() {
    let machine = load_machine("verification_attempt");
    let start = machine
        .decide(
            &machine.initial_state(),
            None,
            &EffectStatus::None,
            &json!({"name": "start"}),
        )
        .expect("start decide");
    let Decision::Emit {
        event: start_ev, ..
    } = start
    else {
        panic!("start emit");
    };
    let running = machine
        .apply(&machine.initial_state(), &start_ev)
        .expect("start apply")
        .state;
    let submit = json!({"name":"submit","args":{"artifact_digest":"a1","baseline_digest":"b1"}});
    assert!(deterministic_fold(&machine, &running, &EffectStatus::None, &submit).expect("fold"));
    let Decision::Emit {
        event: submit_ev, ..
    } = machine
        .decide(&running, None, &EffectStatus::None, &submit)
        .expect("submit")
    else {
        panic!("submit emit");
    };
    let submitted = machine
        .apply(&running, &submit_ev)
        .expect("submit apply")
        .state;
    let Decision::Emit {
        event: reject_ev, ..
    } = machine
        .decide(
            &submitted,
            None,
            &EffectStatus::None,
            &json!({"name":"reject"}),
        )
        .expect("reject")
    else {
        panic!("reject emit");
    };
    let mut flipped = submitted.clone();
    flipped["attempt_count"] = json!(2);
    assert!(
        frozen_event_stable(&machine, &submitted, &flipped, &reject_ev).expect("frozen"),
        "frozen reject must not re-select under flipped facts"
    );
    let applied = machine.apply(&flipped, &reject_ev).expect("apply frozen");
    assert_eq!(applied.state["state"], "repairable");
}

#[test]
fn stale_generation_is_monotone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::create(&dir.path().join("s.db")).expect("store");
    let machine = load_machine("effect_probe");
    let identity = machine.identity().clone();
    let agg = AggregateId("meta-mono".into());
    let created = store
        .create_aggregate(&machine, &agg, &identity)
        .expect("create");
    let old = created.capability.clone();
    store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity,
                capability: created.capability,
                command: json!({"name":"plan","args":{
                    "effect_id":"e1","payload_digest":"p1","idempotency_key":"k1"
                }}),
                observation: None,
            },
        )
        .expect("plan");
    let current = store.show(&agg).expect("show").capability;
    assert!(stale_never_current(&old, &current));
}
