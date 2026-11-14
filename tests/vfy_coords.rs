//! VFY-13: logical coordinates stay stable across spec-hash changes.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::def::OutcomeDef;
use ik::declarative::{parse_definition, plan_diff, verification_plan, Definition};

fn va_def() -> Definition {
    parse_definition(&std::fs::read_to_string(machine_def("verification_attempt")).unwrap())
        .expect("va")
}

#[test]
fn name_change_does_not_rebind_obligations() {
    let old = va_def();
    let mut new = old.clone();
    new.name = Some("renamed".into());
    let diff = plan_diff(&verification_plan(&old), &verification_plan(&new));
    assert!(diff.obligations_added().is_empty(), "{diff:?}");
    assert!(diff.obligations_removed().is_empty(), "{diff:?}");
    assert!(diff.obligations_modified().is_empty(), "{diff:?}");
    assert_eq!(
        diff.obligations_unchanged().len(),
        verification_plan(&old).obligations.len()
    );
}

#[test]
fn added_outcome_is_the_only_new_obligation() {
    let old = va_def();
    let mut new = old.clone();
    new.commands
        .get_mut("reject")
        .expect("reject")
        .outcomes
        .as_mut()
        .expect("outcomes")
        .push(OutcomeDef {
            when: Some("attempt_count < 0".into()),
            to: "repairable".into(),
            then: None,
        });
    let diff = plan_diff(&verification_plan(&old), &verification_plan(&new));
    let added = diff.obligations_added();
    assert!(
        added.iter().any(|id| id == "outcome/reject/2/executable"),
        "{added:?}"
    );
    assert!(
        !added.iter().any(|id| id == "capability/stale"),
        "spec-hash rebound leaked: {added:?}"
    );
}

#[test]
fn guard_edit_modifies_only_that_outcome() {
    let old = va_def();
    let mut new = old.clone();
    new.commands
        .get_mut("reject")
        .expect("reject")
        .outcomes
        .as_mut()
        .expect("outcomes")[0]
        .when = Some("attempt_count + 1 <= max_attempts".into());
    let diff = plan_diff(&verification_plan(&old), &verification_plan(&new));
    let modified = diff.obligations_modified();
    assert!(
        modified
            .iter()
            .any(|id| id == "outcome/reject/0/executable"),
        "{modified:?}"
    );
    assert!(
        !modified
            .iter()
            .any(|id| id == "outcome/reject/1/executable"),
        "reject.1 must not inherit reject.0's edit: {modified:?}"
    );
    assert!(diff
        .obligations_unchanged()
        .iter()
        .any(|id| id == "outcome/reject/1/executable"));
    assert!(diff.obligations_added().is_empty());
}
