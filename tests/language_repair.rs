//! Language repairs: counter overflow, then `+N` on replace facts, guard
//! arithmetic, and diagnostics for `*`/`/`.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::canonical::canonical_json;
use ik::declarative::expr::{self, Expr, Operand};
use ik::declarative::parse_and_compile;
use ik::diagnostic::DiagnosticCode;
use ik::effect::EffectStatus;
use ik::machine::{Decision, Machine};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn header() -> &'static str {
    r#"
schema_version = 1
machine_id = "repair"
machine_version = 1
"#
}

fn machine_toml(body: &str) -> String {
    format!("{}\n{body}", header())
}

fn compile(toml: &str) -> ik::declarative::CompiledMachine {
    parse_and_compile(toml).unwrap_or_else(|errors| panic!("compile: {errors:?}"))
}

fn decide(
    machine: &ik::declarative::CompiledMachine,
    command: Value,
) -> Result<Decision, ik::diagnostic::Diagnostic> {
    machine.decide(
        &machine.initial_state(),
        None,
        &EffectStatus::None,
        &command,
    )
}

fn decide_err(toml: &str, command: Value) -> ik::diagnostic::Diagnostic {
    decide(&compile(toml), command).expect_err("must refuse")
}

fn emitted(decision: Decision) -> ik::machine::DomainEvent {
    match decision {
        Decision::Emit { event, .. } => Some(event),
        Decision::Noop { .. } => None,
    }
    .expect("expected emit")
}

fn apply_ok(machine: &ik::declarative::CompiledMachine, command: Value) -> Value {
    let event = emitted(decide(machine, command).expect("decide"));
    machine
        .apply(&machine.initial_state(), &event)
        .expect("apply")
        .state
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ik"))
}

fn shipped_dir() -> PathBuf {
    repo_root().join("examples/shipped")
}

#[test]
fn counter_increment_overflow_refuses_without_state_change() {
    let toml = machine_toml(
        r#"
[facts]
count = { type = "int", policy = "counter", initial = 9223372036854775807 }
[states]
s = { initial = true }
t = {}
[commands.inc]
from = ["s"]
to = "t"
then = { count = "+1" }
"#,
    );
    let machine = compile(&toml);
    let before = machine.initial_state();
    let event = emitted(decide(&machine, json!({"name": "inc"})).expect("decide is legal"));
    let err = machine
        .apply(&before, &event)
        .expect_err("overflow must refuse");
    assert_eq!(err.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(
        err.message.matches("integer arithmetic overflow").count(),
        1,
        "{}",
        err.message
    );
    assert_eq!(before["count"], json!(9223372036854775807i64));
    assert_eq!(before["state"], "s");
}

#[test]
fn replace_plus_n_is_rejected_naming_the_fact() {
    let toml = machine_toml(
        r#"
[facts]
n = { type = "int", policy = "replace", initial = 5 }
[states]
s = { initial = true }
t = {}
[commands.go]
from = ["s"]
to = "t"
then = { n = "+1" }
"#,
    );
    let errors = parse_and_compile(&toml)
        .err()
        .expect("must reject +N on a replace fact");
    let joined = errors.join("\n");
    assert!(joined.contains("`n`"), "{joined}");
    assert!(
        joined.contains("`+N`, which is only for counter facts"),
        "{joined}"
    );
}

#[test]
fn star_slash_diagnostics_name_supported_forms() {
    let err = expr::parse("amount * pct / 100 == 200").expect_err("chained * is unsupported");
    assert!(err.contains("`*`"), "{err}");
    assert!(err.contains("supported forms"), "{err}");
    assert!(err.contains("positive integer constant"), "{err}");
    let err = expr::parse("a % 2 == 0").expect_err("% is unsupported");
    assert!(err.contains("supported forms"), "{err}");
    assert!(
        expr::parse("amount / 50 == 200").is_ok(),
        "{:?}",
        expr::parse("amount / 50 == 200")
    );
    assert!(expr::parse("$deposit_cents == amount / 50").is_ok());
    assert!(expr::parse("units * 175 == 350").is_ok());
}

#[test]
fn remain_after_accepts_honest_and_refuses_lie() {
    let toml = machine_toml(
        r#"
[facts]
remaining_qty = { type = "int", policy = "replace", initial = 10 }
[states]
s = { initial = true }
t = {}
[commands.fill]
from = ["s"]
to = "t"
args = { remain_after = "int", units = "int" }
when = "$remain_after == remaining_qty - $units"
then = { remaining_qty = "$remain_after" }
"#,
    );
    let machine = compile(&toml);
    let state = apply_ok(
        &machine,
        json!({"name": "fill", "args": {"remain_after": 6, "units": 4}}),
    );
    assert_eq!(state["remaining_qty"], 6);
    let err = decide_err(
        &toml,
        json!({"name": "fill", "args": {"remain_after": 99, "units": 4}}),
    );
    assert_eq!(err.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(
        err.message.matches("no outcome of command `fill`").count(),
        1,
        "{}",
        err.message
    );
}

#[test]
fn deposit_cents_division_toward_zero() {
    let toml = machine_toml(
        r#"
[facts]
amount = { type = "int", policy = "replace", initial = 175 }
[states]
s = { initial = true }
t = {}
[commands.quote]
from = ["s"]
to = "t"
args = { deposit_cents = "int" }
when = "$deposit_cents == amount / 50"
then = { amount = "$deposit_cents" }
"#,
    );
    let machine = compile(&toml);
    let state = apply_ok(
        &machine,
        json!({"name": "quote", "args": {"deposit_cents": 3}}),
    );
    assert_eq!(state["amount"], 3);
    let err = decide(
        &machine,
        json!({"name": "quote", "args": {"deposit_cents": 4}}),
    )
    .expect_err("4 is a lie");
    assert_eq!(err.code, DiagnosticCode::IkInvariantFailed);
}

#[test]
fn deadline_is_arg_plus_fact() {
    let toml = machine_toml(
        r#"
[facts]
hold_ms = { type = "int", policy = "replace", initial = 5000 }
[states]
s = { initial = true }
t = {}
[commands.hold]
from = ["s"]
to = "t"
args = { d = "int", now_ms = "int" }
when = "$d == $now_ms + hold_ms"
then = { hold_ms = "$d" }
"#,
    );
    let machine = compile(&toml);
    let state = apply_ok(
        &machine,
        json!({"name": "hold", "args": {"d": 6000, "now_ms": 1000}}),
    );
    assert_eq!(state["hold_ms"], 6000);
    decide(
        &machine,
        json!({"name": "hold", "args": {"d": 6001, "now_ms": 1000}}),
    )
    .expect_err("lie");
}

#[test]
fn amount_is_units_times_constant() {
    let toml = machine_toml(
        r#"
[facts]
units = { type = "int", policy = "replace", initial = 2 }
[states]
s = { initial = true }
t = {}
[commands.price]
from = ["s"]
to = "t"
args = { amount = "int" }
when = "$amount == units * 175"
then = { units = "$amount" }
"#,
    );
    let machine = compile(&toml);
    let state = apply_ok(&machine, json!({"name": "price", "args": {"amount": 350}}));
    assert_eq!(state["units"], 350);
    decide(&machine, json!({"name": "price", "args": {"amount": 351}})).expect_err("lie");
}

#[test]
fn guard_arithmetic_overflow_refuses() {
    let add = machine_toml(
        r#"
[facts]
hi = { type = "int", policy = "replace", initial = 9223372036854775807 }
one = { type = "int", policy = "replace", initial = 1 }
[states]
s = { initial = true }
t = {}
[commands.go]
from = ["s"]
to = "t"
when = "hi + one == 0"
"#,
    );
    let err = decide_err(&add, json!({"name": "go"}));
    assert_eq!(
        err.message.matches("integer arithmetic overflow").count(),
        1,
        "{}",
        err.message
    );

    let mul = machine_toml(
        r#"
[facts]
hi = { type = "int", policy = "replace", initial = 9223372036854775807 }
[states]
s = { initial = true }
t = {}
[commands.go]
from = ["s"]
to = "t"
when = "hi * 2 == 0"
"#,
    );
    let err = decide_err(&mul, json!({"name": "go"}));
    assert_eq!(
        err.message.matches("integer arithmetic overflow").count(),
        1,
        "{}",
        err.message
    );
}

#[test]
fn unset_fact_in_new_arith_does_not_fall_through() {
    let toml = machine_toml(
        r#"
[facts]
remaining_qty = { type = "int", policy = "replace" }
flag = { type = "int", policy = "replace", initial = 0 }
[states]
s = { initial = true }
a = {}
b = {}
[commands.fill]
from = ["s"]
outcomes = [
  { when = "$remain_after == remaining_qty - $units", to = "a" },
  { when = "flag == 0", to = "b" }
]
args = { remain_after = "int", units = "int" }
"#,
    );
    let err = decide_err(
        &toml,
        json!({"name": "fill", "args": {"remain_after": 1, "units": 1}}),
    );
    assert_eq!(err.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(
        err.message.matches("could not be evaluated").count(),
        1,
        "{}",
        err.message
    );
}

#[test]
fn division_by_zero_is_rejected_at_check_time() {
    let err = expr::parse("n / 0 == 1").expect_err("div 0");
    assert!(err.contains("positive integer constant"), "{err}");
    let toml = machine_toml(
        r#"
[facts]
n = { type = "int", policy = "replace", initial = 1 }
[states]
s = { initial = true }
t = {}
[commands.go]
from = ["s"]
to = "t"
when = "n / 0 == 1"
"#,
    );
    let errors = parse_and_compile(&toml)
        .err()
        .expect("division by zero must fail static check");
    let joined = errors.join("\n");
    assert!(joined.contains("positive integer constant"), "{joined}");
}

#[test]
fn constant_add_encoding_and_shipped_identity_are_stable() {
    let cmp = match expr::parse("n + 1 == 6").expect("parse") {
        Expr::Cmp(cmp) => Some(cmp),
        _ => None,
    }
    .expect("comparison");
    assert_eq!(
        cmp.left,
        Operand::Arith {
            base: Box::new(Operand::Fact("n".into())),
            add: true,
            amount: 1,
        }
    );
    let identity = canonical_json(load_machine("verification_attempt").identity());
    assert_eq!(
        identity,
        r#"{"machine_id":"verification_attempt","spec_hash":"aae4f03a6e4169bdc86021914bc57db4bc391878651fa61fcd2a9d0d215b4a7f","version":1}"#
    );
}

#[test]
fn new_arith_forms_hash_distinctly_from_constant_add() {
    let constant = compile(&machine_toml(
        r#"
[facts]
n = { type = "int", policy = "replace", initial = 1 }
m = { type = "int", policy = "replace", initial = 1 }
[states]
s = { initial = true }
t = {}
[commands.go]
from = ["s"]
to = "t"
when = "n + 1 == 2"
"#,
    ));
    let operand = compile(&machine_toml(
        r#"
[facts]
n = { type = "int", policy = "replace", initial = 1 }
m = { type = "int", policy = "replace", initial = 1 }
[states]
s = { initial = true }
t = {}
[commands.go]
from = ["s"]
to = "t"
when = "n + m == 2"
"#,
    ));
    assert_ne!(constant.spec_hash(), operand.spec_hash());
}

#[test]
fn each_new_form_passes_check_and_scenario() {
    let dir = tempfile::tempdir().expect("tmp");
    let machine = dir.path().join("derived.ik.toml");
    fs::write(
        &machine,
        r#"
schema_version = 1
machine_id = "derived_guard"
machine_version = 1
[facts]
remaining_qty = { type = "int", policy = "replace", initial = 10 }
amount = { type = "int", policy = "replace", initial = 175 }
units = { type = "int", policy = "replace", initial = 2 }
hold_ms = { type = "int", policy = "replace", initial = 5000 }
[states]
s = { initial = true }
t = { terminal = true }
[commands.go]
from = ["s"]
to = "t"
args = { remain_after = "int", take = "int", deposit_cents = "int", d = "int", now_ms = "int", priced = "int" }
when = "$remain_after == remaining_qty - $take && $deposit_cents == amount / 50 && $d == $now_ms + hold_ms && $priced == units * 175"
then = { remaining_qty = "$remain_after" }
"#,
    )
    .expect("machine");
    let corpus = dir.path().join("derived.cases.json");
    fs::write(
        &corpus,
        r#"{"commands":{"go":[{"name":"honest","args":{"remain_after":6,"take":4,"deposit_cents":3,"d":6000,"now_ms":1000,"priced":350}}]}}"#,
    )
    .expect("corpus");
    let profile = dir.path().join("checks.toml");
    fs::write(
        &profile,
        r#"
profile_version = 1
[limits]
max_nodes = 32
max_depth = 4
[[goals]]
id = "t"
kind = "reach_terminal"
terminal = "t"
required = true
"#,
    )
    .expect("profile");
    let scenario = dir.path().join("scenario.json");
    fs::write(
        &scenario,
        r#"
{
  "scenario_version": 1,
  "machine": "derived.ik.toml",
  "aggregate": "d1",
  "steps": [
    {
      "kind": "command",
      "label": "go",
      "name": "go",
      "args": {
        "remain_after": 6,
        "take": 4,
        "deposit_cents": 3,
        "d": 6000,
        "now_ms": 1000,
        "priced": 350
      },
      "expect": "committed"
    }
  ]
}
"#,
    )
    .expect("scenario");
    let out = bin()
        .args([
            "check",
            machine.to_str().unwrap(),
            "--profile",
            profile.to_str().unwrap(),
            "--corpus",
            corpus.to_str().unwrap(),
        ])
        .output()
        .expect("check");
    assert_eq!(
        out.status.code(),
        Some(0),
        "ik check: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let dest = dir.path().join("run");
    let out = bin()
        .args([
            "scenario",
            "run",
            machine.to_str().unwrap(),
            scenario.to_str().unwrap(),
            "--out",
            dest.to_str().unwrap(),
        ])
        .output()
        .expect("scenario");
    assert_eq!(
        out.status.code(),
        Some(0),
        "ik scenario run: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn shipped_machines_pass_check_and_scenario() {
    let dir = shipped_dir();
    let mut machines: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("examples/shipped: {e}"))
        .map(|e| e.expect("entry").path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".ik.toml"))
        })
        .collect();
    machines.sort();
    assert!(
        !machines.is_empty(),
        "examples/shipped must contain at least one .ik.toml"
    );
    let profile = dir.join("checks.toml");
    for machine in machines {
        parse_and_compile(&fs::read_to_string(&machine).expect("read"))
            .unwrap_or_else(|e| panic!("{}: {e:?}", machine.display()));
        let corpus = machine.with_extension("cases.json");
        let mut check = bin();
        check.arg("check").arg(&machine);
        let specific = machine.with_extension("checks.toml");
        if specific.exists() {
            check.arg("--profile").arg(&specific);
        } else if profile.exists() {
            check.arg("--profile").arg(&profile);
        }
        if corpus.exists() {
            check.arg("--corpus").arg(&corpus);
        }
        let out = check.output().expect("ik check");
        assert_eq!(
            out.status.code(),
            Some(0),
            "ik check {}: {}\n{}",
            machine.display(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let scenarios = dir.join("scenarios");
        if scenarios.is_dir() {
            for entry in fs::read_dir(&scenarios).expect("scenarios") {
                let path = entry.expect("entry").path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                if !scenario_targets(&path, &machine) {
                    continue;
                }
                let dest = tempfile::tempdir().expect("out");
                let out = bin()
                    .args([
                        "scenario",
                        "run",
                        machine.to_str().unwrap(),
                        path.to_str().unwrap(),
                        "--out",
                        dest.path().to_str().unwrap(),
                    ])
                    .output()
                    .expect("scenario");
                assert_eq!(
                    out.status.code(),
                    Some(0),
                    "ik scenario run {} {}: {}\n{}",
                    machine.display(),
                    path.display(),
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
            }
        }
    }
}

fn scenario_targets(scenario: &PathBuf, machine: &PathBuf) -> bool {
    let text = fs::read_to_string(scenario).expect("read scenario");
    let parsed: Value = serde_json::from_str(&text).expect("scenario json");
    let listed = parsed
        .get("machine")
        .and_then(Value::as_str)
        .expect("scenario machine");
    std::path::Path::new(listed).file_name() == machine.file_name()
}
