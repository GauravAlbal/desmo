//! Host side of the external analysis-adapter contract.
//!
//! Adapters are approved executables. This module reads their manifests, builds
//! one request, spawns the pinned argv, and validates the normalized result.
//! It does not accept a backend release verdict.

mod cli;
mod conformance;
mod manifest;
mod paths;
mod request;
mod result;
mod runner;

pub use cli::{describe_manifest, run_cli, validate_stored};
pub use request::{AnalysisRequest, GoalRef, InputRef};
pub use result::{
    requirement_status, Coverage, PropertyResult, Reproduction, RequirementAssessment,
};
pub use runner::{run_adapter, EffectiveControls, HostFinding, HostReport, RunSpec};
