//! Replayable falsification artifact.
//!
//! The reproduce blob is opaque evidence for an external engine. It is not
//! semantic authority and cannot change transition legality.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A minimized falsifying witness bound to an obligation or challenge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterexampleV1 {
    pub schema_version: u64,
    pub source_id: String,
    pub machine_id: String,
    pub spec_hash: String,
    pub failure_class: String,
    pub minimal_steps: Vec<String>,
    pub minimal_inputs: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
    pub engine_version: String,
    pub reproduce_blob: String,
}

/// Replay did not reproduce the same semantic failure class.
pub const INVALID_NONDETERMINISTIC_COUNTEREXAMPLE: &str = "INVALID_NONDETERMINISTIC_COUNTEREXAMPLE";

/// Classify a fresh replay against the captured failure class.
pub fn classify_replay(expected_class: &str, observed_class: Option<&str>) -> String {
    match observed_class {
        Some(observed) if observed == expected_class => "reproduced".to_string(),
        _ => INVALID_NONDETERMINISTIC_COUNTEREXAMPLE.to_string(),
    }
}
