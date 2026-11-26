//! Shared outcome-selection and invariant evaluation for declarative machines.
//!
//! [`Machine::decide`], [`Machine::apply`], and developer explanation consume
//! these results. There is no second selector and no second invariant checker.

use super::expr::{self, EvalCtx, Expr};
use crate::diagnostic::{Diagnostic, DiagnosticCode};
use serde::{Deserialize, Serialize};

/// Result of evaluating one guard or assertion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardEvaluation {
    True,
    False,
    Error,
    Unconditional,
    NotConsidered,
}

/// One ordered outcome candidate after (or without) guard evaluation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateEvaluation {
    pub outcome_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    pub evaluation: GuardEvaluation,
    pub selected: bool,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// First-match-wins outcome selection plus per-candidate provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OutcomeSelection {
    pub source_state: String,
    pub source_legal: bool,
    pub candidates: Vec<CandidateEvaluation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Diagnostic>,
}

/// One global invariant after gating and assertion evaluation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantEvaluation {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    pub gate: GuardEvaluation,
    pub assertion: GuardEvaluation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Invariant pass/fail plus structured per-invariant results.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InvariantCheck {
    pub results: Vec<InvariantEvaluation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Diagnostic>,
}

/// Guard text and compiled expression for one outcome.
pub struct GuardSpec<'a> {
    pub when: Option<&'a Expr>,
    pub when_src: Option<&'a str>,
    pub when_rule: &'a str,
    pub target: &'a str,
}

/// Compiled invariant plus the original source text used in explanations.
pub struct InvariantSpec<'a> {
    pub name: &'a str,
    pub when: Option<&'a Expr>,
    pub when_src: Option<&'a str>,
    pub when_rule: &'a str,
    pub assert: &'a Expr,
    pub assert_rule: &'a str,
}

/// Evaluate ordered outcomes. A guard evaluation error refuses; it does not
/// become `false` and it does not fall through to a later candidate.
pub fn select_outcomes(
    current_state: &str,
    source_legal: bool,
    specs: &[GuardSpec<'_>],
    ctx: &EvalCtx<'_>,
) -> OutcomeSelection {
    let mut candidates = Vec::with_capacity(specs.len());
    let mut selected = None;
    for (index, spec) in specs.iter().enumerate() {
        if selected.is_some() {
            candidates.push(not_considered(index, spec));
            continue;
        }
        match evaluate_gate(spec.when, ctx) {
            Gate::Unconditional | Gate::True => {
                selected = Some(index);
                candidates.push(candidate(
                    index,
                    spec,
                    match spec.when {
                        None => GuardEvaluation::Unconditional,
                        Some(_) => GuardEvaluation::True,
                    },
                    true,
                    None,
                ));
            }
            Gate::False => {
                candidates.push(candidate(index, spec, GuardEvaluation::False, false, None));
            }
            Gate::Error(reason) => {
                candidates.push(candidate(
                    index,
                    spec,
                    GuardEvaluation::Error,
                    false,
                    Some(reason.clone()),
                ));
                return OutcomeSelection {
                    source_state: current_state.to_string(),
                    source_legal,
                    candidates,
                    selected: None,
                    error: Some(Diagnostic::new(
                        DiagnosticCode::IkInvariantFailed,
                        Some(spec.when_rule.to_string()),
                        format!(
                            "guard `{}` could not be evaluated: {reason}",
                            spec.when_rule
                        ),
                    )),
                };
            }
        }
    }
    OutcomeSelection {
        source_state: current_state.to_string(),
        source_legal,
        candidates,
        selected,
        error: None,
    }
}

/// Evaluate global invariants. A non-evaluable `when` refuses rather than
/// skipping. An assertion that cannot be evaluated is distinct from false.
pub fn evaluate_invariants(specs: &[InvariantSpec<'_>], ctx: &EvalCtx<'_>) -> InvariantCheck {
    let mut results = Vec::with_capacity(specs.len());
    for spec in specs {
        match evaluate_gate(spec.when, ctx) {
            Gate::False => {
                results.push(InvariantEvaluation {
                    name: spec.name.to_string(),
                    when: spec.when_src.map(str::to_string),
                    gate: GuardEvaluation::False,
                    assertion: GuardEvaluation::NotConsidered,
                    error: None,
                });
                continue;
            }
            Gate::Error(reason) => {
                results.push(InvariantEvaluation {
                    name: spec.name.to_string(),
                    when: spec.when_src.map(str::to_string),
                    gate: GuardEvaluation::Error,
                    assertion: GuardEvaluation::NotConsidered,
                    error: Some(reason.clone()),
                });
                return InvariantCheck {
                    results,
                    error: Some(Diagnostic::new(
                        DiagnosticCode::IkInvariantFailed,
                        Some(spec.when_rule.to_string()),
                        format!(
                            "invariant `{}` when could not be evaluated: {reason}",
                            spec.name
                        ),
                    )),
                };
            }
            Gate::Unconditional | Gate::True => {}
        }
        let gate = if spec.when.is_none() {
            GuardEvaluation::Unconditional
        } else {
            GuardEvaluation::True
        };
        match expr::eval(spec.assert, ctx) {
            Ok(true) => results.push(InvariantEvaluation {
                name: spec.name.to_string(),
                when: spec.when_src.map(str::to_string),
                gate,
                assertion: GuardEvaluation::True,
                error: None,
            }),
            Ok(false) => {
                results.push(InvariantEvaluation {
                    name: spec.name.to_string(),
                    when: spec.when_src.map(str::to_string),
                    gate,
                    assertion: GuardEvaluation::False,
                    error: None,
                });
                return InvariantCheck {
                    results,
                    error: Some(Diagnostic::new(
                        DiagnosticCode::IkInvariantFailed,
                        Some(spec.name.to_string()),
                        format!("global invariant `{}` violated", spec.name),
                    )),
                };
            }
            Err(reason) => {
                results.push(InvariantEvaluation {
                    name: spec.name.to_string(),
                    when: spec.when_src.map(str::to_string),
                    gate,
                    assertion: GuardEvaluation::Error,
                    error: Some(reason.clone()),
                });
                return InvariantCheck {
                    results,
                    error: Some(Diagnostic::new(
                        DiagnosticCode::IkInvariantFailed,
                        Some(spec.assert_rule.to_string()),
                        format!(
                            "invariant `{}` assert could not be evaluated: {reason}",
                            spec.name
                        ),
                    )),
                };
            }
        }
    }
    InvariantCheck {
        results,
        error: None,
    }
}

/// Fast-path outcome selection for runtime `decide()`: avoids allocating explanation vectors.
pub fn select_outcome_fast(
    specs: &[GuardSpec<'_>],
    ctx: &EvalCtx<'_>,
) -> Result<Option<usize>, Diagnostic> {
    for (index, spec) in specs.iter().enumerate() {
        match evaluate_gate(spec.when, ctx) {
            Gate::Unconditional | Gate::True => return Ok(Some(index)),
            Gate::False => continue,
            Gate::Error(reason) => {
                return Err(Diagnostic::new(
                    DiagnosticCode::IkInvariantFailed,
                    Some(spec.when_rule.to_string()),
                    format!(
                        "guard `{}` could not be evaluated: {reason}",
                        spec.when_rule
                    ),
                ));
            }
        }
    }
    Ok(None)
}

/// Fast-path global invariant check for runtime `apply()`: zero allocation on success.
pub fn check_invariants_fast(
    specs: &[InvariantSpec<'_>],
    ctx: &EvalCtx<'_>,
) -> Result<(), Diagnostic> {
    for spec in specs {
        match evaluate_gate(spec.when, ctx) {
            Gate::False => continue,
            Gate::Error(reason) => {
                return Err(Diagnostic::new(
                    DiagnosticCode::IkInvariantFailed,
                    Some(spec.when_rule.to_string()),
                    format!(
                        "invariant `{}` when could not be evaluated: {reason}",
                        spec.name
                    ),
                ));
            }
            Gate::Unconditional | Gate::True => {}
        }
        match expr::eval(spec.assert, ctx) {
            Ok(true) => continue,
            Ok(false) => {
                return Err(Diagnostic::new(
                    DiagnosticCode::IkInvariantFailed,
                    Some(spec.name.to_string()),
                    format!("global invariant `{}` violated", spec.name),
                ));
            }
            Err(reason) => {
                return Err(Diagnostic::new(
                    DiagnosticCode::IkInvariantFailed,
                    Some(spec.assert_rule.to_string()),
                    format!(
                        "invariant `{}` assert could not be evaluated: {reason}",
                        spec.name
                    ),
                ));
            }
        }
    }
    Ok(())
}

enum Gate {
    Unconditional,
    True,
    False,
    Error(String),
}

fn evaluate_gate(when: Option<&Expr>, ctx: &EvalCtx<'_>) -> Gate {
    match when {
        None => Gate::Unconditional,
        Some(expr) => match expr::eval(expr, ctx) {
            Ok(true) => Gate::True,
            Ok(false) => Gate::False,
            Err(reason) => Gate::Error(reason),
        },
    }
}

fn candidate(
    index: usize,
    spec: &GuardSpec<'_>,
    evaluation: GuardEvaluation,
    selected: bool,
    error: Option<String>,
) -> CandidateEvaluation {
    CandidateEvaluation {
        outcome_index: index,
        guard: spec.when_src.map(str::to_string),
        evaluation,
        selected,
        target: spec.target.to_string(),
        error,
    }
}

fn not_considered(index: usize, spec: &GuardSpec<'_>) -> CandidateEvaluation {
    candidate(index, spec, GuardEvaluation::NotConsidered, false, None)
}
