//! Registry-boundary behavior of the declarative CLI entrypoint: the compiled
//! machine is registered under its exact identity and every Store operation
//! (create/command/replay/verify) resolves the machine from the registry, so a
//! mismatched or unregistered identity fails closed with the stable `IK_MACHINE_*`
//! diagnostics while the registered identity succeeds.

#[path = "common/mod.rs"]
mod common;

use common::*;
use serde_json::{json, Value};
use std::path::Path;

fn envelope(op: &str, request_id: &str) -> Value {
    json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": "0.7.0",
        "request_id": request_id,
        "op": op,
    })
}

/// Run the `ik` binary once and parse its envelope. Each call is exactly one
/// process invocation (mutating ops must never run twice).
fn run_one(store: &Path, machine: Option<&Path>, request: &Value) -> (Value, i32) {
    let (stdout, code) = run_cli(store, machine, request);
    let response = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("ik did not emit a JSON envelope: {e}; raw: {stdout}"));
    (response, code)
}

/// The registered declarative machine drives create -> command -> replay ->
/// verify successfully, with every machine resolved from the registry.
#[test]
fn declarative_entrypoint_succeeds_through_registered_identity() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store.db");
    let machine = machine_def("verification_attempt");
    let identity = identity_of("verification_attempt");

    let mut create = envelope("create", "req-create");
    create["aggregate_id"] = json!("agg-1");
    create["machine"] = serde_json::to_value(&identity).unwrap();
    let (response, code) = run_one(&store, Some(&machine), &create);
    assert_eq!(code, 0, "create exit; {response}");
    assert_eq!(response["ok"], json!(true), "{response}");
    let capability = response["result"]["capability"].clone();

    let mut cmd = envelope("command", "req-command");
    cmd["aggregate_id"] = json!("agg-1");
    cmd["machine"] = serde_json::to_value(&identity).unwrap();
    cmd["capability"] = capability;
    cmd["command"] = json!({"name": "start"});
    let (response, code) = run_one(&store, Some(&machine), &cmd);
    assert_eq!(code, 0, "command exit; {response}");
    assert_eq!(response["ok"], json!(true), "{response}");

    let mut replay = envelope("replay", "req-replay");
    replay["aggregate_id"] = json!("agg-1");
    let (response, code) = run_one(&store, Some(&machine), &replay);
    assert_eq!(code, 0, "replay exit; {response}");
    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(response["result"]["aggregates"][0]["rebuilt"], json!(false));

    let mut verify = envelope("verify", "req-verify");
    verify["aggregate_id"] = json!("agg-1");
    let (response, code) = run_one(&store, Some(&machine), &verify);
    assert_eq!(code, 0, "verify exit; {response}");
    assert_eq!(response["ok"], json!(true), "{response}");
    assert_eq!(response["result"]["verified"], json!(true));
}

/// Every identity deviation (unknown id, unknown version, spec-hash mismatch)
/// fails closed with the stable classification; a command presenting a foreign
/// machine identity refuses; and replay with no registered machine refuses.
#[test]
fn declarative_entrypoint_fails_closed_on_mismatched_or_unregistered_identity() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store.db");
    let machine = machine_def("verification_attempt");
    let identity = identity_of("verification_attempt");
    let base = serde_json::to_value(&identity).unwrap();

    // Unknown machine id.
    let mut wrong_id = base.clone();
    wrong_id["machine_id"] = json!("someone.else");
    let mut req = envelope("create", "req-id");
    req["aggregate_id"] = json!("agg-id");
    req["machine"] = wrong_id;
    let (response, code) = run_one(&store, Some(&machine), &req);
    assert_eq!(code, 1, "{response}");
    assert_eq!(
        response["diagnostic"]["code"].as_str(),
        Some("IK_MACHINE_UNKNOWN")
    );

    // Unknown version.
    let mut wrong_version = base.clone();
    wrong_version["version"] = json!(identity.version.0 + 1);
    let mut req = envelope("create", "req-version");
    req["aggregate_id"] = json!("agg-version");
    req["machine"] = wrong_version;
    let (response, code) = run_one(&store, Some(&machine), &req);
    assert_eq!(code, 1, "{response}");
    assert_eq!(
        response["diagnostic"]["code"].as_str(),
        Some("IK_MACHINE_VERSION_UNKNOWN")
    );

    // Spec-hash mismatch.
    let mut wrong_hash = base.clone();
    wrong_hash["spec_hash"] = json!("0".repeat(64));
    let mut req = envelope("create", "req-hash");
    req["aggregate_id"] = json!("agg-hash");
    req["machine"] = wrong_hash;
    let (response, code) = run_one(&store, Some(&machine), &req);
    assert_eq!(code, 1, "{response}");
    assert_eq!(
        response["diagnostic"]["code"].as_str(),
        Some("IK_MACHINE_HASH_MISMATCH")
    );

    // None of the refused creates wrote an aggregate: the store verifies clean.
    let verify = envelope("verify", "req-verify");
    let (response, code) = run_one(&store, Some(&machine), &verify);
    assert_eq!(code, 0, "verify on empty store; {response}");
    assert_eq!(response["result"]["aggregates"], json!(0));

    // A committed aggregate refuses a command presenting a foreign machine's
    // identity: IK_MACHINE_MISMATCH against the recorded identity.
    let mut create = envelope("create", "req-create");
    create["aggregate_id"] = json!("agg-1");
    create["machine"] = serde_json::to_value(&identity).unwrap();
    let (response, code) = run_one(&store, Some(&machine), &create);
    assert_eq!(code, 0, "create exit; {response}");
    let capability = response["result"]["capability"].clone();
    let mut cmd = envelope("command", "req-foreign");
    cmd["aggregate_id"] = json!("agg-1");
    cmd["machine"] = serde_json::to_value(identity_of("effect_probe")).unwrap();
    cmd["capability"] = capability;
    cmd["command"] = json!({"name": "start"});
    let (response, code) = run_one(&store, Some(&machine), &cmd);
    assert_eq!(code, 1, "{response}");
    assert_eq!(
        response["diagnostic"]["code"].as_str(),
        Some("IK_MACHINE_MISMATCH")
    );

    // Replay without --machine means nothing is registered: the unregistered
    // path fails closed before any Store call.
    let mut replay = envelope("replay", "req-replay");
    replay["aggregate_id"] = json!("agg-1");
    let (response, code) = run_one(&store, None, &replay);
    assert_eq!(code, 1, "{response}");
    assert_eq!(
        response["diagnostic"]["code"].as_str(),
        Some("IK_PROTOCOL_INVALID")
    );
    assert_eq!(
        response["diagnostic"]["rule"].as_str(),
        Some("request.Replay.machine")
    );
}
