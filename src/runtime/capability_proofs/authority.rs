//! Invalid-authority proofs over the actual pure runtime entrypoints.
//! Generations are arbitrary independent u64s; nonces are independent ASCII
//! strings of length 0..=4. Effect identities range over "a"/"b", JSON is Null
//! or absent, receipt fields are fixed, and status/terminal/receipt presence vary.
//! Runtime assert-before-assume cutpoints prove the mutation suffix unreachable;
//! they do not replace diagnostics, duplicate replay, or any reachable runtime code.
use super::{bounded_nonce, snapshot};
use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::effect::{EffectIntentV1, EffectOutcomeV1, EffectStatus};
use crate::identity::{AggregateId, Capability, EffectId};
use crate::runtime::{execute_outcome_binding, AggregateSnapshot, Receipt, Step};

pub(super) struct AuthorityFixture {
    pub(super) snapshot: AggregateSnapshot,
    pub(super) capability: Capability,
    pub(super) receipt: Receipt,
    pub(super) outcome: EffectOutcomeV1,
    pub(super) has_receipt: bool,
    pub(super) exact_duplicate: bool,
}

pub(super) fn fixture(status: u8, has_receipt: bool) -> AuthorityFixture {
    let current: u64 = kani::any();
    let supplied: u64 = kani::any();
    let current_nonce = bounded_nonce();
    let supplied_nonce = bounded_nonce();
    kani::assume(current != supplied || current_nonce != supplied_nonce);
    kani::assume(status <= 2);
    let recorded = status == 2;
    let foreign: bool = kani::any();
    let mask: u8 = kani::any();
    let pick = |bit| if mask & bit == 0 { "a" } else { "b" };
    let aggregate = AggregateId("a".into());
    let intent_generation: u64 = kani::any();
    let intent = EffectIntentV1 { effect_id: EffectId("a".into()), aggregate_id: aggregate.clone(), generation: intent_generation,
        effect_kind: "a".into(), payload_digest: "a".into(), idempotency_key: "a".into(), payload: None };
    let outcome_generation: u64 = kani::any();
    let outcome = EffectOutcomeV1 { effect_id: EffectId(pick(1).into()), aggregate_id: AggregateId(if foreign { "b" } else { "a" }.into()),
        planned_generation: outcome_generation, payload_digest: pick(2).into(), idempotency_key: pick(4).into(), outcome_digest: pick(8).into() };
    let exact_duplicate = recorded && outcome.effect_id == intent.effect_id && outcome.aggregate_id == intent.aggregate_id
        && outcome.planned_generation == intent.generation && outcome.payload_digest == intent.payload_digest
        && outcome.idempotency_key == intent.idempotency_key && outcome.outcome_digest == "a";
    let effect = match status { 0 => EffectStatus::None, 1 => EffectStatus::Pending(intent.clone()), _ => EffectStatus::Recorded(intent, "a".into(), None) };
    let mut snap = snapshot(current, current_nonce);
    snap.aggregate_id = aggregate.clone(); snap.terminal = kani::any(); snap.sequence = kani::any(); snap.effect = effect;
    let receipt = Receipt { receipt_id: "r".into(), aggregate_id: aggregate, sequence: 1, generation: 2,
        machine_id: "m".into(), machine_version: 3, machine_spec_hash: "s".into(), state_digest: "d".into(),
        event_hash: "h".into(), effect: None, note: None };
    AuthorityFixture { snapshot: snap, capability: Capability { generation: supplied, nonce: supplied_nonce }, receipt,
        outcome, has_receipt, exact_duplicate }
}

pub(super) fn assert_receipt_unchanged(actual: &Receipt, expected: &Receipt) {
    assert!(actual.receipt_id == expected.receipt_id);
    assert!(actual.aggregate_id == expected.aggregate_id);
    assert!(actual.sequence == expected.sequence);
    assert!(actual.generation == expected.generation);
    assert!(actual.machine_id == expected.machine_id);
    assert!(actual.machine_version == expected.machine_version);
    assert!(actual.machine_spec_hash == expected.machine_spec_hash);
    assert!(actual.state_digest == expected.state_digest);
    assert!(actual.event_hash == expected.event_hash);
    // Both optional fields are absent in this explicitly finite receipt domain.
    assert!(expected.effect.is_none() && actual.effect.is_none());
    assert!(expected.note.is_none() && actual.note.is_none());
}

fn check_outcome_authority(f: &AuthorityFixture, result: &Result<Step, Diagnostic>) {
    let recorded = matches!(&f.snapshot.effect, EffectStatus::Recorded(..));
    let pending = matches!(&f.snapshot.effect, EffectStatus::Pending(..));
    let exact = f.exact_duplicate;
    let foreign = f.outcome.aggregate_id != f.snapshot.aggregate_id;
    let same_gen = f.snapshot.generation == f.capability.generation;
    let same_nonce = f.snapshot.nonce == f.capability.nonce;
    kani::cover!(same_gen && !same_nonce && matches!(&result, Err(_)), "nonce-only authority refusal");
    kani::cover!(!same_gen && same_nonce && matches!(&result, Err(_)), "generation-only authority refusal");
    kani::cover!(!same_gen && !same_nonce && matches!(&result, Err(_)), "both authority components refused");
    kani::cover!(recorded && exact && !same_gen && same_nonce && f.has_receipt && matches!(&result, Ok(Step::DuplicateReplay(_))), "exact invalid-authority replay");
    kani::cover!(recorded && !exact && !foreign && matches!(&result, Err(_)), "divergent recorded refusal");
    kani::cover!(recorded && foreign && matches!(&result, Err(_)), "foreign aggregate refusal");
    kani::cover!(recorded && exact && !f.has_receipt && matches!(&result, Err(_)), "missing receipt refusal");
    kani::cover!(matches!(&f.snapshot.effect, EffectStatus::None) && matches!(&result, Err(_)), "none status refusal");
    kani::cover!(pending && matches!(&result, Err(_)), "pending status refusal");
    assert!(!matches!(&result, Ok(Step::Commit(_)) | Ok(Step::Noop { .. })));
    if exact && f.has_receipt {
        match &result {
            Ok(Step::DuplicateReplay(r)) => assert_receipt_unchanged(r, &f.receipt),
            _ => panic!("exact historical duplicate must replay"),
        }
    } else {
        assert!(result.is_err());
    }
    if !exact && !foreign {
        if let Err(d) = &result {
            if !same_gen { assert!(matches!(d.code, DiagnosticCode::IkCapabilityStale)); assert!(d.rule.as_deref() == Some("capability.generation")); }
            else { assert!(matches!(d.code, DiagnosticCode::IkNonceInvalid)); assert!(d.rule.as_deref() == Some("capability.nonce")); }
        }
    }
}

fn run_outcome_authority_contract(status: u8, has_receipt: bool) {
    let f = fixture(status, has_receipt);
    // Keep optional references concrete at the callsite; both cases remain proved.
    if f.has_receipt {
        let result = execute_outcome_binding(&f.snapshot, Some(&f.receipt), &f.capability, &f.outcome);
        check_outcome_authority(&f, &result);
        core::mem::forget(result);
    } else {
        let result = execute_outcome_binding(&f.snapshot, None, &f.capability, &f.outcome);
        check_outcome_authority(&f, &result);
        core::mem::forget(result);
    }
    // Caller cleanup after the borrowed entrypoint returns is outside this theorem.
    core::mem::forget(f);
}

#[kani::proof]
#[kani::unwind(24)]
fn outcome_authority_contract() {
    run_outcome_authority_contract(kani::any(), kani::any());
}

#[kani::proof]
#[kani::unwind(24)]
fn outcome_partition_none_without_receipt() {
    run_outcome_authority_contract(0, false);
}

#[kani::proof]
#[kani::unwind(24)]
fn outcome_partition_none_with_receipt() {
    run_outcome_authority_contract(0, true);
}

#[kani::proof]
#[kani::unwind(24)]
fn outcome_partition_pending_without_receipt() {
    run_outcome_authority_contract(1, false);
}

#[kani::proof]
#[kani::unwind(24)]
fn outcome_partition_pending_with_receipt() {
    run_outcome_authority_contract(1, true);
}

#[kani::proof]
#[kani::unwind(24)]
fn outcome_partition_recorded_without_receipt() {
    run_outcome_authority_contract(2, false);
}

#[kani::proof]
#[kani::unwind(24)]
fn outcome_partition_recorded_with_receipt() {
    run_outcome_authority_contract(2, true);
}

#[kani::proof]
#[kani::unwind(24)]
fn outcome_partition_recorded_exact_with_receipt() {
    let f = fixture(2, true);
    kani::assume(f.exact_duplicate);
    let result = execute_outcome_binding(&f.snapshot, Some(&f.receipt), &f.capability, &f.outcome);
    check_outcome_authority(&f, &result);
    core::mem::forget(result);
    core::mem::forget(f);
}

#[kani::proof]
#[kani::unwind(24)]
fn outcome_partition_recorded_nonduplicate_with_receipt() {
    let f = fixture(2, true);
    kani::assume(!f.exact_duplicate);
    let result = execute_outcome_binding(&f.snapshot, Some(&f.receipt), &f.capability, &f.outcome);
    check_outcome_authority(&f, &result);
    core::mem::forget(result);
    core::mem::forget(f);
}

#[kani::proof]
#[kani::unwind(24)]
fn intentionally_wrong_invalid_duplicate_is_rejected() {
    let f = fixture(kani::any(), kani::any());
    kani::assume(f.exact_duplicate && f.has_receipt);
    let result = execute_outcome_binding(&f.snapshot, Some(&f.receipt), &f.capability, &f.outcome);
    assert!(result.is_err(), "deliberately false: invalid-authority duplicate rejected");
    core::mem::forget(result);
    core::mem::forget(f);
}

// Discriminator: construct the duplicate directly, without the branching fixture.
#[kani::proof]
#[kani::unwind(24)]
fn minimal_invalid_duplicate_control() {
    let generation: u64 = kani::any();
    let mut s = snapshot(generation, String::new());
    let intent = EffectIntentV1 {
        effect_id: EffectId(String::new()), aggregate_id: s.aggregate_id.clone(),
        generation, effect_kind: String::new(), payload_digest: String::new(),
        idempotency_key: String::new(), payload: None,
    };
    let outcome = EffectOutcomeV1 {
        effect_id: intent.effect_id.clone(), aggregate_id: intent.aggregate_id.clone(),
        planned_generation: generation, payload_digest: String::new(),
        idempotency_key: String::new(), outcome_digest: String::new(),
    };
    s.effect = EffectStatus::Recorded(intent, String::new(), None);
    let receipt = Receipt {
        receipt_id: String::new(), aggregate_id: s.aggregate_id.clone(), sequence: 0,
        generation, machine_id: String::new(), machine_version: 0,
        machine_spec_hash: String::new(), state_digest: String::new(),
        event_hash: String::new(), effect: None, note: None,
    };
    let supplied: u64 = kani::any();
    kani::assume(supplied != generation);
    let capability = Capability { generation: supplied, nonce: String::new() };
    let result = execute_outcome_binding(&s, Some(&receipt), &capability, &outcome);
    assert!(result.is_err(), "deliberately false: direct invalid duplicate rejected");
}


#[kani::proof]
#[kani::unwind(24)]
fn intentionally_wrong_valid_outcome_suffix_is_unreachable() {
    let snapshot = snapshot(0, String::new());
    let capability = Capability { generation: 0, nonce: String::new() };
    let outcome = EffectOutcomeV1 {
        effect_id: EffectId(String::new()), aggregate_id: snapshot.aggregate_id.clone(),
        planned_generation: 0, payload_digest: String::new(),
        idempotency_key: String::new(), outcome_digest: String::new(),
    };
    let result = execute_outcome_binding(&snapshot, None, &capability, &outcome);
    core::mem::forget(result);
    core::mem::forget(snapshot);
}

// Original failed control retained: caller-side Receipt/Value drop expansion times out.
#[kani::proof]
#[kani::unwind(24)]
fn invalid_duplicate_caller_drop_reproducer() {
    let f = fixture(kani::any(), kani::any());
    kani::assume(f.exact_duplicate && f.has_receipt);
    let result = execute_outcome_binding(&f.snapshot, Some(&f.receipt), &f.capability, &f.outcome);
    assert!(result.is_err(), "deliberately false: invalid-authority duplicate rejected");
}

mod command;
