use super::{assert_receipt_unchanged, fixture};
use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::effect::{EffectIntentV1, EffectOutcomeV1, EffectStatus};
use crate::identity::MachineIdentity;
use crate::machine::{ApplyOutcome, Decision, DomainEvent, Machine};
use crate::runtime::{execute_command, AggregateSnapshot, Step};
use serde_json::Value;

struct FiniteMachine {
    identity: MachineIdentity,
    preflight: bool,
    decision: u8,
    event: u8,
    apply: u8,
    effect: Option<EffectIntentV1>,
    outcome: Option<EffectOutcomeV1>,
}
impl Machine for FiniteMachine {

    fn identity(&self) -> &MachineIdentity { &self.identity }
    fn initial_state(&self) -> Value { Value::Null }
    fn requires_capability_preflight(&self) -> bool { self.preflight }
    // Finite callbacks: state, command and event payload are Null; observation is absent (None).
    fn decide(&self, _: &Value, _: Option<&Value>, _: &EffectStatus, _: &Value) -> Result<Decision, Diagnostic> {
        match self.decision {
            0 => Err(Diagnostic::new(DiagnosticCode::IkIllegalState, Some("test"), "finite refusal")),
            1 => Ok(Decision::noop()),
            _ => {
                let (event_type, version) = match self.event {
                    0 => ("", 1), 1 => ("genesis", 1), 2 => ("kernel.test", 1),
                    3 => ("test", 0), _ => ("test", 1),
                };
                Ok(Decision::emit(DomainEvent::new(event_type, version, Value::Null)))
            }
        }
    }
    fn apply(&self, _: &Value, _: &DomainEvent) -> Result<ApplyOutcome, Diagnostic> {
        if self.apply == 0 {
            return Err(Diagnostic::new(DiagnosticCode::IkIllegalState, Some("test"), "finite refusal"));
        }
        let mut applied = ApplyOutcome::new(Value::Null);
        applied.effect = self.effect.clone();
        applied.outcome = self.outcome.clone();
        Ok(applied)
    }
}

fn assert_capability_precedence(error: &Diagnostic, snapshot: &AggregateSnapshot, capability: &crate::identity::Capability) {
    if snapshot.generation != capability.generation {
        assert_eq!(error.code, DiagnosticCode::IkCapabilityStale);
        assert_eq!(error.rule.as_deref(), Some("capability.generation"));
    } else {
        assert_eq!(error.code, DiagnosticCode::IkNonceInvalid);
        assert_eq!(error.rule.as_deref(), Some("capability.nonce"));
    }
}

#[kani::proof]
#[kani::unwind(24)]
fn command_authority_contract() {
    let f = fixture(kani::any(), kani::any());
    let preflight: bool = kani::any();
    let decision: u8 = kani::any(); kani::assume(decision <= 2);
    let event: u8 = kani::any(); kani::assume(event <= 4);
    let apply: u8 = kani::any(); kani::assume(apply <= 1);
    let include_effect: bool = kani::any();
    let include_outcome: bool = kani::any();
    let foreign_outcome: bool = kani::any();
    let generation_mismatch = f.snapshot.generation != f.capability.generation;
    let nonce_mismatch = f.snapshot.nonce != f.capability.nonce;
    let emitted = if foreign_outcome {
        let mut x = f.outcome.clone(); x.aggregate_id.0 = "b".into(); x
    } else { f.outcome.clone() };
    let effect = if include_effect { Some(EffectIntentV1 {
        effect_id: crate::identity::EffectId("a".into()), aggregate_id: f.snapshot.aggregate_id.clone(),
        generation: 0, effect_kind: "a".into(), payload_digest: "a".into(), idempotency_key: "a".into(), payload: None,
    }) } else { None };
    let machine = FiniteMachine { identity: f.snapshot.machine.clone(), preflight, decision, event, apply, effect,
        outcome: if include_outcome { Some(emitted) } else { None } };
    let result = execute_command(&machine, &f.snapshot,
        if f.has_receipt { Some(&f.receipt) } else { None }, &f.capability, &Value::Null, None);
    let valid_event = event == 4;
    let base = !preflight && decision == 2 && valid_event && apply == 1;
    let exact = base && include_outcome && !foreign_outcome && f.exact_duplicate;
    let replay = exact && f.has_receipt;
    let refused = result.is_err();
    let authority_refusal = matches!(&result, Err(error) if
        (generation_mismatch && error.code == DiagnosticCode::IkCapabilityStale) ||
        (!generation_mismatch && error.code == DiagnosticCode::IkNonceInvalid));
    kani::cover!(replay && matches!(&result, Ok(Step::DuplicateReplay(_))), "native invalid duplicate replay");
    kani::cover!(preflight && authority_refusal, "preflight refusal");
    kani::cover!(!preflight && decision == 0 && authority_refusal, "decision refusal");
    kani::cover!(!preflight && decision == 1 && authority_refusal, "noop refusal");
    kani::cover!(!preflight && decision == 2 && !valid_event && authority_refusal, "bad-event refusal");
    kani::cover!(!preflight && decision == 2 && valid_event && apply == 0 && authority_refusal, "apply refusal");
    kani::cover!(base && include_effect && !exact && authority_refusal, "optional new effect emission refused");
    kani::cover!(base && include_outcome && foreign_outcome && authority_refusal, "foreign outcome refusal");
    kani::cover!(generation_mismatch && !nonce_mismatch && authority_refusal, "generation-only refusal");
    kani::cover!(!generation_mismatch && nonce_mismatch && authority_refusal, "nonce-only refusal");
    kani::cover!(generation_mismatch && nonce_mismatch && authority_refusal, "both mismatch refusal");
    kani::cover!(exact && !f.has_receipt && refused, "missing receipt refusal");
    kani::cover!(base && !include_outcome && authority_refusal, "no outcome refusal");
    if replay {
        match &result {
            Ok(Step::DuplicateReplay(receipt)) => assert_receipt_unchanged(receipt, &f.receipt),
            _ => panic!("expected historical replay"),
        }
    } else {
        match &result {
            Err(error) if exact && !f.has_receipt => {
                assert_eq!(error.code, DiagnosticCode::IkInternal);
                assert!(error.rule.is_none());
            }
            Err(error) => assert_capability_precedence(error, &f.snapshot, &f.capability),
            _ => panic!("unauthorized command accepted"),
        }
    }
    core::mem::forget(result);
    core::mem::forget(machine);
    core::mem::forget(f);
}

#[kani::proof]
#[kani::unwind(24)]
fn intentionally_wrong_valid_command_suffix_is_unreachable() {
    let snapshot = super::super::snapshot(0, String::new());
    let machine = FiniteMachine { identity: snapshot.machine.clone(), preflight: false, decision: 2, event: 4, apply: 1, effect: None, outcome: None };
    let capability = crate::identity::Capability { generation: 0, nonce: String::new() };
    let result = execute_command(&machine, &snapshot, None, &capability, &Value::Null, None);
    core::mem::forget(result);
    core::mem::forget(machine);
    core::mem::forget(snapshot);
}

fn check_noop_authority(status: u8, preflight: bool, generation_case: Option<bool>) {
    let f = fixture(status, kani::any());
    match generation_case {
        None => {}
        Some(true) => kani::assume(f.snapshot.generation != f.capability.generation),
        Some(false) => kani::assume(
            f.snapshot.generation == f.capability.generation
                && f.snapshot.nonce != f.capability.nonce,
        ),
    }
    let machine = FiniteMachine { identity: f.snapshot.machine.clone(), preflight, decision: 1, event: 4, apply: 0, effect: None, outcome: None };
    let result = execute_command(&machine, &f.snapshot,
        if f.has_receipt { Some(&f.receipt) } else { None }, &f.capability, &Value::Null, None);
    match &result {
        Err(error) => assert_capability_precedence(error, &f.snapshot, &f.capability),
        _ => panic!("unauthorized noop accepted"),
    }
    let generation_mismatch = f.snapshot.generation != f.capability.generation;
    let nonce_mismatch = f.snapshot.nonce != f.capability.nonce;
    kani::cover!(generation_mismatch && !nonce_mismatch, "noop generation-only refusal");
    kani::cover!(!generation_mismatch && nonce_mismatch, "noop nonce-only refusal");
    kani::cover!(generation_mismatch && nonce_mismatch, "noop both mismatch refusal");
    core::mem::forget(result);
    core::mem::forget(machine);
    core::mem::forget(f);
}

#[kani::proof]
#[kani::unwind(24)]
fn command_authority_noop() {
    check_noop_authority(kani::any(), false, None);
}

#[kani::proof]
#[kani::unwind(24)]
fn command_partition_noop_none_native() {
    check_noop_authority(0, false, None);
}

#[kani::proof]
#[kani::unwind(24)]
fn command_partition_noop_none_native_generation() {
    check_noop_authority(0, false, Some(true));
}

#[kani::proof]
#[kani::unwind(24)]
fn command_partition_noop_none_native_nonce() {
    check_noop_authority(0, false, Some(false));
}

#[kani::proof]
#[kani::unwind(24)]
fn intentionally_wrong_valid_noop_is_rejected() {
    let generation: u64 = kani::any();
    let snapshot = super::super::snapshot(generation, String::new());
    let machine = FiniteMachine { identity: snapshot.machine.clone(), preflight: false, decision: 1, event: 4, apply: 0, effect: None, outcome: None };
    let capability = crate::identity::Capability { generation, nonce: String::new() };
    let result = execute_command(&machine, &snapshot, None, &capability, &Value::Null, None);
    assert!(result.is_err(), "deliberately false: valid-authority noop rejected");
    core::mem::forget(result);
    core::mem::forget(machine);
    core::mem::forget(snapshot);
}
