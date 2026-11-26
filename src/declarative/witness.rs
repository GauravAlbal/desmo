//! Rule witnesses: examples that show a guard accepts and rejects what its author says,
//! and that tell the guard apart from its small variants.
//!
//! An example is a unit-level input to the guard (arguments, facts, state). It does not
//! show the state is reachable and it does not show the rule matches the business
//! intent; it shows the guard text behaves as the author claims on these inputs.
//!
//! A witness set is adequate when
//! 1. every example's stated outcome matches what the guard evaluates to,
//! 2. every atomic clause is true in some example and false in another, and
//! 3. every one-step variant of the guard (a flipped comparison, a boundary shifted by
//!    one, a dropped clause, a negated existence test) changes the outcome of at least
//!    one example. A variant that no example separates is reported with its description
//!    so the author can add the missing example or declare it equivalent.

use super::expr::{self, CmpOp, EvalCtx, Expr, Operand, TextIn};
use serde_json::Value;
use std::collections::BTreeMap;

/// One example input and the outcome its author states.
pub struct Example {
    pub args: BTreeMap<String, Value>,
    pub facts: BTreeMap<String, Value>,
    pub state: String,
    pub accepts: bool,
}

/// What a witness set does and does not establish about one guard.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Verdict {
    /// Examples whose stated outcome disagrees with the guard, or that cannot be evaluated.
    pub mismatches: Vec<String>,
    /// Clauses that no example toggles (never true, or never false).
    pub uncovered_clauses: Vec<String>,
    /// Variants no example separates from the guard and that were not declared equivalent.
    pub surviving: Vec<String>,
    /// How many variants were tried.
    pub variants_checked: usize,
}

impl Verdict {
    /// True when the witness set is adequate.
    pub fn adequate(&self) -> bool {
        self.mismatches.is_empty() && self.uncovered_clauses.is_empty() && self.surviving.is_empty()
    }
}

/// Check `examples` against one guard. `equivalent` lists variant descriptions the
/// author declared equivalent to the guard.
pub fn check(guard: &str, examples: &[Example], equivalent: &[String]) -> Result<Verdict, String> {
    let expr = expr::parse(guard)?;
    let mut verdict = Verdict {
        mismatches: mismatches(&expr, examples),
        ..Verdict::default()
    };
    if !verdict.mismatches.is_empty() {
        return Ok(verdict);
    }
    verdict.uncovered_clauses = uncovered(&expr, examples);
    let base = outcomes(&expr, examples);
    let tried = variants(&expr);
    verdict.variants_checked = tried.len();
    for (label, variant) in tried {
        if outcomes(&variant, examples) == base && !equivalent.contains(&label) {
            verdict.surviving.push(label);
        }
    }
    Ok(verdict)
}

fn evaluate(expr: &Expr, example: &Example) -> Result<bool, String> {
    let ctx = EvalCtx {
        facts: &example.facts,
        args: &example.args,
        state: &example.state,
        effect: None,
    };
    expr::eval(expr, &ctx)
}

fn outcomes(expr: &Expr, examples: &[Example]) -> Vec<Option<bool>> {
    examples
        .iter()
        .map(|example| evaluate(expr, example).ok())
        .collect()
}

fn mismatches(expr: &Expr, examples: &[Example]) -> Vec<String> {
    let mut out = Vec::new();
    for (index, example) in examples.iter().enumerate() {
        let number = index + 1;
        match evaluate(expr, example) {
            Err(message) => out.push(format!("example {number} cannot be evaluated: {message}")),
            Ok(actual) if actual != example.accepts => out.push(format!(
                "example {number} is stated as {} but the guard {} it",
                if example.accepts { "accepted" } else { "rejected" },
                if actual { "accepts" } else { "rejects" },
            )),
            Ok(_) => {}
        }
    }
    out
}

fn leaves(expr: &Expr) -> Vec<&Expr> {
    match expr {
        Expr::And(parts) | Expr::Or(parts) => parts.iter().flat_map(leaves).collect(),
        leaf => vec![leaf],
    }
}

fn uncovered(expr: &Expr, examples: &[Example]) -> Vec<String> {
    let mut out = Vec::new();
    for leaf in leaves(expr) {
        let seen = outcomes(leaf, examples);
        let text = expr::format_expr(leaf);
        if !seen.contains(&Some(true)) {
            out.push(format!("{text}: no example makes it true"));
        }
        if !seen.contains(&Some(false)) {
            out.push(format!("{text}: no example makes it false"));
        }
    }
    out
}

type Variant = (String, Expr);

/// Every one-step variant of the guard, each with a stable description.
fn variants(expr: &Expr) -> Vec<Variant> {
    match expr {
        Expr::And(parts) => compound(parts, Expr::And),
        Expr::Or(parts) => compound(parts, Expr::Or),
        leaf => {
            let mut out = leaf_variants(leaf);
            out.push((format!("drop {}", expr::format_expr(leaf)), Expr::And(Vec::new())));
            out.push((
                format!("never accept {}", expr::format_expr(leaf)),
                Expr::Or(Vec::new()),
            ));
            out
        }
    }
}

fn compound(parts: &[Expr], build: fn(Vec<Expr>) -> Expr) -> Vec<Variant> {
    let mut out = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let inner = match part {
            Expr::And(_) | Expr::Or(_) => variants(part),
            leaf => leaf_variants(leaf),
        };
        for (label, replacement) in inner {
            let mut next = parts.to_vec();
            next[index] = replacement;
            out.push((label, build(next)));
        }
        let mut rest = parts.to_vec();
        rest.remove(index);
        out.push((format!("drop {}", expr::format_expr(part)), build(rest)));
    }
    out
}

fn leaf_variants(leaf: &Expr) -> Vec<Variant> {
    match leaf {
        Expr::Cmp(cmp) => cmp_variants(leaf, cmp),
        Expr::Exists(name) => vec![relabel(leaf, Expr::NotExists(name.clone()))],
        Expr::NotExists(name) => vec![relabel(leaf, Expr::Exists(name.clone()))],
        Expr::TextIn(text) => text_in_variants(leaf, text),
        Expr::And(_) | Expr::Or(_) => Vec::new(),
    }
}

fn relabel(from: &Expr, to: Expr) -> Variant {
    (
        format!("{} → {}", expr::format_expr(from), expr::format_expr(&to)),
        to,
    )
}

fn neighbours(op: CmpOp) -> &'static [CmpOp] {
    match op {
        CmpOp::Eq => &[CmpOp::Ne],
        CmpOp::Ne => &[CmpOp::Eq],
        CmpOp::Lt => &[CmpOp::Le, CmpOp::Ge],
        CmpOp::Le => &[CmpOp::Lt, CmpOp::Gt],
        CmpOp::Gt => &[CmpOp::Ge, CmpOp::Le],
        CmpOp::Ge => &[CmpOp::Gt, CmpOp::Lt],
    }
}

fn cmp_variants(leaf: &Expr, cmp: &expr::Cmp) -> Vec<Variant> {
    let mut out = Vec::new();
    for op in neighbours(cmp.op) {
        let mut next = cmp.clone();
        next.op = *op;
        out.push(relabel(leaf, Expr::Cmp(next)));
    }
    for operand in nudges(&cmp.left) {
        let mut next = cmp.clone();
        next.left = operand;
        out.push(relabel(leaf, Expr::Cmp(next)));
    }
    for operand in nudges(&cmp.right) {
        let mut next = cmp.clone();
        next.right = operand;
        out.push(relabel(leaf, Expr::Cmp(next)));
    }
    out
}

/// Move an integer constant inside an operand by one in each direction.
fn nudges(operand: &Operand) -> Vec<Operand> {
    match operand {
        Operand::Arith { base, add, amount } => shifted(*amount, 0)
            .into_iter()
            .map(|amount| Operand::Arith {
                base: base.clone(),
                add: *add,
                amount,
            })
            .collect(),
        Operand::Mul { base, amount } => shifted(*amount, 1)
            .into_iter()
            .map(|amount| Operand::Mul {
                base: base.clone(),
                amount,
            })
            .collect(),
        Operand::Div { base, amount } => shifted(*amount, 1)
            .into_iter()
            .map(|amount| Operand::Div {
                base: base.clone(),
                amount,
            })
            .collect(),
        Operand::Literal(value) => match value.as_i64() {
            Some(number) => shifted(number, 0)
                .into_iter()
                .map(|next| Operand::Literal(Value::from(next)))
                .collect(),
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// `amount ± 1`, never below `floor`, never overflowing.
fn shifted(amount: i64, floor: i64) -> Vec<i64> {
    let mut out = Vec::new();
    if let Some(up) = amount.checked_add(1) {
        out.push(up);
    }
    if amount > floor {
        out.push(amount - 1);
    }
    out
}

fn text_in_variants(leaf: &Expr, text: &TextIn) -> Vec<Variant> {
    let mut out = Vec::new();
    let mut push = |min: u32, max: u32| {
        let mut next = text.clone();
        next.min = min;
        next.max = max;
        out.push(relabel(leaf, Expr::TextIn(next)));
    };
    if text.min > 0 {
        push(text.min - 1, text.max);
    }
    if text.min < text.max {
        push(text.min + 1, text.max);
    }
    if text.max > text.min {
        push(text.min, text.max - 1);
    }
    if let Some(up) = text.max.checked_add(1) {
        push(text.min, up);
    }
    out
}
