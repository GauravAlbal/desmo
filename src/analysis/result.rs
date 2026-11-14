//! `ik.analysis.result.v1` and the host's reading of it.
//!
//! Execution status, finding, evidence class, coverage, and reproduction are
//! separate axes. A backend verdict is not a field of this schema and is never
//! adopted as a host acceptance.

use super::paths::{contained_file, non_empty_regular_file, PATH_TRAVERSAL, SYMLINK_REJECTED};
use super::request::{AnalysisRequest, GoalRef, InputRef, InputSnapshot, Subject};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::Path;

pub const RESULT_PROTOCOL: &str = "ik.analysis.result.v1";

const EXECUTION: &[&str] = &[
    "completed",
    "timed_out",
    "cancelled",
    "tool_unavailable",
    "unsupported",
    "failed",
];
const FINDINGS: &[&str] = &[
    "violation",
    "witness_found",
    "no_violation_found",
    "no_witness_found",
    "holds_in_scope",
    "inconclusive",
];
const EVIDENCE: &[&str] = &[
    "sampled_execution",
    "bounded_paths",
    "exhaustive_finite_model",
    "observed_trace",
    "conformance_test",
];
const REPRODUCTION: &[&str] = &[
    "not_attempted",
    "model_only",
    "concrete_reproduced",
    "prefix_cycle_validated",
    "not_reproduced",
    "mapping_incomplete",
    "nondeterministic",
];
const VERDICT_KEYS: &[&str] = &[
    "verdict",
    "overall",
    "accept",
    "acceptance",
    "release_verdict",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisResult {
    pub protocol: String,
    pub request_id: String,
    pub run_id: String,
    pub subject: Subject,
    pub goals: Vec<GoalDigest>,
    pub profile_digest: String,
    pub inputs: Vec<InputRef>,
    pub execution_status: String,
    pub properties: Vec<PropertyResult>,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalDigest {
    pub id: String,
    pub content_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyResult {
    pub goal_id: String,
    pub property_class: String,
    pub finding: String,
    pub evidence_class: String,
    pub coverage: Coverage,
    pub reproduction: Reproduction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub complete: bool,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reproduction {
    pub status: String,
    #[serde(default)]
    pub artifact: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RequirementAssessment {
    pub goal_id: String,
    pub required: bool,
    pub status: String,
    pub reason: String,
    pub meets_requirement: bool,
}

pub fn requirement_status(
    goals: &[GoalRef],
    execution_status: &str,
    properties: &[PropertyResult],
) -> Vec<RequirementAssessment> {
    goals
        .iter()
        .map(|goal| assess_goal(goal, execution_status, properties))
        .collect()
}

fn assess_goal(
    goal: &GoalRef,
    execution_status: &str,
    properties: &[PropertyResult],
) -> RequirementAssessment {
    if !goal.required {
        return assessment(goal, "not_required", "optional_exclusion", false);
    }
    if let Some(blocked) = blocked_by_execution(goal, execution_status) {
        return blocked;
    }
    match properties.iter().find(|prop| prop.goal_id == goal.id) {
        None => assessment(goal, "blocked", "missing", false),
        Some(prop) => assess_finding(goal, prop),
    }
}

fn blocked_by_execution(goal: &GoalRef, status: &str) -> Option<RequirementAssessment> {
    let reason = match status {
        "completed" => return None,
        "unsupported" | "tool_unavailable" => "unsupported",
        _ => "incomplete",
    };
    Some(assessment(goal, "blocked", reason, false))
}

fn assess_finding(goal: &GoalRef, prop: &PropertyResult) -> RequirementAssessment {
    if prop.finding == "witness_found" && prop.property_class == "reachability" {
        return assessment(goal, "met", "witness_found", true);
    }
    if prop.finding == "violation" {
        return assessment(goal, "not_met", "violation", false);
    }
    if !prop.coverage.complete || prop.finding == "inconclusive" {
        return assessment(goal, "blocked", "incomplete", false);
    }
    if prop.finding == "holds_in_scope" {
        return assessment(goal, "met", "holds_in_scope", true);
    }
    assessment(goal, "not_met", prop.finding.as_str(), false)
}

fn assessment(goal: &GoalRef, status: &str, reason: &str, meets: bool) -> RequirementAssessment {
    RequirementAssessment {
        goal_id: goal.id.clone(),
        required: goal.required,
        status: status.to_string(),
        reason: reason.to_string(),
        meets_requirement: meets,
    }
}

pub fn host_interpretation(finding: &str, coverage_complete: bool) -> String {
    if finding == "no_witness_found" && !coverage_complete {
        "inconclusive_not_unreachable".to_string()
    } else if finding == "no_violation_found" {
        "no_violation_found_not_proof".to_string()
    } else {
        finding.to_string()
    }
}

pub fn validate_result_bytes(
    request: &AnalysisRequest,
    raw: &str,
    adapter_out: &Path,
    before: &[InputSnapshot],
    after: &[InputSnapshot],
) -> Result<AnalysisResult, String> {
    if before != after {
        return Err("input_modified".into());
    }
    let value: Value = serde_json::from_str(raw).map_err(|_| "malformed_json".to_string())?;
    reject_backend_verdict(&value)?;
    let result: AnalysisResult =
        serde_json::from_value(value).map_err(|_| "malformed_result".to_string())?;
    check_result_shape(&result)?;
    check_correlation(request, &result)?;
    check_support(&result)?;
    check_output_paths(adapter_out, &result)?;
    Ok(result)
}

fn reject_backend_verdict(value: &Value) -> Result<(), String> {
    let Some(object) = value.as_object() else {
        return Err("malformed_json".into());
    };
    if VERDICT_KEYS.iter().any(|key| object.contains_key(*key)) {
        return Err("backend_verdict_rejected".into());
    }
    Ok(())
}

fn check_result_shape(result: &AnalysisResult) -> Result<(), String> {
    if result.protocol != RESULT_PROTOCOL {
        return Err("malformed_result".into());
    }
    if !EXECUTION.contains(&result.execution_status.as_str()) {
        return Err("malformed_result".into());
    }
    if result.properties.iter().any(property_unknown) {
        return Err("malformed_result".into());
    }
    if duplicate_goal(&result.properties) {
        return Err("malformed_result".into());
    }
    Ok(())
}

fn property_unknown(prop: &PropertyResult) -> bool {
    !FINDINGS.contains(&prop.finding.as_str())
        || !EVIDENCE.contains(&prop.evidence_class.as_str())
        || !REPRODUCTION.contains(&prop.reproduction.status.as_str())
        || prop.goal_id.is_empty()
        || prop.property_class.is_empty()
}

fn duplicate_goal(properties: &[PropertyResult]) -> bool {
    let mut ids: Vec<&str> = properties
        .iter()
        .map(|prop| prop.goal_id.as_str())
        .collect();
    let before = ids.len();
    ids.sort_unstable();
    ids.dedup();
    ids.len() != before
}

fn check_correlation(request: &AnalysisRequest, result: &AnalysisResult) -> Result<(), String> {
    if request.request_id != result.request_id {
        return Err("request_id_mismatch".into());
    }
    if request.run_id != result.run_id {
        return Err("run_id_mismatch".into());
    }
    if let Some(reason) = subject_reason(&request.subject, &result.subject) {
        return Err(reason.into());
    }
    if request.profile_digest != result.profile_digest {
        return Err("stale_profile_digest".into());
    }
    if !same_goals(&request.goals, &result.goals) {
        return Err("stale_goal_digest".into());
    }
    if !same_inputs(&request.inputs, &result.inputs) {
        return Err("stale_input_digest".into());
    }
    if result
        .properties
        .iter()
        .any(|prop| prop.property_class != request.property_class)
    {
        return Err("property_mismatch".into());
    }
    Ok(())
}

fn subject_reason(expected: &Subject, actual: &Subject) -> Option<&'static str> {
    if expected.identity.spec_hash != actual.identity.spec_hash {
        return Some("stale_subject_hash");
    }
    if expected.kind != actual.kind || expected.identity.machine_id != actual.identity.machine_id {
        Some("subject_mismatch")
    } else {
        None
    }
}

fn same_goals(expected: &[GoalRef], actual: &[GoalDigest]) -> bool {
    let mut left: Vec<(&str, &str)> = expected
        .iter()
        .map(|goal| (goal.id.as_str(), goal.content_digest.as_str()))
        .collect();
    let mut right: Vec<(&str, &str)> = actual
        .iter()
        .map(|goal| (goal.id.as_str(), goal.content_digest.as_str()))
        .collect();
    left.sort_unstable();
    right.sort_unstable();
    left == right
}

fn same_inputs(expected: &[InputRef], actual: &[InputRef]) -> bool {
    let mut left: Vec<(&str, &str)> = expected
        .iter()
        .map(|input| (input.path.as_str(), input.sha256.as_str()))
        .collect();
    let mut right: Vec<(&str, &str)> = actual
        .iter()
        .map(|input| (input.path.as_str(), input.sha256.as_str()))
        .collect();
    left.sort_unstable();
    right.sort_unstable();
    left == right
}

fn check_support(result: &AnalysisResult) -> Result<(), String> {
    if result.properties.iter().any(finding_unsupported) {
        return Err("evidence_does_not_support_finding".into());
    }
    if result.execution_status != "completed"
        && result
            .properties
            .iter()
            .any(|prop| prop.finding == "holds_in_scope")
    {
        return Err("evidence_does_not_support_finding".into());
    }
    Ok(())
}

fn finding_unsupported(prop: &PropertyResult) -> bool {
    prop.finding == "holds_in_scope"
        && (prop.evidence_class != "exhaustive_finite_model" || !prop.coverage.complete)
}

fn check_output_paths(adapter_out: &Path, result: &AnalysisResult) -> Result<(), String> {
    for relative in artifact_paths(result) {
        let path = contained_file(adapter_out, relative).map_err(str::to_string)?;
        if !non_empty_regular_file(&path) {
            return Err("truncated_artifact".into());
        }
        if relative.ends_with(".json") && json_unreadable(&path) {
            return Err("truncated_artifact".into());
        }
    }
    for prop in &result.properties {
        check_replay(adapter_out, prop)?;
    }
    Ok(())
}

fn artifact_paths(result: &AnalysisResult) -> Vec<&str> {
    let mut paths: Vec<&str> = result.artifacts.iter().map(String::as_str).collect();
    for prop in &result.properties {
        if let Some(artifact) = prop.reproduction.artifact.as_deref() {
            paths.push(artifact);
        }
    }
    paths
}

fn json_unreadable(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return true;
    };
    serde_json::from_str::<Value>(&text).is_err()
}

fn check_replay(adapter_out: &Path, prop: &PropertyResult) -> Result<(), String> {
    if !claims_concrete_replay(&prop.reproduction.status) {
        return Ok(());
    }
    let Some(relative) = prop.reproduction.artifact.as_deref() else {
        return Err("reproduction_claim_unsupported".into());
    };
    let path = contained_file(adapter_out, relative).map_err(replay_or_path)?;
    let text =
        fs::read_to_string(path).map_err(|_| "reproduction_claim_unsupported".to_string())?;
    let value: Value =
        serde_json::from_str(&text).map_err(|_| "reproduction_claim_unsupported".to_string())?;
    if replayable(&value) {
        Ok(())
    } else {
        Err("reproduction_claim_unsupported".into())
    }
}

fn claims_concrete_replay(status: &str) -> bool {
    status == "concrete_reproduced" || status == "prefix_cycle_validated"
}

fn replay_or_path(reason: &str) -> String {
    if reason == PATH_TRAVERSAL || reason == SYMLINK_REJECTED {
        reason.to_string()
    } else {
        "reproduction_claim_unsupported".into()
    }
}

fn replayable(value: &Value) -> bool {
    value.get("replayable").and_then(Value::as_bool) == Some(true)
        && value
            .get("steps")
            .and_then(Value::as_array)
            .is_some_and(|steps| !steps.is_empty())
}
