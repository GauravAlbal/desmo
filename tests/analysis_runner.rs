//! Host conformance for the pinned analysis-adapter runner.
//!
//! The fixture is a mock. These tests prove host validation and process
//! control. They do not prove an analysis engine.

use ik::analysis::{
    requirement_status, run_adapter, Coverage, GoalRef, PropertyResult, Reproduction, RunSpec,
};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn adapter_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/analysis/adapter.py")
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ik"))
}

fn sha256(path: &Path) -> String {
    ik::canonical::sha256_hex(&fs::read(path).expect("read"))
}

fn copy_adapter(dir: &Path) -> PathBuf {
    let dest = dir.join("adapter.py");
    fs::copy(adapter_src(), &dest).expect("copy adapter");
    let mut perms = fs::metadata(&dest).expect("meta").permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    fs::set_permissions(&dest, perms).expect("chmod");
    dest
}

struct Case {
    root: PathBuf,
    adapter: PathBuf,
    manifest: PathBuf,
    request: PathBuf,
    out: PathBuf,
    bundle: PathBuf,
    pid_file: PathBuf,
}

fn prepare(behavior: &str, property_class: &str, max_output_bytes: u64) -> Case {
    let root =
        std::env::temp_dir().join(format!("ik-analysis-{}-{}", std::process::id(), unique()));
    fs::create_dir_all(&root).expect("root");
    let adapter = copy_adapter(&root);
    let bundle = root.join("bundle");
    fs::create_dir_all(&bundle).expect("bundle");
    fs::write(bundle.join("input.txt"), b"input-v1").expect("input");
    let input_sha = sha256(&bundle.join("input.txt"));
    let out = root.join("out");
    fs::create_dir_all(&out).expect("out");
    let manifest = root.join("manifest.json");
    let request = root.join("request.json");
    let pid_file = root.join("pids");
    fs::write(
        &manifest,
        serde_json::to_vec_pretty(&manifest_value(&adapter, &sha256(&adapter), false, false))
            .unwrap(),
    )
    .unwrap();
    fs::write(
        &request,
        serde_json::to_vec_pretty(&request_value(
            &bundle,
            &out,
            &input_sha,
            behavior,
            property_class,
            max_output_bytes,
            &pid_file,
        ))
        .unwrap(),
    )
    .unwrap();
    Case {
        root,
        adapter,
        manifest,
        request,
        out,
        bundle,
        pid_file,
    }
}

fn unique() -> u64 {
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

fn manifest_value(adapter: &Path, sha: &str, store: bool, everything: bool) -> Value {
    let properties = if everything {
        json!(["supports_everything"])
    } else {
        json!(["safety", "reachability"])
    };
    json!({
        "protocol": "ik.analysis.adapter.v1",
        "adapter": {"id": "conformance-fixture", "version": "1"},
        "invocation": {"argv": [adapter.display().to_string()], "executable_sha256": sha},
        "engine": {"id": "fixture", "version_constraint": "1"},
        "inputs": ["observation_trace"],
        "properties": properties,
        "evidence": ["sampled_execution", "bounded_paths", "exhaustive_finite_model", "observed_trace"],
        "requires": {"production_store_access": store, "network": false},
        "result_schema": "ik.analysis.result.v1"
    })
}

fn request_value(
    bundle: &Path,
    scratch_root: &Path,
    input_sha: &str,
    behavior: &str,
    property_class: &str,
    max_output_bytes: u64,
    pid_file: &Path,
) -> Value {
    json!({
        "protocol": "ik.analysis.request.v1",
        "request_id": "req-1",
        "run_id": "run-1",
        "subject": {"kind": "machine", "identity": {"machine_id": "job", "spec_hash": "spec-hash-1"}},
        "goals": [
            {"id": "required-goal", "content_digest": "digest-required", "required": true},
            {"id": "optional-goal", "content_digest": "digest-optional", "required": false}
        ],
        "profile_digest": "profile-digest-1",
        "property_class": property_class,
        "inputs": [{"path": "input.txt", "sha256": input_sha}],
        "bundle_dir": bundle.display().to_string(),
        "limits": {"timeout_seconds": 30, "max_output_bytes": max_output_bytes},
        "permissions": {
            "scratch_root": scratch_root.display().to_string(),
            "production_store_access": false
        },
        "controls": {
            "behavior": behavior,
            "pid_file": pid_file.display().to_string(),
            "input_path": "input.txt"
        }
    })
}

fn run_case(case: &Case, timeout: Option<&str>) -> Output {
    let mut args = vec![
        "analysis".to_string(),
        "run".to_string(),
        case.manifest.display().to_string(),
        "--request".to_string(),
        case.request.display().to_string(),
        "--out".to_string(),
        case.out.display().to_string(),
    ];
    if let Some(timeout) = timeout {
        args.push("--timeout-seconds".into());
        args.push(timeout.into());
    }
    bin()
        .args(&args)
        .env("IK_ANALYSIS_BEHAVIOR", "timeout")
        .env("AWS_SECRET_ACCESS_KEY", "secret")
        .output()
        .expect("run ik")
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

fn goal<'a>(report: &'a Value, id: &str) -> Option<&'a Value> {
    report["requirement_status"]
        .as_array()?
        .iter()
        .find(|item| item["goal_id"] == id)
}

fn marker(case: &Case) -> PathBuf {
    case.root.join("executed.marker")
}

fn pids(path: &Path) -> Option<(u32, u32)> {
    let text = fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let parent = lines.next()?.parse().ok()?;
    let child = lines.next()?.parse().ok()?;
    Some((parent, child))
}

fn running(pid: u32) -> bool {
    let output = Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "state="])
        .output()
        .ok();
    match output {
        Some(output) if output.status.success() => {
            let state = String::from_utf8_lossy(&output.stdout);
            let state = state.trim();
            !state.is_empty() && !state.starts_with('Z')
        }
        _ => false,
    }
}

fn became_dead(pid: u32) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        if !running(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    !running(pid)
}

fn accepted_correct(case: &Case) -> Value {
    let output = run_case(case, Some("5"));
    report_of(&output)
}

#[test]
fn describe_reads_the_manifest_and_executes_nothing() {
    let case = prepare("correct", "safety", 65536);
    let described = bin()
        .args(["analysis", "describe", &case.manifest.display().to_string()])
        .output()
        .expect("describe");
    let body = report_of(&described);
    assert_eq!(described.status.code(), Some(0));
    assert_eq!(body["executed"], false);
    assert!(body["rejection_reason"].is_null());
    assert_eq!(body["adapter"]["id"], "conformance-fixture");
    assert!(!marker(&case).exists(), "describe executed the adapter");

    let direct = Command::new(&case.adapter).output().expect("direct");
    assert!(!direct.status.success());
    assert!(
        marker(&case).exists(),
        "negative control: executing the adapter did not create a marker"
    );
    fs::remove_file(marker(&case)).unwrap();
    let again = bin()
        .args(["analysis", "describe", &case.manifest.display().to_string()])
        .output()
        .expect("describe again");
    assert_eq!(again.status.code(), Some(0));
    assert!(
        !marker(&case).exists(),
        "describe executed the adapter after the control"
    );
    let _ = fs::remove_dir_all(&case.root);
}

#[test]
fn relative_executable_is_refused_before_execution() {
    let case = prepare("correct", "safety", 65536);
    let mut manifest = manifest_value(&case.adapter, &sha256(&case.adapter), false, false);
    manifest["invocation"]["argv"] = json!(["./adapter.py"]);
    fs::write(
        &case.manifest,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let refused = bin()
        .current_dir(&case.root)
        .args([
            "analysis",
            "run",
            "manifest.json",
            "--request",
            &case.request.display().to_string(),
            "--out",
            &case.out.display().to_string(),
        ])
        .output()
        .expect("run");
    let body = report_of(&refused);
    assert_eq!(refused.status.code(), Some(2));
    assert_eq!(body["rejection_reason"], "relative_executable");
    assert_eq!(body["executed"], false);
    assert!(!marker(&case).exists(), "relative path was executed");

    manifest["invocation"]["argv"] = json!([case.adapter.display().to_string()]);
    fs::write(
        &case.manifest,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let accepted = report_of(&run_case(&case, Some("5")));
    assert_eq!(accepted["result_accepted"], true);
    assert!(marker(&case).exists(), "absolute pin did not run");
    let _ = fs::remove_dir_all(&case.root);
}

#[test]
fn wrong_executable_sha256_is_refused_before_execution() {
    let case = prepare("correct", "safety", 65536);
    let mut manifest = manifest_value(&case.adapter, &"0".repeat(64), false, false);
    fs::write(
        &case.manifest,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let refused = report_of(&run_case(&case, Some("5")));
    assert_eq!(refused["rejection_reason"], "executable_sha256_mismatch");
    assert_eq!(refused["executed"], false);
    assert_eq!(refused["result_accepted"], false);
    assert!(!marker(&case).exists(), "wrong sha256 was executed");

    manifest = manifest_value(&case.adapter, &sha256(&case.adapter), false, false);
    fs::write(
        &case.manifest,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let accepted = report_of(&run_case(&case, Some("5")));
    assert_eq!(accepted["result_accepted"], true);
    assert!(marker(&case).exists());
    let _ = fs::remove_dir_all(&case.root);
}

#[test]
fn supports_everything_is_refused_before_execution() {
    let case = prepare("correct", "safety", 65536);
    let manifest = manifest_value(&case.adapter, &sha256(&case.adapter), false, true);
    fs::write(
        &case.manifest,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let output = run_case(&case, Some("5"));
    let body = report_of(&output);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(body["rejection_reason"], "supports_everything");
    assert!(!marker(&case).exists(), "broad claim was executed");
    let _ = fs::remove_dir_all(&case.root);
}

#[test]
fn correct_result_is_accepted_without_a_backend_verdict() {
    let case = prepare("correct", "safety", 65536);
    let output = run_case(&case, Some("5"));
    let body = report_of(&output);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(body["result_accepted"], true);
    assert!(body["rejection_reason"].is_null());
    assert!(body.get("verdict").is_none());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("\"ACCEPT\""));
    assert_eq!(body["findings"][0]["finding"], "no_violation_found");
    assert_eq!(
        body["findings"][0]["host_interpretation"],
        "no_violation_found_not_proof"
    );
    let required = goal(&body, "required-goal").expect("required goal");
    assert_eq!(required["status"], "blocked");
    assert_eq!(required["reason"], "incomplete");
    assert_eq!(required["meets_requirement"], false);
    let optional = goal(&body, "optional-goal").expect("optional goal");
    assert_eq!(optional["status"], "not_required");
    assert_eq!(optional["reason"], "optional_exclusion");
    assert_eq!(optional["meets_requirement"], false);
    assert_eq!(body["controls"]["hostile_code_sandbox"], false);
    assert_eq!(body["controls"]["shell_invocation"], false);
    assert_eq!(body["controls"]["credentials_passed"], false);
    assert_eq!(body["controls"]["production_store_path_passed"], false);
    assert_eq!(body["controls"]["environment_allowlist"], json!([]));
    assert_eq!(body["controls"]["network_restricted_by_host"], false);
    let env = fs::read_to_string(case.out.join("adapter/observed-env.txt")).expect("env");
    assert!(!env.lines().any(|line| line == "IK_ANALYSIS_BEHAVIOR"));
    assert!(!env.lines().any(|line| line == "AWS_SECRET_ACCESS_KEY"));
    assert!(case.out.join("host-report.json").is_file());
    assert!(fs::read_dir(&case.out).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
    let _ = fs::remove_dir_all(&case.root);
}

#[test]
fn violation_with_nonzero_exit_is_accepted_and_exit_without_result_is_not() {
    let violation = prepare("violation", "safety", 65536);
    let output = run_case(&violation, Some("5"));
    let body = report_of(&output);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(body["result_accepted"], true);
    assert_eq!(body["adapter_exit_code"], 2);
    assert_eq!(body["findings"][0]["finding"], "violation");
    let required = goal(&body, "required-goal").unwrap();
    assert_eq!(required["status"], "not_met");
    assert_eq!(required["reason"], "violation");
    assert_eq!(required["meets_requirement"], false);
    let _ = fs::remove_dir_all(&violation.root);

    let missing = prepare("exit_0_no_result", "safety", 65536);
    let output = run_case(&missing, Some("5"));
    let body = report_of(&output);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(body["result_accepted"], false);
    assert_eq!(body["rejection_reason"], "exit_without_result");
    assert!(body["findings"].as_array().unwrap().is_empty());
    assert_eq!(
        goal(&body, "required-goal").unwrap()["meets_requirement"],
        false
    );
    let _ = fs::remove_dir_all(&missing.root);

    let crash = prepare("crash", "safety", 65536);
    let body = report_of(&run_case(&crash, Some("5")));
    assert_eq!(body["result_accepted"], false);
    assert_eq!(body["rejection_reason"], "failed");
    let _ = fs::remove_dir_all(&crash.root);
}

#[test]
fn reachability_witness_meets_and_incomplete_search_is_not_unreachable() {
    let witness = prepare("reachability_witness", "reachability", 65536);
    let body = report_of(&run_case(&witness, Some("5")));
    assert_eq!(body["result_accepted"], true);
    assert_eq!(body["findings"][0]["finding"], "witness_found");
    let required = goal(&body, "required-goal").unwrap();
    assert_eq!(required["status"], "met");
    assert_eq!(required["reason"], "witness_found");
    assert_eq!(required["meets_requirement"], true);
    let _ = fs::remove_dir_all(&witness.root);

    let incomplete = prepare("no_witness_incomplete", "reachability", 65536);
    let body = report_of(&run_case(&incomplete, Some("5")));
    assert_eq!(body["result_accepted"], true);
    assert_eq!(body["findings"][0]["finding"], "no_witness_found");
    assert_eq!(
        body["findings"][0]["host_interpretation"],
        "inconclusive_not_unreachable"
    );
    assert_ne!(body["findings"][0]["host_interpretation"], "unreachable");
    let required = goal(&body, "required-goal").unwrap();
    assert_eq!(required["status"], "blocked");
    assert_eq!(required["reason"], "incomplete");
    assert_eq!(required["meets_requirement"], false);
    let _ = fs::remove_dir_all(&incomplete.root);
}

#[test]
fn unsupported_and_missing_tool_block_the_required_goal() {
    let unsupported = prepare("unsupported", "safety", 65536);
    let body = report_of(&run_case(&unsupported, Some("5")));
    assert_eq!(body["result_accepted"], true);
    assert_eq!(body["execution_status"], "unsupported");
    let required = goal(&body, "required-goal").unwrap();
    assert_eq!(required["status"], "blocked");
    assert_eq!(required["reason"], "unsupported");
    assert_eq!(required["meets_requirement"], false);
    let _ = fs::remove_dir_all(&unsupported.root);

    let missing = prepare("missing_tool", "safety", 65536);
    let body = report_of(&run_case(&missing, Some("5")));
    assert_eq!(body["result_accepted"], true);
    assert_eq!(body["execution_status"], "tool_unavailable");
    assert_eq!(
        goal(&body, "required-goal").unwrap()["reason"],
        "unsupported"
    );
    assert_eq!(
        goal(&body, "required-goal").unwrap()["meets_requirement"],
        false
    );
    let _ = fs::remove_dir_all(&missing.root);

    let omitted = prepare("missing_finding", "safety", 65536);
    let body = report_of(&run_case(&omitted, Some("5")));
    assert_eq!(body["result_accepted"], true);
    assert_eq!(goal(&body, "required-goal").unwrap()["reason"], "missing");
    assert_eq!(goal(&body, "required-goal").unwrap()["status"], "blocked");
    let _ = fs::remove_dir_all(&omitted.root);
}

#[test]
fn wrong_request_id_and_stale_subject_hash_are_rejected() {
    let good = prepare("correct", "safety", 65536);
    assert_eq!(accepted_correct(&good)["result_accepted"], true);
    let _ = fs::remove_dir_all(&good.root);

    let wrong = prepare("wrong_request_id", "safety", 65536);
    let body = report_of(&run_case(&wrong, Some("5")));
    assert_eq!(body["result_accepted"], false);
    assert_eq!(body["rejection_reason"], "request_id_mismatch");
    assert!(body["findings"].as_array().unwrap().is_empty());
    let _ = fs::remove_dir_all(&wrong.root);

    let stale = prepare("stale_subject_hash", "safety", 65536);
    let body = report_of(&run_case(&stale, Some("5")));
    assert_eq!(body["result_accepted"], false);
    assert_eq!(body["rejection_reason"], "stale_subject_hash");
    let _ = fs::remove_dir_all(&stale.root);
}

#[test]
fn malformed_truncated_traversal_and_symlink_outputs_are_rejected() {
    let good = prepare("correct", "safety", 65536);
    assert_eq!(accepted_correct(&good)["result_accepted"], true);
    let _ = fs::remove_dir_all(&good.root);

    let malformed = prepare("malformed_json", "safety", 65536);
    let body = report_of(&run_case(&malformed, Some("5")));
    assert_eq!(body["rejection_reason"], "malformed_json");
    assert_eq!(body["result_accepted"], false);
    let _ = fs::remove_dir_all(&malformed.root);

    let truncated = prepare("truncated_artifact", "safety", 65536);
    let body = report_of(&run_case(&truncated, Some("5")));
    assert_eq!(body["rejection_reason"], "truncated_artifact");
    assert_eq!(body["result_accepted"], false);
    let _ = fs::remove_dir_all(&truncated.root);

    let traversal = prepare("illegal_traversal", "safety", 65536);
    let body = report_of(&run_case(&traversal, Some("5")));
    assert_eq!(body["rejection_reason"], "path_traversal");
    assert_eq!(body["result_accepted"], false);
    let _ = fs::remove_dir_all(&traversal.root);

    let symlink = prepare("illegal_symlink", "safety", 65536);
    let body = report_of(&run_case(&symlink, Some("5")));
    assert_eq!(body["rejection_reason"], "symlink_rejected");
    assert_eq!(body["result_accepted"], false);
    let _ = fs::remove_dir_all(&symlink.root);
}

#[test]
fn timeout_kills_the_process_tree_and_does_not_accept_a_written_result() {
    let case = prepare("timeout", "safety", 65536);
    let output = run_case(&case, Some("1"));
    let body = report_of(&output);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(body["result_accepted"], false);
    assert_eq!(body["rejection_reason"], "timed_out");
    assert_eq!(body["execution_status"], "timed_out");
    assert!(body["findings"].as_array().unwrap().is_empty());
    let native = fs::read_to_string(case.out.join("adapter/result.json")).expect("native result");
    assert!(native.contains("no_violation_found"));
    assert_eq!(goal(&body, "required-goal").unwrap()["status"], "blocked");
    assert_eq!(
        goal(&body, "required-goal").unwrap()["reason"],
        "incomplete"
    );
    let (parent, child) = pids(&case.pid_file).expect("pid file");
    assert!(became_dead(parent), "parent {parent} still running");
    assert!(became_dead(child), "child {child} still running");
    let _ = fs::remove_dir_all(&case.root);
}

#[test]
fn kill_mid_output_keeps_partial_logs_and_rejects() {
    let case = prepare("kill_mid_output", "safety", 65536);
    let body = report_of(&run_case(&case, Some("1")));
    assert_eq!(body["result_accepted"], false);
    assert_eq!(body["rejection_reason"], "timed_out");
    assert!(body["findings"].as_array().unwrap().is_empty());
    let partial = fs::read_to_string(case.out.join("adapter/result.json")).expect("partial");
    assert!(partial.starts_with("{\"protocol\":"));
    assert!(!serde_json::from_str::<Value>(&partial).is_ok());
    let stdout = fs::read_to_string(case.out.join("native/stdout.log")).expect("stdout log");
    assert!(stdout.contains("partial-stdout"));
    assert!(body["partial_logs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == "native/stdout.log"));
    let (parent, child) = pids(&case.pid_file).expect("pid file");
    assert!(became_dead(parent), "parent {parent} still running");
    assert!(became_dead(child), "child {child} still running");
    let _ = fs::remove_dir_all(&case.root);
}

#[test]
fn cancellation_kills_the_process_tree() {
    let case = prepare("timeout", "safety", 65536);
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    let manifest = case.manifest.clone();
    let request = case.request.clone();
    let out = case.out.clone();
    let pid_file = case.pid_file.clone();
    let handle = std::thread::spawn(move || {
        run_adapter(&RunSpec {
            manifest_path: manifest,
            request_path: request,
            out_dir: out,
            timeout_override: Some(Duration::from_secs(6)),
            cancel: Some(flag),
        })
    });
    let start = Instant::now();
    while !pid_file.exists() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        pid_file.exists(),
        "adapter did not record pids before cancel"
    );
    cancel.store(true, Ordering::Relaxed);
    let report = handle.join().expect("join");
    assert_eq!(report.rejection_reason.as_deref(), Some("cancelled"));
    assert!(!report.result_accepted);
    assert!(report.findings.is_empty());
    let (parent, child) = pids(&case.pid_file).unwrap();
    assert!(became_dead(parent), "parent {parent} still running");
    assert!(became_dead(child), "child {child} still running");
    let _ = fs::remove_dir_all(&case.root);
}

#[test]
fn witness_that_does_not_replay_is_rejected_and_a_replayable_witness_is_not() {
    let bad = prepare("witness_no_replay", "reachability", 65536);
    let body = report_of(&run_case(&bad, Some("5")));
    assert_eq!(body["result_accepted"], false);
    assert_eq!(body["rejection_reason"], "reproduction_claim_unsupported");
    assert_eq!(
        goal(&body, "required-goal").unwrap()["meets_requirement"],
        false
    );
    let _ = fs::remove_dir_all(&bad.root);

    let good = prepare("witness_replay_ok", "reachability", 65536);
    let body = report_of(&run_case(&good, Some("5")));
    assert_eq!(body["result_accepted"], true);
    assert_eq!(
        body["findings"][0]["reproduction_status"],
        "concrete_reproduced"
    );
    assert_eq!(
        goal(&body, "required-goal").unwrap()["meets_requirement"],
        true
    );
    let _ = fs::remove_dir_all(&good.root);
}

#[test]
fn holds_in_scope_requires_exhaustive_complete_coverage() {
    let sample = prepare("holds_from_sample", "safety", 65536);
    let body = report_of(&run_case(&sample, Some("5")));
    assert_eq!(body["result_accepted"], false);
    assert_eq!(
        body["rejection_reason"],
        "evidence_does_not_support_finding"
    );
    let _ = fs::remove_dir_all(&sample.root);

    let holds = prepare("holds", "safety", 65536);
    let body = report_of(&run_case(&holds, Some("5")));
    assert_eq!(body["result_accepted"], true);
    assert_eq!(body["findings"][0]["finding"], "holds_in_scope");
    assert_eq!(
        body["findings"][0]["evidence_class"],
        "exhaustive_finite_model"
    );
    let required = goal(&body, "required-goal").unwrap();
    assert_eq!(required["status"], "met");
    assert_eq!(required["reason"], "holds_in_scope");
    assert_eq!(required["meets_requirement"], true);
    let _ = fs::remove_dir_all(&holds.root);
}

#[test]
fn input_modified_during_the_run_invalidates_the_result() {
    let good = prepare("correct", "safety", 65536);
    assert_eq!(accepted_correct(&good)["result_accepted"], true);
    let unchanged = fs::read(good.bundle.join("input.txt")).unwrap();
    assert_eq!(unchanged, b"input-v1");
    let _ = fs::remove_dir_all(&good.root);

    let dirty = prepare("modify_input", "safety", 65536);
    let body = report_of(&run_case(&dirty, Some("5")));
    assert_eq!(body["result_accepted"], false);
    assert_eq!(body["rejection_reason"], "input_modified");
    assert_ne!(
        fs::read(dirty.bundle.join("input.txt")).unwrap(),
        b"input-v1"
    );
    let _ = fs::remove_dir_all(&dirty.root);
}

#[test]
fn backend_accept_verdict_is_rejected() {
    let case = prepare("backend_accept", "safety", 65536);
    let output = run_case(&case, Some("5"));
    let body = report_of(&output);
    assert_eq!(body["result_accepted"], false);
    assert_eq!(body["rejection_reason"], "backend_verdict_rejected");
    assert!(body.get("verdict").is_none());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("\"ACCEPT\""));
    let _ = fs::remove_dir_all(&case.root);
}

#[test]
fn production_store_access_is_refused_before_execution() {
    let case = prepare("correct", "safety", 65536);
    let manifest = manifest_value(&case.adapter, &sha256(&case.adapter), true, false);
    fs::write(
        &case.manifest,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let body = report_of(&run_case(&case, Some("5")));
    assert_eq!(
        body["rejection_reason"],
        "production_store_access_forbidden"
    );
    assert_eq!(body["executed"], false);
    assert!(!marker(&case).exists(), "store access claim was executed");
    let _ = fs::remove_dir_all(&case.root);
}

#[test]
fn stdout_over_the_limit_is_killed_and_bounded() {
    let case = prepare("flood_stdout", "safety", 128);
    let body = report_of(&run_case(&case, Some("5")));
    assert_eq!(body["result_accepted"], false);
    assert_eq!(body["rejection_reason"], "output_limit_exceeded");
    let stdout = fs::read(case.out.join("native/stdout.log")).expect("bounded log");
    assert!(stdout.len() <= 128, "stdout log grew to {}", stdout.len());
    let (parent, child) = pids(&case.pid_file).expect("pid file");
    assert!(became_dead(parent), "parent {parent} still running");
    assert!(became_dead(child), "child {child} still running");
    let _ = fs::remove_dir_all(&case.root);
}

#[test]
fn validate_result_checks_a_stored_result_without_execution() {
    let case = prepare("correct", "safety", 65536);
    let result = case.out.join("stored-result.json");
    fs::write(
        &result,
        serde_json::to_vec(&json!({
            "protocol": "ik.analysis.result.v1",
            "request_id": "req-1",
            "run_id": "run-1",
            "subject": {"kind": "machine", "identity": {"machine_id": "job", "spec_hash": "spec-hash-1"}},
            "goals": [
                {"id": "required-goal", "content_digest": "digest-required"},
                {"id": "optional-goal", "content_digest": "digest-optional"}
            ],
            "profile_digest": "profile-digest-1",
            "inputs": [{"path": "input.txt", "sha256": sha256(&case.bundle.join("input.txt"))}],
            "execution_status": "completed",
            "properties": [{
                "goal_id": "required-goal",
                "property_class": "safety",
                "finding": "holds_in_scope",
                "evidence_class": "exhaustive_finite_model",
                "coverage": {"complete": true, "reason": "closed"},
                "reproduction": {"status": "model_only", "artifact": null}
            }],
            "artifacts": []
        }))
        .unwrap(),
    )
    .unwrap();
    let output = bin()
        .args([
            "analysis",
            "validate-result",
            &case.request.display().to_string(),
            &result.display().to_string(),
        ])
        .output()
        .expect("validate");
    let body = report_of(&output);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(body["result_accepted"], true);
    assert_eq!(
        goal(&body, "required-goal").unwrap()["meets_requirement"],
        true
    );
    assert!(
        !marker(&case).exists(),
        "validate-result executed the adapter"
    );

    let mut stale: Value = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
    stale["subject"]["identity"]["spec_hash"] = json!("other-hash");
    fs::write(&result, serde_json::to_vec(&stale).unwrap()).unwrap();
    let output = bin()
        .args([
            "analysis",
            "validate-result",
            &case.request.display().to_string(),
            &result.display().to_string(),
        ])
        .output()
        .expect("validate stale");
    let body = report_of(&output);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(body["rejection_reason"], "stale_subject_hash");
    assert!(!marker(&case).exists());
    let _ = fs::remove_dir_all(&case.root);
}

#[test]
fn requirement_mapping_blocks_required_gaps_and_meets_only_supported_findings() {
    let goals = vec![
        GoalRef {
            id: "required-goal".into(),
            content_digest: "digest-required".into(),
            required: true,
        },
        GoalRef {
            id: "optional-goal".into(),
            content_digest: "digest-optional".into(),
            required: false,
        },
    ];
    let incomplete = property("no_violation_found", "sampled_execution", false);
    let blocked = requirement_status(&goals, "completed", &[incomplete]);
    assert_eq!(blocked[0].reason, "incomplete");
    assert!(!blocked[0].meets_requirement);
    assert_eq!(blocked[1].status, "not_required");

    let holds = property("holds_in_scope", "exhaustive_finite_model", true);
    let met = requirement_status(&goals, "completed", &[holds]);
    assert_eq!(met[0].status, "met");
    assert!(met[0].meets_requirement);

    let missing = requirement_status(&goals, "tool_unavailable", &[]);
    assert_eq!(missing[0].status, "blocked");
    assert_eq!(missing[0].reason, "unsupported");
    assert!(!missing[0].meets_requirement);
}

fn property(finding: &str, evidence: &str, complete: bool) -> PropertyResult {
    PropertyResult {
        goal_id: "required-goal".into(),
        property_class: "safety".into(),
        finding: finding.into(),
        evidence_class: evidence.into(),
        coverage: Coverage {
            complete,
            reason: "scope".into(),
        },
        reproduction: Reproduction {
            status: "not_attempted".into(),
            artifact: None,
        },
    }
}
