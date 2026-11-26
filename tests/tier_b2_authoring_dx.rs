//! Tier B2 integration tests: JSON Schema validation structure, native TOML bool/int `then`
//! literals preserving `spec_hash`, and checked `i64` arithmetic in `then` assignments.

use ik::declarative::{parse_and_compile, parse_definition};
use ik::identity::AggregateId;
use ik::machine::Machine;
use ik::store::{CommandRequest, Store};
use serde_json::{json, Value};
use std::path::Path;

#[test]
fn schema_file_is_valid_draft07_covering_schema_1_and_2() {
    let raw = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/ik.schema.json"),
    )
    .expect("read schema/ik.schema.json");
    let v: Value = serde_json::from_str(&raw).expect("valid JSON");
    assert_eq!(v["$schema"], "http://json-schema.org/draft-07/schema#");
    assert!(v["oneOf"].as_array().is_some_and(|a| a.len() == 2));
    assert!(v["definitions"]["FlatMachine"]["properties"]["states"].is_object());
    assert!(v["definitions"]["RegionMachine"]["properties"]["regions"].is_object());
    assert!(v["definitions"]["ThenMap"]["additionalProperties"]["oneOf"].is_array());
}

#[test]
fn native_toml_literals_preserve_exact_spec_hash_and_runtime_semantics() {
    let quoted = r#"
schema_version = 1
machine_id = "native_lit"
machine_version = 1

[facts]
approved = { type = "bool", policy = "replace" }
retries = { type = "int", policy = "replace" }
note = { type = "string", policy = "replace" }

[states]
draft = { initial = true }
done = { terminal = true }

[commands.finish]
from = ["draft"]
to = "done"
then = { approved = "true", retries = "3", note = '"ok"' }
"#;

    let native = r#"
schema_version = 1
machine_id = "native_lit"
machine_version = 1

[facts]
approved = { type = "bool", policy = "replace" }
retries = { type = "int", policy = "replace" }
note = { type = "string", policy = "replace" }

[states]
draft = { initial = true }
done = { terminal = true }

[commands.finish]
from = ["draft"]
to = "done"
then = { approved = true, retries = 3, note = '"ok"' }
"#;

    let m_quoted = parse_and_compile(quoted).expect("quoted compiles");
    let m_native = parse_and_compile(native).expect("native compiles");
    assert_eq!(
        m_quoted.identity().spec_hash,
        m_native.identity().spec_hash,
        "native bool/int in `then` must hash identically to quoted forms"
    );

    let ev = m_native
        .decide(
            &m_native.initial_state(),
            None,
            &ik::effect::EffectStatus::None,
            &json!({ "name": "finish", "args": {} }),
        )
        .expect("decide");
    let ik::machine::Decision::Emit { event, .. } = ev else {
        panic!("expected emit");
    };
    let applied = m_native
        .apply(&m_native.initial_state(), &event)
        .expect("apply");
    assert_eq!(applied.state["approved"], true);
    assert_eq!(applied.state["retries"], 3);
    assert_eq!(applied.state["note"], "ok");

    // Also works in schema-2 joint commands!
    let s2 = r#"
schema_version = 2
machine_id = "joint_native"
machine_version = 1

[facts]
flag = { type = "bool", policy = "replace" }
qty = { type = "int", policy = "replace" }

[regions.a.states.a0]
initial = true
[regions.a.states.a1]
terminal = true

[regions.b.states.b0]
initial = true
[regions.b.states.b1]
terminal = true

[commands.go]
from = { a = ["a0"], b = ["b0"] }
to = { a = "a1", b = "b1" }
then = { flag = true, qty = 7 }
"#;
    let m_s2 = parse_and_compile(s2).expect("schema-2 native literals compile");
    let ev2 = m_s2
        .decide(
            &m_s2.initial_state(),
            None,
            &ik::effect::EffectStatus::None,
            &json!({ "name": "go", "args": {} }),
        )
        .expect("decide s2");
    let ik::machine::Decision::Emit { event: event2, .. } = ev2 else {
        panic!("expected emit s2");
    };
    let applied2 = m_s2.apply(&m_s2.initial_state(), &event2).expect("apply s2");
    assert_eq!(applied2.state["flag"], true);
    assert_eq!(applied2.state["qty"], 7);
}

#[test]
fn checked_then_arithmetic_evaluates_simultaneously_and_refuses_overflow() {
    let toml = r#"
schema_version = 1
machine_id = "inventory"
machine_version = 1

[facts]
remaining = { type = "int", policy = "replace", initial = 10 }
reserved = { type = "int", policy = "replace", initial = 0 }
total_cents = { type = "int", policy = "replace", initial = 0 }

[states]
open = { initial = true }
locked = {}
overflowed = { terminal = true }

[commands.reserve]
from = ["open", "locked"]
to = "locked"
args = { units = "int" }
when = "$units > 0 && remaining >= $units"
then = { remaining = "remaining - $units", reserved = "reserved + $units", total_cents = "$units * 250" }

[commands.bump]
from = ["open", "locked"]
to = "open"
args = { x = "int" }
then = { remaining = "$x + 1" }
"#;

    let machine = parse_and_compile(toml).expect("inventory compiles");
    let id = machine.identity().clone();
    let db = std::env::temp_dir().join(format!(
        "desmo-tier-b2-arith-{}.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&db);
    let mut store = Store::open_or_create(&db).unwrap();

    let created = store
        .create_aggregate(&machine, &AggregateId("inv-1".into()), &id)
        .unwrap();
    let after1 = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: AggregateId("inv-1".into()),
                machine: id.clone(),
                capability: created.capability,
                command: json!({ "name": "reserve", "args": { "units": 3 } }),
                observation: None,
            },
        )
        .unwrap();
    assert_eq!(after1.state["remaining"], 7);
    assert_eq!(after1.state["reserved"], 3);
    assert_eq!(after1.state["total_cents"], 750);

    // Overflow in `then` refuses cleanly with IK_INVARIANT_FAILED and leaves state untouched.
    let overflow_err = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: AggregateId("inv-1".into()),
                machine: id.clone(),
                capability: after1.capability,
                command: json!({ "name": "bump", "args": { "x": i64::MAX } }),
                observation: None,
            },
        )
        .expect_err("i64::MAX + 1 in then must refuse");
    assert_eq!(overflow_err.code.as_str(), "IK_INVARIANT_FAILED");
    assert!(
        overflow_err.message.contains("integer arithmetic overflow"),
        "msg={}",
        overflow_err.message
    );
    let fresh = store.show(&AggregateId("inv-1".into())).unwrap();
    assert_eq!(fresh.state["remaining"], 7);

    // Static check rejects non-int arithmetic and unknown args in `then`.
    let bad_type = r#"
schema_version = 1
machine_id = "bad_arith"
machine_version = 1
[facts]
label = { type = "string", policy = "replace" }
[states]
a = { initial = true }
b = {}
[commands.go]
from = ["a"]
to = "b"
args = { n = "int" }
then = { label = "$n + 1" }
"#;
    let errs = parse_definition(bad_type).expect_err("string fact with arithmetic then must fail");
    assert!(
        errs.iter().any(|e| e.contains("requires an `int` fact")),
        "errs={errs:?}"
    );
    let _ = std::fs::remove_file(&db);
}
