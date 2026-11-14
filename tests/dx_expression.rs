//! DX-01: fail-closed guard/invariant evaluation, checked arithmetic, shadowing.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::parse_and_compile;
use ik::diagnostic::DiagnosticCode;
use ik::effect::EffectStatus;
use ik::machine::{Decision, Machine};
use serde_json::json;

fn header() -> &'static str {
    r#"
schema_version = 1
machine_id = "dx"
machine_version = 1
"#
}

fn decide_err(toml: &str, command: serde_json::Value) -> ik::diagnostic::Diagnostic {
    let machine = parse_and_compile(toml).expect("compiles");
    machine
        .decide(
            &machine.initial_state(),
            None,
            &EffectStatus::None,
            &command,
        )
        .expect_err("must refuse")
}

#[test]
fn missing_fact_guard_does_not_become_false() {
    let toml = format!(
        r#"
{header}
[facts]
unset = {{ type = "string", policy = "replace" }}
flag = {{ type = "int", policy = "replace", initial = 0 }}
[states]
s = {{ initial = true }}
a = {{}}
b = {{}}
[commands.probe]
from = ["s"]
outcomes = [
  {{ when = "unset == 'x'", to = "a" }},
  {{ when = "flag == 0", to = "b" }}
]
"#,
        header = header()
    );
    let err = decide_err(&toml, json!({"name": "probe"}));
    assert_eq!(err.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(err.rule.as_deref(), Some("commands.probe.outcomes[0].when"));
    assert!(
        err.message.contains("could not be evaluated"),
        "{}",
        err.message
    );
}

#[test]
fn failed_invariant_when_does_not_skip_invariant() {
    let toml = format!(
        r#"
{header}
[facts]
unset = {{ type = "string", policy = "replace" }}
x = {{ type = "int", policy = "replace", initial = 0 }}
[states]
s = {{ initial = true }}
t = {{}}
[commands.go]
from = ["s"]
to = "t"
[invariants.gated]
when = "unset == 'x'"
assert = "1 == 0"
"#,
        header = header()
    );
    let machine = parse_and_compile(&toml).expect("compiles");
    let decision = machine
        .decide(
            &machine.initial_state(),
            None,
            &EffectStatus::None,
            &json!({"name": "go"}),
        )
        .expect("command is legal");
    let Decision::Emit { event, .. } = decision else {
        panic!("expected emit");
    };
    let err = machine
        .apply(&machine.initial_state(), &event)
        .expect_err("unevaluable when must refuse");
    assert_eq!(err.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(err.rule.as_deref(), Some("invariants.gated.when"));
}

#[test]
fn invariant_assert_eval_error_is_not_bare_false() {
    let toml = format!(
        r#"
{header}
[facts]
unset = {{ type = "string", policy = "replace" }}
[states]
s = {{ initial = true }}
t = {{}}
[commands.go]
from = ["s"]
to = "t"
[invariants.bad]
assert = "unset == 'x'"
"#,
        header = header()
    );
    let machine = parse_and_compile(&toml).expect("compiles");
    let decision = machine
        .decide(
            &machine.initial_state(),
            None,
            &EffectStatus::None,
            &json!({"name": "go"}),
        )
        .expect("command is legal");
    let Decision::Emit { event, .. } = decision else {
        panic!("expected emit");
    };
    let err = machine
        .apply(&machine.initial_state(), &event)
        .expect_err("unevaluable assert must refuse");
    assert_eq!(err.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(err.rule.as_deref(), Some("invariants.bad.assert"));
    assert!(
        err.message.contains("could not be evaluated"),
        "{}",
        err.message
    );
}

#[test]
fn expression_overflow_refuses_without_panic() {
    let toml = format!(
        r#"
{header}
[facts]
hi = {{ type = "int", policy = "replace", initial = 9223372036854775807 }}
[states]
s = {{ initial = true }}
t = {{}}
[commands.overflow]
from = ["s"]
to = "t"
when = "hi + 1 > 0"
"#,
        header = header()
    );
    let err = decide_err(&toml, json!({"name": "overflow"}));
    assert_eq!(err.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(err.rule.as_deref(), Some("commands.overflow.when"));
    assert!(err.message.contains("overflow"), "{}", err.message);
}

#[test]
fn expression_underflow_refuses_without_panic() {
    let toml = format!(
        r#"
{header}
[facts]
lo = {{ type = "int", policy = "replace", initial = -9223372036854775808 }}
[states]
s = {{ initial = true }}
t = {{}}
[commands.underflow]
from = ["s"]
to = "t"
when = "lo - 1 < 0"
"#,
        header = header()
    );
    let err = decide_err(&toml, json!({"name": "underflow"}));
    assert_eq!(err.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(err.rule.as_deref(), Some("commands.underflow.when"));
    assert!(err.message.contains("overflow"), "{}", err.message);
}

#[test]
fn in_range_arithmetic_still_holds() {
    let toml = format!(
        r#"
{header}
[facts]
n = {{ type = "int", policy = "replace", initial = 5 }}
[states]
s = {{ initial = true }}
t = {{}}
[commands.ok]
from = ["s"]
to = "t"
when = "n + 1 == 6"
"#,
        header = header()
    );
    let machine = parse_and_compile(&toml).expect("compiles");
    let decision = machine
        .decide(
            &machine.initial_state(),
            None,
            &EffectStatus::None,
            &json!({"name": "ok"}),
        )
        .expect("in-range arithmetic is true");
    let Decision::Emit { event, .. } = decision else {
        panic!("expected emit");
    };
    let applied = machine
        .apply(&machine.initial_state(), &event)
        .expect("apply");
    assert_eq!(applied.state["state"], "t");
}

#[test]
fn unconditional_outcome_shadows_later_outcomes() {
    let toml = format!(
        r#"
{header}
[facts]
flag = {{ type = "int", policy = "replace", initial = 0 }}
[states]
s = {{ initial = true }}
a = {{}}
b = {{}}
[commands.probe]
from = ["s"]
outcomes = [
  {{ to = "a" }},
  {{ when = "flag == 0", to = "b" }}
]
"#,
        header = header()
    );
    let errors = match parse_and_compile(&toml) {
        Err(errors) => errors,
        Ok(_) => panic!("shadowing must be rejected"),
    };
    assert!(
        errors
            .iter()
            .any(|e| e.contains("unconditional") && e.contains("unreachable")),
        "{errors:?}"
    );
}
#[test]
fn trailing_unconditional_outcome_is_legal() {
    let toml = format!(
        r#"
{header}
[facts]
flag = {{ type = "int", policy = "replace", initial = 0 }}
[states]
s = {{ initial = true }}
a = {{}}
b = {{}}
[commands.probe]
from = ["s"]
outcomes = [
  {{ when = "flag == 1", to = "a" }},
  {{ to = "b" }}
]
"#,
        header = header()
    );
    let machine = parse_and_compile(&toml).expect("guard then default is legal");
    let decision = machine
        .decide(
            &machine.initial_state(),
            None,
            &EffectStatus::None,
            &json!({"name": "probe"}),
        )
        .expect("default outcome applies");
    let Decision::Emit { event, .. } = decision else {
        panic!("expected emit");
    };
    assert_eq!(event.payload["outcome"], 1);
}

#[test]
fn verification_attempt_still_loads() {
    let machine = load_machine("verification_attempt");
    assert_eq!(machine.identity().machine_id, "verification_attempt");
    assert_eq!(machine.initial_state()["state"], "created");
}
