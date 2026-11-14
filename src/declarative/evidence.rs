//! Adapters from existing IK-DX artifacts onto verification obligations.
//!
//! These functions read [`Explanation`], [`Exploration`], and
//! [`crate::declarative::MachineGraph`] outputs. They do not decide, apply, or
//! persist.

use super::analyze::{MachineGraph, StructuralAnalysis};
use super::evaluate::InvariantEvaluation;
use super::explain::Explanation;
use super::explore::Exploration;
use super::plan::{VerificationObligationV1, VerificationPlanV1};
use crate::canonical::digest_json;
use crate::diagnostic::Diagnostic;
use crate::effect::EffectIntentV1;
use crate::machine::DomainEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Canonical read-only record of one semantic execution.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TransitionEvidenceV1 {
    pub schema_version: u64,
    pub machine_id: String,
    pub spec_hash: String,
    pub pre_state_digest: String,
    pub command: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_outcome: Option<usize>,
    pub refused: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<DomainEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_state_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<bool>,
    pub invariants: Vec<InvariantEvaluation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<EffectIntentV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<Diagnostic>,
}

/// How one obligation stands against a specific evidence artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObligationDischarge {
    pub obligation_id: String,
    pub status: String,
    pub engine: String,
}

/// Project an [`Explanation`] into transition evidence. No second evaluator.
pub fn transition_evidence(explanation: &Explanation) -> TransitionEvidenceV1 {
    TransitionEvidenceV1 {
        schema_version: 1,
        machine_id: explanation.machine.machine_id.clone(),
        spec_hash: explanation.machine.spec_hash.clone(),
        pre_state_digest: explanation.basis.state_digest.clone(),
        command: explanation.command.clone(),
        source_state: explanation
            .decision
            .as_ref()
            .map(|d| d.source_state.clone()),
        selected_outcome: explanation.decision.as_ref().and_then(|d| d.selected),
        refused: explanation.verdict != "would_commit",
        event: explanation.event.clone(),
        post_state_digest: explanation.post_state.as_ref().map(digest_json),
        terminal: explanation.terminal,
        invariants: explanation.invariants.clone(),
        effect: explanation.effect.clone(),
        diagnostic: explanation.diagnostic.clone(),
    }
}

/// Map exploration coverage onto a plan. Stale spec hashes are inapplicable.
pub fn discharge_exploration(
    plan: &VerificationPlanV1,
    run: &Exploration,
) -> Vec<ObligationDischarge> {
    if run.spec_hash != plan.machine.spec_hash {
        return inapplicable(plan, "explore");
    }
    plan.obligations
        .iter()
        .map(|obligation| ObligationDischarge {
            obligation_id: obligation.obligation_id.clone(),
            status: exploration_status(obligation, run, plan).to_string(),
            engine: "explore".to_string(),
        })
        .collect()
}

/// Loading a machine is not evidence. Every obligation stays uncovered.
pub fn discharge_hollow(plan: &VerificationPlanV1) -> Vec<ObligationDischarge> {
    plan.obligations
        .iter()
        .map(|obligation| ObligationDischarge {
            obligation_id: obligation.obligation_id.clone(),
            status: "uncovered".to_string(),
            engine: "hollow".to_string(),
        })
        .collect()
}

/// Structural reachability is partial evidence and never discharges executable obligations.
pub fn discharge_graph(
    plan: &VerificationPlanV1,
    graph: &MachineGraph,
    _analysis: &StructuralAnalysis,
) -> Vec<ObligationDischarge> {
    if graph.spec_hash != plan.machine.spec_hash {
        return inapplicable(plan, "graph");
    }
    plan.obligations
        .iter()
        .map(|obligation| ObligationDischarge {
            obligation_id: obligation.obligation_id.clone(),
            status: "uncovered".to_string(),
            engine: "graph".to_string(),
        })
        .collect()
}

fn inapplicable(plan: &VerificationPlanV1, engine: &str) -> Vec<ObligationDischarge> {
    plan.obligations
        .iter()
        .map(|obligation| ObligationDischarge {
            obligation_id: obligation.obligation_id.clone(),
            status: "inapplicable".to_string(),
            engine: engine.to_string(),
        })
        .collect()
}

fn exploration_status(
    obligation: &VerificationObligationV1,
    run: &Exploration,
    plan: &VerificationPlanV1,
) -> &'static str {
    if obligation.obligation_id.starts_with("authored/") {
        return authored_status(obligation, run, plan);
    }
    match obligation.kind.as_str() {
        "OBL-OUT-001" => {
            let command = obligation.coordinate.command.as_deref();
            let index = obligation.coordinate.outcome_index;
            if run.coverage.executed_outcomes.iter().any(|outcome| {
                Some(outcome.command.as_str()) == command && Some(outcome.outcome_index) == index
            }) {
                "satisfied"
            } else {
                "uncovered"
            }
        }
        "OBL-STATE-001" => match obligation.coordinate.state.as_deref() {
            Some(state)
                if run
                    .coverage
                    .executably_reached_states
                    .iter()
                    .any(|s| s == state) =>
            {
                "satisfied"
            }
            _ => "uncovered",
        },
        "OBL-TERM-001" => match obligation.coordinate.state.as_deref() {
            Some(state)
                if run
                    .coverage
                    .reached_terminal_states
                    .iter()
                    .any(|s| s == state) =>
            {
                "satisfied"
            }
            _ => "uncovered",
        },
        "OBL-CMD-001" => {
            let command = obligation.coordinate.command.as_deref();
            let source = obligation.coordinate.source.as_deref();
            if command.is_some()
                && source.is_some()
                && run.coverage.executed_command_sources.iter().any(|row| {
                    Some(row.command.as_str()) == command && Some(row.source.as_str()) == source
                })
            {
                "satisfied"
            } else {
                "uncovered"
            }
        }
        _ => "uncovered",
    }
}

fn authored_status(
    obligation: &VerificationObligationV1,
    run: &Exploration,
    plan: &VerificationPlanV1,
) -> &'static str {
    let Some(goal) = plan
        .authored_goals
        .iter()
        .find(|goal| goal.obligation_id == obligation.obligation_id)
    else {
        return "uncovered";
    };
    let witnessed = match goal.coordinate.kind.as_str() {
        "outcome" => goal
            .generated_ids
            .iter()
            .any(|id| generated_outcome_executed(id, run)),
        "terminal" => product_copy_reached(
            &goal.generated_ids,
            &run.coverage.executably_reached_states,
        ),
        _ => product_copy_reached(
            &goal.generated_ids,
            &run.coverage.executably_reached_states,
        ),
    };
    if witnessed {
        "satisfied"
    } else {
        "uncovered"
    }
}

fn generated_outcome_executed(id: &str, run: &Exploration) -> bool {
    let parts: Vec<&str> = id.split('/').collect();
    if parts.len() != 4 || parts[0] != "outcome" || parts[3] != "executable" {
        return false;
    }
    let Ok(index) = parts[2].parse::<usize>() else {
        return false;
    };
    run.coverage.executed_outcomes.iter().any(|outcome| {
        outcome.command == parts[1] && outcome.outcome_index == index
    })
}

fn product_copy_reached(generated_ids: &[String], labels: &[String]) -> bool {
    labels.iter().any(|label| {
        let product = format!("product/{label}");
        let state = format!("state/{label}/executable");
        let terminal = format!("terminal/{label}/reachable");
        generated_ids
            .iter()
            .any(|id| id == &product || id == &state || id == &terminal)
    })
}
