//! `ik.analysis.adapter.v1` manifest.
//!
//! A manifest is a claim about an approved executable. It is not certification,
//! and it cannot advertise an unbounded capability.

use super::paths::{is_symlink, sha256_file};
use super::request::is_lower_hex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const MANIFEST_PROTOCOL: &str = "ik.analysis.adapter.v1";
pub const RESULT_SCHEMA: &str = "ik.analysis.result.v1";
const FORBIDDEN_CLAIM: &str = "supports_everything";

const INPUT_KINDS: &[&str] = &[
    "ik.runtime-driver.v1",
    "finite_runtime_graph",
    "native_model",
    "observation_trace",
    "executable_driver",
];

const PROPERTY_KINDS: &[&str] = &[
    "safety",
    "reachability",
    "finite_scope_stuck",
    "temporal",
    "conformance",
    "observational",
];

const EVIDENCE_CAPABILITIES: &[&str] = &[
    "sampled_execution",
    "bounded_paths",
    "exhaustive_finite_model",
    "observed_trace",
    "conformance_test",
    "checked_induction_or_certificate",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterManifest {
    pub protocol: String,
    pub adapter: AdapterIdentity,
    pub invocation: Invocation,
    pub engine: EngineIdentity,
    pub inputs: Vec<String>,
    pub properties: Vec<String>,
    pub evidence: Vec<String>,
    pub requires: Requirements,
    pub result_schema: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterIdentity {
    pub id: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    pub argv: Vec<String>,
    pub executable_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineIdentity {
    pub id: String,
    pub version_constraint: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirements {
    pub production_store_access: bool,
    pub network: bool,
}

pub fn property_kind_known(kind: &str) -> bool {
    PROPERTY_KINDS.contains(&kind)
}

pub fn load_manifest(path: &Path) -> Result<AdapterManifest, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    parse_manifest(&text)
}

/// Document checks only. Does not open or execute the declared executable.
pub fn parse_manifest(text: &str) -> Result<AdapterManifest, String> {
    if text.contains(FORBIDDEN_CLAIM) {
        return Err("supports_everything".into());
    }
    let manifest: AdapterManifest =
        serde_json::from_str(text).map_err(|_| "malformed_manifest".to_string())?;
    check_document(&manifest)?;
    Ok(manifest)
}

fn check_document(manifest: &AdapterManifest) -> Result<(), String> {
    if manifest.protocol != MANIFEST_PROTOCOL {
        return Err("protocol_mismatch".into());
    }
    if manifest.result_schema != RESULT_SCHEMA {
        return Err("result_schema_mismatch".into());
    }
    if manifest.requires.production_store_access {
        return Err("production_store_access_forbidden".into());
    }
    if identity_blank(&manifest.adapter) || engine_blank(&manifest.engine) {
        return Err("malformed_manifest".into());
    }
    if !closed_set(&manifest.inputs, INPUT_KINDS)
        || !closed_set(&manifest.properties, PROPERTY_KINDS)
        || !closed_set(&manifest.evidence, EVIDENCE_CAPABILITIES)
    {
        return Err("unknown_capability".into());
    }
    check_argv(&manifest.invocation)
}

fn identity_blank(identity: &AdapterIdentity) -> bool {
    identity.id.is_empty() || identity.version.is_empty()
}

fn engine_blank(engine: &EngineIdentity) -> bool {
    engine.id.is_empty() || engine.version_constraint.is_empty()
}

fn closed_set(values: &[String], allowed: &[&str]) -> bool {
    !values.is_empty() && values.iter().all(|value| allowed.contains(&value.as_str()))
}

fn check_argv(invocation: &Invocation) -> Result<(), String> {
    let Some(executable) = invocation.argv.first() else {
        return Err("malformed_manifest".into());
    };
    if invocation
        .argv
        .iter()
        .any(|arg| arg.is_empty() || arg.contains('\0'))
    {
        return Err("malformed_manifest".into());
    }
    if !is_lower_hex(&invocation.executable_sha256) {
        return Err("malformed_manifest".into());
    }
    if !Path::new(executable).is_absolute() {
        return Err("relative_executable".into());
    }
    Ok(())
}

/// Read the pinned executable and compare its digest. Never executes it.
pub fn check_executable(manifest: &AdapterManifest) -> Result<PathBuf, String> {
    let executable = PathBuf::from(&manifest.invocation.argv[0]);
    if !executable.is_absolute() {
        return Err("relative_executable".into());
    }
    if is_symlink(&executable) {
        return Err("symlink_rejected".into());
    }
    let digest = sha256_file(&executable).map_err(|_| "executable_sha256_mismatch".to_string())?;
    if digest != manifest.invocation.executable_sha256 {
        return Err("executable_sha256_mismatch".into());
    }
    Ok(executable)
}
