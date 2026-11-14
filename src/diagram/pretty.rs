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
        Expr::Cmp(cmp) => format!(
            "{} {} {}",
            pretty_operand(&cmp.left, args, diagram),
            cmp_word(cmp.op),
            pretty_operand(&cmp.right, args, diagram)
        ),
        Expr::Exists(name) => format!("{} is present", fact_word(name, args, diagram)),
        Expr::NotExists(name) => format!("{} is absent", fact_word(name, args, diagram)),
        Expr::TextIn(pred) => format!(
            "{} in {} length {}..{}",
            pretty_operand(&pred.value, args, diagram),
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

fn pretty_operand(op: &Operand, args: &BTreeSet<String>, diagram: bool) -> String {
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
        Operand::Literal(value) => pretty_literal(value),
        Operand::Arith { base, add, amount } => {
            let sign = if *add { "+" } else { "-" };
            format!(
                "{} {sign} {}",
                pretty_operand(base, args, diagram),
                pretty_ms(*amount)
            )
        }
        Operand::BinArith { left, add, right } => {
            let sign = if *add { "+" } else { "-" };
            format!(
                "{} {sign} {}",
                pretty_operand(left, args, diagram),
                pretty_operand(right, args, diagram)
            )
        }
        Operand::Mul { base, amount } => {
            format!("{} * {}", pretty_operand(base, args, diagram), pretty_ms(*amount))
        }
        Operand::Div { base, amount } => {
            format!("{} / {}", pretty_operand(base, args, diagram), pretty_ms(*amount))
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

fn pretty_literal(value: &Value) -> String {
    if let Some(n) = value.as_i64() {
        pretty_ms(n)
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
