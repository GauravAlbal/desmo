//! C2/C3/C6: validated admission, revision invalidation, starter corpus, reached faults.

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn ik() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ik"))
}

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/approval-revision")
        .join(name)
}

fn run(args: &[String]) -> std::process::Output {
    let mut cmd = ik();
    cmd.args(args);
    cmd.output().expect("run ik")
}

fn arg(path: &std::path::Path) -> String {
    path.display().to_string()
}

#[test]
fn corpus_init_emits_an_editable_example_and_int_boundaries() {
    let dir = tempfile::tempdir().expect("temp");
    let out = dir.path().join("cases.json");
    let output = run(&[
        "corpus".into(),
        "init".into(),
        arg(&example("approval_revision.ik.toml")),
        "--out".into(),
        arg(&out),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value = serde_json::from_slice(&fs::read(&out).expect("cases")).expect("json");
    assert_eq!(body["_note"], "example cases, not an exhaustive domain");
    let approve = &body["commands"]["approve"];
    let names: Vec<_> = approve
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["example", "boundary_min", "boundary_max"]);
    assert_eq!(approve[0]["args"]["revision"], 1);
    assert_eq!(approve[1]["args"]["revision"], i64::MIN);
    assert_eq!(approve[2]["args"]["revision"], i64::MAX);
    assert_eq!(
        body["commands"]["edit"][0]["args"]["artifact_digest"],
        "example"
    );
}

#[test]
fn corpus_less_check_refusal_names_the_next_command_without_changing_exit() {
    let machine = example("approval_revision.ik.toml");
    let output = run(&["check".into(), arg(&machine), "--json".into()]);
    assert_eq!(output.status.code(), Some(1));
    let body: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert!(body.get("verdict").is_some());
    assert!(body.get("judgment").is_some());
    let next = body["next_actions"][0].as_str().expect("next");
    let sibling = machine.with_extension("cases.json");
    assert_eq!(
        next,
        format!(
            "ik corpus init {} --out {}",
            machine.display(),
            sibling.display()
        )
    );
    let text = run(&["check".into(), arg(&machine)]);
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(stdout.contains("REFUSED"), "{stdout}");
    assert!(
        stdout.contains(&format!("NEXT COMMAND\n  {next}")),
        "{stdout}"
    );
}

#[test]
fn revision_pattern_reaches_authorize_and_the_always_refuse_variant_does_not() {
    let profile = example("checks.toml");
    let cases = example("approval_revision.cases.json");
    let useful = run(&[
        "check".into(),
        arg(&example("approval_revision.ik.toml")),
        "--corpus".into(),
        arg(&cases),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    assert_eq!(
        useful.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&useful.stderr)
    );
    let doc: Value = serde_json::from_slice(&useful.stdout).expect("json");
    assert_eq!(doc["profile"]["goals"][0]["id"], "authorized");
    assert_eq!(doc["profile"]["goals"][0]["status"], "witness_found");
    assert!(
        doc.get("next_actions").is_none(),
        "a supplied corpus must not add the hint"
    );

    let refused = run(&[
        "check".into(),
        arg(&example("approval_revision_always_refuse.ik.toml")),
        "--corpus".into(),
        arg(&cases),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    assert_eq!(
        refused.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&refused.stdout)
    );
    let doc: Value = serde_json::from_slice(&refused.stdout).expect("json");
    assert_ne!(doc["profile"]["goals"][0]["status"], "witness_found");
}

#[test]
fn scenarios_invalidate_reapprove_and_fail_a_drop_that_did_not_commit() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/approval-revision");
    let machine = root.join("approval_revision.ik.toml");
    for (name, code) in [
        ("useful.json", 0),
        ("edit-after-approval.json", 0),
        ("reapprove.json", 0),
        ("missed-drop.json", 1),
    ] {
        let dir = tempfile::tempdir().expect("temp");
        let out = dir.path().join("out");
        let output = run(&[
            "scenario".into(),
            "run".into(),
            arg(&machine),
            arg(&root.join("scenarios").join(name)),
            "--out".into(),
            arg(&out),
        ]);
        assert_eq!(
            output.status.code(),
            Some(code),
            "{name} stderr {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let dir = tempfile::tempdir().expect("temp");
    let out = dir.path().join("out");
    let output = run(&[
        "scenario".into(),
        "run".into(),
        arg(&machine),
        arg(&root.join("scenarios/useful.json")),
        "--out".into(),
        arg(&out),
    ]);
    assert_eq!(output.status.code(), Some(0));
    let result: Value =
        serde_json::from_slice(&fs::read(out.join("result.json")).unwrap()).unwrap();
    assert_eq!(
        result["steps"].as_array().unwrap().last().unwrap()["state_after"]["state"],
        "authorized"
    );

    let dir = tempfile::tempdir().expect("temp");
    let out = dir.path().join("out");
    let _ = run(&[
        "scenario".into(),
        "run".into(),
        arg(&machine),
        arg(&root.join("scenarios/missed-drop.json")),
        "--out".into(),
        arg(&out),
    ]);
    let result: Value =
        serde_json::from_slice(&fs::read(out.join("result.json")).unwrap()).unwrap();
    let fault = result["faults_applied"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "reply_dropped")
        .unwrap();
    assert_eq!(fault["seam_fired"], false);
    assert_eq!(fault["proof"]["lookup"]["outcome"], "not_recorded");
}

#[test]
fn create_with_command_hides_invalid_admission_and_replays_the_operation() {
    let dir = tempfile::tempdir().expect("temp");
    let store = dir.path().join("store.db");
    let machine = example("approval_revision.ik.toml");
    let identity = machine_identity(&machine);
    let bad = json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": "0.7.0",
        "request_id": "bad",
        "op": "create_with_command",
        "aggregate_id": "doc-bad",
        "machine": identity.clone(),
        "command": {"name": "approve", "args": {"revision": 1}},
        "operation_id": "op-bad"
    });
    let (stdout, code) = invoke(&store, &machine, &bad);
    assert_eq!(code, 1, "{stdout}");
    let missing = json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": "0.7.0",
        "request_id": "show",
        "op": "show",
        "aggregate_id": "doc-bad"
    });
    let (shown, show_code) = invoke(&store, &machine, &missing);
    assert_eq!(show_code, 1, "{shown}");
    assert!(
        shown.contains("not found") || shown.contains("IK_ILLEGAL_STATE"),
        "{shown}"
    );

    let good = json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": "0.7.0",
        "request_id": "good",
        "op": "create_with_command",
        "aggregate_id": "doc-good",
        "machine": identity.clone(),
        "command": {"name": "edit", "args": {"artifact_digest": "doc-1"}},
        "operation_id": "op-good"
    });
    let (created, code) = invoke(&store, &machine, &good);
    assert_eq!(code, 0, "{created}");
    let created: Value = serde_json::from_str(&created).unwrap();
    assert_eq!(created["result"]["state"]["state"], "draft");
    assert_eq!(created["result"]["generation"], 1);
    assert_eq!(created["result"]["mutation"], true);
    let (again, code) = invoke(&store, &machine, &good);
    assert_eq!(code, 0, "{again}");
    let again: Value = serde_json::from_str(&again).unwrap();
    assert_eq!(again["result"]["historical"], true);
    assert_eq!(again["result"]["operation"]["kind"], "create_with_command");

    let lookup = json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": "0.7.0",
        "request_id": "look",
        "op": "lookup_operation",
        "aggregate_id": "doc-good",
        "machine": identity.clone(),
        "command": {"name": "edit", "args": {"artifact_digest": "doc-1"}},
        "operation_id": "op-good"
    });
    let (found, code) = invoke(&store, &machine, &lookup);
    assert_eq!(code, 0, "{found}");
    let found: Value = serde_json::from_str(&found).unwrap();
    assert_eq!(found["result"]["outcome"], "committed_exact");
    let _ = json!({});
}

fn machine_identity(machine: &std::path::Path) -> Value {
    let output = run(&["identity".into(), arg(machine)]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("identity")
}

fn invoke(store: &std::path::Path, machine: &std::path::Path, request: &Value) -> (String, i32) {
    let mut child = ik();
    child
        .arg("--store")
        .arg(store)
        .arg("--machine")
        .arg(machine)
        .arg(request["op"].as_str().unwrap());
    child
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = child.spawn().expect("spawn");
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(request).unwrap().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status.code().unwrap_or(1),
    )
}
