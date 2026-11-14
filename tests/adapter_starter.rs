//! Starter adapter conformance and the prepared P route.
//!
//! The positive control exports a real graph and runs `ik analysis conformance`
//! against the shipped starter. Negative controls copy that source, edit one
//! marker, and require the named case to fail.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ik"))
}

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn sha256(path: &Path) -> String {
    ik::canonical::sha256_hex(&fs::read(path).expect("read"))
}

fn temp_root(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ik-starter-{}-{}-{}",
        std::process::id(),
        label,
        unique()
    ));
    fs::create_dir_all(&path).expect("temp");
    path
}

fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

fn export(machine: &str, extra: &[&str], out: &Path) -> Value {
    let done = bin()
        .arg("graph")
        .arg("export")
        .arg(repo(&format!("tests/fixtures/profile/{machine}")))
        .arg("--out")
        .arg(out)
        .args(extra)
        .output()
        .expect("export");
    assert_eq!(
        done.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    serde_json::from_str(&fs::read_to_string(out).expect("graph")).expect("json")
}

fn stage_fixtures(root: &Path, graphs: &[(&str, &Path)]) -> PathBuf {
    let dest = root.join("fixtures");
    let cases = dest.join("cases");
    fs::create_dir_all(&cases).expect("cases");
    let shipped = repo("adapters/starter/fixtures/cases");
    for entry in fs::read_dir(&shipped).expect("shipped cases") {
        let entry = entry.expect("entry");
        fs::copy(entry.path(), cases.join(entry.file_name())).expect("copy case");
    }
    let graphs_dir = dest.join("graphs");
    fs::create_dir_all(&graphs_dir).expect("graphs");
    for (name, path) in graphs {
        fs::copy(path, graphs_dir.join(name)).expect("copy graph");
    }
    dest
}

fn write_manifest(dir: &Path, adapter: &Path, id: &str, engine: &str, constraint: &str) -> PathBuf {
    let path = dir.join("manifest.json");
    let body = json!({
        "protocol": "ik.analysis.adapter.v1",
        "adapter": {"id": id, "version": "1"},
        "invocation": {
            "argv": [adapter.display().to_string()],
            "executable_sha256": sha256(adapter)
        },
        "engine": {"id": engine, "version_constraint": constraint},
        "inputs": ["finite_runtime_graph"],
        "properties": ["finite_scope_stuck"],
        "evidence": ["bounded_paths", "exhaustive_finite_model"],
        "requires": {"production_store_access": false, "network": false},
        "result_schema": "ik.analysis.result.v1"
    });
    fs::write(&path, serde_json::to_vec_pretty(&body).unwrap()).unwrap();
    path
}

fn conformance(manifest: &Path, fixtures: &Path) -> (i32, Value) {
    let done = bin()
        .args([
            "analysis",
            "conformance",
            &manifest.display().to_string(),
            "--fixtures",
            &fixtures.display().to_string(),
        ])
        .output()
        .expect("conformance");
    let body = serde_json::from_slice(&done.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout not json: {} stderr: {}",
            String::from_utf8_lossy(&done.stdout),
            String::from_utf8_lossy(&done.stderr)
        )
    });
    (done.status.code().unwrap_or(99), body)
}

fn case<'a>(body: &'a Value, id: &str) -> &'a Value {
    body["cases"]
        .as_array()
        .and_then(|cases| cases.iter().find(|case| case["id"] == id))
        .unwrap_or_else(|| panic!("missing case {id} in {body}"))
}

fn variant(dir: &Path, from: &str, to: &str) -> PathBuf {
    let mut text = fs::read_to_string(repo("adapters/starter/adapter.py")).expect("starter");
    assert!(text.contains(from), "marker missing: {from}");
    text = text.replace(from, to);
    let dest = dir.join("adapter.py");
    fs::write(&dest, text).expect("write variant");
    let mut perms = fs::metadata(&dest).expect("meta").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&dest, perms).expect("chmod");
    dest
}

fn staged(root: &Path) -> (PathBuf, Value, Value, Value) {
    let linear_path = root.join("linear.json");
    let dead_path = root.join("dead.json");
    let cut_path = root.join("cut.json");
    let linear = export("linear.ik.toml", &[], &linear_path);
    let dead = export("dead_end.ik.toml", &[], &dead_path);
    let cut = export("linear.ik.toml", &["--max-depth", "1"], &cut_path);
    let fixtures = stage_fixtures(
        root,
        &[
            ("linear-complete.json", &linear_path),
            ("dead-end-complete.json", &dead_path),
            ("linear-depth-1.json", &cut_path),
        ],
    );
    (fixtures, linear, dead, cut)
}

#[test]
fn shipped_starter_passes_conformance_on_an_exported_graph() {
    let root = temp_root("pass");
    let (fixtures, linear, dead, cut) = staged(&root);
    assert_eq!(linear["schema"], "ik.graph.v1");
    assert_eq!(linear["complete"], true);
    assert_eq!(dead["complete"], true);
    assert_eq!(cut["complete"], false);
    assert_eq!(cut["incomplete_reason"], "max_depth");
    fs::write(
        fixtures.join("cases/skipped_example.json"),
        r#"{"id":"skipped_example","graph":"graphs/linear-complete.json","property_class":"finite_scope_stuck","question":"dead_end_count","skip":"not a required behavior","expect":{}}"#,
    )
    .expect("skip case");
    let adapter = repo("adapters/starter/adapter.py");
    let manifest = write_manifest(&root, &adapter, "starter-graph", "starter-graph", "1");
    let (code, body) = conformance(&manifest, &fixtures);
    assert_eq!(code, 0, "{body}");
    assert_eq!(body["passed"], true);
    assert_eq!(body["engine_correctness_proven"], false);
    assert!(body["note"]
        .as_str()
        .unwrap()
        .contains("engine correctness"));
    assert_eq!(body["failure_count"], 0);
    for id in [
        "correct_result",
        "real_violation",
        "incomplete_search",
        "unsupported_property",
        "preserves_native_logs",
        "honors_timeout",
        "honors_cancellation",
        "wrong_request_id",
        "stale_subject_hash",
        "malformed_output",
        "path_traversal",
        "exit_without_result",
    ] {
        assert_eq!(
            case(&body, id)["status"],
            "pass",
            "{id} {}",
            case(&body, id)
        );
    }
    assert_eq!(case(&body, "real_violation")["reason"], "violation");
    assert_eq!(case(&body, "incomplete_search")["reason"], "inconclusive");
    assert_eq!(
        case(&body, "wrong_request_id")["reason"],
        "request_id_mismatch"
    );
    assert_eq!(
        case(&body, "exit_without_result")["reason"],
        "exit_without_result"
    );
    assert_eq!(case(&body, "skipped_example")["status"], "skipped");
    assert_eq!(body["failure_count"], 0);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn wrong_request_id_variant_fails_the_correlated_case() {
    let root = temp_root("id");
    let (fixtures, _, _, _) = staged(&root);
    let adapter = variant(
        &root,
        "REQUEST_ID_MODE = \"echo\"",
        "REQUEST_ID_MODE = \"wrong\"",
    );
    let manifest = write_manifest(&root, &adapter, "starter-graph", "starter-graph", "1");
    let (code, body) = conformance(&manifest, &fixtures);
    assert_eq!(code, 1);
    assert_eq!(case(&body, "correct_result")["status"], "fail");
    assert!(case(&body, "correct_result")["reason"]
        .as_str()
        .unwrap()
        .contains("request_id_mismatch"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn outside_write_variant_fails_the_contained_case() {
    let root = temp_root("escape");
    let (fixtures, _, _, _) = staged(&root);
    let adapter = variant(
        &root,
        "CONTAINED_WRITES = \"contained\"",
        "CONTAINED_WRITES = \"escape\"",
    );
    let manifest = write_manifest(&root, &adapter, "starter-graph", "starter-graph", "1");
    let (code, body) = conformance(&manifest, &fixtures);
    assert_eq!(code, 1);
    assert_eq!(case(&body, "correct_result")["status"], "fail");
    assert!(case(&body, "correct_result")["reason"]
        .as_str()
        .unwrap()
        .contains("path_traversal"));
    assert_eq!(case(&body, "unsupported_property")["status"], "pass");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn exit_without_result_variant_fails_the_result_case() {
    let root = temp_root("noresult");
    let (fixtures, _, _, _) = staged(&root);
    let adapter = variant(&root, "WRITE_RESULT = \"write\"", "WRITE_RESULT = \"skip\"");
    let manifest = write_manifest(&root, &adapter, "starter-graph", "starter-graph", "1");
    let (code, body) = conformance(&manifest, &fixtures);
    assert_eq!(code, 1);
    assert_eq!(case(&body, "correct_result")["status"], "fail");
    assert!(case(&body, "correct_result")["reason"]
        .as_str()
        .unwrap()
        .contains("exit_without_result"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn overclaim_on_incomplete_graph_fails_the_honesty_case() {
    let root = temp_root("overclaim");
    let (fixtures, _, _, _) = staged(&root);
    let adapter = variant(
        &root,
        "INCOMPLETE_FINDING = \"honest\"",
        "INCOMPLETE_FINDING = \"overclaim\"",
    );
    let manifest = write_manifest(&root, &adapter, "starter-graph", "starter-graph", "1");
    let (code, body) = conformance(&manifest, &fixtures);
    assert_eq!(code, 1);
    assert_eq!(case(&body, "incomplete_search")["status"], "fail");
    assert!(case(&body, "incomplete_search")["reason"]
        .as_str()
        .unwrap()
        .contains("holds_in_scope"));
    assert_eq!(case(&body, "correct_result")["status"], "pass");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn dropped_native_log_fails_the_log_case() {
    let root = temp_root("log");
    let (fixtures, _, _, _) = staged(&root);
    let adapter = variant(
        &root,
        "KEEP_NATIVE_LOG = \"keep\"",
        "KEEP_NATIVE_LOG = \"drop\"",
    );
    let manifest = write_manifest(&root, &adapter, "starter-graph", "starter-graph", "1");
    let (code, body) = conformance(&manifest, &fixtures);
    assert_eq!(code, 1);
    assert_eq!(case(&body, "preserves_native_logs")["status"], "fail");
    assert!(case(&body, "preserves_native_logs")["reason"]
        .as_str()
        .unwrap()
        .contains("native_log"));
    assert_eq!(case(&body, "correct_result")["status"], "pass");
    let _ = fs::remove_dir_all(&root);
}

fn set_case_timeout(fixtures: &Path, id: &str, seconds: u64) {
    let path = fixtures.join(format!("cases/{id}.json"));
    let mut spec: Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("case")).expect("json");
    spec["timeout_seconds"] = json!(seconds);
    fs::write(
        &path,
        serde_json::to_vec_pretty(&spec).expect("serialize"),
    )
    .expect("write case");
}

#[test]
fn timeout_hook_removed_fails_the_timeout_case() {
    let root = temp_root("timeout");
    let (fixtures, _, _, _) = staged(&root);
    // Skip must finish; 1s wall time is not the timeout contract under load.
    set_case_timeout(&fixtures, "honors_timeout", 60);
    let adapter = variant(
        &root,
        "HONOR_TIMEOUT = \"sleep\"",
        "HONOR_TIMEOUT = \"skip\"",
    );
    let manifest = write_manifest(&root, &adapter, "starter-graph", "starter-graph", "1");
    let (code, body) = conformance(&manifest, &fixtures);
    assert_eq!(code, 1);
    assert_eq!(case(&body, "honors_timeout")["status"], "fail");
    assert_ne!(case(&body, "honors_timeout")["reason"], "timed_out");
    assert_eq!(case(&body, "correct_result")["status"], "pass");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn cancel_hook_removed_fails_the_cancellation_case() {
    let root = temp_root("cancel");
    let (fixtures, _, _, _) = staged(&root);
    let adapter = variant(&root, "HONOR_CANCEL = \"sleep\"", "HONOR_CANCEL = \"skip\"");
    let manifest = write_manifest(&root, &adapter, "starter-graph", "starter-graph", "1");
    let (code, body) = conformance(&manifest, &fixtures);
    assert_eq!(code, 1);
    assert_eq!(case(&body, "honors_cancellation")["status"], "fail");
    assert_eq!(case(&body, "correct_result")["status"], "pass");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn p_describe_reports_tool_unavailable_and_executes_nothing() {
    let root = temp_root("p");
    let script = repo("adapters/p/adapter.py");
    let which_p = Command::new("which").arg("p").status().expect("which p");
    let which_dotnet = Command::new("which")
        .arg("dotnet")
        .status()
        .expect("which dotnet");
    assert!(!which_p.success());
    assert!(!which_dotnet.success());
    let described = Command::new(&script)
        .arg("describe")
        .current_dir(&root)
        .output()
        .expect("p describe");
    assert_eq!(described.status.code(), Some(1));
    let body: Value = serde_json::from_slice(&described.stdout).expect("describe json");
    assert_eq!(body["execution_status"], "tool_unavailable");
    assert_eq!(body["probe_found"], false);
    assert_eq!(body["dotnet_found"], false);
    assert_eq!(body["checker_executed"], false);
    assert_eq!(body["executed"], false);
    assert!(fs::read_dir(&root).expect("dir").next().is_none());
    let manifest = write_manifest(&root, &script, "p-route", "P", "p --version");
    let host = bin()
        .args(["analysis", "describe", &manifest.display().to_string()])
        .output()
        .expect("host describe");
    assert_eq!(host.status.code(), Some(0));
    let host_body: Value = serde_json::from_slice(&host.stdout).expect("host json");
    assert_eq!(host_body["executed"], false);
    assert!(host_body["rejection_reason"].is_null());
    let bug = Command::new(&script)
        .args([
            "parse",
            "--fixture",
            &repo("adapters/p/fixtures/bug-found.txt")
                .display()
                .to_string(),
        ])
        .output()
        .expect("parse bug");
    let bug_body: Value = serde_json::from_slice(&bug.stdout).expect("bug json");
    assert_eq!(bug_body["finding"], "violation");
    assert_eq!(bug_body["holds_in_scope"], false);
    let bounded = Command::new(&script)
        .args([
            "parse",
            "--fixture",
            &repo("adapters/p/fixtures/bounded-pass.txt")
                .display()
                .to_string(),
        ])
        .output()
        .expect("parse pass");
    let bounded_body: Value = serde_json::from_slice(&bounded.stdout).expect("pass json");
    assert_eq!(bounded_body["finding"], "no_violation_found");
    assert_eq!(bounded_body["coverage_complete"], false);
    assert_ne!(bounded_body["finding"], "holds_in_scope");
    let _ = fs::remove_dir_all(&root);
}
