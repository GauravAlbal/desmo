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

/// Discrete outcome-gate classification shared by runtime selection, ordered diagnostics,
/// and Kani bounded proofs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GateStep {
    Unconditional,
    True,
    False,
    Error,
}

fn gate_step(gate: &Gate) -> GateStep {
    match gate {
        Gate::Unconditional => GateStep::Unconditional,
        Gate::True => GateStep::True,
        Gate::False => GateStep::False,
        Gate::Error(_) => GateStep::Error,
    }
}

/// Core ordered-gate decision rule:
/// - `Some(Ok(true))`: match, stop here (`Unconditional` | `True`)
/// - `None`: continue to next arm (`False`)
/// - `Some(Err(()))`: refuse immediately (`Error`) and never fall through
pub(crate) fn decide_gate_step(step: GateStep) -> Option<Result<(), ()>> {
    match step {
        GateStep::Unconditional | GateStep::True => Some(Ok(())),
        GateStep::False => None,
        GateStep::Error => Some(Err(())),
    }
}

/// Evaluate ordered outcomes across a slice of discrete gate results.
#[cfg(any(kani, test))]
pub(crate) fn select_first_matching_gate(gates: &[GateStep]) -> Result<Option<usize>, usize> {
    let mut index = 0;
    while index < gates.len() {
        let step = gates[index];
        if let Some(decision) = decide_gate_step(step) {
            return match decision {
                Ok(()) => Ok(Some(index)),
                Err(()) => Err(index),
            };
        }
        index += 1;
    }
    Ok(None)
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
        let gate = evaluate_gate(spec.when, ctx);
        match decide_gate_step(gate_step(&gate)) {
            Some(Ok(())) => {
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
            None => {
                candidates.push(candidate(index, spec, GuardEvaluation::False, false, None));
            }
            Some(Err(())) => {
                let Gate::Error(reason) = gate else {
                    unreachable!("decide_gate_step returned Err only for Gate::Error");
                };
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
        let gate = evaluate_gate(spec.when, ctx);
        match decide_gate_step(gate_step(&gate)) {
            Some(Ok(())) => return Ok(Some(index)),
            None => continue,
            Some(Err(())) => {
                let Gate::Error(reason) = gate else {
                    unreachable!("decide_gate_step returned Err only for Gate::Error");
                };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_gate_step_never_falls_through_after_error() {
        assert_eq!(
            select_first_matching_gate(&[GateStep::False, GateStep::True, GateStep::Unconditional]),
            Ok(Some(1))
        );
        assert_eq!(
            select_first_matching_gate(&[GateStep::False, GateStep::Error, GateStep::True]),
            Err(1)
        );
        assert_eq!(
            select_first_matching_gate(&[GateStep::False, GateStep::False]),
            Ok(None)
        );
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    fn any_gate_step() -> GateStep {
        let tag: u8 = kani::any();
        kani::assume(tag < 4);
        match tag {
            0 => GateStep::Unconditional,
            1 => GateStep::True,
            2 => GateStep::False,
            _ => GateStep::Error,
        }
    }

    #[kani::proof]
    #[kani::unwind(6)]
    fn prove_ordered_guard_error_never_falls_through() {
        let gates = [
            any_gate_step(),
            any_gate_step(),
            any_gate_step(),
            any_gate_step(),
        ];
        let len_u8: u8 = kani::any();
        kani::assume(len_u8 <= 4);
        let len = len_u8 as usize;
        let slice = &gates[..len];

        match select_first_matching_gate(slice) {
            Ok(Some(winner)) => {
                assert!(winner < len);
                assert!(matches!(
                    slice[winner],
                    GateStep::Unconditional | GateStep::True
                ));
                let mut i = 0;
                while i < winner {
                    assert_eq!(slice[i], GateStep::False);
                    i += 1;
                }
                kani::cover!(
                    winner > 0 && slice[0] == GateStep::False,
                    "fallthrough only past false"
                );
            }
            Err(err_idx) => {
                assert!(err_idx < len);
                assert_eq!(slice[err_idx], GateStep::Error);
                let mut i = 0;
                while i < err_idx {
                    assert_eq!(slice[i], GateStep::False);
                    i += 1;
                }
                kani::cover!(
                    err_idx + 1 < len
                        && matches!(slice[err_idx + 1], GateStep::True | GateStep::Unconditional),
                    "error stops before later matching arm"
                );
            }
            Ok(None) => {
                let mut i = 0;
                while i < len {
                    assert_eq!(slice[i], GateStep::False);
                    i += 1;
                }
                kani::cover!(len > 0, "all false yields none");
            }
        }
    }
}
