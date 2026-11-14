//! Advisory structured explanation of a declarative transition.
//!
//! Explanation reuses [`CompiledMachine`]'s outcome selector and reducer. It
//! never writes a store, never advances generation, and is not authorization.

use super::compile::CompiledMachine;
use super::evaluate::{InvariantEvaluation, OutcomeSelection};
use crate::canonical::digest_json;
use crate::diagnostic::Diagnostic;
use crate::effect::EffectIntentV1;
use crate::machine::{DomainEvent, Machine};
use serde::Serialize;
use serde_json::Value;

/// Developer-facing explanation of one command against one state.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Explanation {
    pub schema_version: u64,
    pub machine: ExplanationMachine,
    pub basis: ExplanationBasis,
    pub command: Value,
    pub verdict: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<ExplanationDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<DomainEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_state: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<bool>,
    pub invariants: Vec<InvariantEvaluation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<EffectIntentV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<Diagnostic>,
}

/// Identity of the machine that produced the explanation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExplanationMachine {
    pub machine_id: String,
    pub version: u64,
    pub spec_hash: String,
}

/// State the explanation was evaluated against.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExplanationBasis {
    pub state_digest: String,
    pub state: Value,
}

/// Outcome-selection provenance shared with [`crate::machine::Machine::decide`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExplanationDecision {
    pub source_state: String,
    pub source_legal: bool,
    pub candidates: Vec<super::evaluate::CandidateEvaluation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<usize>,
}

/// Explain what `decide`/`apply` would do. Pure; no IO.
pub fn explain(machine: &CompiledMachine, state: &Value, command: &Value) -> Explanation {
    let identity = machine.identity();
    let mut out = Explanation {
        schema_version: 1,
        machine: ExplanationMachine {
            machine_id: identity.machine_id.clone(),
            version: identity.version.0,
            spec_hash: identity.spec_hash.0.clone(),
        },
        basis: ExplanationBasis {
            state_digest: digest_json(state),
            state: state.clone(),
        },
        command: command.clone(),
        verdict: "refused".to_string(),
        decision: None,
        event: None,
        post_state: None,
        terminal: None,
        invariants: Vec::new(),
        effect: None,
        diagnostic: None,
    };

    let prepared = match machine.prepare(state, command) {
        Ok(prepared) => prepared,
        Err(diagnostic) => {
            out.diagnostic = Some(diagnostic);
            return out;
        }
    };

    if !prepared.source_legal {
        out.decision = Some(ExplanationDecision {
            source_state: prepared.current_state.clone(),
            source_legal: false,
            candidates: Vec::new(),
            selected: None,
        });
        out.diagnostic = Some(Diagnostic::new(
            crate::diagnostic::DiagnosticCode::IkIllegalState,
            Some(prepared.name.clone()),
            format!(
                "command `{}` is not legal from state `{}`",
                prepared.name, prepared.current_state
            ),
        ));
        return out;
    }

    let selection = machine.select_command_outcomes(
        &prepared.name,
        prepared.cmd,
        &prepared.current_state,
        true,
        &prepared.current_facts,
        &prepared.args,
    );
    out.decision = Some(decision_from(&selection));
    if let Some(error) = selection.error {
        out.diagnostic = Some(error);
        return out;
    }
    let Some(index) = selection.selected else {
        out.diagnostic = Some(Diagnostic::new(
            crate::diagnostic::DiagnosticCode::IkInvariantFailed,
            Some(format!("{}.when", prepared.name)),
            format!(
                "no outcome of command `{}` applies in state `{}`",
                prepared.name, prepared.current_state
            ),
        ));
        return out;
    };

    let event = DomainEvent::new(
        prepared.name.clone(),
        2,
        serde_json::json!({
            "source": prepared.current_state,
            "outcome": index,
            "args": prepared.args
        }),
    );
    match machine.reduce_event(state, &event) {
        Ok(reduced) => {
            out.event = Some(event);
            out.invariants = reduced.invariants.results.clone();
            if let Some(error) = reduced.invariants.error {
                out.diagnostic = Some(error);
                return out;
            }
            out.verdict = "would_commit".to_string();
            out.post_state = Some(reduced.state);
            out.terminal = Some(reduced.terminal);
            out.effect = reduced.effect;
            out
        }
        Err(diagnostic) => {
            out.event = Some(event);
            out.diagnostic = Some(diagnostic);
            out
        }
    }
}

fn decision_from(selection: &OutcomeSelection) -> ExplanationDecision {
    ExplanationDecision {
        source_state: selection.source_state.clone(),
        source_legal: selection.source_legal,
        candidates: selection.candidates.clone(),
        selected: selection.selected,
    }
}
