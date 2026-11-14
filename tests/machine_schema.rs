//! IK-08: `ik schema` is a stable, machine-derived projection for code generators.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

fn machine(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("machines")
        .join(format!("{name}.ik.toml"))
}

fn schema_bytes(name: &str) -> Vec<u8> {
    let out = Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("schema")
        .arg(machine(name))
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn schema(name: &str) -> Value {
    serde_json::from_slice(&schema_bytes(name)).unwrap()
}

fn named<'a>(rows: &'a Value, key: &str, name: &str) -> &'a Value {
    rows[key]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("no {key} named {name}"))
}

#[test]
fn the_schema_carries_the_authoritative_machine_identity() {
    let schema = schema("verification_attempt");
    let identity = Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("identity")
        .arg(machine("verification_attempt"))
        .output()
        .unwrap();
    let identity: Value = serde_json::from_slice(&identity.stdout).unwrap();
    assert_eq!(schema["schema"], "ikaros.machine-schema.v1");
    assert_eq!(schema["machine"]["machine_id"], identity["machine_id"]);
    assert_eq!(schema["machine"]["version"], identity["version"]);
    assert_eq!(schema["machine"]["spec_hash"], identity["spec_hash"]);
}

#[test]
fn facts_without_an_initial_value_are_not_required() {
    let schema = schema("verification_attempt");
    assert_eq!(named(&schema, "facts", "attempt_count")["required"], true);
    assert_eq!(named(&schema, "facts", "max_attempts")["required"], true);
    assert_eq!(
        named(&schema, "facts", "artifact_digest")["required"],
        false,
        "a fact set by a later command is conditionally absent and must not be modelled as total"
    );
}

#[test]
fn a_command_with_several_outcomes_has_a_conditional_successor_union() {
    let schema = schema("verification_attempt");
    let reject = named(&schema, "commands", "reject");
    assert_eq!(reject["conditional_successor"], true);
    assert_eq!(
        reject["successors"],
        serde_json::json!(["repairable", "exhausted"])
    );
    let start = named(&schema, "commands", "start");
    assert_eq!(start["conditional_successor"], false);
    assert_eq!(start["successors"], serde_json::json!(["running"]));
    let submit = named(&schema, "commands", "submit");
    assert_eq!(
        submit["args"],
        serde_json::json!([
            {"name": "artifact_digest", "type": "string"},
            {"name": "baseline_digest", "type": "string"}
        ])
    );
}

#[test]
fn the_export_is_deterministic_and_leaks_no_guard_logic() {
    let first = schema_bytes("verification_attempt");
    let second = schema_bytes("verification_attempt");
    assert_eq!(first, second);
    let text = String::from_utf8(first).unwrap();
    assert!(
        !text.contains("attempt_count + 1"),
        "guard expressions are not schema"
    );
    assert!(!text.contains("assert"), "invariant bodies are not schema");
}

#[test]
fn a_missing_machine_is_a_usage_failure_not_an_empty_schema() {
    let out = Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("schema")
        .arg("machines/does_not_exist.ik.toml")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}
