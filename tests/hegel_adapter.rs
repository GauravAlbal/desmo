//! Hegel adapter through the host analysis runner.
//!
//! These tests build the example and invoke the real engine. A parser mock
//! would not see a law-mutant counterexample.

use ik::canonical::sha256_hex;
use ik::declarative::load_def;
use ik::machine::Machine;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, OnceLock};

fn cargo_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"))
}

fn bins() -> &'static (PathBuf, PathBuf) {
    static BINS: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
    BINS.get_or_init(build_bins)
}

fn build_bins() -> (PathBuf, PathBuf) {
    let _guard = cargo_lock().lock().expect("cargo lock");
    let plain = build_example(false);
    let featured = build_example(true);
    (featured, plain)
}

fn build_example(features: bool) -> PathBuf {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("CARGO_TARGET_DIR", target_dir())
        .args(["build", "--example", "ik_hegel_adapter", "--offline"]);
    if features {
        cmd.args(["--features", "test-support"]);
    }
    let output = cmd.output().expect("build example");
    assert!(
        output.status.success(),
        "example build failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let built = target_dir().join("debug/examples/ik_hegel_adapter");
    let dest = std::env::temp_dir().join(format!(
        "ik-hegel-adapter-{}-{}",
        std::process::id(),
        if features { "support" } else { "plain" }
    ));
    fs::copy(&built, &dest).expect("copy example");
    dest
}

fn ik() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ik"))
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hegel")
        .join(name)
}

fn sha(path: &Path) -> String {
    sha256_hex(&fs::read(path).expect("read"))
}

struct Case {
    root: PathBuf,
    manifest: PathBuf,
    request: PathBuf,
    out: PathBuf,
}

fn prepare(
    adapter: &Path,
    machine_name: &str,
    property_class: &str,
    goals: &[&str],
    controls: Value,
) -> Case {
    let root = std::env::temp_dir().join(format!("ik-hegel-{}-{}", std::process::id(), unique()));
    fs::create_dir_all(&root).expect("root");
    let bundle = root.join("bundle");
    fs::create_dir_all(&bundle).expect("bundle");
    let machine = bundle.join(machine_name);
    fs::copy(fixture(machine_name), &machine).expect("copy machine");
    let loaded = load_def(&machine).expect("load machine");
    let identity = loaded.identity();
    let out = root.join("out");
    fs::create_dir_all(&out).expect("out");
    let adapter_sha = sha(adapter);
    let machine_sha = sha(&machine);
    let manifest = root.join("manifest.json");
    let request = root.join("request.json");
    fs::write(
        &manifest,
        serde_json::to_vec_pretty(&json!({
            "protocol": "ik.analysis.adapter.v1",
            "adapter": {"id": "hegel-runtime", "version": "1"},
            "invocation": {
                "argv": [adapter.display().to_string()],
                "executable_sha256": adapter_sha
            },
            "engine": {"id": "hegeltest", "version_constraint": "=0.30.0"},
            "inputs": ["ik.runtime-driver.v1", "executable_driver"],
            "properties": ["safety", "reachability"],
            "evidence": ["sampled_execution"],
            "requires": {"production_store_access": false, "network": false},
            "result_schema": "ik.analysis.result.v1"
        }))
        .unwrap(),
    )
    .unwrap();
    let goal_rows: Vec<Value> = goals
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "content_digest": sha256_hex(id.as_bytes()),
                "required": true
            })
        })
        .collect();
    fs::write(
        &request,
        serde_json::to_vec_pretty(&json!({
            "protocol": "ik.analysis.request.v1",
            "request_id": "req-hegel",
            "run_id": "run-hegel",
            "subject": {
                "kind": "machine",
                "identity": {
                    "machine_id": identity.machine_id,
                    "spec_hash": identity.spec_hash.0
                }
            },
            "goals": goal_rows,
            "profile_digest": "profile-hegel",
            "property_class": property_class,
            "inputs": [{"path": machine_name, "sha256": machine_sha}],
            "bundle_dir": bundle.display().to_string(),
            "limits": {"timeout_seconds": 180, "max_output_bytes": 2000000},
            "permissions": {
                "scratch_root": out.display().to_string(),
                "production_store_access": false
            },
            "controls": controls
        }))
        .unwrap(),
    )
    .unwrap();
    Case {
        root,
        manifest,
        request,
        out,
    }
}

fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

fn run_case(case: &Case) -> Output {
    ik().args([
        "analysis",
        "run",
        &case.manifest.display().to_string(),
        "--request",
        &case.request.display().to_string(),
        "--out",
        &case.out.display().to_string(),
    ])
    .output()
    .expect("ik analysis run")
}

fn report_of(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout not json: {} stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn finding<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|item| item["goal_id"] == id)
        .unwrap_or_else(|| panic!("missing finding {id} in {report}"))
}

fn requirement<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["requirement_status"]
        .as_array()
        .expect("requirements")
        .iter()
        .find(|item| item["goal_id"] == id)
        .expect("requirement")
}

#[test]
fn known_good_machine_finds_no_safety_violation() {
    let adapter = &bins().0;
    let case = prepare(
        adapter,
        "good.ik.toml",
        "safety",
        &[
            "refusal_is_atomic",
            "store_verifies",
            "stale_capability_refused",
            "declared_invariants_hold",
        ],
        json!({"cases": "12", "steps": "5", "seed": "1"}),
    );
    let output = run_case(&case);
    let report = report_of(&output);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert_eq!(report["result_accepted"], true);
    for id in [
        "refusal_is_atomic",
        "store_verifies",
        "stale_capability_refused",
        "declared_invariants_hold",
    ] {
        let row = finding(&report, id);
        assert_eq!(row["finding"], "no_violation_found", "{id} {row}");
        assert_eq!(row["evidence_class"], "sampled_execution");
        assert_ne!(row["finding"], "holds_in_scope");
        assert_eq!(row["coverage_complete"], false);
    }
}

#[test]
fn mutant_violation_shrinks_and_does_not_replay_on_the_packaged_runtime() {
    let adapter = &bins().0;
    let case = prepare(
        adapter,
        "invariant_probe.ik.toml",
        "safety",
        &["declared_invariants_hold"],
        json!({
            "cases": "8",
            "steps": "4",
            "seed": "1",
            "law_mutant": "SkipInvariantValidation"
        }),
    );
    let output = run_case(&case);
    let report = report_of(&output);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert_eq!(report["result_accepted"], true);
    let row = finding(&report, "declared_invariants_hold");
    assert_eq!(row["finding"], "violation", "{row}");
    assert_eq!(row["evidence_class"], "sampled_execution");
    assert_eq!(row["reproduction_status"], "concrete_reproduced");
    assert_eq!(
        requirement(&report, "declared_invariants_hold")["status"],
        "not_met"
    );
    let bundle = case.out.join("adapter/findings/declared_invariants_hold");
    let scenario: Value =
        serde_json::from_slice(&fs::read(bundle.join("scenario.json")).expect("scenario")).unwrap();
    let steps = scenario["steps"].as_array().expect("steps");
    assert!(!steps.is_empty());
    assert!(steps.len() <= 3, "counterexample was not short: {scenario}");
    assert!(
        steps.iter().any(|step| step["name"] == "violate"),
        "{scenario}"
    );
    let blob =
        fs::read_to_string(case.out.join("adapter/native/hegel-reproduce.txt")).expect("blob");
    assert!(blob.contains("reproduce_failure"), "{blob}");
    let fresh = case.root.join("repro");
    let replay = ik()
        .args([
            "scenario",
            "reproduce",
            &bundle.display().to_string(),
            "--out",
            &fresh.display().to_string(),
        ])
        .output()
        .expect("scenario reproduce");
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&replay.stdout),
        String::from_utf8_lossy(&replay.stderr)
    );
    assert_eq!(replay.status.code(), Some(1), "{text}");
    assert!(text.contains("not_reproduced"), "{text}");
    let recorded: Value =
        serde_json::from_slice(&fs::read(bundle.join("result.json")).unwrap()).unwrap();
    let fresh_result: Value =
        serde_json::from_slice(&fs::read(fresh.join("result.json")).unwrap()).unwrap();
    let recorded_step = command_step(&recorded, "s0");
    let fresh_step = command_step(&fresh_result, "s0");
    assert_eq!(
        recorded_step["response"]["outcome"], "committed",
        "{recorded}"
    );
    assert_eq!(
        fresh_step["response"]["outcome"], "refused",
        "{fresh_result}"
    );
    assert_eq!(
        fresh_step["response"]["diagnostic_code"],
        "IK_INVARIANT_FAILED"
    );
    assert_eq!(recorded_step["state_after"]["facts"]["x"], 5);
    assert_eq!(fresh_step["state_after"]["facts"]["x"], 0);
}

fn command_step<'a>(result: &'a Value, label: &str) -> &'a Value {
    result["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .find(|step| step["label"] == label && step["kind"] == "command")
        .unwrap_or_else(|| panic!("missing command {label} in {result}"))
}

#[test]
fn reach_terminal_on_always_refuse_is_incomplete_not_unreachable() {
    let adapter = &bins().0;
    let case = prepare(
        adapter,
        "always_refuse.ik.toml",
        "reachability",
        &["reach_terminal"],
        json!({"cases": "8", "steps": "4", "seed": "1"}),
    );
    let output = run_case(&case);
    let report = report_of(&output);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert_eq!(report["result_accepted"], true);
    let row = finding(&report, "reach_terminal");
    assert_eq!(row["finding"], "no_witness_found", "{row}");
    assert_eq!(row["coverage_complete"], false);
    assert_eq!(row["host_interpretation"], "inconclusive_not_unreachable");
    assert_eq!(requirement(&report, "reach_terminal")["status"], "blocked");
    assert_eq!(
        requirement(&report, "reach_terminal")["reason"],
        "incomplete"
    );
}

#[test]
fn always_refuse_does_not_fail_refusal_or_stale_properties() {
    let adapter = &bins().0;
    let case = prepare(
        adapter,
        "always_refuse.ik.toml",
        "safety",
        &[
            "refusal_is_atomic",
            "stale_capability_refused",
            "store_verifies",
        ],
        json!({"cases": "8", "steps": "4", "seed": "1"}),
    );
    let output = run_case(&case);
    let report = report_of(&output);
    assert_eq!(output.status.code(), Some(0), "{report}");
    for id in [
        "refusal_is_atomic",
        "stale_capability_refused",
        "store_verifies",
    ] {
        let row = finding(&report, id);
        assert_eq!(row["finding"], "no_violation_found", "{id} {row}");
        assert_ne!(row["finding"], "violation");
    }
}

#[test]
fn temporal_property_is_unsupported() {
    let adapter = &bins().0;
    let case = prepare(
        adapter,
        "good.ik.toml",
        "temporal",
        &["eventually_done"],
        json!({}),
    );
    let output = run_case(&case);
    let report = report_of(&output);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert_eq!(report["result_accepted"], true);
    assert_eq!(report["execution_status"], "unsupported");
    assert_eq!(requirement(&report, "eventually_done")["status"], "blocked");
    assert_eq!(
        requirement(&report, "eventually_done")["reason"],
        "unsupported"
    );
    let row = finding(&report, "eventually_done");
    assert_ne!(row["finding"], "no_violation_found");
    assert_ne!(row["finding"], "holds_in_scope");
}

#[test]
fn defective_driver_passes_when_the_broken_property_is_disabled() {
    let adapter = &bins().0;
    let case = prepare(
        adapter,
        "invariant_probe.ik.toml",
        "safety",
        &[
            "declared_invariants_hold",
            "store_verifies",
            "refusal_is_atomic",
        ],
        json!({
            "cases": "8",
            "steps": "4",
            "seed": "1",
            "law_mutant": "SkipInvariantValidation",
            "disable": "declared_invariants_hold"
        }),
    );
    let output = run_case(&case);
    let report = report_of(&output);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert_eq!(report["result_accepted"], true);
    assert_ne!(
        finding(&report, "declared_invariants_hold")["finding"],
        "violation"
    );
    assert_eq!(
        finding(&report, "store_verifies")["finding"],
        "no_violation_found"
    );
    assert_eq!(
        finding(&report, "refusal_is_atomic")["finding"],
        "no_violation_found"
    );
    assert!(report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["finding"] != "violation"));
}

#[test]
fn law_mutant_field_is_unsupported_without_test_support() {
    let adapter = &bins().1;
    let case = prepare(
        adapter,
        "invariant_probe.ik.toml",
        "safety",
        &["declared_invariants_hold"],
        json!({"law_mutant": "SkipInvariantValidation", "cases": "2", "steps": "2"}),
    );
    let output = run_case(&case);
    let report = report_of(&output);
    assert_eq!(output.status.code(), Some(0), "{report}");
    assert_eq!(report["execution_status"], "unsupported");
    assert_ne!(
        finding(&report, "declared_invariants_hold")["finding"],
        "no_violation_found"
    );
    assert_ne!(
        finding(&report, "declared_invariants_hold")["finding"],
        "violation"
    );
}

#[test]
fn hegeltest_is_absent_from_the_shipped_ik_dependency_tree() {
    let _guard = cargo_lock().lock().expect("cargo lock");
    let normal = tree_text(&[
        "tree",
        "-e",
        "normal",
        "-p",
        "invariant-kernel",
        "--offline",
    ]);
    let with_dev = tree_text(&[
        "tree",
        "-e",
        "normal,dev",
        "-p",
        "invariant-kernel",
        "--offline",
    ]);
    assert!(normal.0, "{}", normal.1);
    assert!(!normal.1.contains("hegeltest"), "{}", normal.1);
    assert!(with_dev.0, "{}", with_dev.1);
    assert!(
        with_dev.1.contains("hegeltest"),
        "dev edges should name the engine, otherwise the absence check is vacuous: {}",
        with_dev.1
    );
}

fn tree_text(args: &[&str]) -> (bool, String) {
    let output = Command::new("cargo")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("CARGO_TARGET_DIR", target_dir())
        .args(args)
        .output()
        .expect("cargo tree");
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}
