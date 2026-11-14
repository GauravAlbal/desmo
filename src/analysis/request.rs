//! `ik.analysis.request.v1`.
//!
//! Adapter controls live in this document. The host does not copy them into the
//! process environment.

use super::paths::{contained_file, is_symlink, relative_is_local, sha256_file};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const REQUEST_PROTOCOL: &str = "ik.analysis.request.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisRequest {
    pub protocol: String,
    pub request_id: String,
    pub run_id: String,
    pub subject: Subject,
    pub goals: Vec<GoalRef>,
    pub profile_digest: String,
    pub property_class: String,
    pub inputs: Vec<InputRef>,
    pub bundle_dir: PathBuf,
    pub limits: Limits,
    pub permissions: Permissions,
    #[serde(default)]
    pub controls: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subject {
    pub kind: String,
    pub identity: SubjectIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectIdentity {
    pub machine_id: String,
    pub spec_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalRef {
    pub id: String,
    pub content_digest: String,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRef {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub timeout_seconds: u64,
    pub max_output_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    pub scratch_root: PathBuf,
    pub production_store_access: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSnapshot {
    pub path: String,
    pub sha256: String,
}

pub fn load_request(path: &Path) -> Result<AnalysisRequest, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    parse_request(&text)
}

pub fn parse_request(text: &str) -> Result<AnalysisRequest, String> {
    let request: AnalysisRequest = serde_json::from_str(text).map_err(|error| error.to_string())?;
    check_request_fields(&request)?;
    Ok(request)
}

fn check_request_fields(request: &AnalysisRequest) -> Result<(), String> {
    if request.protocol != REQUEST_PROTOCOL {
        return Err("protocol_mismatch".into());
    }
    if request.request_id.is_empty() || request.run_id.is_empty() {
        return Err("malformed_request".into());
    }
    if request.subject.kind.is_empty()
        || request.subject.identity.machine_id.is_empty()
        || request.subject.identity.spec_hash.is_empty()
        || request.profile_digest.is_empty()
    {
        return Err("malformed_request".into());
    }
    if !super::manifest::property_kind_known(&request.property_class) {
        return Err("malformed_request".into());
    }
    if request.goals.is_empty() || goal_ids_repeat(&request.goals) {
        return Err("malformed_request".into());
    }
    if request.limits.timeout_seconds == 0 || request.limits.max_output_bytes == 0 {
        return Err("malformed_request".into());
    }
    if !request.bundle_dir.is_absolute() || !request.permissions.scratch_root.is_absolute() {
        return Err("malformed_request".into());
    }
    if request.permissions.production_store_access {
        return Err("production_store_access_forbidden".into());
    }
    if !inputs_locally_addressed(&request.inputs) {
        return Err("path_traversal".into());
    }
    Ok(())
}

fn goal_ids_repeat(goals: &[GoalRef]) -> bool {
    let mut ids: Vec<&str> = goals.iter().map(|goal| goal.id.as_str()).collect();
    let before = ids.len();
    ids.sort_unstable();
    ids.dedup();
    ids.len() != before || goals.iter().any(goal_incomplete)
}

fn goal_incomplete(goal: &GoalRef) -> bool {
    goal.id.is_empty() || goal.content_digest.is_empty()
}

fn inputs_locally_addressed(inputs: &[InputRef]) -> bool {
    inputs
        .iter()
        .all(|input| relative_is_local(&input.path) && is_lower_hex(&input.sha256))
}

pub fn is_lower_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// Hash declared inputs. A declared digest that does not match the bytes is stale.
/// Symlinks and paths outside the bundle are refused before any adapter starts.
pub fn snapshot_inputs(request: &AnalysisRequest) -> Result<Vec<InputSnapshot>, String> {
    if is_symlink(&request.bundle_dir) {
        return Err("symlink_rejected".into());
    }
    let mut snapshots = Vec::with_capacity(request.inputs.len());
    for input in &request.inputs {
        let path = contained_file(&request.bundle_dir, &input.path).map_err(str::to_string)?;
        if !path.is_file() {
            return Err("stale_input_digest".into());
        }
        let digest = sha256_file(&path).map_err(|_| "stale_input_digest".to_string())?;
        if digest != input.sha256 {
            return Err("stale_input_digest".into());
        }
        snapshots.push(InputSnapshot {
            path: input.path.clone(),
            sha256: digest,
        });
    }
    Ok(snapshots)
}
