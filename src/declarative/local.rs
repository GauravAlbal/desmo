//! Change-local semantic identifiability.
//!
//! `MachineDiff` names changed coordinates. This module asks whether current
//! evidence distinguishes the declared guard from nearby comparison operators.

use super::challenge::{apply_guard_op, comparison_op};
use super::compile::compile;
use super::def::Definition;
use super::diff::diff_machines;
use super::explore::{explore, Bounds, CaseCorpus};
use super::expr::CmpOp;
use super::plan::SemanticCoordinate;
use serde::{Deserialize, Serialize};

/// One neighboring meaning for a changed guard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalNeighbor {
    pub operator: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub witness: Option<String>,
}

/// Adequacy of a changed guard against its comparison neighborhood.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalAdequacy {
    pub coordinate: SemanticCoordinate,
    pub declared_operator: String,
    pub neighbors: Vec<LocalNeighbor>,
}

fn neighbor_ops(declared: CmpOp) -> &'static [CmpOp] {
    match declared {
        CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge => {
            &[CmpOp::Lt, CmpOp::Le, CmpOp::Gt, CmpOp::Ge]
        }
        CmpOp::Eq | CmpOp::Ne => &[CmpOp::Eq, CmpOp::Ne],
    }
}

/// For each changed command guard, distinguish declared meaning from neighbors.
pub fn change_local_guards(
    old: &Definition,
    new: &Definition,
    corpus: &CaseCorpus,
    bounds: Bounds,
) -> Result<Vec<LocalAdequacy>, String> {
    let diff = diff_machines(old, new, corpus, bounds)?;
    let mut out = Vec::new();
    for command in &diff.structural.guards_changed {
        let Some(cmd) = new.commands.get(command) else {
            continue;
        };
        let count = cmd.outcomes.as_ref().map(|o| o.len()).unwrap_or(1);
        for index in 0..count {
            let Some(declared) = declared_op(cmd, index) else {
                continue;
            };
            let mut neighbors = Vec::new();
            for op in neighbor_ops(declared) {
                if *op == declared {
                    continue;
                }
                neighbors.push(probe_neighbor(new, command, index, *op, corpus, bounds));
            }
            out.push(LocalAdequacy {
                coordinate: SemanticCoordinate {
                    command: Some(command.clone()),
                    outcome_index: Some(index),
                    ..SemanticCoordinate::default()
                },
                declared_operator: op_src(declared).to_string(),
                neighbors,
            });
        }
    }
    Ok(out)
}

fn declared_op(cmd: &super::def::CommandDef, index: usize) -> Option<CmpOp> {
    if let Some(outcomes) = &cmd.outcomes {
        return comparison_op(outcomes.get(index)?.when.as_deref()?);
    }
    if index == 0 {
        return comparison_op(cmd.when.as_deref()?);
    }
    None
}

fn probe_neighbor(
    declared: &Definition,
    command: &str,
    index: usize,
    op: CmpOp,
    corpus: &CaseCorpus,
    bounds: Bounds,
) -> LocalNeighbor {
    let Some(neighbor_def) = apply_guard_op(declared, command, index, op) else {
        return LocalNeighbor {
            operator: op_src(op).to_string(),
            status: "incomplete".to_string(),
            witness: None,
        };
    };
    let Ok(left) = compile(declared.clone()) else {
        return LocalNeighbor {
            operator: op_src(op).to_string(),
            status: "incomplete".to_string(),
            witness: None,
        };
    };
    let Ok(right) = compile(neighbor_def.clone()) else {
        return LocalNeighbor {
            operator: op_src(op).to_string(),
            status: "incomplete".to_string(),
            witness: None,
        };
    };
    let old_run = explore(&left, corpus, bounds);
    let new_run = explore(&right, corpus, bounds);
    let diff = diff_machines(declared, &neighbor_def, corpus, bounds).ok();
    let killed = old_run.coverage.executed_outcomes != new_run.coverage.executed_outcomes
        || old_run.coverage.reached_terminal_states != new_run.coverage.reached_terminal_states
        || diff
            .as_ref()
            .and_then(|d| d.executable.dimension.as_ref())
            .is_some();
    let witness = diff.and_then(|d| {
        d.executable.witness.map(|steps| {
            steps
                .iter()
                .map(|s| s.command.clone())
                .collect::<Vec<_>>()
                .join(",")
        })
    });
    LocalNeighbor {
        operator: op_src(op).to_string(),
        status: if killed { "killed" } else { "survived" }.to_string(),
        witness,
    }
}

fn op_src(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "==",
        CmpOp::Ne => "!=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
    }
}
