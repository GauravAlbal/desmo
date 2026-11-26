//! Three-valued encoding of [`Expr`] into SMT-LIB2 text.

use super::Unsupported;
use crate::declarative::expr::{Cmp, CmpOp, Expr, Operand};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const I64_MIN_SMT: &str = "(- 9223372036854775808)";
pub const I64_MAX_SMT: &str = "9223372036854775807";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardValue {
    True,
    False,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Int,
    Bool,
    Str,
}

/// Collected string literals as distinct integer ids `0..k-1`.
///
/// A single remainder id (`k`) is exact only in comparisons against those
/// literals. String facts and arguments are not forced into that one id:
/// remainder values `k, k+1, …` stay distinct so `$a == $b` is not always
/// true and `$a != $b` is not a false impossibility.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringPart {
    lits: Vec<String>,
}

impl StringPart {
    pub fn new(lits: impl IntoIterator<Item = String>) -> Self {
        let set: BTreeSet<String> = lits.into_iter().collect();
        Self {
            lits: set.into_iter().collect(),
        }
    }

    pub fn try_id(&self, s: &str) -> Option<i64> {
        self.lits
            .iter()
            .position(|item| item == s)
            .map(|i| i as i64)
    }

    /// First unused id. Not a collapsed OTHER bucket for variables.
    pub fn remainder(&self) -> i64 {
        self.lits.len() as i64
    }

    pub fn decode(&self, id: i64) -> Option<String> {
        let idx = usize::try_from(id).ok()?;
        self.lits.get(idx).cloned()
    }

    /// Concrete string for a model id. Remainder ids become distinct
    /// sentinels so equality among unseen values is preserved under replay.
    pub fn decode_or_remainder(&self, id: i64) -> String {
        self.decode(id)
            .unwrap_or_else(|| format!("\u{10FFFF}#{id}"))
    }

    pub fn all(&self) -> &[String] {
        &self.lits
    }
}

#[derive(Clone, Debug)]
pub struct Names {
    pub facts: BTreeMap<String, Sort>,
    pub args: BTreeMap<String, Sort>,
    pub strings: StringPart,
    pub states: Vec<String>,
}

impl Names {
    pub fn state_id(&self, name: &str) -> Option<i64> {
        self.states
            .iter()
            .position(|item| item == name)
            .map(|i| i as i64)
    }

    pub fn state_name(&self, id: i64) -> Option<&str> {
        let idx = usize::try_from(id).ok()?;
        self.states.get(idx).map(String::as_str)
    }
}

#[derive(Clone, Debug)]
pub struct Term {
    pub sort: Sort,
    pub value: String,
    pub error: String,
}

pub fn fact_val(name: &str) -> String {
    format!("|fv:{name}|")
}

pub fn fact_set(name: &str) -> String {
    format!("|fs:{name}|")
}

pub fn arg_val(name: &str) -> String {
    format!("|av:{name}|")
}

pub fn state_sym() -> &'static str {
    "|st|"
}

pub fn bool_ok(value: &str) -> Term {
    Term {
        sort: Sort::Bool,
        value: value.to_string(),
        error: "false".to_string(),
    }
}

pub fn collect_strings(expr: &Expr) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect_expr_strings(expr, &mut out);
    out
}

fn collect_expr_strings(expr: &Expr, out: &mut BTreeSet<String>) {
    match expr {
        Expr::And(parts) | Expr::Or(parts) => {
            for part in parts {
                collect_expr_strings(part, out);
            }
        }
        Expr::Cmp(cmp) => {
            collect_operand_strings(&cmp.left, out);
            collect_operand_strings(&cmp.right, out);
        }
        Expr::Exists(_) | Expr::NotExists(_) => {}
        Expr::TextIn(_) => {}
    }
}

fn collect_operand_strings(op: &Operand, out: &mut BTreeSet<String>) {
    match op {
        Operand::Literal(v) => {
            if let Some(s) = v.as_str() {
                out.insert(s.to_string());
            }
        }
        Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => {
            collect_operand_strings(base, out);
        }
        Operand::BinArith { left, right, .. } => {
            collect_operand_strings(left, out);
            collect_operand_strings(right, out);
        }
        Operand::Fact(_)
        | Operand::Arg(_)
        | Operand::State
        | Operand::EffectStatus
        | Operand::EffectKind
        | Operand::EffectOutcome => {}
    }
}

pub fn encode_expr(expr: &Expr, names: &Names) -> Result<Term, Unsupported> {
    match expr {
        Expr::And(parts) => fold_connective(parts, names, and2),
        Expr::Or(parts) => fold_connective(parts, names, or2),
        Expr::Cmp(cmp) => encode_cmp(cmp, names),
        Expr::Exists(name) => Ok(exists_term(name, true)),
        Expr::NotExists(name) => Ok(exists_term(name, false)),
        Expr::TextIn(_) => Err(Unsupported::new(
            "text_in is outside the exact SMT fragment",
        )),
    }
}

fn fold_connective(
    parts: &[Expr],
    names: &Names,
    join: fn(Term, Term) -> Term,
) -> Result<Term, Unsupported> {
    let Some((first, rest)) = parts.split_first() else {
        return Ok(bool_ok("true"));
    };
    let mut acc = encode_expr(first, names)?;
    for part in rest {
        acc = join(acc, encode_expr(part, names)?);
    }
    Ok(acc)
}

fn and2(a: Term, b: Term) -> Term {
    let err = or_smt(&a.error, &and_smt(&a.value, &b.error));
    let val = and_smt(&a.value, &b.value);
    Term {
        sort: Sort::Bool,
        value: val,
        error: err,
    }
}

fn or2(a: Term, b: Term) -> Term {
    let not_a = not_smt(&a.value);
    let err = or_smt(&a.error, &and_smt(&not_a, &b.error));
    let val = or_smt(&a.value, &b.value);
    Term {
        sort: Sort::Bool,
        value: val,
        error: err,
    }
}

fn exists_term(name: &str, want_set: bool) -> Term {
    let set = fact_set(name);
    let value = if want_set { set } else { not_smt(&set) };
    bool_ok(&value)
}

fn encode_cmp(cmp: &Cmp, names: &Names) -> Result<Term, Unsupported> {
    let left = encode_operand(&cmp.left, names)?;
    let right = encode_operand(&cmp.right, names)?;
    if left.sort != right.sort {
        return Ok(err_join(&left, &right));
    }
    if left.sort == Sort::Str && !matches!(cmp.op, CmpOp::Eq | CmpOp::Ne) {
        return Err(Unsupported::new(
            "string ordering is outside the exact equality partition",
        ));
    }
    if left.sort == Sort::Bool && !matches!(cmp.op, CmpOp::Eq | CmpOp::Ne) {
        return Ok(err_join(&left, &right));
    }
    let err = or_smt(&left.error, &right.error);
    let val = cmp_smt(cmp.op, &left.value, &right.value);
    Ok(Term {
        sort: Sort::Bool,
        value: val,
        error: err,
    })
}

fn err_join(left: &Term, right: &Term) -> Term {
    Term {
        sort: Sort::Bool,
        value: "false".to_string(),
        error: or_smt(&left.error, &or_smt(&right.error, "true")),
    }
}

fn cmp_smt(op: CmpOp, left: &str, right: &str) -> String {
    match op {
        CmpOp::Eq => format!("(= {left} {right})"),
        CmpOp::Ne => format!("(not (= {left} {right}))"),
        CmpOp::Lt => format!("(< {left} {right})"),
        CmpOp::Le => format!("(<= {left} {right})"),
        CmpOp::Gt => format!("(> {left} {right})"),
        CmpOp::Ge => format!("(>= {left} {right})"),
    }
}

fn encode_operand(op: &Operand, names: &Names) -> Result<Term, Unsupported> {
    match op {
        Operand::Fact(name) => Ok(fact_term(name, names)),
        Operand::Arg(name) => Ok(arg_term(name, names)),
        Operand::State => Ok(state_term(names)),
        Operand::EffectStatus | Operand::EffectKind | Operand::EffectOutcome => Err(
            Unsupported::new("effect operands are outside the exact SMT fragment"),
        ),
        Operand::Literal(v) => literal_term(v, names),
        Operand::Arith { base, add, amount } => encode_arith(base, *add, *amount, names),
        Operand::BinArith { left, add, right } => encode_bin_arith(left, *add, right, names),
        Operand::Mul { base, amount } => encode_mul(base, *amount, names),
        Operand::Div { base, amount } => encode_div(base, *amount, names),
    }
}

fn fact_term(name: &str, names: &Names) -> Term {
    let sort = names.facts.get(name).copied().unwrap_or(Sort::Int);
    Term {
        sort,
        value: fact_val(name),
        error: not_smt(&fact_set(name)),
    }
}

fn arg_term(name: &str, names: &Names) -> Term {
    let sort = names.args.get(name).copied().unwrap_or(Sort::Int);
    Term {
        sort,
        value: arg_val(name),
        error: "false".to_string(),
    }
}

fn state_term(names: &Names) -> Term {
    Term {
        sort: Sort::Str,
        value: state_as_string(names),
        error: "false".to_string(),
    }
}

fn state_as_string(names: &Names) -> String {
    let mut acc = smt_int(names.strings.remainder());
    for (index, name) in names.states.iter().enumerate().rev() {
        let sid = names
            .strings
            .try_id(name)
            .unwrap_or_else(|| names.strings.remainder());
        acc = format!(
            "(ite (= {} {}) {} {acc})",
            state_sym(),
            smt_int(index as i64),
            smt_int(sid)
        );
    }
    acc
}

fn literal_term(v: &Value, names: &Names) -> Result<Term, Unsupported> {
    if let Some(n) = v.as_i64() {
        return Ok(Term {
            sort: Sort::Int,
            value: smt_int(n),
            error: "false".to_string(),
        });
    }
    if let Some(b) = v.as_bool() {
        return Ok(bool_ok(if b { "true" } else { "false" }));
    }
    if let Some(s) = v.as_str() {
        let id = names
            .strings
            .try_id(s)
            .ok_or_else(|| Unsupported::new("string literal is not in the collected partition"))?;
        return Ok(Term {
            sort: Sort::Str,
            value: smt_int(id),
            error: "false".to_string(),
        });
    }
    Err(Unsupported::new("literal is not int, bool, or string"))
}

fn encode_arith(
    base: &Operand,
    add: bool,
    amount: i64,
    names: &Names,
) -> Result<Term, Unsupported> {
    let left = expect_int(encode_operand(base, names)?);
    let right = Term {
        sort: Sort::Int,
        value: smt_int(amount),
        error: "false".to_string(),
    };
    Ok(checked_add(left, add, right))
}

fn encode_bin_arith(
    left: &Operand,
    add: bool,
    right: &Operand,
    names: &Names,
) -> Result<Term, Unsupported> {
    let l = expect_int(encode_operand(left, names)?);
    let r = expect_int(encode_operand(right, names)?);
    Ok(checked_add(l, add, r))
}

fn encode_mul(base: &Operand, amount: i64, names: &Names) -> Result<Term, Unsupported> {
    let left = expect_int(encode_operand(base, names)?);
    let body = format!("(* {} {})", left.value, smt_int(amount));
    let ov = overflow_pred(&body);
    Ok(Term {
        sort: Sort::Int,
        value: body,
        error: or_smt(&left.error, &ov),
    })
}

fn encode_div(base: &Operand, amount: i64, names: &Names) -> Result<Term, Unsupported> {
    if amount <= 0 {
        return Ok(Term {
            sort: Sort::Int,
            value: "0".to_string(),
            error: "true".to_string(),
        });
    }
    let left = expect_int(encode_operand(base, names)?);
    let q = toward_zero(&left.value, amount);
    Ok(Term {
        sort: Sort::Int,
        value: q,
        error: left.error,
    })
}

fn expect_int(term: Term) -> Term {
    if term.sort == Sort::Int {
        term
    } else {
        Term {
            sort: Sort::Int,
            value: "0".to_string(),
            error: or_smt(&term.error, "true"),
        }
    }
}

fn checked_add(left: Term, add: bool, right: Term) -> Term {
    let op = if add { "+" } else { "-" };
    let body = format!("({op} {} {})", left.value, right.value);
    let ov = overflow_pred(&body);
    Term {
        sort: Sort::Int,
        value: body,
        error: or_smt(&or_smt(&left.error, &right.error), &ov),
    }
}

fn overflow_pred(body: &str) -> String {
    format!("(or (< {body} {I64_MIN_SMT}) (> {body} {I64_MAX_SMT}))")
}

fn toward_zero(x: &str, d: i64) -> String {
    format!("(ite (>= {x} 0) (div {x} {d}) (- (div (- {x}) {d})))")
}

pub fn smt_int(n: i64) -> String {
    if n < 0 {
        format!("(- {})", n.unsigned_abs())
    } else {
        n.to_string()
    }
}

fn and_smt(a: &str, b: &str) -> String {
    if a == "true" {
        b.to_string()
    } else if b == "true" {
        a.to_string()
    } else if a == "false" || b == "false" {
        "false".to_string()
    } else {
        format!("(and {a} {b})")
    }
}

fn or_smt(a: &str, b: &str) -> String {
    if a == "false" {
        b.to_string()
    } else if b == "false" {
        a.to_string()
    } else if a == "true" || b == "true" {
        "true".to_string()
    } else {
        format!("(or {a} {b})")
    }
}

fn not_smt(a: &str) -> String {
    if a == "true" {
        "false".to_string()
    } else if a == "false" {
        "true".to_string()
    } else {
        format!("(not {a})")
    }
}

pub fn in_i64_range(expr: &str) -> String {
    format!("(and (>= {expr} {I64_MIN_SMT}) (<= {expr} {I64_MAX_SMT}))")
}

/// Non-negative i64. Literals occupy `0..k-1`; variables may take any
/// remainder id so two non-literal strings stay distinguishable.
pub fn in_string_id_range(expr: &str) -> String {
    format!("(and (>= {expr} 0) (<= {expr} {I64_MAX_SMT}))")
}

pub fn conjunct(parts: &[String]) -> String {
    match parts.len() {
        0 => "true".to_string(),
        1 => parts[0].clone(),
        _ => format!("(and {})", parts.join(" ")),
    }
}

pub fn disjunct(parts: &[String]) -> String {
    match parts.len() {
        0 => "false".to_string(),
        1 => parts[0].clone(),
        n if n <= 16 => format!("(or {})", parts.join(" ")),
        n => {
            let mid = n / 2;
            format!(
                "(or {} {})",
                disjunct(&parts[..mid]),
                disjunct(&parts[mid..])
            )
        }
    }
}
