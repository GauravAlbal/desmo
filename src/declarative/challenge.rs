//! Bounded semantic-challenge compiler.
//!
//! Challenges are plausible neighboring meanings derived from declared
//! comparison guards. They do not change transition legality.

use super::def::{CommandDef, Definition};
use super::expr::{self, CmpOp, Expr, Operand};
use super::plan::{SemanticChallengeV1, SemanticCoordinate};
use serde_json::Value;

/// Derive a deterministic, type-valid challenge set from a validated definition.
pub fn semantic_challenges(def: &Definition, _spec_hash: &str) -> Vec<SemanticChallengeV1> {
    let mut out = Vec::new();
    for (command, cmd) in &def.commands {
        emit_command_challenges(command, cmd, &mut out);
    }
    for (name, inv) in &def.invariants {
        if let Some(when) = &inv.when {
            let obligation_id = format!("invariant/{name}/evaluated");
            emit_guard_challenges(
                GuardChallenge {
                    scope: "invariant",
                    name,
                    index: None,
                    source: when,
                    obligation_id: &obligation_id,
                    coordinate: SemanticCoordinate {
                        invariant: Some(name.clone()),
                        ..SemanticCoordinate::default()
                    },
                },
                &mut out,
            );
        }
        let obligation_id = format!("invariant/{name}/evaluated");
        emit_guard_challenges(
            GuardChallenge {
                scope: "invariant",
                name,
                index: None,
                source: &inv.assert,
                obligation_id: &obligation_id,
                coordinate: SemanticCoordinate {
                    invariant: Some(name.clone()),
                    ..SemanticCoordinate::default()
                },
            },
            &mut out,
        );
    }
    out.sort_by(|a, b| a.challenge_id.cmp(&b.challenge_id));
    out
}

/// Apply the adjacent comparison operator to one outcome/command guard.
pub fn apply_guard_boundary(def: &Definition, command: &str, index: usize) -> Option<Definition> {
    let mut edited = def.clone();
    let cmd = edited.commands.get_mut(command)?;
    let source = if let Some(outcomes) = cmd.outcomes.as_mut() {
        outcomes.get_mut(index)?.when.as_mut()?
    } else if index == 0 {
        cmd.when.as_mut()?
    } else {
        return None;
    };
    let Expr::Cmp(cmp) = expr::parse(source).ok()? else {
        return None;
    };
    let neighbor = adjacent_op(cmp.op)?;
    *source = render_cmp(&cmp.left, neighbor, &cmp.right);
    Some(edited)
}

/// Replace one outcome/command comparison operator.
pub fn apply_guard_op(
    def: &Definition,
    command: &str,
    index: usize,
    op: crate::declarative::expr::CmpOp,
) -> Option<Definition> {
    let mut edited = def.clone();
    let cmd = edited.commands.get_mut(command)?;
    let source = if let Some(outcomes) = cmd.outcomes.as_mut() {
        outcomes.get_mut(index)?.when.as_mut()?
    } else if index == 0 {
        cmd.when.as_mut()?
    } else {
        return None;
    };
    let Expr::Cmp(cmp) = expr::parse(source).ok()? else {
        return None;
    };
    *source = render_cmp(&cmp.left, op, &cmp.right);
    Some(edited)
}

pub(crate) fn comparison_op(source: &str) -> Option<crate::declarative::expr::CmpOp> {
    match expr::parse(source).ok()? {
        Expr::Cmp(cmp) => Some(cmp.op),
        _ => None,
    }
}

struct GuardChallenge<'a> {
    scope: &'a str,
    name: &'a str,
    index: Option<usize>,
    source: &'a str,
    obligation_id: &'a str,
    coordinate: SemanticCoordinate,
}

fn emit_command_challenges(command: &str, cmd: &CommandDef, out: &mut Vec<SemanticChallengeV1>) {
    if let Some(when) = &cmd.when {
        let obligation_id = outcome_obligation_id(command, 0);
        emit_guard_challenges(
            GuardChallenge {
                scope: command,
                name: command,
                index: Some(0),
                source: when,
                obligation_id: &obligation_id,
                coordinate: SemanticCoordinate {
                    command: Some(command.to_string()),
                    outcome_index: Some(0),
                    ..SemanticCoordinate::default()
                },
            },
            out,
        );
    }
    if let Some(outcomes) = &cmd.outcomes {
        for (index, outcome) in outcomes.iter().enumerate() {
            let Some(when) = &outcome.when else { continue };
            let obligation_id = outcome_obligation_id(command, index);
            emit_guard_challenges(
                GuardChallenge {
                    scope: command,
                    name: command,
                    index: Some(index),
                    source: when,
                    obligation_id: &obligation_id,
                    coordinate: SemanticCoordinate {
                        command: Some(command.to_string()),
                        outcome_index: Some(index),
                        ..SemanticCoordinate::default()
                    },
                },
                out,
            );
        }
    }
}

fn emit_guard_challenges(input: GuardChallenge<'_>, out: &mut Vec<SemanticChallengeV1>) {
    let Ok(expr) = expr::parse(input.source) else {
        return;
    };
    let Expr::Cmp(cmp) = expr else { return };
    let Some(neighbor) = adjacent_op(cmp.op) else {
        return;
    };
    let original = render_cmp(&cmp.left, cmp.op, &cmp.right);
    let mutated = render_cmp(&cmp.left, neighbor, &cmp.right);
    let tag = format!("{}_{}", op_tag(cmp.op), op_tag(neighbor));
    let loc = match input.index {
        Some(i) => format!("{}/{}", input.scope, i),
        None => input.scope.to_string(),
    };
    out.push(SemanticChallengeV1 {
        challenge_id: format!("challenge/guard-boundary/{loc}/{tag}"),
        kind: "CHL-GUARD-BOUNDARY".to_string(),
        source_obligation_ids: vec![input.obligation_id.to_string()],
        coordinate: input.coordinate,
        transformation: format!("{original} -> {mutated}"),
        local_semantic_digest: crate::canonical::digest_json(&format!("{original} -> {mutated}")),
        expected_relation: "distinguish".to_string(),
        mandatory: input.name == "reject" && input.index == Some(0) && cmp.op == CmpOp::Lt,
    });
}

fn adjacent_op(op: CmpOp) -> Option<CmpOp> {
    match op {
        CmpOp::Lt => Some(CmpOp::Le),
        CmpOp::Le => Some(CmpOp::Lt),
        CmpOp::Gt => Some(CmpOp::Ge),
        CmpOp::Ge => Some(CmpOp::Gt),
        CmpOp::Eq => Some(CmpOp::Ne),
        CmpOp::Ne => Some(CmpOp::Eq),
    }
}

fn op_tag(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "eq",
        CmpOp::Ne => "ne",
        CmpOp::Lt => "lt",
        CmpOp::Le => "le",
        CmpOp::Gt => "gt",
        CmpOp::Ge => "ge",
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

fn render_cmp(left: &Operand, op: CmpOp, right: &Operand) -> String {
    format!(
        "{} {} {}",
        render_operand(left),
        op_src(op),
        render_operand(right)
    )
}

fn render_operand(op: &Operand) -> String {
    match op {
        Operand::Fact(name) => name.clone(),
        Operand::Arg(name) => format!("${name}"),
        Operand::State => "state".to_string(),
        Operand::EffectStatus => "effect_status".to_string(),
        Operand::EffectKind => "effect_kind".to_string(),
        Operand::EffectOutcome => "effect_outcome".to_string(),
        Operand::Literal(value) => render_literal(value),
        Operand::Arith { base, add, amount } => {
            let sign = if *add { "+" } else { "-" };
            format!("{} {} {amount}", render_operand(base), sign)
        }
        Operand::BinArith { left, add, right } => {
            let sign = if *add { "+" } else { "-" };
            format!(
                "{} {} {}",
                render_operand(left),
                sign,
                render_operand(right)
            )
        }
        Operand::Mul { base, amount } => {
            format!("{} * {amount}", render_operand(base))
        }
        Operand::Div { base, amount } => {
            format!("{} / {amount}", render_operand(base))
        }
    }
}

fn render_literal(value: &Value) -> String {
    match value {
        Value::String(s) => format!("'{s}'"),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn outcome_obligation_id(command: &str, index: usize) -> String {
    format!("outcome/{command}/{index}/executable")
}
