//! K2 — Host-neutral protocol over the public CLI.
//!
//! Defends: strict versioned request/response over subprocess stdin/stdout; unknown
//! protocol major refuses; machine hash/version mismatches refuse; canonical
//! normalized outputs are byte-stable for identical accepted requests; the transport
//! layer carries no host lifecycle semantics (the closed request envelope refuses
//! lifecycle/destination-shaped fields outright).

#[path = "common/mod.rs"]
mod common;

use common::*;
use serde_json::{json, Value};

fn va_machine() -> Box<dyn ik::machine::Machine> {
    Box::new(load_machine("verification_attempt"))
}

fn fresh_store() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = dir.path().join("s.db");
    (dir, store)
}

fn create_request(_store: &std::path::Path) -> Value {
    let machine = va_machine();
    let identity = serde_json::to_value(machine.identity()).expect("identity json");
    json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": "0.6.0",
        "request_id": "c1",
        "op": "create",
        "aggregate_id": "agg-1",
        "machine": identity,
    })
}

#[test]
fn strict_versioned_roundtrip_over_subprocess() {
    let (dir, store) = fresh_store();
    let machine = machine_def("verification_attempt");
    let request = create_request(&store);
    let (stdout, code) = run_cli(&store, Some(&machine), &request);
    assert_eq!(code, 0, "create exits 0: {stdout}");
    let response: Value = serde_json::from_str(&stdout).expect("JSON envelope");
    assert_eq!(response["ok"], true);
    assert_eq!(response["request_id"], "c1");
    assert_eq!(response["protocol"]["major"], 1);
    assert_eq!(response["result"]["generation"], 0);
    let _ = dir;
}

#[test]
fn unknown_major_protocol_refuses() {
    let (dir, store) = fresh_store();
    let machine = machine_def("verification_attempt");
    let mut request = create_request(&store);
    request["protocol"]["major"] = json!(2);
    let (stdout, code) = run_cli(&store, Some(&machine), &request);
    assert_eq!(code, 1, "unknown major exits 1: {stdout}");
    let response: Value = serde_json::from_str(&stdout).expect("JSON envelope");
    assert_eq!(response["ok"], false);
    assert_eq!(
        response["diagnostic"]["code"],
        "IK_PROTOCOL_VERSION_UNKNOWN"
    );
    let _ = dir;
}

#[test]
fn unknown_runtime_version_refuses() {
    let (dir, store) = fresh_store();
    let machine = machine_def("verification_attempt");
    let mut request = create_request(&store);
    request["runtime_version"] = json!("9.9.9");
    let (stdout, code) = run_cli(&store, Some(&machine), &request);
    assert_eq!(code, 1, "unknown runtime version exits 1: {stdout}");
    let response: Value = serde_json::from_str(&stdout).expect("JSON envelope");
    assert_eq!(
        response["diagnostic"]["code"],
        "IK_PROTOCOL_VERSION_UNKNOWN"
    );
    let _ = dir;
}

#[test]
fn machine_identity_mismatches_refuse_with_stable_codes() {
    let (dir, store) = fresh_store();
    let machine = machine_def("verification_attempt");
    let mut request = create_request(&store);
    // Unknown machine id.
    request["machine"]["machine_id"] = json!("ghost");
    let (stdout, _) = run_cli(&store, Some(&machine), &request);
    let response: Value = serde_json::from_str(&stdout).expect("JSON envelope");
    assert_eq!(response["diagnostic"]["code"], "IK_MACHINE_UNKNOWN");

    // Unknown version.
    let mut request = create_request(&store);
    request["machine"]["version"] = json!(99);
    let (stdout, _) = run_cli(&store, Some(&machine), &request);
    let response: Value = serde_json::from_str(&stdout).expect("JSON envelope");
    assert_eq!(response["diagnostic"]["code"], "IK_MACHINE_VERSION_UNKNOWN");

    // Hash mismatch.
    let mut request = create_request(&store);
    request["machine"]["spec_hash"] = json!("deadbeef");
    let (stdout, _) = run_cli(&store, Some(&machine), &request);
    let response: Value = serde_json::from_str(&stdout).expect("JSON envelope");
    assert_eq!(response["diagnostic"]["code"], "IK_MACHINE_HASH_MISMATCH");
    let _ = dir;
}

#[test]
fn malformed_and_unknown_field_requests_fail_closed() {
    let (dir, store) = fresh_store();
    let machine = machine_def("verification_attempt");

    // Malformed raw JSON is covered by the protocol_refusals corpus trace; here we
    // send a valid envelope with an unknown op instead.
    let request = json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": "0.6.0",
        "request_id": "x",
        "op": "frobnicate",
    });
    let (stdout, code) = run_cli(&store, Some(&machine), &request);
    assert_eq!(code, 1);
    let response: Value = serde_json::from_str(&stdout).expect("JSON envelope");
    assert_eq!(response["diagnostic"]["code"], "IK_PROTOCOL_INVALID");
    let _ = dir;
}

#[test]
fn canonical_responses_are_byte_stable() {
    // Two identical stores, identical accepted requests -> byte-identical responses
    // (deterministic nonces, canonical JSON).
    let machine = va_machine();
    let identity = serde_json::to_value(machine.identity()).expect("identity json");
    let (dir1, store1) = fresh_store();
    let (dir2, store2) = fresh_store();
    let def = machine_def("verification_attempt");

    let create = |store: &std::path::Path| -> String {
        let request = json!({
            "protocol": {"major": 1, "minor": 0},
            "runtime_version": "0.6.0",
            "request_id": "c",
            "op": "create",
            "aggregate_id": "agg-1",
            "machine": identity,
        });
        run_cli(store, Some(&def), &request).0
    };
    let out1 = create(&store1);
    let out2 = create(&store2);
    assert_eq!(out1, out2, "create responses must be byte-identical");

    // A command stream on the two stores is byte-identical too.
    let stream = |store: &std::path::Path, capability: Value| -> String {
        let request = json!({
            "protocol": {"major": 1, "minor": 0},
            "runtime_version": "0.6.0",
            "request_id": "s",
            "op": "command",
            "aggregate_id": "agg-1",
            "machine": identity,
            "capability": capability,
            "command": {"name": "start"},
        });
        run_cli(store, Some(&def), &request).0
    };
    let parsed1: Value = serde_json::from_str(&out1).expect("envelope");
    let parsed2: Value = serde_json::from_str(&out2).expect("envelope");
    let cap1 = parsed1["result"]["capability"].clone();
    let cap2 = parsed2["result"]["capability"].clone();
    assert_eq!(stream(&store1, cap1), stream(&store2, cap2));
    let _ = (dir1, dir2);
}

#[test]
fn canonical_json_shape_is_strict() {
    // Sorted keys, no whitespace, omitted absent optionals.
    let sample = json!({"b": 1, "a": "x"});
    assert_eq!(ik::canonical::canonical_json(&sample), r#"{"a":"x","b":1}"#);
    // The response envelope omits absent optional fields.
    let response = ik::protocol::EnvelopeResponse::failure(
        "r",
        ik::diagnostic::Diagnostic::new(
            ik::diagnostic::DiagnosticCode::IkCapabilityStale,
            Some("capability.generation"),
            "stale",
        ),
    );
    // Parse the canonical output and assert the exact envelope field set and
    // values (no substring matching): absent optionals are omitted, present
    // fields carry exact values, and the failure half of the envelope is closed.
    let text = ik::canonical::canonical_json(&response);
    let parsed: Value = serde_json::from_str(&text).expect("canonical JSON must parse");
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["request_id"], "r");
    assert_eq!(
        parsed["protocol"]["major"].as_u64(),
        Some(ik::protocol::PROTOCOL_MAJOR)
    );
    assert_eq!(parsed["runtime_version"], ik::protocol::RUNTIME_VERSION);
    assert!(
        parsed.get("result").is_none(),
        "absent `result` must be omitted from the failure envelope"
    );
    assert_eq!(parsed["diagnostic"]["code"], "IK_CAPABILITY_STALE");
    assert_eq!(parsed["diagnostic"]["rule"], "capability.generation");
    assert_eq!(parsed["diagnostic"]["message"], "stale");
    let mut keys: Vec<&str> = parsed
        .as_object()
        .expect("envelope is an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "diagnostic",
            "ok",
            "protocol",
            "request_id",
            "runtime_version"
        ]
    );

    // The success half is the mirror image: `result` instead of `diagnostic`.
    let ok = ik::protocol::EnvelopeResponse::success("r", json!({"generation": 0}));
    let ok_text = ik::canonical::canonical_json(&ok);
    let ok_parsed: Value = serde_json::from_str(&ok_text).expect("canonical JSON must parse");
    assert!(ok_parsed.get("diagnostic").is_none());
    assert_eq!(ok_parsed["result"]["generation"], 0);
    let mut ok_keys: Vec<&str> = ok_parsed
        .as_object()
        .expect("envelope is an object")
        .keys()
        .map(String::as_str)
        .collect();
    ok_keys.sort_unstable();
    assert_eq!(
        ok_keys,
        vec!["ok", "protocol", "request_id", "result", "runtime_version"]
    );
}

#[test]
fn transport_rejects_lifecycle_and_destination_fields() {
    // The request envelope is closed (`deny_unknown_fields`), so any
    // lifecycle/destination/host-vocabulary-shaped field a stateful transport
    // would carry is refused outright: the transport cannot acquire host
    // lifecycle or consumer/approval semantics from the wire.
    let (dir, store) = fresh_store();
    let machine = machine_def("verification_attempt");
    for (field, value) in [
        ("ready", json!(true)),
        ("readiness", json!("ready")),
        ("terminal_disposition", json!("success")),
        ("next_action", json!("approve")),
        ("workflow", json!({"step": "approve"})),
        ("scheduler", json!("cron")),
        ("principal", json!("alice")),
        ("approval", json!({"by": "alice"})),
        ("package_id", json!("pkg-1")),
    ] {
        let mut request = create_request(&store);
        request[field] = value;
        let (stdout, code) = run_cli(&store, Some(&machine), &request);
        assert_eq!(code, 1, "{field} must fail closed: {stdout}");
        let response: Value = serde_json::from_str(&stdout).expect("JSON envelope");
        assert_eq!(
            response["diagnostic"]["code"], "IK_PROTOCOL_INVALID",
            "{field} must be refused by the closed envelope"
        );
    }
    let _ = dir;
}

#[test]
fn partial_effect_refusal_echoes_request_id() {
    let (dir, store) = fresh_store();
    let machine = machine_def("effect_probe");
    let request = json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": "0.6.0",
        "request_id": "partial-effect",
        "op": "record",
        "aggregate_id": "agg-1",
        "effect": {"effect_id": "e1"},
    });
    let (stdout, code) = run_cli(&store, Some(&machine), &request);
    assert_eq!(code, 1, "{stdout}");
    let response: Value = serde_json::from_str(&stdout).expect("JSON envelope");
    assert_eq!(response["ok"], false);
    assert_eq!(response["request_id"], "partial-effect");
    assert_eq!(response["diagnostic"]["code"], "IK_PROTOCOL_INVALID");
    let _ = dir;
}
