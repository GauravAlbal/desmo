//! `ik analysis conformance`: host-contract battery for an adapter author.
//!
//! Each fixture is run through [`super::runner::run_adapter`], the same path as
//! `ik analysis run`. Passing proves the adapter met the host contract on these
//! fixtures. It does not prove the checker or the engine is correct.
//!
//! Fixtures whose fault an honest adapter cannot emit are injected after the
//! process exits, then judged by the same result checker. `timeout` and
//! `cancel` are passed to the adapter as `controls.conformance_fault` so the
//! host can kill the process and refuse a result that was already written.

use super::manifest::{check_executable, load_manifest};
use super::paths::sha256_file;
use super::request::{self, snapshot_inputs, AnalysisRequest};
use super::runner::{judge_written_result, run_adapter, HostReport, RunSpec};
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub const PROTOCOL: &str = "ik.analysis.conformance.v1";
pub const NOTE: &str =
    "Passing conformance proves host-contract conformance, not engine correctness.";
const CANCEL_DELAY: Duration = Duration::from_millis(2000);
static WORK_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize)]
pub struct ConformanceReport {
    pub protocol: &'static str,
    pub engine_correctness_proven: bool,
    pub note: &'static str,
    pub passed: bool,
    pub failure_count: usize,
    pub cases: Vec<CaseReport>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaseReport {
    pub id: String,
    pub status: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
struct Battery {
    manifest: PathBuf,
    fixtures: PathBuf,
    cases: Vec<CaseSpec>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct CaseSpec {
    id: String,
    graph: String,
    property_class: String,
    question: String,
    #[serde(default)]
    skip: Option<String>,
    #[serde(default)]
    fault: Option<String>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    expect: Expect,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct Expect {
    #[serde(default)]
    result_accepted: Option<bool>,
    #[serde(default)]
    rejection_reason: Option<String>,
    #[serde(default)]
    execution_status: Option<String>,
    #[serde(default)]
    finding: Option<String>,
    #[serde(default)]
    evidence_class: Option<String>,
    #[serde(default)]
    coverage_complete: Option<bool>,
    #[serde(default)]
    host_interpretation: Option<String>,
    #[serde(default)]
    native_log: Option<String>,
    #[serde(default)]
    stdout_preserved: Option<bool>,
}

struct PreparedCase {
    request_path: PathBuf,
    request: AnalysisRequest,
    out_dir: PathBuf,
    work: PathBuf,
}

pub fn run(manifest: &Path, fixtures: Option<&Path>) -> ConformanceReport {
    match load_battery(manifest, fixtures) {
        Err(error) => usage_report(&error),
        Ok(battery) => execute_battery(&battery),
    }
}

fn usage_report(error: &str) -> ConformanceReport {
    ConformanceReport {
        protocol: PROTOCOL,
        engine_correctness_proven: false,
        note: NOTE,
        passed: false,
        failure_count: 0,
        cases: Vec::new(),
        error: Some(error.to_string()),
    }
}

fn load_battery(manifest: &Path, fixtures: Option<&Path>) -> Result<Battery, String> {
    let loaded = load_manifest(manifest)?;
    check_executable(&loaded)?;
    let fixtures = resolve_fixtures(manifest, fixtures)?;
    let cases = read_cases(&fixtures)?;
    Ok(Battery {
        manifest: manifest.to_path_buf(),
        fixtures,
        cases,
    })
}

fn resolve_fixtures(manifest: &Path, fixtures: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = fixtures {
        return fixture_dir(path);
    }
    let parent = manifest.parent().unwrap_or_else(|| Path::new("."));
    fixture_dir(&parent.join("fixtures"))
}

fn fixture_dir(path: &Path) -> Result<PathBuf, String> {
    let cases = path.join("cases");
    if !cases.is_dir() {
        return Err(format!(
            "fixtures missing cases directory: {}",
            path.display()
        ));
    }
    Ok(path.to_path_buf())
}

fn read_cases(fixtures: &Path) -> Result<Vec<CaseSpec>, String> {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixtures.join("cases"))
        .map_err(|error| error.to_string())?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err("no conformance cases".into());
    }
    paths.iter().map(|path| read_case(path)).collect()
}

fn read_case(path: &Path) -> Result<CaseSpec, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn execute_battery(battery: &Battery) -> ConformanceReport {
    let cases: Vec<CaseReport> = battery
        .cases
        .iter()
        .map(|case| execute_case(battery, case))
        .collect();
    let failure_count = cases.iter().filter(|case| case.status == "fail").count();
    ConformanceReport {
        protocol: PROTOCOL,
        engine_correctness_proven: false,
        note: NOTE,
        passed: failure_count == 0,
        failure_count,
        cases,
        error: None,
    }
}

fn execute_case(battery: &Battery, case: &CaseSpec) -> CaseReport {
    if let Some(reason) = case.skip.as_deref() {
        return skipped(&case.id, reason);
    }
    let prepared = match prepare_case(&battery.fixtures, case) {
        Ok(prepared) => prepared,
        Err(reason) => return fail(&case.id, &reason),
    };
    let report = judge_case(&battery.manifest, case, &prepared);
    let _ = fs::remove_dir_all(&prepared.work);
    report
}

fn skipped(id: &str, reason: &str) -> CaseReport {
    CaseReport {
        id: id.to_string(),
        status: "skipped".into(),
        reason: reason.to_string(),
    }
}

fn fail(id: &str, reason: &str) -> CaseReport {
    CaseReport {
        id: id.to_string(),
        status: "fail".into(),
        reason: reason.to_string(),
    }
}

fn pass(id: &str, reason: &str) -> CaseReport {
    CaseReport {
        id: id.to_string(),
        status: "pass".into(),
        reason: reason.to_string(),
    }
}

fn prepare_case(fixtures: &Path, case: &CaseSpec) -> Result<PreparedCase, String> {
    let work = work_dir(&case.id);
    let bundle = work.join("bundle");
    let out_dir = work.join("out");
    fs::create_dir_all(&bundle).map_err(|error| error.to_string())?;
    let graph_src = fixtures.join(&case.graph);
    let graph_bytes = fs::read(&graph_src).map_err(|_| format!("missing graph {}", case.graph))?;
    fs::write(bundle.join("graph.json"), &graph_bytes).map_err(|error| error.to_string())?;
    let graph: Value =
        serde_json::from_slice(&graph_bytes).map_err(|_| "graph is not json".to_string())?;
    let request = request_value(
        case,
        &bundle,
        &work,
        &graph,
        &sha256_file(&bundle.join("graph.json"))?,
    )?;
    let request_path = work.join("request.json");
    let bytes = serde_json::to_vec_pretty(&request).map_err(|error| error.to_string())?;
    fs::write(&request_path, bytes).map_err(|error| error.to_string())?;
    let loaded = request::load_request(&request_path)?;
    Ok(PreparedCase {
        request_path,
        request: loaded,
        out_dir,
        work,
    })
}

fn work_dir(id: &str) -> PathBuf {
    let n = WORK_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ik-conformance-{}-{}-{id}", std::process::id(), n))
}

fn request_value(
    case: &CaseSpec,
    bundle: &Path,
    scratch_root: &Path,
    graph: &Value,
    digest: &str,
) -> Result<Value, String> {
    let machine_id = graph_str(graph, "machine_id")?;
    let spec_hash = graph_str(graph, "spec_hash")?;
    let mut controls = serde_json::Map::new();
    controls.insert("question".into(), json!(case.question));
    if sdk_fault(case.fault.as_deref()) {
        controls.insert(
            "conformance_fault".into(),
            json!(case.fault.clone().unwrap_or_default()),
        );
    }
    Ok(json!({
        "protocol": "ik.analysis.request.v1",
        "request_id": format!("conf-{}", case.id),
        "run_id": format!("run-{}", case.id),
        "subject": {
            "kind": "finite_runtime_graph",
            "identity": {"machine_id": machine_id, "spec_hash": spec_hash}
        },
        "goals": [{
            "id": case.question,
            "content_digest": sha256_text(&case.question),
            "required": true
        }],
        "profile_digest": sha256_text(&case.property_class),
        "property_class": case.property_class,
        "inputs": [{"path": "graph.json", "sha256": digest}],
        "bundle_dir": bundle.display().to_string(),
        "limits": {"timeout_seconds": 30, "max_output_bytes": 65536},
        "permissions": {
            "scratch_root": scratch_root.display().to_string(),
            "production_store_access": false
        },
        "controls": controls
    }))
}

fn graph_str(graph: &Value, key: &str) -> Result<String, String> {
    graph
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("graph missing {key}"))
}

fn sha256_text(text: &str) -> String {
    crate::canonical::sha256_hex(text.as_bytes())
}

fn sdk_fault(fault: Option<&str>) -> bool {
    matches!(fault, Some("timeout" | "cancel"))
}

fn host_fault(fault: Option<&str>) -> bool {
    matches!(
        fault,
        Some(
            "wrong_request_id"
                | "stale_subject_hash"
                | "malformed_output"
                | "path_traversal"
                | "exit_without_result"
        )
    )
}

fn judge_case(manifest: &Path, case: &CaseSpec, prepared: &PreparedCase) -> CaseReport {
    if let Some(fault) = case.fault.as_deref() {
        if !sdk_fault(Some(fault)) && !host_fault(Some(fault)) {
            return fail(&case.id, &format!("unknown_fault:{fault}"));
        }
    }
    let report = spawn_case(manifest, case, prepared);
    let judged = match case.fault.as_deref() {
        Some(fault) if host_fault(Some(fault)) => inject_and_judge(fault, prepared, &report),
        _ => Ok(report),
    };
    match judged {
        Err(reason) => fail(&case.id, &reason),
        Ok(report) => match compare(&report, &case.expect, &prepared.out_dir) {
            Ok(reason) => pass(&case.id, &reason),
            Err(reason) => fail(&case.id, &reason),
        },
    }
}

fn spawn_case(manifest: &Path, case: &CaseSpec, prepared: &PreparedCase) -> HostReport {
    let cancel = cancel_flag(case);
    let handle = cancel.as_ref().map(|flag| cancel_later(Arc::clone(flag)));
    let report = run_adapter(&RunSpec {
        manifest_path: manifest.to_path_buf(),
        request_path: prepared.request_path.clone(),
        out_dir: prepared.out_dir.clone(),
        timeout_override: case.timeout_seconds.map(Duration::from_secs),
        cancel,
    });
    if let Some(handle) = handle {
        let _ = handle.join();
    }
    report
}

fn cancel_flag(case: &CaseSpec) -> Option<Arc<AtomicBool>> {
    if case.fault.as_deref() == Some("cancel") {
        Some(Arc::new(AtomicBool::new(false)))
    } else {
        None
    }
}

fn cancel_later(flag: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        thread::sleep(CANCEL_DELAY);
        flag.store(true, Ordering::Relaxed);
    })
}

fn inject_and_judge(
    fault: &str,
    prepared: &PreparedCase,
    report: &HostReport,
) -> Result<HostReport, String> {
    if !report.result_accepted {
        return Err(format!(
            "fault_needs_accepted_result: {}",
            report.rejection_reason.as_deref().unwrap_or("rejected")
        ));
    }
    let adapter_out = prepared.out_dir.join("adapter");
    apply_host_fault(fault, &adapter_out)?;
    let before = snapshot_inputs(&prepared.request)?;
    Ok(judge_written_result(
        &prepared.request,
        &adapter_out,
        Some(0),
        &before,
    ))
}

fn apply_host_fault(fault: &str, adapter_out: &Path) -> Result<(), String> {
    match fault {
        "wrong_request_id" => rewrite_field(adapter_out, rewrite_request_id),
        "stale_subject_hash" => rewrite_field(adapter_out, rewrite_spec_hash),
        "malformed_output" => write_bytes(adapter_out, b"{"),
        "path_traversal" => rewrite_field(adapter_out, add_traversal),
        "exit_without_result" => remove_result(adapter_out),
        _ => Err(format!("unknown_fault:{fault}")),
    }
}

fn rewrite_field(adapter_out: &Path, edit: fn(&mut Value)) -> Result<(), String> {
    let path = adapter_out.join("result.json");
    let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let mut value: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    edit(&mut value);
    let bytes = serde_json::to_vec(&value).map_err(|error| error.to_string())?;
    fs::write(path, bytes).map_err(|error| error.to_string())
}

fn rewrite_request_id(value: &mut Value) {
    if let Some(id) = value.get("request_id").and_then(Value::as_str) {
        value["request_id"] = json!(format!("{id}-stale"));
    }
}

fn rewrite_spec_hash(value: &mut Value) {
    if let Some(identity) = value.pointer_mut("/subject/identity") {
        identity["spec_hash"] = json!("stale-spec-hash");
    }
}

fn add_traversal(value: &mut Value) {
    let artifacts = value
        .as_object_mut()
        .and_then(|object| object.get_mut("artifacts"))
        .and_then(Value::as_array_mut);
    if let Some(artifacts) = artifacts {
        artifacts.push(json!("../escaped.txt"));
    }
}

fn write_bytes(adapter_out: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(adapter_out.join("result.json"), bytes).map_err(|error| error.to_string())
}

fn remove_result(adapter_out: &Path) -> Result<(), String> {
    fs::remove_file(adapter_out.join("result.json")).map_err(|error| error.to_string())
}

fn compare(report: &HostReport, expect: &Expect, host_out: &Path) -> Result<String, String> {
    let mut gaps = Vec::new();
    eq_bool(
        &mut gaps,
        "result_accepted",
        expect.result_accepted,
        report.result_accepted,
    );
    eq_opt(
        &mut gaps,
        "rejection_reason",
        expect.rejection_reason.as_deref(),
        report.rejection_reason.as_deref(),
    );
    eq_opt(
        &mut gaps,
        "execution_status",
        expect.execution_status.as_deref(),
        report.execution_status.as_deref(),
    );
    eq_finding(&mut gaps, expect, report);
    eq_logs(&mut gaps, expect, report, host_out);
    if gaps.is_empty() {
        Ok(pass_reason(report))
    } else {
        Err(format!(
            "{}; rejection_reason={}",
            gaps.join("; "),
            report.rejection_reason.as_deref().unwrap_or("")
        ))
    }
}

fn pass_reason(report: &HostReport) -> String {
    if report.result_accepted {
        report
            .findings
            .first()
            .map(|finding| finding.finding.clone())
            .unwrap_or_else(|| {
                report
                    .execution_status
                    .clone()
                    .unwrap_or_else(|| "accepted".into())
            })
    } else {
        report
            .rejection_reason
            .clone()
            .unwrap_or_else(|| "rejected".into())
    }
}

fn eq_bool(gaps: &mut Vec<String>, name: &str, expected: Option<bool>, actual: bool) {
    if let Some(expected) = expected {
        if expected != actual {
            gaps.push(format!("{name}: expected {expected} got {actual}"));
        }
    }
}

fn eq_opt(gaps: &mut Vec<String>, name: &str, expected: Option<&str>, actual: Option<&str>) {
    if let Some(expected) = expected {
        if actual != Some(expected) {
            gaps.push(format!(
                "{name}: expected {expected} got {}",
                actual.unwrap_or("")
            ));
        }
    }
}

fn eq_finding(gaps: &mut Vec<String>, expect: &Expect, report: &HostReport) {
    if !finding_expected(expect) {
        return;
    }
    let Some(finding) = report.findings.first() else {
        gaps.push("finding: missing".into());
        return;
    };
    eq_opt(
        gaps,
        "finding",
        expect.finding.as_deref(),
        Some(finding.finding.as_str()),
    );
    eq_opt(
        gaps,
        "evidence_class",
        expect.evidence_class.as_deref(),
        Some(finding.evidence_class.as_str()),
    );
    eq_bool(
        gaps,
        "coverage_complete",
        expect.coverage_complete,
        finding.coverage_complete,
    );
    eq_opt(
        gaps,
        "host_interpretation",
        expect.host_interpretation.as_deref(),
        Some(finding.host_interpretation.as_str()),
    );
}

fn finding_expected(expect: &Expect) -> bool {
    expect.finding.is_some()
        || expect.evidence_class.is_some()
        || expect.coverage_complete.is_some()
        || expect.host_interpretation.is_some()
}

fn eq_logs(gaps: &mut Vec<String>, expect: &Expect, report: &HostReport, host_out: &Path) {
    if let Some(relative) = expect.native_log.as_deref() {
        let path = host_out.join("adapter").join(relative);
        if !fs::metadata(&path)
            .map(|meta| meta.is_file() && meta.len() > 0)
            .unwrap_or(false)
        {
            gaps.push(format!("native_log missing: {relative}"));
        }
    }
    if expect.stdout_preserved == Some(true) && !stdout_kept(report, host_out) {
        gaps.push("stdout not preserved".into());
    }
}

fn stdout_kept(report: &HostReport, host_out: &Path) -> bool {
    report.partial_logs.iter().any(|log| {
        log.ends_with("stdout.log")
            && fs::metadata(host_out.join(log))
                .map(|meta| meta.len() > 0)
                .unwrap_or(false)
    })
}
