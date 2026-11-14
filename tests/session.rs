//! `ik --session` — normative, hermetic, sleep-free tests for IK-PROP-010/011 and the session
//! protocol. Every test drives a REAL `ik --session` child over its public stdin/stdout protocol
//! against a fresh temporary store and the repo's own declarative machines (no Python, no network,
//! no ambient state, no wall-clock dependence).
//!
//! ```text
//! IK-PROP-010 transport cannot change committed outcomes: for every conformance trace the
//!             session response stream is byte-identical to the one-shot stream  -> ik_prop_010_*
//! IK-PROP-011 a long-lived session never accepts a stale capability issued before another
//!             writer's commit (CORE_LAWS-3/4 hold across sessions)               -> ik_prop_011_*
//! ```
//!
//! Protocol walls: a malformed/unknown request fails closed without ending the session; protocol
//! and runtime-version mismatches refuse; EOF exits 0; SIGKILL leaves exactly the committed state.

#[path = "common/mod.rs"]
mod common;

use common::*;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

fn envelope(op: &str, request_id: &str, extra: Value) -> Value {
    let mut request = json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": ik::protocol::RUNTIME_VERSION,
        "request_id": request_id,
        "op": op,
    });
    for (key, value) in extra.as_object().expect("extra is an object") {
        request[key] = value.clone();
    }
    request
}

fn corpus_file(relative: &str) -> Value {
    let path = repo_root().join("conformance").join(relative);
    serde_json::from_str(&std::fs::read_to_string(&path).expect("corpus file readable"))
        .expect("corpus file is JSON")
}

fn trace_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(repo_root().join("conformance/traces"))
        .expect("traces dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| path.file_stem().unwrap().to_string_lossy().to_string())
        .collect();
    names.sort();
    names
}

enum Step<'a> {
    Raw(&'a str),
    Request(Value),
}

/// Replays a conformance trace through `send`, which returns the raw response line.
fn drive_trace(trace_name: &str, send: &mut dyn FnMut(Step) -> String) -> Vec<String> {
    let trace = corpus_file(&format!("traces/{trace_name}.json"));
    let machine_id = trace["machine"]
        .as_str()
        .expect("trace machine")
        .to_string();
    let identity = serde_json::to_value(identity_of(&machine_id)).unwrap();
    let mut ctx = serde_json::Map::new();
    ctx.insert("machine".to_string(), identity.clone());
    ctx.insert("steps".to_string(), Value::Array(Vec::new()));
    let mut lines = Vec::new();
    for (index, step) in trace["steps"].as_array().expect("steps").iter().enumerate() {
        let line = if let Some(raw) = step.get("raw") {
            send(Step::Raw(raw.as_str().expect("raw is a string")))
        } else {
            let mut request = resolve_refs(step, &ctx);
            if matches!(
                request["op"].as_str(),
                Some("create" | "command" | "record")
            ) && request.get("machine").is_none()
            {
                request["machine"] = identity.clone();
            }
            send(Step::Request(request))
        };
        let response: Value = serde_json::from_str(&line)
            .unwrap_or_else(|e| panic!("trace {trace_name} step {index}: not JSON: {e}: {line}"));
        if index == 0 {
            ctx.insert("create".to_string(), response.clone());
        }
        ctx.get_mut("steps")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push(response);
        lines.push(line);
    }
    lines
}

fn one_shot_line(store: &Path, machine: &Path, step: Step) -> String {
    match step {
        Step::Request(request) => run_cli(store, Some(machine), &request)
            .0
            .trim_end()
            .to_string(),
        Step::Raw(raw) => {
            let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_ik"))
                .arg("--store")
                .arg(store)
                .arg("--machine")
                .arg(machine)
                .arg("command")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn ik");
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(raw.as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            String::from_utf8_lossy(&output.stdout)
                .trim_end()
                .to_string()
        }
    }
}

/// `store_id` is a per-store random UUID by design; it is the only field allowed to differ
/// between two independent stores, so it is the only field normalized.
fn normalized(line: &str) -> String {
    fn scrub(value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, inner) in map.iter_mut() {
                    if key == "store_id" {
                        *inner = json!("<store>");
                    } else {
                        scrub(inner);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(scrub),
            _ => {}
        }
    }
    let mut value: Value = serde_json::from_str(line).expect("response is JSON");
    scrub(&mut value);
    serde_json::to_string(&value).unwrap()
}

#[test]
fn ik_prop_010_session_matches_one_shot_over_the_conformance_corpus() {
    let names = trace_names();
    assert!(
        names.len() >= 18,
        "the corpus must keep >= 18 trace classes"
    );
    for name in names {
        let machine_id = corpus_file(&format!("traces/{name}.json"))["machine"]
            .as_str()
            .unwrap()
            .to_string();
        let machine = machine_def(&machine_id);

        let one_shot_dir = tempfile::tempdir().unwrap();
        let one_shot_store = one_shot_dir.path().join("store.db");
        let one_shot = drive_trace(&name, &mut |step| {
            one_shot_line(&one_shot_store, &machine, step)
        });

        let session_dir = tempfile::tempdir().unwrap();
        let session_store = session_dir.path().join("store.db");
        let mut session = Session::open(&session_store, Some(&machine));
        let via_session = drive_trace(&name, &mut |step| match step {
            Step::Request(request) => {
                session.exchange_line(&serde_json::to_string(&request).unwrap())
            }
            Step::Raw(raw) => session.exchange_line(raw),
        });
        assert_eq!(
            session.close(),
            0,
            "trace {name}: session must exit 0 on EOF"
        );

        assert_eq!(
            one_shot.len(),
            via_session.len(),
            "trace {name}: step count differs between transports"
        );
        for (index, (left, right)) in one_shot.iter().zip(&via_session).enumerate() {
            assert_eq!(
                normalized(left),
                normalized(right),
                "trace {name} step {index}: session response differs from one-shot"
            );
        }
    }
}

#[test]
fn ik_prop_011_stale_capability_is_refused_across_independent_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store.db");
    let machine = machine_def("effect_probe");
    let identity = serde_json::to_value(identity_of("effect_probe")).unwrap();
    let plan = |request_id: &str, capability: &Value| {
        envelope(
            "command",
            request_id,
            json!({
                "aggregate_id": "agg-1",
                "machine": identity,
                "capability": capability,
                "command": {"name": "plan", "args": {
                    "effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1"}},
            }),
        )
    };

    let mut writer = Session::open(&store, Some(&machine));
    let created = writer.request(&envelope(
        "create",
        "c0",
        json!({"aggregate_id": "agg-1", "machine": identity}),
    ));
    assert_eq!(created["ok"], true, "create: {created}");
    let capability = created["result"]["capability"].clone();

    // A second long-lived session warms itself on the aggregate BEFORE the writer commits.
    let mut reader = Session::open(&store, Some(&machine));
    let warm = reader.request(&envelope("show", "s0", json!({"aggregate_id": "agg-1"})));
    assert_eq!(warm["result"]["generation"], 0, "warm show: {warm}");

    let committed = writer.request(&plan("c1", &capability));
    assert_eq!(committed["ok"], true, "writer commit: {committed}");
    assert_eq!(committed["result"]["generation"], 1);

    // The reader still holds the pre-commit capability: it MUST be refused, with no mutation.
    let refused = reader.request(&plan("c2", &capability));
    assert_eq!(
        refused["ok"], false,
        "stale submission must fail: {refused}"
    );
    assert_eq!(refused["diagnostic"]["code"], "IK_CAPABILITY_STALE");
    let after = reader.request(&envelope("show", "s1", json!({"aggregate_id": "agg-1"})));
    assert_eq!(
        after["result"]["generation"], 1,
        "refusal mutated the aggregate: {after}"
    );
    assert_eq!(after["result"]["state"], committed["result"]["state"]);
    assert_eq!(writer.close(), 0);
    assert_eq!(reader.close(), 0);
}

#[test]
fn malformed_and_unknown_requests_fail_closed_without_ending_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store.db");
    let machine = machine_def("effect_probe");
    let identity = serde_json::to_value(identity_of("effect_probe")).unwrap();
    let mut session = Session::open(&store, Some(&machine));

    let garbage: Value = serde_json::from_str(&session.exchange_line("{not json")).unwrap();
    assert_eq!(garbage["ok"], false);
    assert_eq!(garbage["diagnostic"]["code"], "IK_PROTOCOL_INVALID");

    let unknown_field: Value = serde_json::from_str(&session.exchange_line(
        &serde_json::to_string(&envelope("show", "u0", json!({"bogus": 1}))).unwrap(),
    ))
    .unwrap();
    assert_eq!(unknown_field["diagnostic"]["code"], "IK_PROTOCOL_INVALID");

    let mut wrong_major = envelope("show", "v0", json!({"aggregate_id": "x"}));
    wrong_major["protocol"]["major"] = json!(99);
    assert_eq!(
        session.request(&wrong_major)["diagnostic"]["code"],
        "IK_PROTOCOL_VERSION_UNKNOWN"
    );
    let mut wrong_runtime = envelope("show", "v1", json!({"aggregate_id": "x"}));
    wrong_runtime["runtime_version"] = json!("0.0.0");
    assert_eq!(
        session.request(&wrong_runtime)["diagnostic"]["code"],
        "IK_PROTOCOL_VERSION_UNKNOWN"
    );

    // The session is still healthy and the refusals wrote nothing.
    let created = session.request(&envelope(
        "create",
        "ok0",
        json!({"aggregate_id": "agg-1", "machine": identity}),
    ));
    assert_eq!(created["ok"], true, "{created}");
    assert_eq!(session.close(), 0);
}

#[test]
fn sigkill_leaves_exactly_the_committed_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store.db");
    let machine = machine_def("effect_probe");
    let identity = serde_json::to_value(identity_of("effect_probe")).unwrap();
    let mut session = Session::open(&store, Some(&machine));
    let created = session.request(&envelope(
        "create",
        "c0",
        json!({"aggregate_id": "agg-1", "machine": identity}),
    ));
    assert_eq!(created["ok"], true);
    session.kill();

    let shown = run_cli_json(
        &store,
        Some(&machine),
        &envelope("show", "s0", json!({"aggregate_id": "agg-1"})),
    );
    assert_eq!(shown["ok"], true, "{shown}");
    assert_eq!(shown["result"]["generation"], 0);
    let verified = run_cli_json(
        &store,
        Some(&machine),
        &envelope(
            "verify",
            "v0",
            json!({"aggregate_id": "agg-1", "machine": identity}),
        ),
    );
    assert_eq!(verified["ok"], true, "verify after SIGKILL: {verified}");
}

/// Consumers (and the Ranger bind receipt) identify the runtime from `ik --version`; the line is
/// part of the pinned-artifact contract: product, runtime version, protocol major.minor.
#[test]
fn version_flag_reports_runtime_and_protocol_and_exits_zero() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("--version")
        .output()
        .expect("run ik --version");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!(
            "invariant-kernel {} protocol {}.{}",
            ik::protocol::RUNTIME_VERSION,
            ik::protocol::PROTOCOL_MAJOR,
            ik::protocol::PROTOCOL_MINOR
        )
    );
}

#[test]
fn partial_effect_refusal_echoes_request_id_without_ending_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("store.db");
    let machine = machine_def("effect_probe");
    let mut session = Session::open(&store, Some(&machine));
    let partial = envelope(
        "record",
        "partial-effect",
        json!({"aggregate_id": "agg-1", "effect": {"effect_id": "e1"}}),
    );
    let refused: Value =
        serde_json::from_str(&session.exchange_line(&serde_json::to_string(&partial).unwrap()))
            .unwrap();
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["request_id"], "partial-effect");
    assert_eq!(refused["diagnostic"]["code"], "IK_PROTOCOL_INVALID");
    let still = session.request(&envelope(
        "show",
        "after",
        json!({"aggregate_id": "missing"}),
    ));
    assert_eq!(still["request_id"], "after");
    assert_eq!(session.close(), 0);
}
