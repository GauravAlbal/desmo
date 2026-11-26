//! Mechanical pretty-print of guard ASTs. Does not invent facts.

use crate::declarative::expr::{self, CmpOp, Expr, Operand};
use serde_json::Value;
use std::collections::BTreeSet;

pub const INLINE_LIMIT: usize = 48;
const DAY_MS: i64 = 86_400_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrettyRule {
    pub pretty: String,
    pub verbatim: String,
    pub inline: bool,
}

pub fn pretty_guard(source: &str, args: &BTreeSet<String>) -> Option<PrettyRule> {
    pretty_mode(source, args, false)
}

pub fn diagram_guard(source: &str, args: &BTreeSet<String>) -> Option<PrettyRule> {
    pretty_mode(source, args, true)
}

fn pretty_mode(source: &str, args: &BTreeSet<String>, diagram: bool) -> Option<PrettyRule> {
    if source.trim().is_empty() {
        return None;
    }
    let verbatim = source.to_string();
    let pretty = match expr::parse(source) {
        Ok(expr) => pretty_expr(&expr, args, diagram),
        Err(_) => source.to_string(),
    };
    let inline = super::display_width(&pretty) <= INLINE_LIMIT;
    Some(PrettyRule {
        pretty,
        verbatim,
        inline,
    })
}

fn pretty_expr(expr: &Expr, args: &BTreeSet<String>, diagram: bool) -> String {
    match expr {
        Expr::Or(parts) => {
            let sep = if diagram { ", or " } else { " or " };
            join(parts, sep, args, diagram)
        }
        Expr::And(parts) => join(parts, " and ", args, diagram),
        Expr::Cmp(cmp) => {
            let ms_ctx = is_ms_operand(&cmp.left) || is_ms_operand(&cmp.right);
            format!(
                "{} {} {}",
                pretty_operand(&cmp.left, args, diagram, ms_ctx),
                cmp_word(cmp.op),
                pretty_operand(&cmp.right, args, diagram, ms_ctx)
            )
        }
        Expr::Exists(name) => format!("{} is present", fact_word(name, args, diagram)),
        Expr::NotExists(name) => format!("{} is absent", fact_word(name, args, diagram)),
        Expr::TextIn(pred) => format!(
            "{} in {} length {}..{}",
            pretty_operand(&pred.value, args, diagram, false),
            pred.set_src,
            pred.min,
            pred.max
        ),
    }
}

fn join(parts: &[Expr], sep: &str, args: &BTreeSet<String>, diagram: bool) -> String {
    parts
        .iter()
        .map(|part| pretty_expr(part, args, diagram))
        .collect::<Vec<_>>()
        .join(sep)
}

fn cmp_word(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "is",
        CmpOp::Ne => "is not",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
    }
}

fn is_ms_operand(op: &Operand) -> bool {
    match op {
        Operand::Fact(name) | Operand::Arg(name) => name.ends_with("_ms"),
        Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => {
            is_ms_operand(base)
        }
        Operand::BinArith { left, right, .. } => is_ms_operand(left) || is_ms_operand(right),
        Operand::State
        | Operand::EffectStatus
        | Operand::EffectKind
        | Operand::EffectOutcome
        | Operand::Literal(_) => false,
    }
}

fn pretty_operand(op: &Operand, args: &BTreeSet<String>, diagram: bool, ms_ctx: bool) -> String {
    match op {
        Operand::Fact(name) => fact_word(name, args, diagram),
        Operand::Arg(name) => {
            if diagram {
                short_name(name)
            } else {
                format!("{name} (given)")
            }
        }
        Operand::State => "state".into(),
        Operand::EffectStatus => "effect_status".into(),
        Operand::EffectKind => "effect_kind".into(),
        Operand::EffectOutcome => "effect_outcome".into(),
        Operand::Literal(value) => pretty_literal(value, ms_ctx),
        Operand::Arith { base, add, amount } => {
            let sign = if *add { "+" } else { "-" };
            let use_ms = ms_ctx || is_ms_operand(base);
            let amt = if use_ms { pretty_ms(*amount) } else { amount.to_string() };
            format!("{} {sign} {amt}", pretty_operand(base, args, diagram, use_ms))
        }
        Operand::BinArith { left, add, right } => {
            let sign = if *add { "+" } else { "-" };
            let use_ms = ms_ctx || is_ms_operand(left) || is_ms_operand(right);
            format!(
                "{} {sign} {}",
                pretty_operand(left, args, diagram, use_ms),
                pretty_operand(right, args, diagram, use_ms)
            )
        }
        Operand::Mul { base, amount } => {
            format!("{} * {}", pretty_operand(base, args, diagram, ms_ctx), amount)
        }
        Operand::Div { base, amount } => {
            format!("{} / {}", pretty_operand(base, args, diagram, ms_ctx), amount)
        }
    }
}

fn fact_word(name: &str, args: &BTreeSet<String>, diagram: bool) -> String {
    if diagram {
        return short_name(name);
    }
    if args.contains(name) {
        format!("{name} (given)")
    } else {
        format!("{name} (stored)")
    }
}

fn short_name(name: &str) -> String {
    let trimmed = name.strip_suffix("_ms").unwrap_or(name);
    trimmed.strip_prefix("region_").unwrap_or(trimmed).to_string()
}

fn pretty_literal(value: &Value, ms_ctx: bool) -> String {
    if let Some(n) = value.as_i64() {
        if ms_ctx {
            pretty_ms(n)
        } else {
            n.to_string()
        }
    } else if let Some(s) = value.as_str() {
        s.to_string()
    } else if let Some(b) = value.as_bool() {
        b.to_string()
    } else {
        value.to_string()
    }
}

fn pretty_ms(n: i64) -> String {
    if n != 0 && n % DAY_MS == 0 {
        format!("{}d", n / DAY_MS)
    } else {
        n.to_string()
    }
}
