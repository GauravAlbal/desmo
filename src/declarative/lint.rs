//! Zero-corpus static structural lints over declarative machines.
//!
//! These checks require no case corpus and never claim exhaustive proof.
//! They surface immediate authoring defects: unreachable states, reachable
//! non-terminal dead-end sinks, shadowed ordered outcomes, and initial-state
//! invariant violations.

use super::analyze::{analyze, machine_graph};
use super::compile::CompiledMachine;
use super::def::Definition;
use crate::machine::Machine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// One static structural finding derived directly from the machine definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralLint {
    pub kind: String,
    pub subject: String,
    pub message: String,
}

/// Collect deterministic, corpus-independent structural lints for a machine.
pub fn structural_lints(def: &Definition, machine: &CompiledMachine) -> Vec<StructuralLint> {
    let mut lints = Vec::new();
    let graph = machine_graph(def);
    let analysis = analyze(&graph);

    for state in &analysis.unreachable_states {
        lints.push(StructuralLint {
            kind: "unreachable_state".to_string(),
            subject: format!("state:{state}"),
            message: format!(
                "state `{state}` is structurally unreachable from initial state `{}`",
                analysis.initial
            ),
        });
    }

    let states_with_outgoing: BTreeSet<&str> =
        graph.edges.iter().map(|e| e.source.as_str()).collect();
    let unreachable_set: BTreeSet<&str> = analysis
        .unreachable_states
        .iter()
        .map(String::as_str)
        .collect();
    for state in &graph.states {
        if !state.terminal
            && !unreachable_set.contains(state.name.as_str())
            && !states_with_outgoing.contains(state.name.as_str())
        {
            lints.push(StructuralLint {
                kind: "non_terminal_sink".to_string(),
                subject: format!("state:{}", state.name),
                message: format!(
                    "non-terminal state `{}` has no outgoing commands (dead-end sink; mark `terminal = true` or add an outgoing command)",
                    state.name
                ),
            });
        }
    }

    for (cmd_name, cmd) in &def.commands {
        let Some(outcomes) = &cmd.outcomes else {
            continue;
        };
        let mut first_unconditional: Option<usize> = None;
        let mut seen_guards: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for (idx, outcome) in outcomes.iter().enumerate() {
            if let Some(prev) = first_unconditional {
                lints.push(StructuralLint {
                    kind: "shadowed_outcome".to_string(),
                    subject: format!("command:{cmd_name}/outcome:{idx}"),
                    message: format!(
                        "outcome #{idx} (`to = \"{}\"`) of command `{cmd_name}` is shadowed by earlier unconditional outcome #{prev}",
                        outcome.to
                    ),
                });
            } else if let Some(when) = &outcome.when {
                let normalized: String = when.split_whitespace().collect::<Vec<_>>().join(" ");
                if let Some(&prev) = seen_guards.get(&normalized) {
                    lints.push(StructuralLint {
                        kind: "shadowed_outcome".to_string(),
                        subject: format!("command:{cmd_name}/outcome:{idx}"),
                        message: format!(
                            "outcome #{idx} (`to = \"{}\"`) of command `{cmd_name}` duplicates earlier guard `{when}` from outcome #{prev} and can never be selected",
                            outcome.to
                        ),
                    });
                } else {
                    seen_guards.insert(normalized, idx);
                }
            } else {
                first_unconditional = Some(idx);
            }
        }
    }

    if let Err(diag) = machine.validate_initial() {
        let rule = diag
            .rule
            .clone()
            .unwrap_or_else(|| "invariants".to_string());
        lints.push(StructuralLint {
            kind: "genesis_invariant_violation".to_string(),
            subject: format!("invariant:{rule}"),
            message: format!(
                "initial state `{}` violates declared invariant `{rule}`: {}",
                analysis.initial, diag.message
            ),
        });
    }

    lints
}

/// Render structural lints as a human-readable section (empty string if none).
pub fn render_lints(lints: &[StructuralLint]) -> String {
    if lints.is_empty() {
        return String::new();
    }
    let mut out = String::from("\nSTRUCTURAL LINTS (zero-corpus static checks)\n");
    for lint in lints {
        out.push_str(&format!(
            "  - [{}] {}: {}\n",
            lint.kind, lint.subject, lint.message
        ));
    }
    out
}
