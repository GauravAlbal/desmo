//! Spawn an approved adapter argv and validate its result.
//!
//! The host clears the environment, does not use a shell, and kills the process
//! group on timeout, cancellation, or an output overrun. A killed run is never
//! an accepted report, even if a result file was already written.

use super::manifest::{check_executable, load_manifest, AdapterManifest};
use super::paths::{is_symlink, path_inside, write_atomic};
use super::request::{self, snapshot_inputs, AnalysisRequest, InputSnapshot};
use super::result::{
    host_interpretation, requirement_status, validate_result_bytes, AnalysisResult, PropertyResult,
    RequirementAssessment,
};
use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

pub const HOST_REPORT_PROTOCOL: &str = "ik.analysis.host-report.v1";
pub const CONTROLS_NOTE: &str = "Process boundary only: the approved argv is spawned directly, the environment is cleared, and the process group is killed on timeout or cancellation. This is not a hostile-code sandbox. Network and general filesystem isolation are not enforced on this platform.";
const HOST_MAX_OUTPUT: u64 = 8 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(20);

#[cfg(unix)]
const SIGKILL: i32 = 9;

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

#[derive(Clone, Debug)]
pub struct RunSpec {
    pub manifest_path: PathBuf,
    pub request_path: PathBuf,
    pub out_dir: PathBuf,
    pub timeout_override: Option<Duration>,
    pub cancel: Option<Arc<AtomicBool>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HostReport {
    pub protocol: String,
    pub result_accepted: bool,
    pub rejection_reason: Option<String>,
    pub detail: Option<String>,
    pub executed: bool,
    pub refused_before_execution: bool,
    pub execution_status: Option<String>,
    pub findings: Vec<HostFinding>,
    pub requirement_status: Vec<RequirementAssessment>,
    pub partial_logs: Vec<String>,
    pub adapter_exit_code: Option<i32>,
    pub controls: EffectiveControls,
}

#[derive(Clone, Debug, Serialize)]
pub struct HostFinding {
    pub goal_id: String,
    pub property_class: String,
    pub finding: String,
    pub evidence_class: String,
    pub coverage_complete: bool,
    pub coverage_reason: String,
    pub reproduction_status: String,
    pub host_interpretation: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct EffectiveControls {
    pub shell_invocation: bool,
    pub environment_cleared: bool,
    pub environment_allowlist: Vec<String>,
    pub production_store_path_passed: bool,
    pub credentials_passed: bool,
    pub network_requested: bool,
    pub network_restricted_by_host: bool,
    pub hostile_code_sandbox: bool,
    pub process_tree_cleanup: String,
    pub timeout_enforced_by_host: bool,
    pub executable_checked_before_spawn: bool,
    pub timeout_seconds: u64,
    pub max_output_bytes: u64,
    pub note: String,
}

struct Prepared {
    manifest: AdapterManifest,
    request: AnalysisRequest,
    request_path: PathBuf,
    out_dir: PathBuf,
    adapter_out: PathBuf,
    scratch: PathBuf,
    snapshots: Vec<InputSnapshot>,
    timeout: Duration,
    output_limit: usize,
}

struct WaitOutcome {
    exit_code: Option<i32>,
    timed_out: bool,
    cancelled: bool,
    output_exceeded: bool,
}

pub fn run_adapter(spec: &RunSpec) -> HostReport {
    let prepared = match prepare(spec) {
        Ok(prepared) => prepared,
        Err(report) => return report,
    };
    match spawn_and_wait(&prepared, spec.cancel.clone()) {
        Ok((outcome, stdout, stderr)) => finish(prepared, outcome, &stdout, &stderr),
        Err(report) => report,
    }
}

fn prepare(spec: &RunSpec) -> Result<Prepared, HostReport> {
    let manifest = load_manifest(&spec.manifest_path).map_err(|reason| {
        refuse(
            &spec.out_dir,
            &reason,
            None,
            false,
            &controls(false, 0, 0, false),
        )
    })?;
    check_executable(&manifest).map_err(|reason| {
        refuse(
            &spec.out_dir,
            &reason,
            None,
            true,
            &controls(manifest.requires.network, 0, 0, true),
        )
    })?;
    let request = request::load_request(&spec.request_path).map_err(|reason| {
        refuse(
            &spec.out_dir,
            &reason,
            None,
            true,
            &controls(manifest.requires.network, 0, 0, true),
        )
    })?;
    let timeout = spec
        .timeout_override
        .unwrap_or_else(|| Duration::from_secs(request.limits.timeout_seconds));
    let output_limit = output_limit(request.limits.max_output_bytes);
    let controls = controls(
        manifest.requires.network,
        timeout.as_secs().max(1),
        output_limit as u64,
        true,
    );
    let layout = match layout_dirs(&spec.out_dir, &request) {
        Ok(layout) => layout,
        Err(reason) => {
            return Err(refuse(
                &spec.out_dir,
                &reason,
                Some(&request),
                true,
                &controls,
            ));
        }
    };
    let snapshots = match snapshot_inputs(&request) {
        Ok(snapshots) => snapshots,
        Err(reason) => {
            return Err(refuse(
                &spec.out_dir,
                &reason,
                Some(&request),
                true,
                &controls,
            ));
        }
    };
    let request_path = spec
        .request_path
        .canonicalize()
        .unwrap_or_else(|_| spec.request_path.clone());
    Ok(Prepared {
        manifest,
        request,
        request_path,
        out_dir: layout.0,
        adapter_out: layout.1,
        scratch: layout.2,
        snapshots,
        timeout,
        output_limit,
    })
}

fn layout_dirs(
    out_dir: &Path,
    request: &AnalysisRequest,
) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    fs::create_dir_all(out_dir).map_err(|error| error.to_string())?;
    let out_dir = out_dir.canonicalize().map_err(|error| error.to_string())?;
    let scratch = out_dir.join("scratch");
    let adapter_out = out_dir.join("adapter");
    fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;
    fs::create_dir_all(&adapter_out).map_err(|error| error.to_string())?;
    if !path_inside(&scratch, &request.permissions.scratch_root) || !path_inside(&scratch, &out_dir)
    {
        return Err("scratch_not_permitted".into());
    }
    Ok((out_dir, adapter_out, scratch))
}

fn output_limit(requested: u64) -> usize {
    requested.min(HOST_MAX_OUTPUT) as usize
}

fn spawn_and_wait(
    prepared: &Prepared,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<(WaitOutcome, Vec<u8>, Vec<u8>), HostReport> {
    let mut child = spawn_adapter(prepared)
        .map_err(|error| refused_after_approval(prepared, "failed", &error.to_string()))?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let exceeded = Arc::new(AtomicBool::new(false));
    let stdout_buf = Arc::new(Mutex::new(Vec::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::new()));
    let stdout_thread = read_bounded(
        stdout,
        prepared.output_limit,
        Arc::clone(&stdout_buf),
        Arc::clone(&exceeded),
        pid,
    );
    let stderr_thread = read_bounded(
        stderr,
        prepared.output_limit,
        Arc::clone(&stderr_buf),
        Arc::clone(&exceeded),
        pid,
    );
    let outcome = wait_for_exit(&mut child, prepared.timeout, cancel.as_deref(), &exceeded);
    join_reader(stdout_thread);
    join_reader(stderr_thread);
    let stdout = stdout_buf
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let stderr = stderr_buf
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    Ok((outcome, stdout, stderr))
}

fn spawn_adapter(prepared: &Prepared) -> std::io::Result<Child> {
    let argv = &prepared.manifest.invocation.argv;
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .arg("run")
        .arg("--request")
        .arg(&prepared.request_path)
        .arg("--out")
        .arg(&prepared.adapter_out)
        .current_dir(&prepared.scratch)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        let _ = command.process_group(0);
    }
    command.spawn()
}

fn read_bounded(
    pipe: Option<impl Read + Send + 'static>,
    limit: usize,
    sink: Arc<Mutex<Vec<u8>>>,
    exceeded: Arc<AtomicBool>,
    pid: u32,
) -> Option<thread::JoinHandle<()>> {
    let mut pipe = pipe?;
    Some(thread::spawn(move || {
        capture_pipe(&mut pipe, limit, &sink, &exceeded, pid)
    }))
}

fn capture_pipe(
    pipe: &mut impl Read,
    limit: usize,
    sink: &Mutex<Vec<u8>>,
    exceeded: &AtomicBool,
    pid: u32,
) {
    let mut buf = [0u8; 8192];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if !store_chunk(sink, &buf[..n], limit) {
                    // Kill the group before this thread drops the pipe. Closing it
                    // can SIGPIPE the leader; reaping that leader must not skip the group.
                    signal_group(pid);
                    exceeded.store(true, Ordering::Relaxed);
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

fn store_chunk(sink: &Mutex<Vec<u8>>, chunk: &[u8], limit: usize) -> bool {
    let Ok(mut guard) = sink.lock() else {
        return false;
    };
    let room = limit.saturating_sub(guard.len());
    if chunk.len() > room {
        guard.extend_from_slice(&chunk[..room]);
        return false;
    }
    guard.extend_from_slice(chunk);
    true
}

fn join_reader(handle: Option<thread::JoinHandle<()>>) {
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}

fn wait_for_exit(
    child: &mut Child,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
    exceeded: &AtomicBool,
) -> WaitOutcome {
    let start = Instant::now();
    loop {
        if exceeded.load(Ordering::Relaxed) {
            kill_group_if_running(child);
            return killed(false, false, true);
        }
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            kill_group_if_running(child);
            return killed(false, true, false);
        }
        if start.elapsed() >= timeout {
            kill_group_if_running(child);
            return killed(true, false, false);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return WaitOutcome {
                    exit_code: status.code(),
                    timed_out: false,
                    cancelled: false,
                    output_exceeded: false,
                };
            }
            Ok(None) => thread::sleep(POLL),
            Err(_) => {
                kill_group_if_running(child);
                return killed(false, false, false);
            }
        }
    }
}

fn killed(timed_out: bool, cancelled: bool, output_exceeded: bool) -> WaitOutcome {
    WaitOutcome {
        exit_code: None,
        timed_out,
        cancelled,
        output_exceeded,
    }
}

fn kill_group_if_running(child: &mut Child) {
    if !matches!(child.try_wait(), Ok(None)) {
        return;
    }
    signal_group(child.id());
    let _ = child.wait();
}

fn signal_group(pid: u32) {
    #[cfg(unix)]
    {
        let Ok(pgid) = i32::try_from(pid) else {
            return;
        };
        unsafe {
            kill(-pgid, SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

fn finish(prepared: Prepared, outcome: WaitOutcome, stdout: &[u8], stderr: &[u8]) -> HostReport {
    let logs = write_logs(&prepared.out_dir, stdout, stderr);
    let controls = controls(
        prepared.manifest.requires.network,
        prepared.timeout.as_secs().max(1),
        prepared.output_limit as u64,
        true,
    );
    if let Some(reason) = kill_reason(&outcome) {
        return executed_rejection(
            &prepared,
            &reason,
            kill_status(&reason),
            outcome.exit_code,
            &logs,
            &controls,
        );
    }
    let after = snapshot_inputs(&prepared.request).unwrap_or_default();
    match read_adapter_result(&prepared.adapter_out) {
        Err(reason) => completed_rejection(
            &prepared,
            &reason,
            outcome.exit_code,
            &logs,
            &controls,
            &after,
        ),
        Ok(bytes) => accept_or_reject(
            &prepared,
            &bytes,
            outcome.exit_code,
            &logs,
            &controls,
            &after,
        ),
    }
}

fn kill_reason(outcome: &WaitOutcome) -> Option<&'static str> {
    if outcome.timed_out {
        Some("timed_out")
    } else if outcome.cancelled {
        Some("cancelled")
    } else if outcome.output_exceeded {
        Some("output_limit_exceeded")
    } else {
        None
    }
}

fn kill_status(reason: &str) -> &'static str {
    if reason == "timed_out" {
        "timed_out"
    } else if reason == "cancelled" {
        "cancelled"
    } else {
        "failed"
    }
}

fn read_adapter_result(adapter_out: &Path) -> Result<String, String> {
    let path = adapter_out.join("result.json");
    if is_symlink(&path) {
        return Err("symlink_rejected".into());
    }
    if !path.is_file() {
        return Err("missing_result".into());
    }
    fs::read_to_string(&path).map_err(|_| "malformed_json".to_string())
}

fn accept_or_reject(
    prepared: &Prepared,
    bytes: &str,
    exit_code: Option<i32>,
    logs: &[String],
    controls: &EffectiveControls,
    after: &[InputSnapshot],
) -> HostReport {
    match validate_result_bytes(
        &prepared.request,
        bytes,
        &prepared.adapter_out,
        &prepared.snapshots,
        after,
    ) {
        Ok(result) => accepted(prepared, result, exit_code, logs, controls),
        Err(reason) => executed_rejection(prepared, &reason, "failed", exit_code, logs, controls),
    }
}

fn completed_rejection(
    prepared: &Prepared,
    reason: &str,
    exit_code: Option<i32>,
    logs: &[String],
    controls: &EffectiveControls,
    after: &[InputSnapshot],
) -> HostReport {
    let reason = if prepared.snapshots != after {
        "input_modified"
    } else if reason == "missing_result" && exit_code == Some(0) {
        "exit_without_result"
    } else if reason == "missing_result" {
        "failed"
    } else {
        reason
    };
    executed_rejection(prepared, reason, "failed", exit_code, logs, controls)
}

fn accepted(
    prepared: &Prepared,
    result: AnalysisResult,
    exit_code: Option<i32>,
    logs: &[String],
    controls: &EffectiveControls,
) -> HostReport {
    let findings = result.properties.iter().map(finding_view).collect();
    let requirement_status = requirement_status(
        &prepared.request.goals,
        &result.execution_status,
        &result.properties,
    );
    let report = HostReport {
        protocol: HOST_REPORT_PROTOCOL.into(),
        result_accepted: true,
        rejection_reason: None,
        detail: None,
        executed: true,
        refused_before_execution: false,
        execution_status: Some(result.execution_status),
        findings,
        requirement_status,
        partial_logs: logs.to_vec(),
        adapter_exit_code: exit_code,
        controls: controls.clone(),
    };
    persist(&prepared.out_dir, &report);
    report
}

fn finding_view(prop: &PropertyResult) -> HostFinding {
    HostFinding {
        goal_id: prop.goal_id.clone(),
        property_class: prop.property_class.clone(),
        finding: prop.finding.clone(),
        evidence_class: prop.evidence_class.clone(),
        coverage_complete: prop.coverage.complete,
        coverage_reason: prop.coverage.reason.clone(),
        reproduction_status: prop.reproduction.status.clone(),
        host_interpretation: host_interpretation(&prop.finding, prop.coverage.complete),
    }
}

fn executed_rejection(
    prepared: &Prepared,
    reason: &str,
    execution_status: &str,
    exit_code: Option<i32>,
    logs: &[String],
    controls: &EffectiveControls,
) -> HostReport {
    let report = HostReport {
        protocol: HOST_REPORT_PROTOCOL.into(),
        result_accepted: false,
        rejection_reason: Some(reason.to_string()),
        detail: None,
        executed: true,
        refused_before_execution: false,
        execution_status: Some(execution_status.to_string()),
        findings: Vec::new(),
        requirement_status: requirement_status(&prepared.request.goals, execution_status, &[]),
        partial_logs: logs.to_vec(),
        adapter_exit_code: exit_code,
        controls: controls.clone(),
    };
    persist(&prepared.out_dir, &report);
    report
}

fn write_logs(out_dir: &Path, stdout: &[u8], stderr: &[u8]) -> Vec<String> {
    let mut logs = Vec::new();
    if let Some(name) = write_log(out_dir, "stdout.log", stdout) {
        logs.push(name);
    }
    if let Some(name) = write_log(out_dir, "stderr.log", stderr) {
        logs.push(name);
    }
    logs
}

fn write_log(out_dir: &Path, name: &str, bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let dir = out_dir.join("native");
    write_atomic(&dir, name, bytes).ok()?;
    Some(format!("native/{name}"))
}

fn refused_after_approval(prepared: &Prepared, reason: &str, detail: &str) -> HostReport {
    let controls = controls(
        prepared.manifest.requires.network,
        prepared.timeout.as_secs().max(1),
        prepared.output_limit as u64,
        true,
    );
    let mut report = refuse(
        &prepared.out_dir,
        reason,
        Some(&prepared.request),
        true,
        &controls,
    );
    report.detail = Some(detail.to_string());
    report.refused_before_execution = false;
    report
}

fn refuse(
    out_dir: &Path,
    reason: &str,
    request: Option<&AnalysisRequest>,
    executable_checked: bool,
    controls: &EffectiveControls,
) -> HostReport {
    let goals = request
        .map(|request| request.goals.as_slice())
        .unwrap_or(&[]);
    let mut controls = controls.clone();
    controls.executable_checked_before_spawn = executable_checked;
    let report = HostReport {
        protocol: HOST_REPORT_PROTOCOL.into(),
        result_accepted: false,
        rejection_reason: Some(reason.to_string()),
        detail: None,
        executed: false,
        refused_before_execution: true,
        execution_status: None,
        findings: Vec::new(),
        requirement_status: requirement_status(goals, "failed", &[]),
        partial_logs: Vec::new(),
        adapter_exit_code: None,
        controls,
    };
    if out_dir.as_os_str().is_empty() {
        return report;
    }
    let _ = fs::create_dir_all(out_dir);
    persist(out_dir, &report);
    report
}

fn persist(out_dir: &Path, report: &HostReport) {
    let bytes = crate::canonical::canonical_json(report).into_bytes();
    let _ = write_atomic(out_dir, "host-report.json", &bytes);
}

fn controls(
    network_requested: bool,
    timeout_seconds: u64,
    max_output_bytes: u64,
    executable_checked: bool,
) -> EffectiveControls {
    EffectiveControls {
        shell_invocation: false,
        environment_cleared: true,
        environment_allowlist: Vec::new(),
        production_store_path_passed: false,
        credentials_passed: false,
        network_requested,
        network_restricted_by_host: false,
        hostile_code_sandbox: false,
        process_tree_cleanup: "process_group_kill".into(),
        timeout_enforced_by_host: true,
        executable_checked_before_spawn: executable_checked,
        timeout_seconds,
        max_output_bytes,
        note: CONTROLS_NOTE.into(),
    }
}

pub fn controls_for(
    network_requested: bool,
    timeout_seconds: u64,
    max_output_bytes: u64,
    executable_checked: bool,
) -> EffectiveControls {
    controls(
        network_requested,
        timeout_seconds,
        max_output_bytes,
        executable_checked,
    )
}

pub fn rejected_report(
    reason: &str,
    request: Option<&AnalysisRequest>,
    executed: bool,
    controls: &EffectiveControls,
) -> HostReport {
    let goals = request
        .map(|request| request.goals.as_slice())
        .unwrap_or(&[]);
    HostReport {
        protocol: HOST_REPORT_PROTOCOL.into(),
        result_accepted: false,
        rejection_reason: Some(reason.to_string()),
        detail: None,
        executed,
        refused_before_execution: false,
        execution_status: Some("failed".into()),
        findings: Vec::new(),
        requirement_status: requirement_status(goals, "failed", &[]),
        partial_logs: Vec::new(),
        adapter_exit_code: None,
        controls: controls.clone(),
    }
}

pub fn accepted_report(
    request: &AnalysisRequest,
    result: AnalysisResult,
    exit_code: Option<i32>,
    controls: &EffectiveControls,
) -> HostReport {
    HostReport {
        protocol: HOST_REPORT_PROTOCOL.into(),
        result_accepted: true,
        rejection_reason: None,
        detail: None,
        executed: false,
        refused_before_execution: false,
        execution_status: Some(result.execution_status.clone()),
        findings: result.properties.iter().map(finding_view).collect(),
        requirement_status: requirement_status(
            &request.goals,
            &result.execution_status,
            &result.properties,
        ),
        partial_logs: Vec::new(),
        adapter_exit_code: exit_code,
        controls: controls.clone(),
    }
}

fn missing_result_reason(
    reason: &str,
    exit_code: Option<i32>,
    before: &[InputSnapshot],
    after: &[InputSnapshot],
) -> String {
    if before != after {
        return "input_modified".into();
    }
    if reason == "missing_result" && exit_code == Some(0) {
        return "exit_without_result".into();
    }
    if reason == "missing_result" {
        return "failed".into();
    }
    reason.to_string()
}

/// Re-read `adapter_out/result.json` with the same checker `run` uses after exit.
/// Used when a conformance fixture injects a fault into an already written result.
pub fn judge_written_result(
    request: &AnalysisRequest,
    adapter_out: &Path,
    exit_code: Option<i32>,
    before: &[InputSnapshot],
) -> HostReport {
    let controls = controls_for(
        false,
        request.limits.timeout_seconds,
        request.limits.max_output_bytes,
        true,
    );
    let after = snapshot_inputs(request).unwrap_or_default();
    match read_adapter_result(adapter_out) {
        Err(reason) => rejected_report(
            &missing_result_reason(&reason, exit_code, before, &after),
            Some(request),
            true,
            &controls,
        ),
        Ok(bytes) => match validate_result_bytes(request, &bytes, adapter_out, before, &after) {
            Ok(result) => accepted_report(request, result, exit_code, &controls),
            Err(reason) => rejected_report(&reason, Some(request), true, &controls),
        },
    }
}
