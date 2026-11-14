//! VFY-01: VerificationPlan compiler derives obligations from a validated Definition.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::canonical::{canonical_json, digest_json};
use ik::declarative::def::{EffectDef, OutcomeDef};
use ik::declarative::{parse_definition, verification_plan, Definition};

fn va_def() -> Definition {
    parse_definition(&std::fs::read_to_string(machine_def("verification_attempt")).unwrap())
        .expect("va")
}

fn ids_of(def: &Definition) -> Vec<String> {
    verification_plan(def)
        .obligations
        .into_iter()
        .map(|o| o.obligation_id)
        .collect()
}

#[test]
fn plan_is_byte_identical_for_same_definition() {
    let def = va_def();
    let a = verification_plan(&def);
    let b = verification_plan(&def);
    assert_eq!(canonical_json(&a), canonical_json(&b));
    assert_eq!(a.schema_version, ik::declarative::PLAN_SCHEMA_BASE);
    assert_eq!(a.machine.machine_id, "verification_attempt");
    assert_eq!(a.machine.machine_version, 1);
    assert_eq!(a.machine.spec_hash, digest_json(&def));
    assert_eq!(
        a.machine.spec_hash,
        "aae4f03a6e4169bdc86021914bc57db4bc391878651fa61fcd2a9d0d215b4a7f"
    );
    assert!(!a.challenges.is_empty());
    assert!(a
        .obligations
        .iter()
        .all(|o| o.spec_hash == a.machine.spec_hash));
}

#[test]
fn adding_outcome_adds_obl_out_001() {
    let original = va_def();
    let before = ids_of(&original);
    let mut edited = original;
    let reject = edited.commands.get_mut("reject").expect("reject");
    reject
        .outcomes
        .as_mut()
        .expect("outcomes")
        .push(OutcomeDef {
            when: Some("attempt_count < 0".into()),
            to: "repairable".into(),
            then: None,
        });
    let after = verification_plan(&edited);
    let added: Vec<_> = after
        .obligations
        .iter()
        .map(|o| o.obligation_id.as_str())
        .filter(|id| !before.iter().any(|b| b == id))
        .collect();
    assert!(
        added
            .iter()
            .any(|id| id.contains("outcome/reject/2/executable")),
        "missing OBL-OUT-001 for reject.2: {added:?}"
    );
    assert!(after.obligations.iter().any(
        |o| o.kind == "OBL-OUT-001" && o.obligation_id.contains("outcome/reject/2/executable")
    ));
}

#[test]
fn removing_invariant_drops_its_obligation() {
    let original = va_def();
    let before = verification_plan(&original);
    let before_digest = digest_json(&before);
    assert!(before.obligations.iter().any(|o| {
        o.kind == "OBL-INV-001" && o.coordinate.invariant.as_deref() == Some("va_inv_004")
    }));
    let mut edited = original;
    edited.invariants.remove("va_inv_004");
    let after = verification_plan(&edited);
    assert_ne!(digest_json(&after), before_digest);
    assert!(after
        .obligations
        .iter()
        .all(|o| { o.coordinate.invariant.as_deref() != Some("va_inv_004") }));
}

#[test]
fn adding_effect_increases_effect_obligations() {
    let original = va_def();
    let before = verification_plan(&original);
    let before_effects = before
        .obligations
        .iter()
        .filter(|o| o.kind.starts_with("OBL-EFFECT-"))
        .count();
    let mut edited = original;
    let start = edited.commands.get_mut("start").expect("start");
    start.effect = Some(EffectDef {
        kind: "WORK".into(),
        id: "fx".into(),
        payload_digest: Some("p".into()),
        idempotency_key: Some("k".into()),
    });
    let after = verification_plan(&edited);
    let after_effects = after
        .obligations
        .iter()
        .filter(|o| o.kind.starts_with("OBL-EFFECT-"))
        .count();
    assert!(
        after_effects > before_effects,
        "effect obligations did not increase: {before_effects} -> {after_effects}"
    );
    assert!(after.obligations.iter().any(|o| o.kind == "OBL-EFFECT-001"));
}
