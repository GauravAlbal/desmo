//! TLC adapter over a runtime-derived finite graph.
//!
//! These tests invoke the staged Java/TLC toolchain. If those files are absent
//! the test fails; it does not pass vacuously.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

const TOOLCHAIN_ENV: &str = "IK_TLC_TOOLCHAIN";
const JAR_SHA: &str = "936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88";
const TLC_VERSION: &str = "TLC2 Version 2.19 of 08 August 2024 (rev: 5a47802)";

/// Pinned TLC jar: `IK_TLC_JAR`, else `$HOME/.ik-toolchain/tla2tools-1.7.4.jar`.
fn jar() -> String {
    std::env::var("IK_TLC_JAR").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{home}/.ik-toolchain/tla2tools-1.7.4.jar")
    })
}

/// Java path pinned by the toolchain file the adapter will actually use.
fn java() -> String {
    let text = std::fs::read_to_string(toolchain()).expect("toolchain file");
    let value: Value = serde_json::from_str(&text).expect("toolchain json");
    value["java"].as_str().expect("java path").to_string()
}

fn require_toolchain() {
    let jar = jar();
    let java = java();
    let present = Path::new(&java).is_file() && Path::new(&jar).is_file();
    assert!(
        present,
        "TLC toolchain absent (java {java} or jar {jar}); refusing to pass vacuously"
    );
}

fn ik_path() -> String {
    std::env::var("IK_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_ik").to_string())
}

fn bin() -> Command {
    Command::new(ik_path())
}

fn adapter() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("adapters/tlc/ik_tlc_adapter.py")
}

fn toolchain() -> PathBuf {
    match std::env::var(TOOLCHAIN_ENV) {
        Ok(path) => PathBuf::from(path),
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("adapters/tlc/toolchain.json"),
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/profile")
        .join(name)
}

fn sha256(path: &Path) -> String {
    ik::canonical::sha256_hex(&std::fs::read(path).expect("read"))
}

struct Bundle {
    root: tempfile::TempDir,
    graph: Value,
}

fn export_graph(machine: &Path, extra: &[&str]) -> Bundle {
    let root = tempfile::tempdir().expect("temp");
    let graph_path = root.path().join("graph.json");
    let machine_copy = root.path().join("machine.ik.toml");
    std::fs::copy(machine, &machine_copy).expect("copy machine");
    let mut cmd = bin();
    cmd.arg("graph")
        .arg("export")
        .arg(machine)
        .arg("--out")
        .arg(&graph_path);
    for arg in extra {
        cmd.arg(arg);
    }
    let done = cmd.output().expect("export");
    assert!(
        done.status.success(),
        "export failed: {}",
        String::from_utf8_lossy(&done.stderr)
    );
    let graph = serde_json::from_str(&std::fs::read_to_string(&graph_path).unwrap()).unwrap();
    Bundle { root, graph }
}

struct HostRun {
    report: Value,
    result: Value,
    adapter_out: PathBuf,
}

fn host_run(
    bundle: &Bundle,
    property_class: &str,
    controls: Value,
    toolchain_path: &Path,
) -> HostRun {
    let out = bundle.root.path().join("run");
    std::fs::create_dir_all(&out).unwrap();
    let manifest = bundle.root.path().join("manifest.json");
    let request = bundle.root.path().join("request.json");
    let adapter = adapter();
    let graph_sha = sha256(&bundle.root.path().join("graph.json"));
    let machine_sha = sha256(&bundle.root.path().join("machine.ik.toml"));
    std::fs::write(
        &manifest,
        serde_json::to_vec_pretty(&manifest_value(&adapter, toolchain_path)).unwrap(),
    )
    .unwrap();
    std::fs::write(
        &request,
        serde_json::to_vec_pretty(&request_value(
            bundle,
            property_class,
            &graph_sha,
            &machine_sha,
            controls,
        ))
        .unwrap(),
    )
    .unwrap();
    let done = bin()
        .args([
            "analysis",
            "run",
            &manifest.display().to_string(),
            "--request",
            &request.display().to_string(),
            "--out",
            &out.display().to_string(),
            "--timeout-seconds",
            "90",
        ])
        .output()
        .expect("analysis run");
    let report = serde_json::from_slice(&done.stdout).unwrap_or_else(|_| {
        panic!(
            "host stdout not json: {} stderr {}",
            String::from_utf8_lossy(&done.stdout),
            String::from_utf8_lossy(&done.stderr)
        )
    });
    let adapter_out = out.join("adapter");
    let result = serde_json::from_str(
        &std::fs::read_to_string(adapter_out.join("result.json"))
            .unwrap_or_else(|_| panic!("no result: {}", String::from_utf8_lossy(&done.stderr))),
    )
    .unwrap();
    HostRun {
        report,
        result,
        adapter_out,
    }
}

fn manifest_value(adapter: &Path, toolchain_path: &Path) -> Value {
    json!({
        "protocol": "ik.analysis.adapter.v1",
        "adapter": {"id": "tlc-graph", "version": "1"},
        "invocation": {
            "argv": [adapter.display().to_string(), "--toolchain", toolchain_path.display().to_string()],
            "executable_sha256": sha256(adapter)
        },
        "engine": {"id": "TLC", "version_constraint": TLC_VERSION},
        "inputs": ["finite_runtime_graph"],
        "properties": ["safety", "reachability", "finite_scope_stuck", "temporal"],
        "evidence": ["exhaustive_finite_model", "bounded_paths"],
        "requires": {"production_store_access": false, "network": false},
        "result_schema": "ik.analysis.result.v1"
    })
}

fn request_value(
    bundle: &Bundle,
    property_class: &str,
    graph_sha: &str,
    machine_sha: &str,
    controls: Value,
) -> Value {
    let mut controls = controls;
    controls["graph"] = json!("graph.json");
    controls["machine"] = json!("machine.ik.toml");
    controls["ik_bin"] = json!(ik_path());
    json!({
        "protocol": "ik.analysis.request.v1",
        "request_id": "req-tlc",
        "run_id": "run-tlc",
        "subject": {
            "kind": "finite_runtime_graph",
            "identity": {
                "machine_id": bundle.graph["machine_id"],
                "spec_hash": bundle.graph["spec_hash"]
            }
        },
        "goals": [{"id": "g1", "content_digest": "digest-g1", "required": true}],
        "profile_digest": "profile-tlc",
        "property_class": property_class,
        "inputs": [
            {"path": "graph.json", "sha256": graph_sha},
            {"path": "machine.ik.toml", "sha256": machine_sha}
        ],
        "bundle_dir": bundle.root.path().display().to_string(),
        "limits": {"timeout_seconds": 90, "max_output_bytes": 2000000},
        "permissions": {
            "scratch_root": bundle.root.path().display().to_string(),
            "production_store_access": false
        },
        "controls": controls
    })
}

fn finding(result: &Value) -> &Value {
    &result["properties"][0]
}

fn requirement(report: &Value) -> &Value {
    &report["requirement_status"][0]
}

#[test]
fn holding_safety_on_a_complete_graph_is_holds_in_scope_via_the_host() {
    require_toolchain();
    let bundle = export_graph(&fixture("linear.ik.toml"), &[]);
    let run = host_run(
        &bundle,
        "safety",
        json!({"question": "safety_invariant", "predicate": "label != \"absent\""}),
        &toolchain(),
    );
    assert_eq!(run.report["result_accepted"], true);
    assert_eq!(run.report["execution_status"], "completed");
    assert_eq!(finding(&run.result)["finding"], "holds_in_scope");
    assert_eq!(
        finding(&run.result)["evidence_class"],
        "exhaustive_finite_model"
    );
    assert_eq!(finding(&run.result)["coverage"]["complete"], true);
    assert_eq!(requirement(&run.report)["status"], "met");
    let provenance: Value = serde_json::from_str(
        &std::fs::read_to_string(run.adapter_out.join("native/provenance.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(provenance["engine"], "TLC");
    assert_eq!(provenance["engine_version"], TLC_VERSION);
    assert_eq!(provenance["jar_sha256"], JAR_SHA);
    assert_eq!(provenance["evidence_scope"], "model");
    assert!(run.adapter_out.join("native/IkGraph.tla").is_file());
    assert!(run.adapter_out.join("native/stdout.log").is_file());
}

#[test]
fn safety_violation_replays_through_the_scenario_driver() {
    require_toolchain();
    let bundle = export_graph(&fixture("linear.ik.toml"), &[]);
    let run = host_run(
        &bundle,
        "safety",
        json!({"question": "safety_invariant", "predicate": "label != \"done\""}),
        &toolchain(),
    );
    assert_eq!(finding(&run.result)["finding"], "violation");
    assert_eq!(
        finding(&run.result)["evidence_class"],
        "exhaustive_finite_model"
    );
    assert_eq!(
        finding(&run.result)["reproduction"]["status"],
        "concrete_reproduced"
    );
    assert_eq!(requirement(&run.report)["status"], "not_met");
    let scenario: Value = serde_json::from_str(
        &std::fs::read_to_string(run.adapter_out.join("findings/g1/scenario.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(scenario["scenario_version"], 1);
    assert_eq!(scenario["replayable"], true);
    assert!(scenario["steps"].as_array().unwrap().len() >= 2);
    let names: Vec<&str> = scenario["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| step["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["step", "finish"]);
}

#[test]
fn reach_terminal_finds_a_witness_on_the_real_engine() {
    require_toolchain();
    let bundle = export_graph(&fixture("linear.ik.toml"), &[]);
    let run = host_run(
        &bundle,
        "reachability",
        json!({"question": "reach_terminal"}),
        &toolchain(),
    );
    assert_eq!(finding(&run.result)["finding"], "witness_found");
    assert_eq!(
        finding(&run.result)["evidence_class"],
        "exhaustive_finite_model"
    );
    assert_eq!(requirement(&run.report)["status"], "met");
    assert_eq!(requirement(&run.report)["meets_requirement"], true);
    let state = host_run(
        &bundle,
        "reachability",
        json!({"question": "reach_state", "target_label": "mid"}),
        &toolchain(),
    );
    assert_eq!(finding(&state.result)["finding"], "witness_found");
    assert_ne!(finding(&state.result)["finding"], "holds_in_scope");
}

#[test]
fn incomplete_graph_is_inconclusive_and_not_holds_in_scope() {
    require_toolchain();
    let bundle = export_graph(&fixture("linear.ik.toml"), &["--max-depth", "1"]);
    assert_eq!(bundle.graph["complete"], false);
    let run = host_run(
        &bundle,
        "safety",
        json!({"question": "safety_invariant", "predicate": "label != \"absent\""}),
        &toolchain(),
    );
    assert_eq!(finding(&run.result)["finding"], "inconclusive");
    assert_eq!(finding(&run.result)["evidence_class"], "bounded_paths");
    assert_eq!(finding(&run.result)["coverage"]["complete"], false);
    assert_ne!(finding(&run.result)["finding"], "holds_in_scope");
    assert_eq!(requirement(&run.report)["status"], "blocked");
}

#[test]
fn temporal_property_is_unsupported() {
    require_toolchain();
    let bundle = export_graph(&fixture("linear.ik.toml"), &[]);
    let run = host_run(
        &bundle,
        "temporal",
        json!({"question": "temporal"}),
        &toolchain(),
    );
    assert_eq!(run.result["execution_status"], "unsupported");
    assert_eq!(run.report["result_accepted"], true);
    assert_eq!(requirement(&run.report)["status"], "blocked");
    assert!(run.result["properties"].as_array().unwrap().is_empty());
}

#[test]
fn missing_java_is_tool_unavailable() {
    require_toolchain();
    let bundle = export_graph(&fixture("linear.ik.toml"), &[]);
    let bad = bundle.root.path().join("missing-toolchain.json");
    std::fs::write(
        &bad,
        serde_json::to_vec_pretty(&json!({
            "java": "/nonexistent/java",
            "jar": jar(),
            "jar_sha256": JAR_SHA,
            "version": TLC_VERSION
        }))
        .unwrap(),
    )
    .unwrap();
    let run = host_run(
        &bundle,
        "safety",
        json!({"question": "safety_invariant", "predicate": "true"}),
        &bad,
    );
    assert_eq!(run.result["execution_status"], "tool_unavailable");
    assert_eq!(requirement(&run.report)["status"], "blocked");
    assert_ne!(finding_or_empty(&run.result), "holds_in_scope");
}

fn finding_or_empty(result: &Value) -> String {
    result["properties"]
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row["finding"].as_str())
        .unwrap_or("")
        .to_string()
}

#[test]
fn stuck_nonterminal_is_a_finding_and_a_declared_terminal_is_not() {
    require_toolchain();
    let dead = export_graph(&fixture("dead_end.ik.toml"), &[]);
    let stuck = host_run(
        &dead,
        "finite_scope_stuck",
        json!({"question": "stuck"}),
        &toolchain(),
    );
    assert_eq!(finding(&stuck.result)["finding"], "violation");
    assert_eq!(
        finding(&stuck.result)["evidence_class"],
        "exhaustive_finite_model"
    );
    let terminal = dead_end_terminal();
    let closed = export_graph(&terminal, &[]);
    let none = host_run(
        &closed,
        "finite_scope_stuck",
        json!({"question": "stuck"}),
        &toolchain(),
    );
    assert_eq!(finding(&none.result)["finding"], "holds_in_scope");
    assert_ne!(finding(&none.result)["finding"], "violation");
}

fn dead_end_terminal() -> PathBuf {
    let dir = tempfile::tempdir().expect("temp").keep();
    let path = dir.join("dead_end_terminal.ik.toml");
    let text = std::fs::read_to_string(fixture("dead_end.ik.toml")).unwrap();
    std::fs::write(
        &path,
        text.replace("stuck = {}", "stuck = { terminal = true }"),
    )
    .unwrap();
    path
}

#[test]
fn dropping_a_generated_edge_fails_the_adapters_conformance_check() {
    require_toolchain();
    let bundle = export_graph(&fixture("linear.ik.toml"), &[]);
    let faithful = host_run(
        &bundle,
        "safety",
        json!({"question": "safety_invariant", "predicate": "label != \"absent\""}),
        &toolchain(),
    );
    let faithful_check: Value = serde_json::from_str(
        &std::fs::read_to_string(faithful.adapter_out.join("native/conformance.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(faithful_check["checked"], true);
    assert_eq!(faithful_check["match"], true);
    assert_eq!(
        faithful_check["native_reachable"],
        faithful_check["tlc_distinct"]
    );
    assert_eq!(finding(&faithful.result)["finding"], "holds_in_scope");
    let mutated = host_run(
        &bundle,
        "safety",
        json!({"question": "safety_invariant", "predicate": "label != \"absent\"", "mutate": "drop_committed_edge"}),
        &toolchain(),
    );
    let check: Value = serde_json::from_str(
        &std::fs::read_to_string(mutated.adapter_out.join("native/conformance.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(check["checked"], true);
    assert_eq!(check["match"], false);
    assert!(check["tlc_distinct"].as_u64().unwrap() < check["native_reachable"].as_u64().unwrap());
    assert_eq!(mutated.result["execution_status"], "failed");
    assert_ne!(finding(&mutated.result)["finding"], "holds_in_scope");
    let tla = std::fs::read_to_string(mutated.adapter_out.join("native/IkGraph.tla")).unwrap();
    let dropped = bundle.graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|edge| edge["outcome"] == "committed")
        .unwrap();
    let needle = format!("node' = \"{}\"", dropped["to"].as_str().unwrap());
    assert!(
        !tla.contains(&needle),
        "mutated TLA still contains the dropped edge"
    );
}
