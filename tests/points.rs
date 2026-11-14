//! Decision points are the places where someone chose a value or a relation in a guard. The
//! list must be stable, ordered, and honest about what it does not enumerate.

use ik::declarative::points::decision_points;

fn keys(guard: &str) -> Vec<String> {
    decision_points(guard)
        .expect("guard parses")
        .into_iter()
        .map(|point| point.key)
        .collect()
}

#[test]
fn a_text_guard_has_a_character_set_and_two_length_bounds() {
    assert_eq!(
        keys("text_in($id, 'A-Za-z0-9._-', 1, 64)"),
        vec!["charset:A-Za-z0-9._-", "min_length:1", "max_length:64"]
    );
}

#[test]
fn an_operator_and_a_constant_inside_arithmetic_are_both_points() {
    assert_eq!(
        keys("$loan_until_ms == $checkout_ms + 1209600000"),
        vec!["operator:==", "constant:1209600000"]
    );
}

#[test]
fn a_repeated_point_gets_a_numbered_key_so_each_can_carry_its_own_origin() {
    assert_eq!(
        keys("$a == 1 || $b == 1"),
        vec!["operator:==", "constant:1", "operator:==#2", "constant:1#2"]
    );
}

#[test]
fn existence_tests_and_connectives_are_not_enumerated() {
    assert_eq!(keys("exists(a) && not_exists(b)"), Vec::<String>::new());
}

#[test]
fn a_guard_that_does_not_parse_is_an_error_not_an_empty_list() {
    assert_eq!(decision_points("$a ==").is_err(), true);
}

#[test]
fn an_operator_phrase_names_only_its_own_operator() {
    use ik::declarative::expr::CmpOp;
    use ik::declarative::points::operators_named;
    assert_eq!(operators_named("The count must not equal 0."), vec![CmpOp::Ne]);
    assert_eq!(operators_named("no later than Friday"), vec![CmpOp::Le]);
    assert_eq!(operators_named("on or before the due date"), vec![CmpOp::Le]);
    assert_eq!(operators_named("on or after the start"), vec![CmpOp::Ge]);
    assert_eq!(operators_named("a later deadline"), vec![CmpOp::Gt]);
    assert_eq!(operators_named("unrelated words"), Vec::<CmpOp>::new());
}

#[test]
fn an_operator_phrase_must_be_a_whole_word() {
    use ik::declarative::points::operators_named;
    assert_eq!(operators_named("the equalizer setting").is_empty(), true);
}
