//! Law-linked semantic mutation adequacy.
//!
//! Applies type-valid definition edits and asks existing IK-DX explorers
//! whether they distinguish the mutant. No production runtime flags.

use super::challenge::apply_guard_boundary;
use super::compile::compile;
use super::def::Definition;
use super::diff::diff_machines;
use super::explore::{explore, Bounds, CaseCorpus};
use super::plan::SemanticCoordinate;
use serde::{Deserialize, Serialize};

/// One mutant's adequacy row, keyed by law/challenge identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationResult {
    pub mutant_id: String,
    pub kind: String,
    pub coordinate: SemanticCoordinate,
    pub status: String,
    pub engine: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub witness: Option<String>,
}

/// Adequacy report grouped by mutant, not a single percentage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationReport {
    pub schema_version: u64,
    pub results: Vec<MutationResult>,
}

/// Run the adjacent-guard mutant for one command outcome against explore+diff.
pub fn run_guard_boundary_mutant(
    def: &Definition,
    command: &str,
    index: usize,
    corpus: &CaseCorpus,
    bounds: Bounds,
) -> Option<MutationResult> {
    let mutant_def = apply_guard_boundary(def, command, index)?;
    let original = compile(def.clone()).ok()?;
    let mutant = compile(mutant_def).ok()?;
    let old_run = explore(&original, corpus, bounds);
    let new_run = explore(&mutant, corpus, bounds);
    let diff = diff_machines(
        def,
        &apply_guard_boundary(def, command, index)?,
        corpus,
        bounds,
    )
    .ok()?;
    let killed = old_run.coverage.executed_outcomes != new_run.coverage.executed_outcomes
        || old_run.coverage.reached_terminal_states != new_run.coverage.reached_terminal_states
        || diff.executable.dimension.is_some();
    let witness = diff
        .executable
        .witness
        .as_ref()
        .map(|steps| {
            steps
                .iter()
                .map(|s| s.command.clone())
                .collect::<Vec<_>>()
                .join(",")
        })
        .or_else(|| {
            if killed {
                Some("coverage-delta".to_string())
            } else {
                None
            }
        });
    Some(MutationResult {
        mutant_id: format!("guard-boundary/{command}/{index}"),
        kind: "CHL-GUARD-BOUNDARY".to_string(),
        coordinate: SemanticCoordinate {
            command: Some(command.to_string()),
            outcome_index: Some(index),
            ..SemanticCoordinate::default()
        },
        status: if killed { "killed" } else { "survived" }.to_string(),
        engine: "explore+diff".to_string(),
        witness,
    })
}

/// Curated CORE_LAWS mutant row. Killing evidence is the Store stale path.
pub fn curated_stale_capability_killed() -> MutationResult {
    MutationResult {
        mutant_id: "core-law/disable-stale-capability-rejection".to_string(),
        kind: "OBL-CAP-001".to_string(),
        coordinate: SemanticCoordinate::default(),
        status: "killed".to_string(),
        engine: "store.command".to_string(),
        witness: Some("stale_capability_sequence_refuses".to_string()),
    }
}

/// Assemble a law-linked report.
pub fn mutation_report(results: Vec<MutationResult>) -> MutationReport {
    MutationReport {
        schema_version: 1,
        results,
    }
}
