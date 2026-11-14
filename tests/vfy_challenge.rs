//! VFY-03: semantic challenges derive from declared comparison guards.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::canonical::canonical_json;
use ik::declarative::{parse_definition, verification_plan, Definition};

fn va_def() -> Definition {
    parse_definition(&std::fs::read_to_string(machine_def("verification_attempt")).unwrap())
        .expect("va")
}

#[test]
fn reject_guard_emits_adjacent_le_competitor() {
    let plan = verification_plan(&va_def());
    let hit = plan
        .challenges
        .iter()
        .find(|c| {
            c.kind == "CHL-GUARD-BOUNDARY"
                && c.coordinate.command.as_deref() == Some("reject")
                && c.coordinate.outcome_index == Some(0)
                && c.transformation
                    .contains("attempt_count + 1 <= max_attempts")
        })
        .expect("reject.0 <= competitor");
    assert!(hit
        .transformation
        .contains("attempt_count + 1 < max_attempts"));
    assert!(hit.mandatory);
    assert_eq!(hit.expected_relation, "distinguish");
    assert!(hit
        .source_obligation_ids
        .iter()
        .any(|id| id.contains("outcome/reject/0/executable")));
}

#[test]
fn challenges_are_deterministic() {
    let def = va_def();
    let a = verification_plan(&def);
    let b = verification_plan(&def);
    assert_eq!(canonical_json(&a.challenges), canonical_json(&b.challenges));
    assert_eq!(a.challenges.len(), b.challenges.len());
    assert!(!a.challenges.is_empty());
}
