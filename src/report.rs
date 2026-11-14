//! One normalized report over scenario, analysis, and check-profile evidence.
//!
//! Wording keeps sampled, bounded, and exhaustive-finite-model evidence apart.
//! A refused command is not a committed state. A model finding stays model-level
//! until a concrete replay exists. "Smallest found" is not called minimal.

use crate::canonical::canonical_json;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const USAGE: &str = "\
usage: ik report <path>... [--json] [--html <file>]

Each path is a run directory or a result JSON of a known kind:
ik.scenario.result.v1, ik.analysis.result.v1, ik.analysis.host-report.v1,
or an ik check --json document with a profile object.
Unknown or malformed input is an unreadable finding, not a silent skip.
";

const SCHEMA: &str = "ik.report.v1";
const SCENARIO: &str = "ik.scenario.result.v1";
const ANALYSIS: &str = "ik.analysis.result.v1";
const HOST: &str = "ik.analysis.host-report.v1";
const PROFILE: &str = "ik.check.profile.v1";

const BUNDLE_FILES: &[&str] = &[
    "scenario.json",
    "machine.ik.toml",
    "prior_machine.ik.toml",
    "result.json",
    "trace.json",
    "SHA256SUMS",
    "host-report.json",
    "native/stdout.log",
    "native/stderr.log",
];

#[derive(Clone, Debug, Serialize)]
struct Report {
    schema: &'static str,
    findings: Vec<Finding>,
}

#[derive(Clone, Debug, Serialize)]
struct Finding {
    source_kind: String,
    id: String,
    status: String,
    scope: String,
    evidence_class: String,
    completeness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_coordinate: Option<SourceCoordinate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after: Option<Value>,
    attempted: String,
    committed: String,
    reproduction_status: String,
    artifacts: Vec<String>,
    next_action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    present: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
struct SourceCoordinate {
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
}

struct Loaded {
    value: Value,
    root: PathBuf,
    label: String,
    from_dir: bool,
}

struct PropertyView {
    goal_id: String,
    property_class: String,
    finding: String,
    evidence_class: String,
    coverage_complete: bool,
    coverage_reason: String,
    reproduction_status: String,
    artifact: Option<String>,
    interpretation: Option<String>,
}

enum Invocation {
    Help,
    Run {
        json: bool,
        html: Option<PathBuf>,
        paths: Vec<PathBuf>,
    },
}

/// The `ik.report.v1` document for `paths`, as JSON.
pub fn report_value(paths: &[PathBuf]) -> Value {
    serde_json::to_value(build_report(paths)).unwrap_or(Value::Null)
}

/// CLI entry for `ik report`.
pub fn run(args: &[String]) -> i32 {
    match parse_args(args) {
        Ok(Invocation::Help) => {
            println!("{USAGE}");
            0
        }
        Ok(Invocation::Run { json, html, paths }) => emit(&paths, json, html.as_deref()),
        Err(message) => usage_error(&message),
    }
}

fn usage_error(message: &str) -> i32 {
    eprintln!("ik report: {message}");
    eprintln!("{USAGE}");
    2
}

fn parse_args(args: &[String]) -> Result<Invocation, String> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        return Ok(Invocation::Help);
    }
    let mut json = false;
    let mut html = None;
    let mut paths = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--html" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--html requires a file".to_string())?;
                html = Some(PathBuf::from(path));
            }
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            positional => paths.push(PathBuf::from(positional)),
        }
        index += 1;
    }
    if paths.is_empty() {
        return Err("at least one path is required".into());
    }
    Ok(Invocation::Run { json, html, paths })
}

fn emit(paths: &[PathBuf], json: bool, html: Option<&Path>) -> i32 {
    let report = build_report(paths);
    if let Some(path) = html {
        if let Err(err) = fs::write(path, render_html(&report)) {
            eprintln!("ik report: write {}: {err}", path.display());
            return 2;
        }
    }
    if json {
        println!("{}", canonical_json(&report));
    } else {
        print!("{}", render_text(&report));
    }
    exit_code(&report)
}

fn exit_code(report: &Report) -> i32 {
    if report
        .findings
        .iter()
        .any(|finding| finding.status == "unreadable")
    {
        1
    } else {
        0
    }
}

fn build_report(paths: &[PathBuf]) -> Report {
    let mut findings = Vec::new();
    for path in paths {
        findings.extend(ingest(path));
    }
    Report {
        schema: SCHEMA,
        findings,
    }
}

fn ingest(path: &Path) -> Vec<Finding> {
    match load_and_normalize(path) {
        Ok(findings) => findings,
        Err(reason) => vec![unreadable(path, &reason)],
    }
}

fn load_and_normalize(path: &Path) -> Result<Vec<Finding>, String> {
    let loaded = load_document(path)?;
    let mut artifacts = document_paths(&loaded.value);
    if loaded.from_dir {
        artifacts.extend(existing_named(&loaded.root, BUNDLE_FILES));
    }
    artifacts.sort();
    artifacts.dedup();
    findings_of(&loaded.value, &loaded.label, &artifacts)
}

fn load_document(path: &Path) -> Result<Loaded, String> {
    if path.is_dir() {
        return load_dir(path);
    }
    if !path.is_file() {
        return Err(format!("not found: {}", path.display()));
    }
    Ok(Loaded {
        value: read_value(path)?,
        root: path.parent().unwrap_or(path).to_path_buf(),
        label: path.display().to_string(),
        from_dir: false,
    })
}

fn load_dir(path: &Path) -> Result<Loaded, String> {
    let host = path.join("host-report.json");
    let result = path.join("result.json");
    let chosen = if host.is_file() {
        host
    } else if result.is_file() {
        result
    } else {
        return Err(format!("no known result in {}", path.display()));
    };
    Ok(Loaded {
        value: read_value(&chosen)?,
        root: path.to_path_buf(),
        label: path.display().to_string(),
        from_dir: true,
    })
}

fn read_value(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|err| format!("malformed JSON in {}: {err}", path.display()))
}

fn findings_of(value: &Value, label: &str, artifacts: &[String]) -> Result<Vec<Finding>, String> {
    match classify(value) {
        Some(SCENARIO) => scenario_findings(value, label, artifacts),
        Some(ANALYSIS) => analysis_findings(value, label, artifacts, ANALYSIS),
        Some(HOST) => host_findings(value, label, artifacts),
        Some(PROFILE) => profile_findings(value, label, artifacts),
        _ => Err("not a known result or report kind".into()),
    }
}

fn classify(value: &Value) -> Option<&'static str> {
    let schema = field_str(value, "schema");
    let protocol = field_str(value, "protocol");
    if schema == Some(SCENARIO) {
        return Some(SCENARIO);
    }
    if protocol == Some(ANALYSIS) {
        return Some(ANALYSIS);
    }
    if protocol == Some(HOST) {
        return Some(HOST);
    }
    if schema == Some(PROFILE)
        || value.get("profile").is_some()
        || value.get("definition").is_some()
    {
        return Some(PROFILE);
    }
    None
}

fn scenario_findings(
    value: &Value,
    label: &str,
    artifacts: &[String],
) -> Result<Vec<Finding>, String> {
    let steps = value
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| "scenario result has no steps".to_string())?;
    let mut findings = Vec::new();
    let mut previous = None;
    for step in steps {
        findings.push(scenario_finding(value, label, step, previous, artifacts));
        previous = step.get("state_after");
    }
    Ok(findings)
}

fn scenario_finding(
    result: &Value,
    label: &str,
    step: &Value,
    previous: Option<&Value>,
    artifacts: &[String],
) -> Finding {
    let outcome = step
        .get("response")
        .and_then(|response| field_str(response, "outcome"))
        .unwrap_or("unknown");
    let code = step
        .get("response")
        .and_then(|response| field_str(response, "diagnostic_code"));
    let verdict = field_str(step, "verdict").unwrap_or("");
    let status = scenario_status(outcome, verdict);
    let mut finding = bare(SCENARIO, &scenario_id(label, result, step), &status);
    finding.scope = scenario_scope(outcome, code, verdict);
    finding.evidence_class = "observed_trace".into();
    finding.completeness = "observed".into();
    finding.source_coordinate = scenario_coordinate(step);
    finding.before = previous.filter(|value| !value.is_null()).cloned();
    finding.after = step
        .get("state_after")
        .filter(|value| !value.is_null())
        .cloned();
    finding.attempted = attempted_of(step);
    finding.committed = committed_label(outcome).into();
    finding.reproduction_status = "observed".into();
    finding.artifacts = artifacts.to_vec();
    finding.next_action = next_action(&status, &finding.completeness, None, None);
    if let Some(present) = step.get("present") {
        finding.evidence_class = "continuity_observation".into();
        finding.present = Some(present.clone());
        finding.scope = continuity_scope(present, outcome, code);
        finding.next_action = continuity_next(present);
    }
    finding
}

fn continuity_scope(present: &Value, outcome: &str, code: Option<&str>) -> String {
    let created = present
        .get("created_version")
        .and_then(Value::as_u64)
        .map(|version| format!("instances created under version {version} keep version {version}"))
        .unwrap_or_else(|| "instances keep the version they were created under".into());
    let command = present.get("command").and_then(Value::as_str).unwrap_or("");
    if command.is_empty() {
        return created;
    }
    let presented = present
        .get("presented_version")
        .and_then(Value::as_u64)
        .map(|version| format!("presented version {version}"))
        .unwrap_or_else(|| "presented a later definition".into());
    if outcome == "refused" {
        return format!(
            "{created}; {command} is refused on them with {} ({presented})",
            code.unwrap_or("refusal")
        );
    }
    if outcome == "committed" {
        return format!("{created}; {command} still works on them ({presented})");
    }
    format!("{created}; {command} outcome {outcome} ({presented})")
}

fn continuity_next(present: &Value) -> String {
    match present.get("presented_version").and_then(Value::as_u64) {
        Some(version) if present.get("presented").and_then(Value::as_str) == Some("new") => {
            format!(
                "existing instances stay on their version; moving one to version {version} in place is not supported"
            )
        }
        _ => "existing instances stay on their version; moving one to the new version in place is not supported"
            .into(),
    }
}

fn scenario_status(outcome: &str, verdict: &str) -> String {
    if verdict == "fail" {
        return "expectation_failed".into();
    }
    outcome.to_string()
}

fn scenario_scope(outcome: &str, code: Option<&str>, verdict: &str) -> String {
    let mut scope = if let Some(scope) = history_scope(outcome) {
        scope.to_string()
    } else if outcome == "refused" {
        format!(
            "observed runtime trace; refused {}; the proposal was not committed",
            code.unwrap_or("refusal")
        )
    } else if outcome == "committed" {
        "observed runtime trace; the command committed".into()
    } else {
        format!("observed runtime trace; outcome {outcome}")
    };
    if verdict == "fail" {
        scope.push_str("; expectation not met");
    }
    scope
}

fn scenario_id(label: &str, result: &Value, step: &Value) -> String {
    let aggregate = field_str(result, "aggregate").unwrap_or("scenario");
    let step_label = field_str(step, "label").unwrap_or("step");
    format!("{label}#{aggregate}/{step_label}")
}

fn scenario_coordinate(step: &Value) -> Option<SourceCoordinate> {
    let command = step
        .get("request")
        .and_then(|request| field_str(request, "name"))
        .map(str::to_string);
    let state = step
        .get("state_after")
        .and_then(|after| field_str(after, "state"))
        .map(str::to_string);
    coordinate(command, state)
}

fn attempted_of(step: &Value) -> String {
    let Some(request) = step.get("request") else {
        return String::new();
    };
    if let Some(name) = field_str(request, "name") {
        let args = request.get("args").cloned().unwrap_or(Value::Null);
        return format!("{name} {}", canonical_json(&args));
    }
    canonical_json(request)
}

fn committed_label(outcome: &str) -> &'static str {
    if let Some(label) = history_committed(outcome) {
        return label;
    }
    match outcome {
        "committed" => "committed",
        "refused" => "refused",
        "noop" => "noop",
        _ => "not committed",
    }
}

fn history_committed(outcome: &str) -> Option<&'static str> {
    match outcome {
        "committed_exact" => Some(
            "earlier operation: committed (confirmed by lookup); this lookup did not commit again",
        ),
        "historical" => Some(
            "earlier operation: already committed (historical); this attempt did not commit again",
        ),
        "reply_dropped" => Some("the reply was discarded; this step did not commit"),
        "not_recorded" | "not_found" => {
            Some("no recorded operation matched this lookup; this lookup did not commit")
        }
        "conflict" => {
            Some("a record exists but does not match this lookup; this lookup did not commit")
        }
        "not_committed" => Some("the earlier attempt was not committed; this step did not commit"),
        _ => None,
    }
}

fn history_scope(outcome: &str) -> Option<&'static str> {
    match outcome {
        "committed_exact" => Some(
            "established: the earlier operation committed; this lookup did not commit again. uncertain: whether any handle from this lookup is current",
        ),
        "historical" => Some(
            "established: the earlier operation already committed; this attempt did not commit again. uncertain: whether any returned handle is current",
        ),
        "reply_dropped" => Some(
            "established: the reply to the earlier attempt was not delivered. uncertain: this dropped-reply record does not by itself confirm the commit",
        ),
        "not_recorded" | "not_found" => Some(
            "established: no matching recorded operation was found. uncertain: whether the attempt never reached the store or used a different operation id",
        ),
        "conflict" => Some(
            "established: a recorded operation exists but does not match this lookup. uncertain: whether this is the intended operation",
        ),
        "not_committed" => Some(
            "established: the earlier attempt was not committed. uncertain: nothing further about store history from this dropped reply",
        ),
        _ => None,
    }
}

fn analysis_findings(
    value: &Value,
    label: &str,
    artifacts: &[String],
    source_kind: &str,
) -> Result<Vec<Finding>, String> {
    let execution = field_str(value, "execution_status").unwrap_or("completed");
    let properties = value
        .get("properties")
        .and_then(Value::as_array)
        .ok_or_else(|| "analysis result has no properties".to_string())?;
    if properties.is_empty() {
        return Ok(vec![execution_finding(
            label,
            source_kind,
            execution,
            field_str(value, "rejection_reason"),
            artifacts,
        )]);
    }
    let mut findings = Vec::new();
    for prop in properties {
        let view = view_from_property(prop)?;
        findings.push(finding_from_view(
            label,
            source_kind,
            execution,
            &view,
            artifacts,
        ));
    }
    Ok(findings)
}

fn host_findings(value: &Value, label: &str, artifacts: &[String]) -> Result<Vec<Finding>, String> {
    let execution = field_str(value, "execution_status").unwrap_or("");
    let findings = value
        .get("findings")
        .and_then(Value::as_array)
        .ok_or_else(|| "host report has no findings".to_string())?;
    let mut extra = artifacts.to_vec();
    extra.extend(string_list(value, "partial_logs"));
    extra.sort();
    extra.dedup();
    if findings.is_empty() {
        let status = if execution.is_empty() {
            field_str(value, "rejection_reason").unwrap_or("failed")
        } else {
            execution
        };
        return Ok(vec![execution_finding(
            label,
            HOST,
            status,
            field_str(value, "detail").or_else(|| field_str(value, "rejection_reason")),
            &extra,
        )]);
    }
    let mut out = Vec::new();
    for prop in findings {
        let view = view_from_host(prop)?;
        out.push(finding_from_view(label, HOST, execution, &view, &extra));
    }
    Ok(out)
}

fn view_from_property(prop: &Value) -> Result<PropertyView, String> {
    if !prop.is_object() {
        return Err("analysis property is not an object".into());
    }
    let goal_id = required_str(prop, "goal_id", "analysis property")?;
    let (complete, reason) = coverage_parts(prop.get("coverage"));
    let reproduction = prop.get("reproduction");
    Ok(PropertyView {
        goal_id,
        property_class: field_str(prop, "property_class").unwrap_or("").to_string(),
        finding: field_str(prop, "finding")
            .unwrap_or("inconclusive")
            .to_string(),
        evidence_class: field_str(prop, "evidence_class").unwrap_or("").to_string(),
        coverage_complete: complete,
        coverage_reason: reason,
        reproduction_status: reproduction
            .and_then(|item| field_str(item, "status"))
            .unwrap_or("not_attempted")
            .to_string(),
        artifact: reproduction
            .and_then(|item| field_str(item, "artifact"))
            .map(str::to_string),
        interpretation: None,
    })
}

fn view_from_host(prop: &Value) -> Result<PropertyView, String> {
    if !prop.is_object() {
        return Err("host finding is not an object".into());
    }
    Ok(PropertyView {
        goal_id: required_str(prop, "goal_id", "host finding")?,
        property_class: field_str(prop, "property_class").unwrap_or("").to_string(),
        finding: field_str(prop, "finding")
            .unwrap_or("inconclusive")
            .to_string(),
        evidence_class: field_str(prop, "evidence_class").unwrap_or("").to_string(),
        coverage_complete: prop
            .get("coverage_complete")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        coverage_reason: field_str(prop, "coverage_reason").unwrap_or("").to_string(),
        reproduction_status: field_str(prop, "reproduction_status")
            .unwrap_or("not_attempted")
            .to_string(),
        artifact: None,
        interpretation: field_str(prop, "host_interpretation").map(str::to_string),
    })
}

fn finding_from_view(
    label: &str,
    source_kind: &str,
    execution: &str,
    view: &PropertyView,
    artifacts: &[String],
) -> Finding {
    let evidence = if view.evidence_class.is_empty() {
        "none"
    } else {
        view.evidence_class.as_str()
    };
    let status = display_status(execution, &view.finding, evidence, view.coverage_complete);
    let completeness = coverage_word(execution, view.coverage_complete);
    let mut finding = bare(source_kind, &format!("{label}#{}", view.goal_id), &status);
    finding.scope = property_scope(
        evidence,
        &status,
        &view.finding,
        &view.reproduction_status,
        &view.coverage_reason,
        view.interpretation.as_deref(),
    );
    finding.evidence_class = evidence.to_string();
    finding.completeness = completeness;
    finding.attempted = format!("{} {}", view.property_class, view.finding);
    finding.committed = analysis_committed(&view.reproduction_status).into();
    finding.reproduction_status = view.reproduction_status.clone();
    finding.artifacts = with_artifact(artifacts, view.artifact.as_deref());
    finding.next_action = next_action(
        &status,
        &finding.completeness,
        None,
        execution_detail(execution, &view.coverage_reason),
    );
    finding
}

fn execution_finding(
    label: &str,
    source_kind: &str,
    execution: &str,
    detail: Option<&str>,
    artifacts: &[String],
) -> Finding {
    let status = if execution.is_empty() {
        "failed".to_string()
    } else {
        execution.to_string()
    };
    let mut finding = bare(source_kind, &format!("{label}#execution"), &status);
    finding.scope = format!("{}; no property finding", evidence_phrase("none"));
    finding.evidence_class = "none".into();
    finding.completeness = "incomplete".into();
    finding.attempted = status.clone();
    finding.committed = "model-level".into();
    finding.reproduction_status = "not_attempted".into();
    finding.artifacts = artifacts.to_vec();
    finding.next_action = next_action(&status, &finding.completeness, None, detail);
    finding
}

fn display_status(execution: &str, finding: &str, evidence: &str, complete: bool) -> String {
    if blocked_execution(execution) {
        return execution.to_string();
    }
    if finding == "holds_in_scope" && !holds_supported(evidence, complete) {
        return "inconclusive".into();
    }
    finding.to_string()
}

fn blocked_execution(execution: &str) -> bool {
    matches!(
        execution,
        "unsupported" | "tool_unavailable" | "timed_out" | "cancelled" | "failed"
    )
}

fn holds_supported(evidence: &str, complete: bool) -> bool {
    evidence == "exhaustive_finite_model" && complete
}

fn coverage_word(execution: &str, complete: bool) -> String {
    if blocked_execution(execution) || !complete {
        "incomplete".into()
    } else {
        "complete".into()
    }
}

fn analysis_committed(reproduction: &str) -> &'static str {
    if reproduction == "concrete_reproduced" {
        "concrete replay"
    } else {
        "model-level"
    }
}

fn property_scope(
    evidence: &str,
    status: &str,
    source_finding: &str,
    reproduction: &str,
    reason: &str,
    interpretation: Option<&str>,
) -> String {
    let mut parts = vec![evidence_phrase(evidence)];
    if status == "witness_found" {
        parts.push("smallest found within the enumerated search".into());
    }
    if source_finding == "holds_in_scope" && status != "holds_in_scope" {
        parts.push("source finding holds_in_scope is not supported".into());
    }
    push_reproduction(&mut parts, reproduction);
    if let Some(interpretation) = interpretation {
        if interpretation != source_finding && !interpretation.is_empty() {
            parts.push(interpretation.to_string());
        }
    }
    if !reason.is_empty() {
        parts.push(reason.to_string());
    }
    parts.join("; ")
}

fn push_reproduction(parts: &mut Vec<String>, reproduction: &str) {
    if reproduction == "prefix_cycle_validated" {
        parts.push("prefix and cycle validated, not an observed infinite execution".into());
    }
    if reproduction != "concrete_reproduced" {
        parts.push("model-level until a concrete replay exists".into());
    }
}

fn profile_findings(
    value: &Value,
    label: &str,
    artifacts: &[String],
) -> Result<Vec<Finding>, String> {
    let mut findings = Vec::new();
    if let Some(finding) = definition_finding(value, label, artifacts) {
        findings.push(finding);
    }
    match profile_root(value) {
        Ok(profile) => {
            let goals = profile
                .get("goals")
                .and_then(Value::as_array)
                .ok_or_else(|| "profile object has no goals".to_string())?;
            for goal in goals {
                findings.push(profile_finding(profile, label, goal, artifacts));
            }
        }
        Err(err) if findings.is_empty() => return Err(err),
        Err(_) => {}
    }
    Ok(findings)
}

fn definition_finding(value: &Value, label: &str, artifacts: &[String]) -> Option<Finding> {
    let delta = value.get("definition")?;
    let statement = field_str(delta, "statement").unwrap_or("declared definition delta");
    let mut finding = bare(PROFILE, &format!("{label}#definition"), "declared");
    finding.scope = statement.to_string();
    finding.evidence_class = "definition".into();
    finding.completeness = "declared".into();
    finding.attempted = "prior-vs-new comparison".into();
    finding.committed = "not a runtime commit".into();
    finding.reproduction_status = "not_attempted".into();
    finding.artifacts = artifacts.to_vec();
    finding.next_action = "declared only; does not establish execution".into();
    Some(finding)
}

fn profile_root(value: &Value) -> Result<&Value, String> {
    if field_str(value, "schema") == Some(PROFILE) {
        return Ok(value);
    }
    value
        .get("profile")
        .filter(|profile| profile.is_object())
        .ok_or_else(|| "check output has no profile object".to_string())
}

fn profile_finding(profile: &Value, label: &str, goal: &Value, artifacts: &[String]) -> Finding {
    let id = field_str(goal, "id").unwrap_or("goal");
    let status = field_str(goal, "status").unwrap_or("unknown").to_string();
    let completeness = profile_completeness(profile, &status);
    let commands = gap_commands(goal);
    let joined = commands.join(", ");
    let command = if joined.is_empty() {
        None
    } else {
        Some(joined.as_str())
    };
    let mut finding = bare(PROFILE, &format!("{label}#{id}"), &status);
    finding.scope = profile_scope(
        &status,
        field_str(goal, "detail"),
        goal.get("proven_deadlock").and_then(Value::as_bool),
    );
    finding.evidence_class = "bounded_paths".into();
    finding.completeness = completeness;
    finding.source_coordinate = profile_coordinate(goal);
    finding.attempted = format!("{} {id}", field_str(goal, "kind").unwrap_or("goal"));
    finding.committed = "not a runtime commit".into();
    finding.reproduction_status = "not_attempted".into();
    finding.artifacts = artifacts.to_vec();
    finding.next_action = next_action(
        &status,
        &finding.completeness,
        command,
        field_str(profile, "incomplete_reason"),
    );
    finding
}

fn profile_completeness(profile: &Value, status: &str) -> String {
    if matches!(status, "no_witness_found_incomplete" | "input_coverage_gap") {
        return "incomplete".into();
    }
    if profile.get("search_complete").and_then(Value::as_bool) == Some(false) {
        return "incomplete".into();
    }
    "complete".into()
}

fn profile_scope(status: &str, detail: Option<&str>, proven_deadlock: Option<bool>) -> String {
    let mut parts = vec!["bounded search over the supplied corpus".to_string()];
    if status == "witness_found" {
        parts.push("smallest found within the enumerated search".into());
    }
    if proven_deadlock == Some(false) {
        parts.push("not established as a deadlock".into());
    }
    if let Some(detail) = detail {
        if !detail.contains("proven") && !detail.contains("minimal") {
            parts.push(detail.to_string());
        }
    }
    parts.join("; ")
}

fn profile_coordinate(goal: &Value) -> Option<SourceCoordinate> {
    let command =
        first_named(goal, "gaps", "command").or_else(|| first_named(goal, "witness", "command"));
    let state = first_named(goal, "stuck_states", "state");
    coordinate(command, state)
}

fn gap_commands(goal: &Value) -> Vec<String> {
    goal.get("gaps")
        .and_then(Value::as_array)
        .map(|gaps| {
            gaps.iter()
                .filter_map(|gap| field_str(gap, "command").map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn unreadable(path: &Path, reason: &str) -> Finding {
    let mut finding = bare(
        "unreadable",
        &format!("unreadable:{}", path.display()),
        "unreadable",
    );
    finding.scope = format!("unreadable input: {reason}");
    finding.evidence_class = "none".into();
    finding.completeness = "unknown".into();
    finding.attempted = path.display().to_string();
    finding.committed = "not read".into();
    finding.reproduction_status = "not_attempted".into();
    finding.next_action = next_action("unreadable", "unknown", None, None);
    finding
}

fn bare(source_kind: &str, id: &str, status: &str) -> Finding {
    Finding {
        source_kind: source_kind.to_string(),
        id: id.to_string(),
        status: status.to_string(),
        scope: String::new(),
        evidence_class: String::new(),
        completeness: String::new(),
        source_coordinate: None,
        before: None,
        after: None,
        attempted: String::new(),
        committed: String::new(),
        reproduction_status: String::new(),
        artifacts: Vec::new(),
        next_action: String::new(),
        present: None,
    }
}

fn next_action(
    status: &str,
    completeness: &str,
    command: Option<&str>,
    detail: Option<&str>,
) -> String {
    let key = if status == "no_witness_found" && completeness == "incomplete" {
        "no_witness_found_incomplete"
    } else {
        status
    };
    action_for(key, command, detail)
}

fn action_for(status: &str, command: Option<&str>, detail: Option<&str>) -> String {
    if let Some(action) = search_action(status, command) {
        return action;
    }
    behavior_action(status, detail)
}

fn search_action(status: &str, command: Option<&str>) -> Option<String> {
    let action = match status {
        "witness_found" => "promote as regression or continue",
        "input_coverage_gap" => return Some(coverage_action(command)),
        "no_witness_found_incomplete" | "inconclusive" => "raise bounds or add cases",
        "no_witness_found" => {
            "no witness in the completed search; not unreachability outside the searched inputs"
        }
        "no_witness_found_complete" => {
            "the target is unreachable within the supplied corpus; change the corpus or the goal"
        }
        "violation" | "expectation_failed" => "reproduce with `ik scenario reproduce`",
        "stuck_states_found" => {
            "add a corpus case or allow_waiting; this is not established as a deadlock"
        }
        "no_stuck_states_found" => "continue",
        "no_violation_found" => "not a proof; widen the sample or use an exhaustive finite model",
        "holds_in_scope" => "holds in the declared finite model only; not unbounded correctness",
        _ => return None,
    };
    Some(action.to_string())
}

fn behavior_action(status: &str, detail: Option<&str>) -> String {
    if let Some(action) = history_action(status) {
        return action.to_string();
    }
    match status {
        "unsupported" => with_detail("configure the unsupported capability", detail),
        "tool_unavailable" => with_detail("configure or install the missing tool", detail),
        "refused" => {
            "refused, not a committed state; do not treat the proposal as committed".into()
        }
        "committed" => "promote as regression or continue".into(),
        "noop" => "no new commit; continue".into(),
        "unreadable" => "replace or repair the input; it was not skipped".into(),
        "timed_out" | "cancelled" | "failed" => {
            "rerun with a larger limit or fix the adapter invocation".into()
        }
        other => format!("inspect status {other}"),
    }
}

fn history_action(status: &str) -> Option<&'static str> {
    match status {
        "committed_exact" => Some("do not resend; the earlier result stands"),
        "historical" => Some("do not treat this as a new commit or a current handle"),
        "reply_dropped" => {
            Some("look up the earlier operation; do not resend from the dropped reply")
        }
        "not_recorded" | "not_found" => {
            Some("do not treat the earlier attempt as committed from this lookup")
        }
        "conflict" => {
            Some("do not treat this as the same operation; inspect the conflicting record")
        }
        "not_committed" => Some("do not treat a dropped reply as a commit"),
        _ => None,
    }
}

fn coverage_action(command: Option<&str>) -> String {
    let name = command
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("the missing command");
    format!("add corpus cases for {name}")
}

fn with_detail(action: &str, detail: Option<&str>) -> String {
    match detail.map(str::trim).filter(|text| !text.is_empty()) {
        Some(detail) => format!("{action}: {detail}"),
        None => action.to_string(),
    }
}

fn evidence_phrase(evidence: &str) -> String {
    match evidence {
        "sampled_execution" => "sampled execution; not a proof".into(),
        "bounded_paths" => "bounded search over the supplied inputs".into(),
        "definition" => "declared in the new definition, not executed".into(),
        "exhaustive_finite_model" => {
            "exhaustive within the declared finite model, not outside that model".into()
        }
        "observed_trace" => "observed runtime trace".into(),
        "conformance_test" => "conformance test of the stated cases".into(),
        "checked_induction_or_certificate" => {
            "checked certificate for the stated claim, not an unbounded result".into()
        }
        "none" => "no evidence class".into(),
        other => format!("evidence class {other}"),
    }
}

fn execution_detail<'a>(execution: &str, reason: &'a str) -> Option<&'a str> {
    if blocked_execution(execution) && !reason.is_empty() {
        Some(reason)
    } else {
        None
    }
}

fn render_text(report: &Report) -> String {
    let mut text = String::from("ik.report.v1\n");
    for finding in &report.findings {
        append_text(&mut text, finding);
    }
    text
}

fn append_text(text: &mut String, finding: &Finding) {
    text.push_str(&format!("finding {}\n", finding.id));
    push_line(text, "source", &finding.source_kind);
    push_line(text, "status", &finding.status);
    push_line(text, "scope", &finding.scope);
    push_line(text, "evidence", &finding.evidence_class);
    push_line(text, "completeness", &finding.completeness);
    if let Some(coordinate) = &finding.source_coordinate {
        push_line(text, "coordinate", &coordinate_text(coordinate));
    }
    if let Some(before) = &finding.before {
        push_line(text, "before", &canonical_json(before));
    }
    if let Some(after) = &finding.after {
        push_line(text, "after", &canonical_json(after));
    }
    push_line(text, "attempted", &finding.attempted);
    push_line(text, "committed", &finding.committed);
    push_line(text, "reproduction", &finding.reproduction_status);
    for artifact in &finding.artifacts {
        push_line(text, "artifact", artifact);
    }
    push_line(text, "next", &finding.next_action);
}

fn push_line(text: &mut String, key: &str, value: &str) {
    text.push_str(&format!("  {key}: {value}\n"));
}

fn coordinate_text(coordinate: &SourceCoordinate) -> String {
    match (&coordinate.command, &coordinate.state) {
        (Some(command), Some(state)) => format!("command={command} state={state}"),
        (Some(command), None) => format!("command={command}"),
        (None, Some(state)) => format!("state={state}"),
        (None, None) => String::new(),
    }
}

fn render_html(report: &Report) -> String {
    let mut body = String::new();
    for finding in &report.findings {
        body.push_str(&finding_html(finding));
    }
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>ik report</title>\n<style>\nbody {{ font-family: sans-serif; margin: 2rem; }}\narticle {{ border-top: 1px solid #ccc; padding: 1rem 0; }}\ndt {{ font-weight: bold; }}\n</style>\n</head>\n<body>\n<h1>ik.report.v1</h1>\n{body}</body>\n</html>\n"
    )
}

fn finding_html(finding: &Finding) -> String {
    let mut rows = String::new();
    html_row(&mut rows, "source", &finding.source_kind);
    html_row(&mut rows, "status", &finding.status);
    html_row(&mut rows, "scope", &finding.scope);
    html_row(&mut rows, "evidence", &finding.evidence_class);
    html_row(&mut rows, "completeness", &finding.completeness);
    if let Some(coordinate) = &finding.source_coordinate {
        html_row(&mut rows, "coordinate", &coordinate_text(coordinate));
    }
    if let Some(before) = &finding.before {
        html_row(&mut rows, "before", &canonical_json(before));
    }
    if let Some(after) = &finding.after {
        html_row(&mut rows, "after", &canonical_json(after));
    }
    html_row(&mut rows, "attempted", &finding.attempted);
    html_row(&mut rows, "committed", &finding.committed);
    html_row(&mut rows, "reproduction", &finding.reproduction_status);
    for artifact in &finding.artifacts {
        html_row(&mut rows, "artifact", artifact);
    }
    html_row(&mut rows, "next", &finding.next_action);
    format!(
        "<article>\n<h2>{}</h2>\n<dl>\n{rows}</dl>\n</article>\n",
        html_escape(&finding.id)
    )
}

fn html_row(rows: &mut String, key: &str, value: &str) {
    rows.push_str(&format!(
        "<dt>{}</dt><dd>{}</dd>\n",
        html_escape(key),
        html_escape(value)
    ));
}

fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn field_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn required_str(value: &Value, key: &str, what: &str) -> Result<String, String> {
    field_str(value, key)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{what} is missing {key}"))
}

fn coverage_parts(coverage: Option<&Value>) -> (bool, String) {
    let Some(coverage) = coverage else {
        return (false, String::new());
    };
    let complete = coverage
        .get("complete")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let reason = field_str(coverage, "reason").unwrap_or("").to_string();
    (complete, reason)
}

fn coordinate(command: Option<String>, state: Option<String>) -> Option<SourceCoordinate> {
    if command.is_none() && state.is_none() {
        return None;
    }
    Some(SourceCoordinate { command, state })
}

fn first_named(value: &Value, array_key: &str, field: &str) -> Option<String> {
    value
        .get(array_key)
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(|row| field_str(row, field))
        .map(str::to_string)
}

fn document_paths(value: &Value) -> Vec<String> {
    let mut paths = string_list(value, "artifacts");
    paths.extend(string_list(value, "partial_logs"));
    paths
}

fn string_list(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn existing_named(root: &Path, names: &[&str]) -> Vec<String> {
    names
        .iter()
        .filter(|name| root.join(name).is_file())
        .map(|name| root.join(name).display().to_string())
        .collect()
}

fn with_artifact(artifacts: &[String], extra: Option<&str>) -> Vec<String> {
    let mut out = artifacts.to_vec();
    if let Some(extra) = extra {
        if !extra.is_empty() && !out.iter().any(|item| item == extra) {
            out.push(extra.to_string());
        }
    }
    out
}
