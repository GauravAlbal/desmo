//! Generic conformance corpus runner (K2 / M3): every trace in
//! `conformance/traces/` is executed through the public `ik` CLI binary over
//! subprocess stdin/stdout, asserting the contract-derived patterns in
//! `conformance/expected/`. Corruption fixtures in `conformance/mutations/` mutate
//! the SQLite storage directly and assert that `ik verify` refuses.

#[path = "common/mod.rs"]
mod common;

use common::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

fn fixture(name: &str) -> Value {
    let path = repo_root().join("conformance").join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("fixture is valid JSON")
}

fn json_names(dir: &str) -> Vec<String> {
    let dir = repo_root().join("conformance").join(dir);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("conformance dir exists")
        .map(|e| e.expect("readable dir entry").path())
        .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
        .map(|p| p.file_stem().unwrap().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

/// Machine identities for every machine definition in `machines/`.
fn machine_identities() -> serde_json::Map<String, Value> {
    let mut ctx = serde_json::Map::new();
    for id in ["verification_attempt", "effect_probe"] {
        let identity = identity_of(id);
        ctx.insert(id.to_string(), serde_json::to_value(&identity).unwrap());
    }
    ctx
}

/// Run one trace against a fresh store at `store`. Returns the accumulated context
/// (responses) for the mutation runner.
fn run_trace(
    trace_name: &str,
    store: &Path,
    identities: &serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    let trace = fixture(&format!("traces/{trace_name}.json"));
    let expected = fixture(&format!("expected/{trace_name}.json"));
    let machine_id = trace["machine"].as_str().expect("trace declares a machine");
    let machine_path = machine_def(machine_id);

    let mut ctx = serde_json::Map::new();
    ctx.insert("machine".to_string(), identities[machine_id].clone());
    ctx.insert("machines".to_string(), Value::Object(identities.clone()));
    ctx.insert("steps".to_string(), Value::Array(Vec::new()));

    let steps = trace["steps"].as_array().expect("steps array");
    let expected_steps = expected["steps"].as_array().expect("expected steps array");
    assert_eq!(
        steps.len(),
        expected_steps.len(),
        "trace {trace_name}: step count mismatch"
    );

    for (i, (step, expect)) in steps.iter().zip(expected_steps.iter()).enumerate() {
        let (stdout, exit_code) = if let Some(raw) = step.get("raw") {
            let raw = raw.as_str().expect("raw input is a string");
            let bin = env!("CARGO_BIN_EXE_ik");
            let mut child = std::process::Command::new(bin)
                .arg("--store")
                .arg(store)
                .arg("--machine")
                .arg(&machine_path)
                .arg("command")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn ik");
            child
                .stdin
                .as_mut()
                .expect("stdin")
                .write_all(raw.as_bytes())
                .expect("write raw");
            let output = child.wait_with_output().expect("wait ik");
            (
                String::from_utf8_lossy(&output.stdout).to_string(),
                output.status.code().unwrap_or(-1),
            )
        } else {
            let mut request = resolve_refs(step, &ctx);
            // Inject the machine identity for mutating ops that do not name one.
            if matches!(
                request["op"].as_str(),
                Some("create" | "command" | "record")
            ) && request.get("machine").is_none()
            {
                request["machine"] = identities[machine_id].clone();
            }
            run_cli(store, Some(&machine_path), &request)
        };

        let response: Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
            panic!(
                "trace {trace_name} step {i}: ik did not emit a JSON envelope: {e}; raw: {stdout}"
            )
        });
        let mut expected_pattern = resolve_refs(expect, &ctx);
        // The exit code is asserted independently of the envelope body: expected
        // steps declare it explicitly (success exits 0, refusal exits 1).
        let expected_exit = expected_pattern
            .get("exit")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| {
                panic!("trace {trace_name} step {i}: expected step must declare `exit`")
            }) as i32;
        if let Some(obj) = expected_pattern.as_object_mut() {
            obj.remove("exit");
        }
        assert_pattern(
            &response,
            &expected_pattern,
            &format!("trace {trace_name} step {i}"),
        );
        assert_eq!(
            exit_code, expected_exit,
            "trace {trace_name} step {i}: unexpected exit code"
        );

        if i == 0 {
            ctx.insert("create".to_string(), response.clone());
        }
        if let Some(steps_arr) = ctx.get_mut("steps").and_then(|v| v.as_array_mut()) {
            steps_arr.push(response);
        }
    }
    ctx
}

#[test]
fn generic_corpus_passes_through_cli() {
    let identities = machine_identities();
    let names = json_names("traces");
    assert!(
        names.len() >= 18,
        "the spec section-16 corpus requires >= 18 trace classes, found {}",
        names.len()
    );
    for name in &names {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = dir.path().join("store.db");
        run_trace(name, &store, &identities);
    }
}

#[test]
fn corruption_fixtures_mutate_storage_then_verify_refuses() {
    let identities = machine_identities();
    for name in json_names("mutations") {
        let fixture = fixture(&format!("mutations/{name}.json"));
        let trace_name = fixture["trace"].as_str().expect("mutation names a trace");
        let machine_id = fixture["machine"]
            .as_str()
            .unwrap_or("verification_attempt");
        let dir = tempfile::tempdir().expect("tempdir");
        let store = dir.path().join("store.db");
        run_trace(trace_name, &store, &identities);

        // Apply the mutations directly to the SQLite storage.
        let conn = rusqlite::Connection::open(&store).expect("open store for mutation");
        for mutation in fixture["mutations"].as_array().expect("mutations array") {
            let sql = mutation["sql"].as_str().expect("mutation sql");
            conn.execute_batch(sql).expect("apply mutation");
        }
        drop(conn);

        // `ik verify` must refuse with the expected stable code.
        let machine_path = machine_def(machine_id);
        let request = json!({
            "protocol": {"major": 1, "minor": 0},
            "runtime_version": "0.7.0",
            "request_id": "verify",
            "op": "verify",
        });
        let response = run_cli_json(&store, Some(&machine_path), &request);
        let (_, exit_code) = run_cli(&store, Some(&machine_path), &request);
        assert_eq!(
            exit_code, 1,
            "mutation fixture {name}: verify refusal must exit 1"
        );
        let expected_code = fixture["expect_code"].as_str().expect("expect_code");
        assert_eq!(
            response["diagnostic"]["code"].as_str(),
            Some(expected_code),
            "mutation fixture {name}: expected {expected_code}, got {response}"
        );
    }
}

#[test]
fn every_spec_section16_class_has_a_fixture() {
    // Spec §16 names these trace classes; the corpus must cover every one.
    let required = [
        "happy_verification_attempt",
        "wrong_artifact_verification",
        "wrong_baseline_verification",
        "repair_below_limit",
        "exhaustion_at_limit",
        "cancel_from_allowed_states",
        "terminal_mutation_refusal",
        "stale_capability",
        "copied_stale_capability",
        "modified_nonce",
        "wrong_machine_version",
        "malformed_command",
        "concurrent_same_generation_race",
        "replay_equality",
        "projection_corruption",
        "journal_corruption",
        "wrong_pending_effect_id",
        "stale_effect_outcome",
    ];
    let traces: BTreeSet<String> = json_names("traces").into_iter().collect();
    let mutations: BTreeSet<String> = json_names("mutations").into_iter().collect();
    for class in required {
        let covered = traces.contains(class) || mutations.contains(class);
        assert!(
            covered,
            "spec §16 class `{class}` has no conformance fixture"
        );
    }
}
