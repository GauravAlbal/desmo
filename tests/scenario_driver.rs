//! Exact-runtime scenario driver. Every assertion is about a real `ik` process.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ik"))
}

fn machine() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/publish-approval/machines/publish_approval_v3.ik.toml")
}

fn example_scenario() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/publish-approval/scenarios/adverse-ordering.json")
}

fn arg(path: &Path) -> String {
    path.display().to_string()
}

fn run(args: &[&str]) -> Output {
    bin().args(args).output().expect("spawn ik")
}

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ik-scenario-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn write_scenario(dir: &Path, body: &Value) -> PathBuf {
    let path = dir.join("scenario.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(body).expect("scenario bytes"),
    )
    .expect("write scenario");
    path
}

fn relative_machine(scenario: &Path) -> String {
    relative_to(scenario, &machine())
}

fn relative_to(scenario: &Path, machine: &Path) -> String {
    let from = scenario
        .parent()
        .expect("scenario dir")
        .canonicalize()
        .expect("scenario dir");
    let to = machine.canonicalize().expect("machine");
    let mut ups = PathBuf::new();
    let mut base = from.as_path();
    loop {
        if let Ok(rest) = to.strip_prefix(base) {
            let rel = ups.join(rest);
            return rel.to_string_lossy().replace('\\', "/");
        }
        ups.push("..");
        base = base.parent().expect("relative machine path");
    }
}

fn scenario_with(dir: &Path, aggregate: &str, steps: Value) -> PathBuf {
    let path = dir.join("scenario.json");
    let body = json!({
        "scenario_version": 1,
        "machine": relative_machine(&path),
        "aggregate": aggregate,
        "steps": steps,
    });
    write_scenario(dir, &body)
}

fn result_of(out: &Path) -> Value {
    let bytes = fs::read(out.join("result.json")).expect("result.json");
    serde_json::from_slice(&bytes).expect("result json")
}

fn step<'a>(result: &'a Value, label: &str) -> &'a Value {
    result["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["label"] == label)
        .unwrap_or_else(|| panic!("missing step {label} in {result}"))
}

fn generation(step: &Value) -> u64 {
    step["state_after"]["generation"]
        .as_u64()
        .expect("generation")
}

fn names(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("array")
        .iter()
        .map(|item| {
            item.as_str()
                .or_else(|| item.get("name").and_then(Value::as_str))
                .expect("name")
                .to_string()
        })
        .collect()
}

#[test]
fn stale_handle_is_refused_and_a_wrong_expectation_fails_without_committing() {
    let passing = temp_dir("stale-pass");
    let scenario = scenario_with(
        &passing,
        "paper-stale",
        json!([
            {"kind": "command", "label": "edit", "name": "edit", "args": {"artifact_digest": "artifact-1"}, "handle": "cap-edit", "expect": "committed"},
            {"kind": "command", "label": "again", "name": "edit", "args": {"artifact_digest": "artifact-2"}, "expect": "committed"},
            {"kind": "command", "label": "stale", "name": "edit", "args": {"artifact_digest": "artifact-3"}, "use_handle": "cap-edit", "expect": "refused:IK_CAPABILITY_STALE"}
        ]),
    );
    let out = passing.join("out");
    let output = run(&[
        "scenario",
        "run",
        &arg(&machine()),
        &arg(&scenario),
        "--out",
        &arg(&out),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = result_of(&out);
    let stale = step(&result, "stale");
    assert_eq!(stale["response"]["diagnostic_code"], "IK_CAPABILITY_STALE");
    assert_eq!(stale["verdict"], "pass");
    assert_eq!(generation(stale), generation(step(&result, "again")));
    assert!(names(&result["seams_fired"]).contains(&"stale_handle_used".to_string()));
    assert!(!names(&result["seams_fired"]).contains(&"reply_dropped".to_string()));

    let failing = temp_dir("stale-fail");
    let scenario = scenario_with(
        &failing,
        "paper-stale-wrong",
        json!([
            {"kind": "command", "label": "edit", "name": "edit", "args": {"artifact_digest": "artifact-1"}, "handle": "cap-edit", "expect": "committed"},
            {"kind": "command", "label": "again", "name": "edit", "args": {"artifact_digest": "artifact-2"}, "expect": "committed"},
            {"kind": "command", "label": "stale", "name": "edit", "args": {"artifact_digest": "artifact-3"}, "use_handle": "cap-edit", "expect": "committed"}
        ]),
    );
    let out = failing.join("out");
    let output = run(&[
        "scenario",
        "run",
        &arg(&machine()),
        &arg(&scenario),
        "--out",
        &arg(&out),
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = result_of(&out);
    let stale = step(&result, "stale");
    assert_eq!(stale["verdict"], "fail");
    assert_eq!(stale["response"]["diagnostic_code"], "IK_CAPABILITY_STALE");
    assert_eq!(generation(stale), generation(step(&result, "again")));
    assert_eq!(result["all_expectations_met"], false);
}

#[test]
fn duplicate_returns_the_original_and_a_different_request_conflicts() {
    let dir = temp_dir("dup");
    let scenario = scenario_with(
        &dir,
        "paper-dup",
        json!([
            {"kind": "command", "label": "edit", "name": "edit", "args": {"artifact_digest": "artifact-1"}, "expect": "committed"},
            {"kind": "duplicate", "label": "again", "of": "edit"},
            {"kind": "command", "label": "conflict", "name": "edit", "operation_id": "op-edit", "args": {"artifact_digest": "artifact-9"}, "expect": "refused:IK_OPERATION_CONFLICT"}
        ]),
    );
    let out = dir.join("out");
    let output = run(&[
        "scenario",
        "run",
        &arg(&machine()),
        &arg(&scenario),
        "--out",
        &arg(&out),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = result_of(&out);
    let again = step(&result, "again");
    let edit = step(&result, "edit");
    assert_eq!(again["response"]["outcome"], "historical");
    assert_eq!(generation(again), generation(edit));
    assert_eq!(
        step(&result, "conflict")["response"]["diagnostic_code"],
        "IK_OPERATION_CONFLICT"
    );
    assert_eq!(generation(step(&result, "conflict")), generation(edit));
    assert!(names(&result["seams_fired"]).contains(&"duplicate_submission".to_string()));
    let trace: Value = serde_json::from_slice(&fs::read(out.join("trace.json")).expect("trace"))
        .expect("trace json");
    let duplicate = trace["events"]
        .as_array()
        .expect("events")
        .iter()
        .find(|event| event["attempted"] == "duplicate edit")
        .expect("duplicate event");
    assert_eq!(duplicate["kind"], "historical");
    assert!(duplicate.get("toState").is_none());
}

#[test]
fn dropped_reply_stays_committed_exactly_once() {
    let dir = temp_dir("drop");
    let scenario = scenario_with(
        &dir,
        "paper-drop",
        json!([
            {"kind": "command", "label": "edit", "name": "edit", "args": {"artifact_digest": "artifact-1"}, "expect": "committed"},
            {"kind": "drop_reply", "label": "lost", "of": "edit"},
            {"kind": "lookup", "label": "found", "operation_id": "op-edit", "expect": "committed_exact"},
            {"kind": "lookup", "label": "missing", "operation_id": "op-never", "expect": "not_recorded"}
        ]),
    );
    let out = dir.join("out");
    let output = run(&[
        "scenario",
        "run",
        &arg(&machine()),
        &arg(&scenario),
        "--out",
        &arg(&out),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = result_of(&out);
    assert_eq!(generation(step(&result, "edit")), 1);
    assert_eq!(generation(step(&result, "found")), 1);
    assert_eq!(
        step(&result, "found")["response"]["outcome"],
        "committed_exact"
    );
    assert_eq!(
        step(&result, "missing")["response"]["outcome"],
        "not_recorded"
    );
    assert!(names(&result["seams_fired"]).contains(&"reply_dropped".to_string()));
    assert!(!names(&result["faults_applied"]).contains(&"stale_handle_used".to_string()));
    let trace: Value = serde_json::from_slice(&fs::read(out.join("trace.json")).expect("trace"))
        .expect("trace json");
    let found = trace["events"]
        .as_array()
        .expect("events")
        .iter()
        .find(|event| event["attempted"] == "lookup op-edit")
        .expect("lookup event");
    assert_eq!(found["kind"], "historical");
    assert!(found.get("toState").is_none());
}

#[test]
fn reopen_preserves_state_and_a_later_command_still_commits() {
    let dir = temp_dir("reopen");
    let scenario = scenario_with(
        &dir,
        "paper-reopen",
        json!([
            {"kind": "command", "label": "edit", "name": "edit", "args": {"artifact_digest": "artifact-1"}, "expect": "committed"},
            {"kind": "reopen", "label": "restart"},
            {"kind": "command", "label": "approve", "name": "approve", "args": {"artifact_digest": "artifact-1"}, "expect": "committed"}
        ]),
    );
    let out = dir.join("out");
    let output = run(&[
        "scenario",
        "run",
        &arg(&machine()),
        &arg(&scenario),
        "--out",
        &arg(&out),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = result_of(&out);
    assert_eq!(step(&result, "restart")["state_after"]["state"], "draft");
    assert_eq!(
        generation(step(&result, "restart")),
        generation(step(&result, "edit"))
    );
    assert_eq!(
        step(&result, "restart")["state_after"]["facts"]["artifact_digest"],
        "artifact-1"
    );
    assert_eq!(
        generation(step(&result, "approve")),
        generation(step(&result, "edit")) + 1
    );
    assert!(names(&result["seams_fired"]).contains(&"reopened".to_string()));
    let trace: Value = serde_json::from_slice(&fs::read(out.join("trace.json")).expect("trace"))
        .expect("trace json");
    let restart = trace["events"]
        .as_array()
        .expect("events")
        .iter()
        .find(|event| event["attempted"] == "reopen")
        .expect("reopen event");
    assert_eq!(restart["kind"], "read");
    assert!(restart.get("toState").is_none());
}

#[test]
fn conflicting_effect_outcome_is_refused_and_the_first_outcome_stands() {
    let dir = temp_dir("effect");
    let scenario = scenario_with(
        &dir,
        "paper-effect",
        json!([
            {"kind": "command", "label": "edit", "name": "edit", "args": {"artifact_digest": "artifact-1"}, "expect": "committed"},
            {"kind": "command", "label": "approve", "name": "approve", "args": {"artifact_digest": "artifact-1"}, "expect": "committed"},
            {"kind": "command", "label": "plan", "name": "authorize_publish", "args": {"effect_id": "eff-1", "payload_digest": "artifact-1", "idempotency_key": "key-1"}, "expect": "committed"},
            {"kind": "effect_outcome", "label": "record", "effect_id": "eff-1", "payload_digest": "artifact-1", "idempotency_key": "key-1", "planned_generation": 3, "outcome_digest": "outcome-1", "expect": "committed"},
            {"kind": "effect_outcome", "label": "replay", "effect_id": "eff-1", "payload_digest": "artifact-1", "idempotency_key": "key-1", "planned_generation": 3, "outcome_digest": "outcome-1", "expect": "noop"},
            {"kind": "effect_outcome", "label": "conflict", "effect_id": "eff-1", "payload_digest": "artifact-1", "idempotency_key": "key-1", "planned_generation": 3, "outcome_digest": "outcome-2", "expect": "refused:IK_EFFECT_OUTCOME_CONFLICT"}
        ]),
    );
    let out = dir.join("out");
    let output = run(&[
        "scenario",
        "run",
        &arg(&machine()),
        &arg(&scenario),
        "--out",
        &arg(&out),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = result_of(&out);
    assert_eq!(generation(step(&result, "plan")), 3);
    assert_eq!(generation(step(&result, "record")), 4);
    assert_eq!(step(&result, "replay")["response"]["outcome"], "noop");
    assert_eq!(generation(step(&result, "replay")), 4);
    let conflict = step(&result, "conflict");
    assert_eq!(
        conflict["response"]["diagnostic_code"],
        "IK_EFFECT_OUTCOME_CONFLICT"
    );
    assert_eq!(generation(conflict), 4);
    assert_eq!(conflict["state_after"]["effect"]["status"], "recorded");
    assert_eq!(
        conflict["state_after"]["effect"]["outcome_digest"],
        "outcome-1"
    );
    assert!(names(&result["seams_fired"]).contains(&"conflicting_effect_outcome".to_string()));
    let trace: Value = serde_json::from_slice(&fs::read(out.join("trace.json")).expect("trace"))
        .expect("trace json");
    let refused = trace["events"]
        .as_array()
        .expect("events")
        .iter()
        .find(|event| event["attempted"] == "record eff-1" && event["kind"] == "refusal")
        .expect("refusal");
    assert!(refused.get("toState").is_none());
    assert_ne!(refused["kind"], "commit");
}

#[test]
fn reproduce_matches_and_a_tampered_scenario_does_not() {
    let dir = temp_dir("repro");
    let bundle = dir.join("bundle");
    let output = run(&[
        "scenario",
        "run",
        &arg(&machine()),
        &arg(&example_scenario()),
        "--out",
        &arg(&bundle),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let first = result_of(&bundle);
    assert_eq!(first["all_expectations_met"], true);
    assert_eq!(first["schema"], "ik.scenario.result.v1");
    assert_eq!(first["runtime"]["store_schema"], 4);
    let sums = fs::read_to_string(bundle.join("SHA256SUMS")).expect("sums");
    for name in [
        "machine.ik.toml",
        "result.json",
        "scenario.json",
        "trace.json",
    ] {
        let digest = sha256(&fs::read(bundle.join(name)).expect(name));
        assert!(
            sums.contains(&format!("{digest}  {name}")),
            "missing digest for {name}\n{sums}"
        );
    }

    let copied = dir.join("copy");
    let output = run(&[
        "scenario",
        "reproduce",
        &arg(&bundle),
        "--out",
        &arg(&copied),
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout {stdout} stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout.lines().next(), Some("reproduced"));

    let tampered = dir.join("tampered");
    copy_dir(&bundle, &tampered);
    let mut scenario: Value =
        serde_json::from_slice(&fs::read(tampered.join("scenario.json")).expect("scenario"))
            .expect("json");
    scenario["steps"][0]["args"]["artifact_digest"] = json!("tampered");
    fs::write(
        tampered.join("scenario.json"),
        serde_json::to_vec_pretty(&scenario).expect("bytes"),
    )
    .expect("tamper");
    let out = dir.join("tampered-out");
    let output = run(&[
        "scenario",
        "reproduce",
        &arg(&tampered),
        "--out",
        &arg(&out),
    ]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "stdout {stdout}");
    assert_eq!(stdout.lines().next(), Some("not_reproduced"));
    assert!(stdout.contains("first differing step:"), "stdout {stdout}");

    let runtime = dir.join("runtime");
    copy_dir(&bundle, &runtime);
    let mut recorded = result_of(&runtime);
    recorded["runtime"]["binary_sha256"] = json!("0".repeat(64));
    fs::write(
        runtime.join("result.json"),
        serde_json::to_vec(&recorded).expect("bytes"),
    )
    .expect("rewrite result");
    let out = dir.join("runtime-out");
    let output = run(&["scenario", "reproduce", &arg(&runtime), "--out", &arg(&out)]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout {stdout} stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout.lines().next(), Some("reproduced"));
    assert!(
        stdout.contains("runtime difference: binary_sha256"),
        "stdout {stdout}"
    );
}

#[test]
fn a_pre_populated_scratch_store_is_refused_without_being_opened() {
    let dir = temp_dir("pre");
    let out = dir.join("out");
    let scratch = out.join("scratch");
    fs::create_dir_all(&scratch).expect("scratch");
    let store = scratch.join("store.sqlite");
    let original = b"not-a-store-but-named-as-one".to_vec();
    fs::write(&store, &original).expect("plant store");
    let scenario = scenario_with(
        &dir,
        "paper-pre",
        json!([{"kind": "command", "label": "edit", "name": "edit", "args": {"artifact_digest": "artifact-1"}, "expect": "committed"}]),
    );
    let output = run(&[
        "scenario",
        "run",
        &arg(&machine()),
        &arg(&scenario),
        "--out",
        &arg(&out),
    ]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&store).expect("store bytes"), original);
    assert!(!store.with_extension("sqlite-wal").exists());
    assert!(!out.join("result.json").exists());

    let denied = run(&[
        "scenario",
        "run",
        "--store",
        &arg(&store),
        &arg(&machine()),
        &arg(&scenario),
        "--out",
        &arg(&dir.join("other")),
    ]);
    assert_eq!(denied.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&denied.stderr).contains("--store"));
}

#[test]
fn trace_refuses_are_not_commits_and_match_the_v2_shape() {
    let dir = temp_dir("trace");
    let out = dir.join("out");
    let output = run(&[
        "scenario",
        "run",
        &arg(&machine()),
        &arg(&example_scenario()),
        "--out",
        &arg(&out),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let trace: Value = serde_json::from_slice(&fs::read(out.join("trace.json")).expect("trace"))
        .expect("trace json");
    assert_eq!(trace["schema"], "ikaros.trace.v2");
    assert_eq!(trace["advisory"], true);
    assert_eq!(trace["origin"], "observed");
    let events = trace["events"].as_array().expect("events");
    assert!(events
        .iter()
        .any(|event| event["kind"] == "refusal" && event.get("toState").is_none()));
    assert!(events
        .iter()
        .any(|event| event["kind"] == "read" && event.get("toState").is_none()));
    assert!(events
        .iter()
        .all(|event| event["kind"] != "refusal" || event.get("toState").is_none()));
    let parser = Command::new("node")
        .arg("--experimental-strip-types")
        .arg(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/scenario/check-trace.mjs"),
        )
        .arg(out.join("trace.json"))
        .output();
    match parser {
        Ok(parser) if parser.status.success() => {}
        Ok(parser) => panic!(
            "trace parser failed: stdout {} stderr {}",
            String::from_utf8_lossy(&parser.stdout),
            String::from_utf8_lossy(&parser.stderr)
        ),
        Err(err) => panic!("node trace parser could not start: {err}"),
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("copy dest");
    for entry in fs::read_dir(from).expect("read bundle") {
        let entry = entry.expect("entry");
        if entry.path().is_file() {
            fs::copy(entry.path(), to.join(entry.file_name())).expect("copy file");
        }
    }
}

fn sha256(bytes: &[u8]) -> String {
    ik::canonical::sha256_hex(bytes)
}

#[test]
fn a_requested_drop_whose_step_did_not_commit_fails_and_records_seam_fired_false() {
    let dir = temp_dir("missed-drop");
    let scenario = scenario_with(
        &dir,
        "paper-missed",
        json!([
            {"kind": "command", "label": "blocked", "name": "withdraw", "expect": "refused:IK_COMMAND_INVALID"},
            {"kind": "drop_reply", "label": "lost", "of": "blocked"}
        ]),
    );
    let out = dir.join("out");
    let output = run(&[
        "scenario",
        "run",
        &arg(&machine()),
        &arg(&scenario),
        "--out",
        &arg(&out),
    ]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = result_of(&out);
    assert_eq!(result["all_expectations_met"], false);
    let fault = result["faults_applied"]
        .as_array()
        .expect("faults")
        .iter()
        .find(|item| item["name"] == "reply_dropped")
        .expect("reply_dropped fault");
    assert_eq!(fault["seam_fired"], false);
    assert_eq!(fault["proof"]["lookup"]["outcome"], "not_recorded");
    assert_eq!(fault["proof"]["named_step_committed"], false);
    assert!(!names(&result["seams_fired"]).contains(&"reply_dropped".to_string()));
}

fn loan_v1() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("validation/dfy/journeys/fixtures/loan_checkout.ik.toml")
}

fn loan_v2() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("validation/dfy/journeys/fixtures/loan_v2.ik.toml")
}

#[test]
fn prior_machine_records_present_observations_for_existing_instances() {
    let dir = temp_dir("existing-loan");
    let scenario_path = dir.join("scenario.json");
    let body = json!({
        "scenario_version": 1,
        "machine": relative_to(&scenario_path, &loan_v2()),
        "aggregate": "loan-1",
        "steps": [
            {
                "kind": "prior_machine",
                "label": "under_prior",
                "path": relative_to(&scenario_path, &loan_v1())
            },
            {
                "kind": "command",
                "label": "checkout",
                "name": "checkout",
                "args": {
                    "checkout_ms": 1000,
                    "loan_until_ms": 1209601000,
                    "deposit_id": "dep-1"
                },
                "expect": "committed"
            },
            {
                "kind": "command",
                "label": "extend",
                "name": "extend",
                "present": "new",
                "args": { "extra_ms": 1 },
                "expect": "refused:IK_MACHINE_MISMATCH"
            },
            {
                "kind": "command",
                "label": "recall",
                "name": "recall",
                "expect": "committed"
            }
        ]
    });
    write_scenario(&dir, &body);
    let out = dir.join("out");
    let output = run(&[
        "scenario",
        "run",
        &arg(&loan_v2()),
        &arg(&scenario_path),
        "--out",
        &arg(&out),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = result_of(&out);
    assert_eq!(result["all_expectations_met"], true);
    let under = step(&result, "under_prior");
    assert_eq!(under["kind"], "prior_machine");
    assert_eq!(under["present"]["created_version"], 1);
    assert_eq!(under["present"]["presented"], "prior");
    let recall = step(&result, "recall");
    assert_eq!(recall["response"]["outcome"], "committed");
    assert_eq!(recall["present"]["presented"], "prior");
    assert_eq!(recall["present"]["command"], "recall");
    let extend = step(&result, "extend");
    assert_eq!(extend["response"]["outcome"], "refused");
    assert_eq!(extend["response"]["diagnostic_code"], "IK_MACHINE_MISMATCH");
    assert_eq!(extend["present"]["presented"], "new");
    assert_eq!(extend["present"]["presented_version"], 2);
    assert_eq!(extend["present"]["code"], "IK_MACHINE_MISMATCH");
    assert!(out.join("prior_machine.ik.toml").is_file());

    let copied = dir.join("copy");
    let reproduced = run(&["scenario", "reproduce", &arg(&out), "--out", &arg(&copied)]);
    assert_eq!(
        reproduced.status.code(),
        Some(0),
        "stderr {}",
        String::from_utf8_lossy(&reproduced.stderr)
    );
}
