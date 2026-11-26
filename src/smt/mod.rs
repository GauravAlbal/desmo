//! Solver-agnostic SMT-LIB2 encoding of declarative guards and transitions.
//!
//! The kernel AST and compiled definition are the only machine description.
//! Integer arithmetic is checked `i64` (SMT `Int` plus range and an error
//! predicate). String facts and arguments use integer identity over collected
//! literals plus an open remainder (not a collapsed OTHER). String ordering
//! and `text_in` are outside the fragment.

mod certificate;
mod config;
mod discharge;
mod encode;
mod exhaustive;
mod machine;
mod query;
mod replay;
mod score;
mod solve;

pub use certificate::{
    check_invariant, check_invariant_replay, reject_wrong_invariants, InvariantCheck, QueryCheck,
};
pub use config::{
    adapter_configured, adapter_problems, clear_thread_adapter, configured_solvers,
    set_thread_adapter, ThreadAdapter,
};
pub use discharge::{may_discharge, merge_certificates, reverify_report, trust_name};
pub use encode::{
    collect_strings, encode_expr, GuardValue, Names, Sort, StringPart, Term, I64_MAX_SMT,
    I64_MIN_SMT,
};
pub use exhaustive::{
    format_trace, search as exhaustive_search, suggested_cases, DomainScope, ExhaustiveResult,
};
pub use machine::{lower_definition, Lowered, ThenOp, Transition};
pub use query::{
    dump_query, encode_bmc, encode_guard_eval, encode_horn, encode_invariant_exclude,
    encode_invariant_init, encode_invariant_step, encode_selectability, encode_selectability_any,
    Pinned, QueryKind, SmtQuery, Target,
};
pub use replay::{replay_command, replay_trace, state_object, CommandStep};
pub use score::{run_feasibility_backend, run_smt_cli};
pub use solve::{
    analyze_target, discover_solvers, eval_guard_with_solver, kernel_eval, parse_target, run_smt2,
    ObligationReport, ProofStatus, SolverBin, SolverReply, Trust,
};

use crate::declarative::expr::{CmpOp, Expr, Operand};

/// Why a query cannot be encoded exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    pub reason: String,
}

impl Unsupported {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Return an error if `expr` uses string ordering or another unsupported form.
pub fn fragment_error(expr: &Expr, names: &Names) -> Option<Unsupported> {
    walk_unsupported(expr, names)
}

fn walk_unsupported(expr: &Expr, names: &Names) -> Option<Unsupported> {
    match expr {
        Expr::And(parts) | Expr::Or(parts) => {
            for part in parts {
                if let Some(err) = walk_unsupported(part, names) {
                    return Some(err);
                }
            }
            None
        }
        Expr::Cmp(cmp) => cmp_unsupported(cmp.op, &cmp.left, &cmp.right, names),
        Expr::Exists(_) | Expr::NotExists(_) => None,
        Expr::TextIn(_) => Some(Unsupported::new(
            "text_in is outside the exact SMT fragment",
        )),
    }
}

fn cmp_unsupported(
    op: CmpOp,
    left: &Operand,
    right: &Operand,
    names: &Names,
) -> Option<Unsupported> {
    if matches!(op, CmpOp::Eq | CmpOp::Ne) {
        return None;
    }
    let l = operand_sort(left, names);
    let r = operand_sort(right, names);
    if l == Some(Sort::Str) || r == Some(Sort::Str) {
        return Some(Unsupported::new(
            "string ordering is outside the exact equality partition",
        ));
    }
    None
}

fn operand_sort(op: &Operand, names: &Names) -> Option<Sort> {
    match op {
        Operand::State
        | Operand::EffectStatus
        | Operand::EffectKind
        | Operand::EffectOutcome => Some(Sort::Str),
        Operand::Literal(v) => literal_sort(v),
        Operand::Fact(name) => names.facts.get(name).copied(),
        Operand::Arg(name) => names.args.get(name).copied(),
        Operand::Arith { .. }
        | Operand::BinArith { .. }
        | Operand::Mul { .. }
        | Operand::Div { .. } => Some(Sort::Int),
    }
}

fn literal_sort(v: &serde_json::Value) -> Option<Sort> {
    if v.as_i64().is_some() {
        Some(Sort::Int)
    } else if v.as_bool().is_some() {
        Some(Sort::Bool)
    } else if v.as_str().is_some() {
        Some(Sort::Str)
    } else {
        None
    }
}
