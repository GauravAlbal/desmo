use ik::declarative::check::check_machine;
use ik::declarative::def::{check, Definition};
use ik::declarative::explore::{Bounds, CaseCorpus};

fn parse_and_check(raw: &str) -> Result<Definition, Vec<String>> {
    let def: Definition = toml::from_str(raw).map_err(|e| vec![e.to_string()])?;
    let errs = check(&def);
    if errs.is_empty() {
        Ok(def)
    } else {
        Err(errs)
    }
}

#[test]
fn then_arg_type_mismatch_is_rejected_statically() {
    let raw = r#"
schema_version = 1
machine_id = "bad_arg_ty"
machine_version = 1

[facts]
count = { type = "int", policy = "replace" }

[states]
a = { initial = true }
b = {}

[commands.go]
from = ["a"]
to = "b"
args = { label = "string" }
then = { count = "$label" }
"#;
    let err = parse_and_check(raw).expect_err("string arg assigned to int fact must fail");
    let joined = err.join("; ");
    assert!(
        joined.contains("assigns `$label` of type `string` to `int` fact"),
        "unexpected error: {joined}"
    );
}

#[test]
fn then_literal_errors_include_actionable_spelling_hints() {
    let bad_str = r#"
schema_version = 1
machine_id = "bad_str"
machine_version = 1

[facts]
status = { type = "string", policy = "replace" }

[states]
a = { initial = true }
b = {}

[commands.go]
from = ["a"]
to = "b"
then = { status = "paid" }
"#;
    let err = parse_and_check(bad_str)
        .expect_err("unquoted string literal in then must fail")
        .join("; ");
    assert!(
        err.contains(r#"'"paid"'"#),
        "string literal error should hint inner quotes: {err}"
    );

    let bad_bool = r#"
schema_version = 1
machine_id = "bad_bool"
machine_version = 1

[facts]
approved = { type = "bool", policy = "replace" }

[states]
a = { initial = true }
b = {}

[commands.go]
from = ["a"]
to = "b"
then = { approved = '"true"' }
"#;
    let err = parse_and_check(bad_bool)
        .expect_err("inner-quoted bool in then must fail")
        .join("; ");
    assert!(
        err.contains(r#"use "true", "false", or "$arg""#),
        "bool literal error should hint unquoted true/false string: {err}"
    );
}

#[test]
fn flat_ik_check_refusal_lists_uncovered_obligation_ids_without_prior() {
    let raw = std::fs::read_to_string("examples/orders/order.ik.toml").expect("read order.ik.toml");
    let def = parse_and_check(&raw).expect("valid order machine");
    let result = check_machine(
        &def,
        None,
        &CaseCorpus::default(),
        &[],
        Bounds {
            max_nodes: 256,
            max_depth: 16,
        },
    )
    .expect("check_machine runs");
    assert_eq!(result.verdict, "REFUSED");
    assert!(
        result.text.contains("UNCOVERED OBLIGATIONS"),
        "flat refusal without --prior should list uncovered obligations:\n{}",
        result.text
    );
    assert!(
        result.text.contains("OBL-"),
        "UNCOVERED OBLIGATIONS should list concrete obligation ids:\n{}",
        result.text
    );
}
