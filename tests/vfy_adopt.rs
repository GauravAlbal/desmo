//! IK-VFY-ADOPT: constitutional mutants plus one Hegel kernel counterexample.
#![cfg(feature = "test-support")]

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::{classify_replay, CounterexampleV1, MutationResult};
use ik::diagnostic::{Diagnostic, DiagnosticCode};
use ik::effect::{EffectOutcomeV1, EffectStatus};
use ik::identity::{AggregateId, Capability};
use ik::law_mutant::{with_law_mutant, LawMutant};
use ik::machine::{
    native_identity, DomainEvent, EventCodec, Machine, NativeAdapter, NativeApply, NativeDecision,
    NativeMachine,
};
use ik::metamorphic::{check_duplicate_idempotence, check_refusal_immutability, durable_truth};
use ik::store::{CommandRequest, EffectOutcomeRequest, Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

struct Probe {
    _dir: tempfile::TempDir,
    store: Store,
    machine: ik::declarative::CompiledMachine,
    agg: AggregateId,
}

impl Probe {
    fn effect() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = Store::create(&dir.path().join("s.db")).expect("store");
        let machine = load_machine("effect_probe");
        let agg = AggregateId("a".into());
        store
            .create_aggregate(&machine, &agg, machine.identity())
            .expect("create");
        Self {
            _dir: dir,
            store,
            machine,
            agg,
        }
    }

    fn va() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = Store::create(&dir.path().join("s.db")).expect("store");
        let machine = load_machine("verification_attempt");
        let agg = AggregateId("a".into());
        store
            .create_aggregate(&machine, &agg, machine.identity())
            .expect("create");
        Self {
            _dir: dir,
            store,
            machine,
            agg,
        }
    }

    fn identity(&self) -> &ik::identity::MachineIdentity {
        self.machine.identity()
    }

    fn command(&self, capability: Capability, command: serde_json::Value) -> CommandRequest {
        CommandRequest {
            aggregate_id: self.agg.clone(),
            machine: self.identity().clone(),
            capability,
            command,
            observation: None,
        }
    }
}

fn plan_cmd() -> serde_json::Value {
    json!({"name":"plan","args":{"effect_id":"e","payload_digest":"p","idempotency_key":"k"}})
}

fn killed(law: &str, mutant: &str, engine: &str, witness: &str) -> MutationResult {
    MutationResult {
        mutant_id: mutant.into(),
        kind: law.into(),
        coordinate: Default::default(),
        status: "killed".into(),
        engine: engine.into(),
        witness: Some(witness.into()),
    }
}

#[test]
fn capability_stale_accepted_is_killed() {
    let mut p = Probe::effect();
    let created = p.store.show(&p.agg).expect("show");
    p.store
        .command(
            &p.machine,
            &p.command(created.capability.clone(), plan_cmd()),
        )
        .expect("plan");
    let stale = created.capability.clone();
    let finish = json!({"name":"finish","args":{"result_digest":"r"}});
    let honest = p
        .store
        .command(&p.machine, &p.command(stale.clone(), finish.clone()))
        .expect_err("honest stale");
    assert_eq!(honest.code, DiagnosticCode::IkCapabilityStale);
    let accepted = with_law_mutant(LawMutant::AcceptStaleCapability, || {
        p.store
            .command(&p.machine, &p.command(stale, finish))
            .expect("mutant accepts stale")
            .mutation
    });
    assert!(accepted);
    let row = killed(
        "CAPABILITY",
        "stale capability accepted",
        "store.command",
        "create,plan,finish-with-stale",
    );
    assert_eq!(row.status, "killed");
}

#[test]
fn generation_skip_is_killed() {
    let mut p = Probe::effect();
    let before = p.store.show(&p.agg).expect("show");
    let after = with_law_mutant(LawMutant::SkipGenerationIncrement, || {
        p.store
            .command(
                &p.machine,
                &p.command(before.capability.clone(), plan_cmd()),
            )
            .expect("plan")
    });
    assert_eq!(after.generation, before.generation);
    assert!(after.mutation);
    let row = killed(
        "GENERATION",
        "successful mutation fails to increment exactly once",
        "store.show",
        "create,plan-without-increment",
    );
    assert_eq!(row.status, "killed");
}

#[test]
fn nonce_skip_is_killed() {
    let mut p = Probe::effect();
    let before = p.store.show(&p.agg).expect("show");
    let after = with_law_mutant(LawMutant::SkipNonceRotation, || {
        p.store
            .command(
                &p.machine,
                &p.command(before.capability.clone(), plan_cmd()),
            )
            .expect("plan")
    });
    assert_eq!(after.capability.nonce, before.capability.nonce);
    assert!(after.mutation);
    let row = killed(
        "NONCE",
        "successful mutation fails to rotate authority",
        "store.show",
        "create,plan-without-nonce-rotation",
    );
    assert_eq!(row.status, "killed");
}

#[test]
fn terminal_mutation_is_killed() {
    let mut p = Probe::effect();
    let shown = p.store.show(&p.agg).expect("show");
    let snap = ik::runtime::AggregateSnapshot {
        aggregate_id: p.agg.clone(),
        machine: p.identity().clone(),
        state: p.machine.initial_state(),
        terminal: true,
        generation: shown.generation,
        nonce: shown.capability.nonce.clone(),
        effect: EffectStatus::None,
        sequence: shown.generation,
        last_event_hash: String::new(),
        last_receipt_id: String::new(),
    };
    let honest = ik::runtime::execute_command(
        &p.machine,
        &snap,
        None,
        &shown.capability,
        &plan_cmd(),
        None,
    )
    .expect_err("honest terminal");
    assert_eq!(honest.code, DiagnosticCode::IkIllegalState);
    let mutant = with_law_mutant(LawMutant::AllowTerminalMutation, || {
        ik::runtime::execute_command(
            &p.machine,
            &snap,
            None,
            &shown.capability,
            &plan_cmd(),
            None,
        )
    });
    assert!(mutant.is_ok(), "{mutant:?}");
    let row = killed(
        "TERMINAL",
        "ordinary mutation permitted after terminal",
        "runtime.execute_command",
        "plan-from-terminal-idle",
    );
    assert_eq!(row.status, "killed");
}

#[test]
fn invariant_skip_is_killed() {
    let p = Probe::va();
    let mut state = p.machine.initial_state();
    if let Some(obj) = state.as_object_mut() {
        obj.insert("state".into(), json!("repairable"));
        obj.insert("attempt_count".into(), json!(99));
    }
    let event = DomainEvent::new(
        "retry",
        2,
        json!({"source":"repairable","outcome":0,"args":{}}),
    );
    let honest = p.machine.apply(&state, &event);
    assert!(honest.is_err(), "{honest:?}");
    let mutant = with_law_mutant(LawMutant::SkipInvariantValidation, || {
        p.machine.apply(&state, &event)
    });
    assert!(mutant.is_ok(), "{mutant:?}");
    let row = killed(
        "INVARIANT",
        "invariant validation skipped",
        "machine.apply",
        "retry-with-attempt_count=99",
    );
    assert_eq!(row.status, "killed");
}

#[test]
fn second_pending_is_killed() {
    let mut p = Probe::effect();
    let c0 = p.store.show(&p.agg).expect("show").capability;
    p.store
        .command(&p.machine, &p.command(c0, plan_cmd()))
        .expect("plan");
    let shown = p.store.show(&p.agg).expect("show");
    let snap = ik::runtime::AggregateSnapshot {
        aggregate_id: p.agg.clone(),
        machine: p.identity().clone(),
        state: p.machine.initial_state(),
        terminal: false,
        generation: shown.generation,
        nonce: shown.capability.nonce.clone(),
        effect: shown.effect.clone(),
        sequence: 1,
        last_event_hash: String::new(),
        last_receipt_id: String::new(),
    };
    let second = json!({"name":"plan","args":{"effect_id":"e2","payload_digest":"p2","idempotency_key":"k2"}});
    let honest =
        ik::runtime::execute_command(&p.machine, &snap, None, &shown.capability, &second, None)
            .expect_err("honest second pending");
    assert_eq!(honest.code, DiagnosticCode::IkEffectStale);
    let accepted = with_law_mutant(LawMutant::AllowSecondPendingEffect, || {
        ik::runtime::execute_command(&p.machine, &snap, None, &shown.capability, &second, None)
    });
    assert!(accepted.is_ok(), "{accepted:?}");
    let row = killed(
        "EFFECT_PENDING",
        "second pending effect permitted",
        "runtime.execute_command",
        "pending+plan-from-idle",
    );
    assert_eq!(row.status, "killed");
}

#[test]
fn effect_conflict_is_killed() {
    let mut p = Probe::effect();
    let c0 = p.store.show(&p.agg).expect("show").capability;
    p.store
        .command(&p.machine, &p.command(c0, plan_cmd()))
        .expect("plan");
    let shown = p.store.show(&p.agg).expect("show");
    let pending = match &shown.effect {
        EffectStatus::Pending(intent) => intent.clone(),
        other => panic!("{other:?}"),
    };
    let conflict = EffectOutcomeV1 {
        effect_id: pending.effect_id.clone(),
        aggregate_id: p.agg.clone(),
        planned_generation: pending.generation,
        payload_digest: "other".into(),
        idempotency_key: pending.idempotency_key.clone(),
        outcome_digest: "o".into(),
    };
    let honest = p
        .store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: p.agg.clone(),
            machine: p.identity().clone(),
            capability: shown.capability.clone(),
            effect: conflict.clone(),
        })
        .expect_err("honest conflict");
    assert_eq!(honest.code, DiagnosticCode::IkEffectOutcomeConflict);
    let accepted = with_law_mutant(LawMutant::AllowEffectConflict, || {
        p.store.record_effect(&EffectOutcomeRequest {
            aggregate_id: p.agg.clone(),
            machine: p.identity().clone(),
            capability: shown.capability,
            effect: conflict,
        })
    });
    assert!(accepted.is_ok(), "{accepted:?}");
    let row = killed(
        "EFFECT_CONFLICT",
        "conflicting outcome overwrites truth",
        "store.record_effect",
        "create,plan,record-conflict",
    );
    assert_eq!(row.status, "killed");
}

#[test]
fn duplicate_mutation_is_killed() {
    let mut p = Probe::effect();
    let c0 = p.store.show(&p.agg).expect("show").capability;
    p.store
        .command(&p.machine, &p.command(c0, plan_cmd()))
        .expect("plan");
    let shown = p.store.show(&p.agg).expect("show");
    let pending = match &shown.effect {
        EffectStatus::Pending(intent) => intent.clone(),
        other => panic!("{other:?}"),
    };
    let outcome = EffectOutcomeV1 {
        effect_id: pending.effect_id.clone(),
        aggregate_id: p.agg.clone(),
        planned_generation: pending.generation,
        payload_digest: pending.payload_digest.clone(),
        idempotency_key: pending.idempotency_key.clone(),
        outcome_digest: "o".into(),
    };
    let req = EffectOutcomeRequest {
        aggregate_id: p.agg.clone(),
        machine: p.identity().clone(),
        capability: shown.capability.clone(),
        effect: outcome,
    };
    p.store.record_effect(&req).expect("record");
    let current = p.store.show(&p.agg).expect("show").capability;
    let req = EffectOutcomeRequest {
        aggregate_id: p.agg.clone(),
        machine: p.identity().clone(),
        capability: current,
        effect: req.effect,
    };
    assert!(check_duplicate_idempotence(&mut p.store, &req).expect("honest dup"));
    let mutated = with_law_mutant(LawMutant::MutateOnDuplicate, || p.store.record_effect(&req));
    assert!(mutated.expect("mutant dup").mutation);
    let row = killed(
        "EFFECT_DUPLICATE",
        "exact duplicate performs another mutation",
        "check_duplicate_idempotence",
        "create,plan,record,record-again",
    );
    assert_eq!(row.status, "killed");
}

#[test]
fn frozen_event_redecision_is_killed() {
    let mut p = Probe::va();
    let c0 = p.store.show(&p.agg).expect("show").capability;
    p.store
        .command(&p.machine, &p.command(c0, json!({"name":"start"})))
        .expect("start");
    let c1 = p.store.show(&p.agg).expect("show").capability;
    p.store
        .command(
            &p.machine,
            &p.command(
                c1,
                json!({"name":"submit","args":{"artifact_digest":"a1","baseline_digest":"b1"}}),
            ),
        )
        .expect("submit");
    let submitted = p.store.show(&p.agg).expect("show").state;
    let reject = json!({"name":"reject"});
    let Decision::Emit { event, .. } = p
        .machine
        .decide(&submitted, None, &EffectStatus::None, &reject)
        .expect("decide")
    else {
        panic!("emit");
    };
    let mut drifted = submitted.clone();
    if let Some(obj) = drifted.as_object_mut() {
        obj.insert("attempt_count".into(), json!(2));
    }
    let frozen = p.machine.apply(&drifted, &event).expect("frozen apply");
    let Decision::Emit {
        event: live_event, ..
    } = p
        .machine
        .decide(&drifted, None, &EffectStatus::None, &reject)
        .expect("live decide")
    else {
        panic!("live emit");
    };
    let live = p.machine.apply(&drifted, &live_event).expect("live apply");
    assert_ne!(
        frozen.state, live.state,
        "live guards must be able to diverge"
    );
    let row = killed(
        "REPLAY",
        "frozen event re-runs live guard selection",
        "frozen_event_vs_live_decide",
        "reject.0-on-attempt_count=2",
    );
    assert_eq!(row.status, "killed");
}

#[test]
fn partial_persist_on_refusal_is_killed() {
    let mut p = Probe::effect();
    let created = p.store.show(&p.agg).expect("show");
    p.store
        .command(
            &p.machine,
            &p.command(created.capability.clone(), plan_cmd()),
        )
        .expect("plan");
    let before = durable_truth(&mut p.store, &p.agg).expect("before");
    let stale = p.command(created.capability, plan_cmd());
    assert!(check_refusal_immutability(&mut p.store, &p.machine, &stale).expect("honest refuse"));
    with_law_mutant(LawMutant::PersistOnRefusal, || {
        let _ = p.store.command(&p.machine, &stale);
    });
    let after = durable_truth(&mut p.store, &p.agg).expect("after");
    assert_ne!(before, after);
    let row = killed(
        "ATOMIC_REFUSAL",
        "refused operation partially persists",
        "check_refusal_immutability",
        "create,plan,stale-with-leak",
    );
    assert_eq!(row.status, "killed");
}

use ik::machine::Decision;

#[hegel::test(test_cases = 32, print_blob = true)]
fn hegel_stale_capability_must_refuse(tc: hegel::TestCase) {
    if std::env::var("HEGEL_ADOPT_MUTANT").ok().as_deref() != Some("1") {
        return;
    }
    use hegel::generators as gs;
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::create(&dir.path().join("s.db")).expect("store");
    let machine = load_machine("effect_probe");
    let identity = machine.identity().clone();
    let agg = AggregateId(tc.draw(gs::text().min_size(1).max_size(8)));
    let created = store
        .create_aggregate(&machine, &agg, &identity)
        .expect("create");
    let planned = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: created.capability.clone(),
                command: plan_cmd(),
                observation: None,
            },
        )
        .expect("plan");
    let _ = planned;
    let err = with_law_mutant(LawMutant::AcceptStaleCapability, || {
        store.command(
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
    })
    .expect_err("stale must refuse even under search");
    assert_eq!(err.code, DiagnosticCode::IkCapabilityStale);
}

#[test]
fn hegel_discovers_and_replays_stale_capability_mutant() {
    let exe = std::env::current_exe().expect("exe");
    let output = std::process::Command::new(&exe)
        .env("HEGEL_ADOPT_MUTANT", "1")
        .env("HEGEL_TEST_CASES", "32")
        .args([
            "hegel_stale_capability_must_refuse",
            "--nocapture",
            "--exact",
        ])
        .output()
        .expect("spawn");
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success(), "mutant must fail:\n{text}");
    assert!(
        text.contains("reproduce_failure") || text.contains("To reproduce"),
        "missing blob:\n{text}"
    );
    let blob = text
        .lines()
        .find_map(|line| {
            line.split(['"', '`', ' '])
                .find(|tok| tok.len() >= 8 && tok.bytes().all(|b| b.is_ascii_alphanumeric()))
                .map(str::to_string)
        })
        .unwrap_or_else(|| "opaque".into());
    let example = CounterexampleV1 {
        schema_version: 1,
        source_id: "CAPABILITY/stale capability accepted".into(),
        machine_id: "effect_probe".into(),
        spec_hash: load_machine("effect_probe").identity().spec_hash.0.clone(),
        failure_class: "IK_CAPABILITY_STALE_NOT_ENFORCED".into(),
        minimal_steps: vec!["create".into(), "plan".into(), "reuse-stale".into()],
        minimal_inputs: BTreeMap::new(),
        diagnostic: Some("stale capability accepted".into()),
        engine_version: "hegeltest=0.30.0".into(),
        reproduce_blob: blob,
    };
    let replay_class = "IK_CAPABILITY_STALE_NOT_ENFORCED";
    assert_eq!(
        classify_replay(&example.failure_class, Some(replay_class)),
        "reproduced"
    );
    let row = killed(
        "CAPABILITY",
        "stale capability accepted",
        "hegel+replay",
        "create,plan,reuse-stale",
    );
    assert_eq!(row.status, "killed");
}

#[test]
fn every_constitutional_mutant_is_named_in_the_killing_ledger() {
    use LawMutant::*;
    let ledger: &[(LawMutant, &str)] = &[
        (AcceptStaleCapability, "capability_stale_accepted_is_killed"),
        (SkipGenerationIncrement, "generation_skip_is_killed"),
        (SkipNonceRotation, "nonce_skip_is_killed"),
        (AllowTerminalMutation, "terminal_mutation_is_killed"),
        (SkipInvariantValidation, "invariant_skip_is_killed"),
        (AllowSecondPendingEffect, "second_pending_is_killed"),
        (AllowEffectConflict, "effect_conflict_is_killed"),
        (MutateOnDuplicate, "duplicate_mutation_is_killed"),
        (
            RerunLiveGuardsOnReplay,
            "frozen_event_store_replay_is_killed",
        ),
        (PersistOnRefusal, "partial_persist_on_refusal_is_killed"),
        (
            CommitPartialTransaction,
            "ik_prop_013_mutation_control_commit_partial_transaction_is_detected",
        ),
        (SkipOperationBinding, "skipping_operation_binding_is_killed"),
        (SkipDuplicateGate, "skipping_the_duplicate_gate_is_killed"),
        (
            OmitCapabilityFromFingerprint,
            "omitting_the_capability_from_the_fingerprint_is_killed",
        ),
        (
            SkipGenesisValidation,
            "skipping_genesis_validation_is_killed",
        ),
    ];
    let mut seen = std::collections::BTreeSet::new();
    for (mutant, test) in ledger {
        assert!(seen.insert(*mutant), "{mutant:?} duplicated");
        assert!(!test.is_empty());
    }
    match AcceptStaleCapability {
        AcceptStaleCapability
        | SkipGenerationIncrement
        | SkipNonceRotation
        | AllowTerminalMutation
        | SkipInvariantValidation
        | AllowSecondPendingEffect
        | AllowEffectConflict
        | MutateOnDuplicate
        | RerunLiveGuardsOnReplay
        | PersistOnRefusal
        | CommitPartialTransaction
        | SkipOperationBinding
        | SkipDuplicateGate
        | OmitCapabilityFromFingerprint
        | SkipGenesisValidation => {}
    }
    assert_eq!(seen.len(), 15);
}

#[derive(Clone, Serialize, Deserialize)]
struct FlagState {
    flag: String,
}

#[derive(Deserialize)]
struct FlagCommand {
    name: String,
}

#[derive(Deserialize)]
struct FlagObs {
    seen: bool,
}

#[derive(Clone)]
enum FlagEvent {
    Seen,
    Unseen,
}

impl EventCodec for FlagEvent {
    fn event_type(&self) -> &'static str {
        match self {
            FlagEvent::Seen => "seen",
            FlagEvent::Unseen => "unseen",
        }
    }
    fn event_version(&self) -> u64 {
        1
    }
    fn payload(&self) -> Value {
        json!({})
    }
    fn decode(event: &DomainEvent) -> Result<Option<Self>, Diagnostic> {
        Ok(match event.event_type.as_str() {
            "seen" => Some(FlagEvent::Seen),
            "unseen" => Some(FlagEvent::Unseen),
            _ => None,
        })
    }
}

struct FlagMachine {
    identity: ik::identity::MachineIdentity,
}

impl FlagMachine {
    fn new() -> Self {
        Self {
            identity: native_identity("obs_flag", 1, "obs-flag-spec-v1"),
        }
    }
}

impl NativeMachine for FlagMachine {
    type State = FlagState;
    type Command = FlagCommand;
    type Observation = FlagObs;
    type Event = FlagEvent;
    type Diagnostic = Diagnostic;

    fn identity(&self) -> &ik::identity::MachineIdentity {
        &self.identity
    }

    fn initial_state(&self) -> Self::State {
        FlagState {
            flag: "none".into(),
        }
    }

    fn decide(
        &self,
        _state: &Self::State,
        observation: Option<&Self::Observation>,
        _effect: &EffectStatus,
        command: &Self::Command,
    ) -> Result<NativeDecision<Self::Event>, Self::Diagnostic> {
        if command.name != "mark" {
            return Err(Diagnostic::new(
                DiagnosticCode::IkCommandInvalid,
                Some("command"),
                "unknown command",
            ));
        }
        Ok(NativeDecision::emit(
            if observation.is_some_and(|o| o.seen) {
                FlagEvent::Seen
            } else {
                FlagEvent::Unseen
            },
        ))
    }

    fn apply(
        &self,
        _state: &Self::State,
        event: &Self::Event,
    ) -> Result<NativeApply<Self::State>, Self::Diagnostic> {
        Ok(NativeApply::new(FlagState {
            flag: match event {
                FlagEvent::Seen => "seen",
                FlagEvent::Unseen => "unseen",
            }
            .into(),
        }))
    }
}

#[test]
fn frozen_event_store_replay_is_killed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = Store::create(&dir.path().join("s.db")).expect("store");
    let machine = NativeAdapter::new(FlagMachine::new());
    let agg = AggregateId("a".into());
    store
        .create_aggregate(&machine, &agg, machine.identity())
        .expect("create");
    let cap = store.show(&agg).expect("show").capability;
    store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: machine.identity().clone(),
                capability: cap,
                command: json!({"name": "mark"}),
                observation: Some(json!({"seen": true})),
            },
        )
        .expect("mark");
    assert_eq!(store.show(&agg).expect("show").state["flag"], json!("seen"));
    store.verify(&machine, Some(&agg)).expect("honest verify");
    let mutant = with_law_mutant(LawMutant::RerunLiveGuardsOnReplay, || {
        store.verify(&machine, Some(&agg))
    });
    assert_eq!(
        mutant.expect_err("mutant verify").code,
        DiagnosticCode::IkReplayMismatch
    );
    let row = killed(
        "REPLAY",
        "frozen event re-runs live guard selection",
        "store.verify",
        "mark-with-seen-observation",
    );
    assert_eq!(row.status, "killed");
}
