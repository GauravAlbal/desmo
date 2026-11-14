//! Rule witnesses must separate a guard from its one-step variants, not merely agree with it.

use ik::declarative::witness::{check, Example};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn example(args: Value, facts: Value, accepts: bool) -> Example {
    Example {
        args: as_map(args),
        facts: as_map(facts),
        state: "idle".to_string(),
        accepts,
    }
}

fn as_map(value: Value) -> BTreeMap<String, Value> {
    value
        .as_object()
        .map(|object| object.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

fn tag(id: &str, accepts: bool) -> Example {
    example(json!({ "id": id }), json!({}), accepts)
}

const TAG_GUARD: &str = "text_in($id, 'A-Za-z0-9._-', 1, 64)";

#[test]
fn trivial_examples_agree_with_the_guard_but_leave_its_boundaries_untested() {
    let examples = vec![tag("abc", true), tag("", false)];
    let verdict = check(TAG_GUARD, &examples, &[]).expect("check");
    assert_eq!(verdict.mismatches.is_empty(), true, "{verdict:?}");
    assert_eq!(verdict.adequate(), false, "{verdict:?}");
    let surviving = verdict.surviving.join("\n");
    assert_eq!(surviving.contains("1,64) → text_in($id,'A-Za-z0-9._-',1,65)"), true, "{surviving}");
    assert_eq!(surviving.contains("1,63)"), true, "{surviving}");
    assert_eq!(surviving.contains("2,64)"), true, "{surviving}");
}

#[test]
fn boundary_examples_tell_the_tag_rule_from_every_variant() {
    let examples = vec![
        tag("a", true),
        tag(&"x".repeat(64), true),
        tag("a.b-9_Z", true),
        tag("", false),
        tag(&"x".repeat(65), false),
        tag("item/1", false),
    ];
    let verdict = check(TAG_GUARD, &examples, &[]).expect("check");
    assert_eq!(verdict.adequate(), true, "{verdict:?}");
    assert_eq!(verdict.variants_checked > 0, true);
}

#[test]
fn a_far_from_boundary_pair_does_not_tell_greater_than_from_greater_or_equal() {
    let guard = "$new_until_ms > loan_until_ms";
    let facts = json!({ "loan_until_ms": 100 });
    let far = vec![
        example(json!({ "new_until_ms": 200 }), facts.clone(), true),
        example(json!({ "new_until_ms": 50 }), facts.clone(), false),
    ];
    let verdict = check(guard, &far, &[]).expect("check");
    assert_eq!(verdict.adequate(), false, "{verdict:?}");
    assert_eq!(
        verdict.surviving.iter().any(|label| label.ends_with("$new_until_ms >= loan_until_ms")),
        true,
        "{verdict:?}"
    );
    let mut with_boundary = far;
    with_boundary.push(example(json!({ "new_until_ms": 100 }), facts, false));
    let verdict = check(guard, &with_boundary, &[]).expect("check");
    assert_eq!(verdict.adequate(), true, "{verdict:?}");
}

#[test]
fn an_example_whose_stated_outcome_is_wrong_is_reported_and_stops_the_check() {
    let examples = vec![tag("ok", false)];
    let verdict = check(TAG_GUARD, &examples, &[]).expect("check");
    assert_eq!(
        verdict.mismatches,
        vec!["example 1 is stated as rejected but the guard accepts it".to_string()]
    );
    assert_eq!(verdict.surviving.is_empty(), true);
}

#[test]
fn an_example_that_cannot_be_evaluated_is_a_mismatch_not_a_pass() {
    let guard = "$overdue == 0 || recall_placed == 1";
    let examples = vec![example(json!({}), json!({}), true)];
    let verdict = check(guard, &examples, &[]).expect("check");
    assert_eq!(verdict.mismatches.len(), 1, "{verdict:?}");
    assert_eq!(verdict.mismatches[0].contains("cannot be evaluated"), true, "{verdict:?}");
}

#[test]
fn every_clause_of_a_disjunction_needs_an_example_that_toggles_it() {
    let guard = "$overdue == 0 || recall_placed == 1";
    let examples = vec![
        example(json!({ "overdue": 0 }), json!({ "recall_placed": 0 }), true),
        example(json!({ "overdue": 3 }), json!({ "recall_placed": 0 }), false),
    ];
    let verdict = check(guard, &examples, &[]).expect("check");
    assert_eq!(
        verdict.uncovered_clauses.contains(&"recall_placed == 1: no example makes it true".to_string()),
        true,
        "{verdict:?}"
    );
}

#[test]
fn dropping_a_clause_of_a_conjunction_must_change_some_example() {
    let guard = "exists(artifact_digest) && $artifact_digest == artifact_digest";
    let facts = json!({ "artifact_digest": "d1" });
    let examples = vec![
        example(json!({ "artifact_digest": "d1" }), facts.clone(), true),
        example(json!({ "artifact_digest": "d2" }), facts, false),
        example(json!({ "artifact_digest": "d1" }), json!({}), false),
    ];
    let verdict = check(guard, &examples, &[]).expect("check");
    assert_eq!(verdict.adequate(), true, "{verdict:?}");
}

#[test]
fn a_declared_equivalent_variant_is_not_reported_as_surviving() {
    let guard = "$x >= 0";
    let examples = vec![
        example(json!({ "x": 5 }), json!({}), true),
        example(json!({ "x": 0 }), json!({}), true),
    ];
    let first = check(guard, &examples, &[]).expect("check");
    assert_eq!(first.adequate(), false, "{first:?}");
    let declared = first.surviving.clone();
    let second = check(guard, &examples, &declared).expect("check");
    assert_eq!(second.surviving.is_empty(), true, "{second:?}");
}

#[test]
fn a_guard_that_does_not_parse_is_an_error() {
    assert_eq!(check("$x ?? 1", &[], &[]).is_err(), true);
}
