//! VFY-05: Hegel stateful model over authoritative kernel Store APIs.

#[path = "common/mod.rs"]
mod common;

use common::*;
use hegel::generators as gs;
use hegel::TestCase;
use ik::diagnostic::DiagnosticCode;
use ik::effect::{EffectOutcomeV1, EffectStatus};
use ik::identity::{AggregateId, Capability};
use ik::machine::Machine;
use ik::runtime::derive_nonce;
use ik::store::{CommandRequest, EffectOutcomeRequest, Store};
use serde_json::json;
use std::cell::RefCell;

fn cap(machine: &ik::identity::MachineIdentity, agg: &AggregateId, generation: u64) -> Capability {
    Capability {
        generation,
        nonce: derive_nonce(machine, agg, generation),
    }
}

struct KernelVerificationModel {
    _dir: tempfile::TempDir,
    store: RefCell<Store>,
    machine: ik::declarative::CompiledMachine,
    aggregates: hegel::stateful::Pool<AggregateId>,
    stale_caps: hegel::stateful::Pool<(AggregateId, Capability)>,
}

impl KernelVerificationModel {
    fn boot(tc: &TestCase) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::create(&dir.path().join("s.db")).expect("store");
        Self {
            store: RefCell::new(store),
            machine: load_machine("effect_probe"),
            aggregates: hegel::stateful::pool(tc),
            stale_caps: hegel::stateful::pool(tc),
            _dir: dir,
        }
    }

    fn identity(&self) -> &ik::identity::MachineIdentity {
        self.machine.identity()
    }
}

#[hegel::state_machine]
impl KernelVerificationModel {
    #[rule]
    fn create_aggregate(&mut self, tc: TestCase) {
        let raw = tc.draw(gs::text().min_size(1).max_size(16));
        let agg = AggregateId(raw);
        let created =
            self.store
                .borrow_mut()
                .create_aggregate(&self.machine, &agg, self.identity());
        tc.assume(created.is_ok());
        let created = created.expect("created");
        self.stale_caps
            .add((agg.clone(), cap(self.identity(), &agg, 0)));
        self.aggregates.add(agg);
        let _ = created.capability;
    }

    #[rule]
    fn plan_effect(&mut self, tc: TestCase) {
        tc.assume(!self.aggregates.is_empty());
        let agg = tc.draw(self.aggregates.values_reusable()).clone();
        let shown = self.store.borrow_mut().show(&agg).expect("show");
        tc.assume(!shown.terminal);
        tc.assume(matches!(shown.effect, EffectStatus::None));
        let eid = tc.draw(gs::text().min_size(1).max_size(12));
        let payload = tc.draw(gs::text().min_size(1).max_size(12));
        let key = tc.draw(gs::text().min_size(1).max_size(12));
        let req = CommandRequest {
            aggregate_id: agg.clone(),
            machine: self.identity().clone(),
            capability: shown.capability.clone(),
            command: json!({"name":"plan","args":{
                "effect_id": eid,
                "payload_digest": payload,
                "idempotency_key": key
            }}),
            observation: None,
        };
        let result = self.store.borrow_mut().command(&self.machine, &req);
        tc.assume(result.is_ok());
        self.stale_caps.add((agg, shown.capability));
    }

    #[rule]
    fn reuse_stale_capability(&mut self, tc: TestCase) {
        tc.assume(!self.stale_caps.is_empty());
        let (agg, old) = tc.draw(self.stale_caps.values_reusable()).clone();
        let shown = self.store.borrow_mut().show(&agg).expect("show");
        tc.assume(shown.capability != old);
        let err = self
            .store
            .borrow_mut()
            .command(
                &self.machine,
                &CommandRequest {
                    aggregate_id: agg,
                    machine: self.identity().clone(),
                    capability: old,
                    command: json!({"name":"plan","args":{
                        "effect_id":"x","payload_digest":"p","idempotency_key":"k"
                    }}),
                    observation: None,
                },
            )
            .expect_err("stale must refuse");
        assert_eq!(err.code, DiagnosticCode::IkCapabilityStale);
    }

    #[rule]
    fn record_matching(&mut self, tc: TestCase) {
        tc.assume(!self.aggregates.is_empty());
        let agg = tc.draw(self.aggregates.values_reusable()).clone();
        let shown = self.store.borrow_mut().show(&agg).expect("show");
        let EffectStatus::Pending(intent) = shown.effect else {
            tc.reject();
        };
        let digest = tc.draw(gs::text().min_size(1).max_size(12));
        let outcome = EffectOutcomeV1 {
            effect_id: intent.effect_id.clone(),
            aggregate_id: agg.clone(),
            planned_generation: intent.generation,
            payload_digest: intent.payload_digest.clone(),
            idempotency_key: intent.idempotency_key.clone(),
            outcome_digest: digest,
        };
        let result = self
            .store
            .borrow_mut()
            .record_effect(&EffectOutcomeRequest {
                aggregate_id: agg.clone(),
                machine: self.identity().clone(),
                capability: shown.capability.clone(),
                effect: outcome,
            });
        tc.assume(result.is_ok());
        self.stale_caps.add((agg, shown.capability));
    }

    #[rule]
    fn show_verify_replay(&mut self, tc: TestCase) {
        tc.assume(!self.aggregates.is_empty());
        let agg = tc.draw(self.aggregates.values_reusable()).clone();
        let _ = self.store.borrow_mut().show(&agg).expect("show");
        let verified = self
            .store
            .borrow_mut()
            .verify(&self.machine, Some(&agg))
            .expect("verify");
        assert!(verified.verified);
        self.store
            .borrow_mut()
            .replay(&self.machine, Some(&agg))
            .expect("replay");
    }

    #[invariant]
    fn store_verify_holds(&self, _: TestCase) {
        let verified = self
            .store
            .borrow_mut()
            .verify(&self.machine, None)
            .expect("verify");
        assert!(verified.verified);
    }
}

#[hegel::test(test_cases = 16)]
fn hegel_kernel_model_runs(tc: TestCase) {
    hegel::stateful::run(KernelVerificationModel::boot(&tc), tc);
}

#[test]
fn stale_capability_sequence_refuses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::create(&dir.path().join("s.db")).expect("store");
    let machine = load_machine("effect_probe");
    let identity = machine.identity().clone();
    let agg = AggregateId("agg-stale".into());
    let created = store
        .create_aggregate(&machine, &agg, &identity)
        .expect("create");
    let g0 = created.capability.clone();
    store
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
    let err = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg,
                machine: identity,
                capability: g0,
                command: json!({"name":"plan","args":{
                    "effect_id":"e2","payload_digest":"p2","idempotency_key":"k2"
                }}),
                observation: None,
            },
        )
        .expect_err("stale");
    assert_eq!(err.code, DiagnosticCode::IkCapabilityStale);
}

#[test]
fn scripted_effect_cycle_verifies_and_replays() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::create(&dir.path().join("s.db")).expect("store");
    let machine = load_machine("effect_probe");
    let identity = machine.identity().clone();
    let agg = AggregateId("agg-cycle".into());
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
        panic!("expected pending");
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
    let verified = store.verify(&machine, Some(&agg)).expect("verify");
    assert!(verified.verified);
    store.replay(&machine, Some(&agg)).expect("replay");
    let shown = store.show(&agg).expect("show");
    assert_eq!(shown.state["state"], "planned");
}
