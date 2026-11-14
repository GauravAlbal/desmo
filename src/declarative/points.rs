//! Decision points of a guard: the places where someone chose a value or a relation.
//!
//! A point is a comparison operator, a constant, or the character set and length bounds of a
//! `text_in`. Existence tests and the `and`/`or` connectives are not enumerated, and neither is
//! anything outside a declarative guard (reducer code, effects), a rule with several guards, or a
//! rule that is absent. The list is a closed-world count over what is enumerated, never a claim
//! about what exists.

use super::expr::{self, CmpOp, Expr, Operand};
use serde_json::Value;

/// One decision point in a guard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Point {
    /// Stable within one guard text: `kind:label`, with `#n` appended on a repeat.
    pub key: String,
    /// `operator`, `constant`, `charset`, `min_length` or `max_length`.
    pub kind: &'static str,
    /// What is shown to a human.
    pub label: String,
    /// The literal a requirement span must contain to be about this point. None for operators.
    pub literal: Option<String>,
    /// The comparison operator, for operator points.
    pub operator: Option<CmpOp>,
}

/// Enumerate the decision points of a guard, in order of appearance.
pub fn decision_points(guard: &str) -> Result<Vec<Point>, String> {
    let parsed = expr::parse(guard)?;
    let mut raw = Vec::new();
    collect(&parsed, &mut raw);
    Ok(keyed(raw))
}

fn collect(node: &Expr, out: &mut Vec<Point>) {
    match node {
        Expr::And(parts) | Expr::Or(parts) => parts.iter().for_each(|part| collect(part, out)),
        Expr::Cmp(cmp) => {
            out.push(operator_point(cmp.op));
            operand_constants(&cmp.left, out);
            operand_constants(&cmp.right, out);
        }
        Expr::TextIn(text) => {
            operand_constants(&text.value, out);
            out.push(point("charset", text.set_src.clone(), Some(text.set_src.clone()), None));
            out.push(point("min_length", text.min.to_string(), Some(text.min.to_string()), None));
            out.push(point("max_length", text.max.to_string(), Some(text.max.to_string()), None));
        }
        Expr::Exists(_) | Expr::NotExists(_) => {}
    }
}

fn point(kind: &'static str, label: String, literal: Option<String>, operator: Option<CmpOp>) -> Point {
    Point { key: String::new(), kind, label, literal, operator }
}

fn operator_point(op: CmpOp) -> Point {
    point("operator", symbol(op).to_string(), None, Some(op))
}

fn constant(label: String) -> Point {
    point("constant", label.clone(), Some(label), None)
}

fn operand_constants(operand: &Operand, out: &mut Vec<Point>) {
    match operand {
        Operand::Literal(value) => out.push(constant(literal_text(value))),
        Operand::Arith { base, amount, .. } | Operand::Mul { base, amount } | Operand::Div { base, amount } => {
            operand_constants(base, out);
            out.push(constant(amount.to_string()));
        }
        Operand::BinArith { left, right, .. } => {
            operand_constants(left, out);
            operand_constants(right, out);
        }
        Operand::Fact(_) | Operand::Arg(_) | Operand::State => {}
    }
}

fn literal_text(value: &Value) -> String {
    value.as_str().map_or_else(|| value.to_string(), str::to_string)
}

fn keyed(points: Vec<Point>) -> Vec<Point> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for mut item in points {
        let base = format!("{}:{}", item.kind, item.label);
        let repeats = seen.iter().filter(|prior| **prior == base).count();
        item.key = if repeats == 0 { base.clone() } else { format!("{base}#{}", repeats + 1) };
        seen.push(base);
        out.push(item);
    }
    out
}

/// The source symbol of a comparison operator.
pub fn symbol(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "==",
        CmpOp::Ne => "!=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
    }
}

const OPERATORS: [CmpOp; 6] = [CmpOp::Eq, CmpOp::Ne, CmpOp::Lt, CmpOp::Le, CmpOp::Gt, CmpOp::Ge];

/// Phrases a requirement span may use for each operator.
pub fn vocabulary(op: CmpOp) -> &'static [&'static str] {
    match op {
        CmpOp::Eq => &["exactly", "equal", "same as", "must be", "matches"],
        CmpOp::Ne => &["not equal", "different", "other than", "differ"],
        CmpOp::Lt => &["earlier than", "before", "less than", "fewer than", "below", "under"],
        CmpOp::Le => &["at most", "no later than", "not later", "no more than", "up to", "on or before", "or earlier"],
        CmpOp::Gt => &["later than", "later", "after", "greater than", "more than", "exceed", "above", "larger"],
        CmpOp::Ge => &["at least", "no earlier than", "not earlier", "on or after", "or later", "minimum"],
    }
}

/// The operators a span names. Matching is by whole words and longest phrase first, and a
/// matched phrase is consumed, so `not equal` names `!=` and not also `==`, and `no later than`
/// names `<=` and not also `>`. This is a necessary condition for a `stated` operator, not a
/// sufficient one: naming an operator does not show the requirement means it.
pub fn operators_named(span: &str) -> Vec<CmpOp> {
    let mut phrases: Vec<(&'static str, CmpOp)> = OPERATORS
        .iter()
        .flat_map(|op| vocabulary(*op).iter().map(move |phrase| (*phrase, *op)))
        .collect();
    phrases.sort_by_key(|(phrase, _)| std::cmp::Reverse(phrase.len()));
    let mut text = span.to_lowercase();
    let mut named = Vec::new();
    for (phrase, op) in phrases {
        while let Some(start) = find_word(&text, phrase) {
            if !named.contains(&op) {
                named.push(op);
            }
            text.replace_range(start..start + phrase.len(), &" ".repeat(phrase.len()));
        }
    }
    named
}

/// The first whole-word occurrence of `phrase` in `text`.
fn find_word(text: &str, phrase: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(offset) = text[from..].find(phrase) {
        let start = from + offset;
        let end = start + phrase.len();
        let before_ok = text[..start].chars().next_back().map_or(true, |c| !c.is_alphanumeric());
        let after_ok = text[end..].chars().next().map_or(true, |c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return Some(start);
        }
        from = start + phrase.chars().next().map_or(1, char::len_utf8);
    }
    None
}
