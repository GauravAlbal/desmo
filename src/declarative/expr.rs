//! Bounded guard/predicate grammar for declarative machines.
//!
//! Grammar:
//! ```text
//! expr        := or
//! or          := and ( '||' and )*
//! and         := atom ( '&&' atom )*
//! atom        := '(' expr ')' | cmp | 'exists' '(' IDENT ')' | 'not_exists' '(' IDENT ')' | text_in
//! text_in     := 'text_in' '(' value ',' STRING ',' INT ',' INT ')'
//! cmp         := operand ( '==' | '!=' | '<' | '<=' | '>' | '>=' ) operand
//! operand     := value ( ('+' | '-') (INT | value) | '*' POSINT | '/' POSINT )?
//! value       := '$' IDENT | IDENT | STRING | INT | 'true' | 'false' | 'state'
//! POSINT      := INT greater than 0
//! ```
//! `IDENT` names a fact, `$IDENT` names a command argument, `state` is the current
//! state name, and `true`/`false` are boolean literals. Integer arithmetic is limited
//! to one trailing operator: `+`/`-` a constant (existing [`Operand::Arith`] encoding),
//! `+`/`-` an integer operand (fact or `$arg`), or `*`/`/` a positive integer constant.
//! Integer literals are non-negative; `x - 1` uses `-` as an operator. Division is
//! toward-zero `i64`. There is no `%`, chaining, `then` arithmetic, or general
//! expression evaluator.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Or(Vec<Expr>),
    And(Vec<Expr>),
    Cmp(Cmp),
    Exists(String),
    NotExists(String),
    TextIn(TextIn),
}

/// `text_in(VALUE, SET, MIN, MAX)`: VALUE is a `$arg` or string fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextIn {
    pub value: Operand,
    pub set_src: String,
    pub(crate) set: super::text_in::CharSet,
    pub min: u32,
    pub max: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmp {
    pub left: Operand,
    pub op: CmpOp,
    pub right: Operand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operand {
    Fact(String),
    Arg(String),
    State,
    /// Current runtime effect status: `"none"`, `"pending"`, or `"recorded"`.
    EffectStatus,
    /// Pending or recorded effect kind string (eval error when no effect exists).
    EffectKind,
    /// Recorded effect outcome digest string (eval error unless an effect outcome is recorded).
    EffectOutcome,
    Literal(Value),
    /// Existing encoding: one operand ± a non-negative integer constant.
    Arith {
        base: Box<Operand>,
        add: bool,
        amount: i64,
    },
    /// Integer operand ± integer operand (fact or `$arg`). Distinct from [`Operand::Arith`].
    BinArith {
        left: Box<Operand>,
        add: bool,
        right: Box<Operand>,
    },
    /// Operand × a positive integer constant.
    Mul {
        base: Box<Operand>,
        amount: i64,
    },
    /// Operand ÷ a positive integer constant (toward zero).
    Div {
        base: Box<Operand>,
        amount: i64,
    },
}

/// Evaluation context: facts (current values), command args, current state name,
/// and optional runtime effect status (present during command guard evaluation).
pub struct EvalCtx<'a> {
    pub facts: &'a std::collections::BTreeMap<String, Value>,
    pub args: &'a std::collections::BTreeMap<String, Value>,
    pub state: &'a str,
    pub effect: Option<&'a crate::effect::EffectStatus>,
}

/// Whether `expr` references any runtime effect operand (`effect_status`, `effect_kind`,
/// `effect_outcome`).
pub fn uses_effect_operands(expr: &Expr) -> bool {
    match expr {
        Expr::Or(parts) | Expr::And(parts) => parts.iter().any(uses_effect_operands),
        Expr::Cmp(cmp) => operand_uses_effect(&cmp.left) || operand_uses_effect(&cmp.right),
        Expr::TextIn(pred) => operand_uses_effect(&pred.value),
        Expr::Exists(_) | Expr::NotExists(_) => false,
    }
}

fn operand_uses_effect(op: &Operand) -> bool {
    match op {
        Operand::EffectStatus | Operand::EffectKind | Operand::EffectOutcome => true,
        Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => {
            operand_uses_effect(base)
        }
        Operand::BinArith { left, right, .. } => {
            operand_uses_effect(left) || operand_uses_effect(right)
        }
        Operand::Fact(_) | Operand::Arg(_) | Operand::State | Operand::Literal(_) => false,
    }
}

const GUARD_FORMS: &str = "comparisons (`==`, `!=`, `<`, `<=`, `>`, `>=`), `exists`/`not_exists`, `text_in(VALUE, SET, MIN, MAX)`, integer `+`/`-` (constant or operand), and `*`/`/` by a positive integer constant";

/// Parse a guard expression. Returns a human message on syntax errors.
pub fn parse(input: &str) -> Result<Expr, String> {
    let mut tokens = Tokenizer::new(input);
    let expr = parse_or(&mut tokens)?;
    if let Some(tok) = tokens.peek() {
        return Err(trailing_token_error(&tok, tokens.pos));
    }
    Ok(expr)
}

pub fn eval(expr: &Expr, ctx: &EvalCtx) -> Result<bool, String> {
    match expr {
        Expr::Or(parts) => {
            for part in parts {
                if eval(part, ctx)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Expr::And(parts) => {
            for part in parts {
                if !eval(part, ctx)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Expr::Cmp(cmp) => {
            let left = eval_operand(&cmp.left, ctx)?;
            let right = eval_operand(&cmp.right, ctx)?;
            compare(&left, cmp.op, &right)
        }
        Expr::Exists(name) => Ok(ctx.facts.contains_key(name)),
        Expr::NotExists(name) => Ok(!ctx.facts.contains_key(name)),
        Expr::TextIn(pred) => eval_text_in(pred, ctx),
    }
}

/// Checked `i64` arithmetic operations used identically by runtime guards, `then` updates, and Kani proofs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArithOpKind {
    Add,
    Sub,
    Mul,
    DivPos,
}

/// Pure, checked `i64` binary arithmetic kernel with zero heap allocations.
#[inline]
pub(crate) fn checked_i64_binary(
    kind: ArithOpKind,
    left: i64,
    right: i64,
) -> Result<i64, &'static str> {
    match kind {
        ArithOpKind::Add => left.checked_add(right).ok_or("integer arithmetic overflow"),
        ArithOpKind::Sub => left.checked_sub(right).ok_or("integer arithmetic overflow"),
        ArithOpKind::Mul => left.checked_mul(right).ok_or("integer arithmetic overflow"),
        ArithOpKind::DivPos => {
            if right <= 0 {
                Err("division requires a positive integer constant")
            } else {
                Ok(left / right)
            }
        }
    }
}

/// Parse an arithmetic expression used in a `then` assignment.
/// Returns `Ok(Some(op))` only when an explicit `+`, `-`, `*`, or `/` operator is present
/// (simple literals and `$arg` references return `Ok(None)` so existing literal/arg handling applies).
pub fn parse_then_arith(input: &str) -> Result<Option<Operand>, String> {
    let mut tokens = Tokenizer::new(input);
    let Some(first) = tokens.next() else {
        return Ok(None);
    };
    let op = with_arith(operand_from(first)?, &mut tokens)?;
    if let Some(tok) = tokens.peek() {
        return Err(trailing_token_error(&tok, tokens.pos));
    }
    match &op {
        Operand::Arith { .. }
        | Operand::BinArith { .. }
        | Operand::Mul { .. }
        | Operand::Div { .. } => {
            if operand_uses_disallowed_then_builtin(&op) {
                return Err(
                    "`then` arithmetic cannot reference `state` or `effect_*`".to_string(),
                );
            }
            Ok(Some(op))
        }
        _ => Ok(None),
    }
}

fn operand_uses_disallowed_then_builtin(op: &Operand) -> bool {
    match op {
        Operand::State
        | Operand::EffectStatus
        | Operand::EffectKind
        | Operand::EffectOutcome => true,
        Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => {
            operand_uses_disallowed_then_builtin(base)
        }
        Operand::BinArith { left, right, .. } => {
            operand_uses_disallowed_then_builtin(left)
                || operand_uses_disallowed_then_builtin(right)
        }
        Operand::Fact(_) | Operand::Arg(_) | Operand::Literal(_) => false,
    }
}

/// Evaluate a parsed `then` arithmetic operand against pre-transition `facts` and `args`.
pub fn eval_then_arith(
    op: &Operand,
    facts: &std::collections::BTreeMap<String, Value>,
    args: &std::collections::BTreeMap<String, Value>,
) -> Result<Value, String> {
    eval_operand(
        op,
        &EvalCtx {
            facts,
            args,
            state: "",
            effect: None,
        },
    )
}
/// Left-to-right error-preserving three-valued guard evaluation for facts known at lowering time.
///
/// Only comparisons between an explicitly assigned fact and a literal are
/// folded. Arguments, unset facts, arithmetic, and other comparisons remain
/// unknown; callers must preserve expressions that return `None`.
pub fn eval_with_assignments(
    expr: &Expr,
    assignments: &std::collections::BTreeMap<String, Value>,
) -> Option<bool> {
    eval_known(expr, assignments, None)
}

/// Three-valued evaluation with a known source state name.
///
/// [`Operand::State`] comparisons fold against `state`. Lowering continues to
/// call [`eval_with_assignments`], which leaves `state` unknown.
pub fn eval_with_assignments_at(
    expr: &Expr,
    assignments: &std::collections::BTreeMap<String, Value>,
    state: &str,
) -> Option<bool> {
    eval_known(expr, assignments, Some(state))
}

/// Left-to-right error-preserving three-valued evaluation with known facts, bound arguments, and optional state.
///
/// Unbound names, overflow, and comparison errors return `None` immediately in evaluation order so a runtime error can never be masked by a later short-circuit.
pub fn eval_partial(
    expr: &Expr,
    facts: &std::collections::BTreeMap<String, Value>,
    args: &std::collections::BTreeMap<String, Value>,
    state: Option<&str>,
) -> Option<bool> {
    match expr {
        Expr::And(parts) => fold_partial_and(parts, facts, args, state),
        Expr::Or(parts) => fold_partial_or(parts, facts, args, state),
        Expr::Cmp(cmp) => {
            let left = resolve_partial(&cmp.left, facts, args, state)?;
            let right = resolve_partial(&cmp.right, facts, args, state)?;
            compare(&left, cmp.op, &right).ok()
        }
        Expr::Exists(name) => facts.contains_key(name).then_some(true),
        Expr::NotExists(name) => facts.contains_key(name).then_some(false),
        Expr::TextIn(_) => None,
    }
}

fn fold_partial_and(
    parts: &[Expr],
    facts: &std::collections::BTreeMap<String, Value>,
    args: &std::collections::BTreeMap<String, Value>,
    state: Option<&str>,
) -> Option<bool> {
    for part in parts {
        match eval_partial(part, facts, args, state) {
            Some(true) => {}
            Some(false) => return Some(false),
            None => return None,
        }
    }
    Some(true)
}

fn fold_partial_or(
    parts: &[Expr],
    facts: &std::collections::BTreeMap<String, Value>,
    args: &std::collections::BTreeMap<String, Value>,
    state: Option<&str>,
) -> Option<bool> {
    for part in parts {
        match eval_partial(part, facts, args, state) {
            Some(false) => {}
            Some(true) => return Some(true),
            None => return None,
        }
    }
    Some(false)
}

fn resolve_partial(
    operand: &Operand,
    facts: &std::collections::BTreeMap<String, Value>,
    args: &std::collections::BTreeMap<String, Value>,
    state: Option<&str>,
) -> Option<Value> {
    match operand {
        Operand::Fact(name) => facts.get(name).cloned(),
        Operand::Arg(name) => args.get(name).cloned(),
        Operand::State => Some(Value::String(state?.to_string())),
        Operand::Literal(value) => Some(value.clone()),
        _ => resolve_partial_arith(operand, facts, args, state),
    }
}

fn resolve_partial_arith(
    operand: &Operand,
    facts: &std::collections::BTreeMap<String, Value>,
    args: &std::collections::BTreeMap<String, Value>,
    state: Option<&str>,
) -> Option<Value> {
    match operand {
        Operand::Arith { base, add, amount } => int_value(
            checked_add_sub(
                resolve_partial_int(base, facts, args, state)?,
                *add,
                *amount,
            )
            .ok()?,
        )
        .ok(),
        Operand::BinArith { left, add, right } => int_value(
            checked_add_sub(
                resolve_partial_int(left, facts, args, state)?,
                *add,
                resolve_partial_int(right, facts, args, state)?,
            )
            .ok()?,
        )
        .ok(),
        Operand::Mul { base, amount } => {
            int_value(resolve_partial_int(base, facts, args, state)?.checked_mul(*amount)?).ok()
        }
        Operand::Div { base, amount } if *amount > 0 => {
            int_value(resolve_partial_int(base, facts, args, state)? / *amount).ok()
        }
        _ => None,
    }
}

fn resolve_partial_int(
    operand: &Operand,
    facts: &std::collections::BTreeMap<String, Value>,
    args: &std::collections::BTreeMap<String, Value>,
    state: Option<&str>,
) -> Option<i64> {
    resolve_partial(operand, facts, args, state)?.as_i64()
}

fn eval_known(
    expr: &Expr,
    assignments: &std::collections::BTreeMap<String, Value>,
    state: Option<&str>,
) -> Option<bool> {
    match expr {
        Expr::And(parts) => eval_and_with_assignments(parts, assignments, state),
        Expr::Or(parts) => eval_or_with_assignments(parts, assignments, state),
        Expr::Cmp(cmp) => eval_cmp_with_assignments(cmp, assignments, state),
        Expr::Exists(name) => assignments.contains_key(name).then_some(true),
        Expr::NotExists(name) => assignments.contains_key(name).then_some(false),
        Expr::TextIn(_) => None,
    }
}

fn eval_and_with_assignments(
    parts: &[Expr],
    assignments: &std::collections::BTreeMap<String, Value>,
    state: Option<&str>,
) -> Option<bool> {
    for part in parts {
        match eval_known(part, assignments, state) {
            Some(true) => {}
            Some(false) => return Some(false),
            None => return None,
        }
    }
    Some(true)
}

fn eval_or_with_assignments(
    parts: &[Expr],
    assignments: &std::collections::BTreeMap<String, Value>,
    state: Option<&str>,
) -> Option<bool> {
    for part in parts {
        match eval_known(part, assignments, state) {
            Some(false) => {}
            Some(true) => return Some(true),
            None => return None,
        }
    }
    Some(false)
}

fn eval_cmp_with_assignments(
    cmp: &Cmp,
    assignments: &std::collections::BTreeMap<String, Value>,
    state: Option<&str>,
) -> Option<bool> {
    match (&cmp.left, &cmp.right) {
        (Operand::Fact(name), Operand::Literal(value))
        | (Operand::Literal(value), Operand::Fact(name))
            if matches!(cmp.op, CmpOp::Eq | CmpOp::Ne) =>
        {
            compare(assignments.get(name)?, cmp.op, value).ok()
        }
        (Operand::State, Operand::Literal(value)) | (Operand::Literal(value), Operand::State)
            if matches!(cmp.op, CmpOp::Eq | CmpOp::Ne) =>
        {
            let current = state?;
            compare(&Value::String(current.to_string()), cmp.op, value).ok()
        }
        _ => None,
    }
}

fn eval_operand(op: &Operand, ctx: &EvalCtx) -> Result<Value, String> {
    match op {
        Operand::Fact(name) => ctx
            .facts
            .get(name)
            .cloned()
            .ok_or_else(|| format!("fact `{name}` is not set")),
        Operand::Arg(name) => ctx
            .args
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unknown argument `${name}`")),
        Operand::State => Ok(Value::String(ctx.state.to_string())),
        Operand::EffectStatus => {
            let status = match ctx.effect {
                Some(crate::effect::EffectStatus::Pending(_)) => "pending",
                Some(crate::effect::EffectStatus::Recorded(_, _, _)) => "recorded",
                Some(crate::effect::EffectStatus::None) | None => "none",
            };
            Ok(Value::String(status.to_string()))
        }
        Operand::EffectKind => match ctx.effect {
            Some(crate::effect::EffectStatus::Pending(intent))
            | Some(crate::effect::EffectStatus::Recorded(intent, _, _)) => {
                Ok(Value::String(intent.effect_kind.clone()))
            }
            Some(crate::effect::EffectStatus::None) | None => {
                Err("effect_kind is unset (no effect is pending or recorded)".to_string())
            }
        },
        Operand::EffectOutcome => match ctx.effect {
            Some(crate::effect::EffectStatus::Recorded(_, outcome_digest, _)) => {
                Ok(Value::String(outcome_digest.clone()))
            }
            Some(crate::effect::EffectStatus::Pending(_)) => Err(
                "effect_outcome is unset (pending effect has not been recorded yet)".to_string(),
            ),
            Some(crate::effect::EffectStatus::None) | None => {
                Err("effect_outcome is unset (no effect outcome has been recorded)".to_string())
            }
        },
        Operand::Literal(v) => Ok(v.clone()),
        Operand::Arith { base, add, amount } => {
            int_value(checked_add_sub(eval_int(base, ctx)?, *add, *amount)?)
        }
        Operand::BinArith { left, add, right } => int_value(checked_add_sub(
            eval_int(left, ctx)?,
            *add,
            eval_int(right, ctx)?,
        )?),
        Operand::Mul { base, amount } => int_value(
            checked_i64_binary(ArithOpKind::Mul, eval_int(base, ctx)?, *amount)
                .map_err(str::to_string)?,
        ),
        Operand::Div { base, amount } => int_value(
            checked_i64_binary(ArithOpKind::DivPos, eval_int(base, ctx)?, *amount)
                .map_err(str::to_string)?,
        ),
    }
}

fn eval_int(op: &Operand, ctx: &EvalCtx) -> Result<i64, String> {
    eval_operand(op, ctx)?
        .as_i64()
        .ok_or_else(|| "arithmetic requires an integer operand".to_string())
}

fn checked_add_sub(left: i64, add: bool, right: i64) -> Result<i64, String> {
    let kind = if add {
        ArithOpKind::Add
    } else {
        ArithOpKind::Sub
    };
    checked_i64_binary(kind, left, right).map_err(str::to_string)
}

fn int_value(n: i64) -> Result<Value, String> {
    Ok(Value::Number(n.into()))
}

fn compare(left: &Value, op: CmpOp, right: &Value) -> Result<bool, String> {
    let same_type = (left.as_i64().is_some() && right.as_i64().is_some())
        || (left.is_string() && right.is_string())
        || (left.is_boolean() && right.is_boolean());
    if !same_type {
        return Err("type mismatch: cannot compare values of different types".to_string());
    }
    let ord = |a: &Value, b: &Value| -> Result<std::cmp::Ordering, String> {
        if let (Some(a), Some(b)) = (a.as_i64(), b.as_i64()) {
            return Ok(a.cmp(&b));
        }
        if let (Some(a), Some(b)) = (a.as_str(), b.as_str()) {
            return Ok(a.cmp(b));
        }
        Err("comparison requires two integers or two strings".to_string())
    };
    match op {
        CmpOp::Eq => Ok(left == right),
        CmpOp::Ne => Ok(left != right),
        CmpOp::Lt => Ok(ord(left, right)? == std::cmp::Ordering::Less),
        CmpOp::Le => Ok(ord(left, right)? != std::cmp::Ordering::Greater),
        CmpOp::Gt => Ok(ord(left, right)? == std::cmp::Ordering::Greater),
        CmpOp::Ge => Ok(ord(left, right)? != std::cmp::Ordering::Less),
    }
}

fn eval_text_in(pred: &TextIn, ctx: &EvalCtx) -> Result<bool, String> {
    let value = eval_operand(&pred.value, ctx)?;
    let Some(text) = value.as_str() else {
        return Err("text_in requires a string value".to_string());
    };
    Ok(super::text_in::holds(text, &pred.set, pred.min, pred.max))
}

/// Deterministic print used for parse/print digest stability.
pub fn format_expr(expr: &Expr) -> String {
    format_prec(expr, false)
}

fn format_prec(expr: &Expr, parent_and: bool) -> String {
    match expr {
        Expr::Or(parts) => {
            let body = join_parts(parts, " || ", false);
            if parent_and {
                format!("({body})")
            } else {
                body
            }
        }
        Expr::And(parts) => join_parts(parts, " && ", true),
        Expr::Cmp(cmp) => format!(
            "{} {} {}",
            format_operand(&cmp.left),
            cmp_src(cmp.op),
            format_operand(&cmp.right)
        ),
        Expr::Exists(name) => format!("exists({name})"),
        Expr::NotExists(name) => format!("not_exists({name})"),
        Expr::TextIn(pred) => format!(
            "text_in({},{},{},{})",
            format_operand(&pred.value),
            format_set_literal(&pred.set_src),
            pred.min,
            pred.max
        ),
    }
}

fn join_parts(parts: &[Expr], sep: &str, parent_and: bool) -> String {
    parts
        .iter()
        .map(|part| format_prec(part, parent_and))
        .collect::<Vec<_>>()
        .join(sep)
}

fn cmp_src(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "==",
        CmpOp::Ne => "!=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
    }
}

fn format_operand(op: &Operand) -> String {
    match op {
        Operand::Fact(name) => name.clone(),
        Operand::Arg(name) => format!("${name}"),
        Operand::State => "state".to_string(),
        Operand::EffectStatus => "effect_status".to_string(),
        Operand::EffectKind => "effect_kind".to_string(),
        Operand::EffectOutcome => "effect_outcome".to_string(),
        Operand::Literal(value) => format_literal(value),
        Operand::Arith { base, add, amount } => {
            let sign = if *add { "+" } else { "-" };
            format!("{} {sign} {amount}", format_operand(base))
        }
        Operand::BinArith { left, add, right } => {
            let sign = if *add { "+" } else { "-" };
            format!("{} {sign} {}", format_operand(left), format_operand(right))
        }
        Operand::Mul { base, amount } => format!("{} * {amount}", format_operand(base)),
        Operand::Div { base, amount } => format!("{} / {amount}", format_operand(base)),
    }
}

fn format_literal(value: &Value) -> String {
    if let Some(s) = value.as_str() {
        format_set_literal(s)
    } else if let Some(b) = value.as_bool() {
        b.to_string()
    } else if let Some(n) = value.as_i64() {
        n.to_string()
    } else {
        value.to_string()
    }
}

fn format_set_literal(src: &str) -> String {
    if src.contains('\'') {
        format!("\"{src}\"")
    } else {
        format!("'{src}'")
    }
}

// ---------------------------------------------------------------------------
// Tokenizer + recursive descent parser
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Dollar(String),
    Str(String),
    Int(i64),
    Bool(bool),
    Op(String),
    LParen,
    RParen,
    Error(String),
}

struct Tokenizer {
    chars: Vec<char>,
    pos: usize,
}

impl Tokenizer {
    fn new(input: &str) -> Self {
        Tokenizer {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&mut self) -> Option<Tok> {
        let saved = self.pos;
        let tok = self.next();
        self.pos = saved;
        tok
    }

    fn next(&mut self) -> Option<Tok> {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
        if self.pos >= self.chars.len() {
            return None;
        }
        let c = self.chars[self.pos];
        match c {
            '(' => {
                self.pos += 1;
                Some(Tok::LParen)
            }
            ')' => {
                self.pos += 1;
                Some(Tok::RParen)
            }
            '$' => {
                self.pos += 1;
                let name = self.take_ident();
                Some(Tok::Dollar(name))
            }
            '\'' | '"' => self.take_string(c),
            '0'..='9' => self.take_int(),
            '+' | '-' | '*' | '/' | '%' => {
                self.pos += 1;
                Some(Tok::Op(c.to_string()))
            }
            _ if c.is_alphabetic() || c == '_' => {
                let name = self.take_ident();
                match name.as_str() {
                    "true" => Some(Tok::Bool(true)),
                    "false" => Some(Tok::Bool(false)),
                    _ => Some(Tok::Ident(name)),
                }
            }
            _ => {
                // operators: && || == != <= >= < >
                let mut op = String::new();
                while self.pos < self.chars.len() && "&|=!<>".contains(self.chars[self.pos]) {
                    op.push(self.chars[self.pos]);
                    self.pos += 1;
                }
                if op.is_empty() {
                    self.pos += 1;
                    Some(Tok::Op(c.to_string()))
                } else {
                    Some(Tok::Op(op))
                }
            }
        }
    }

    fn take_string(&mut self, quote: char) -> Option<Tok> {
        self.pos += 1;
        let mut s = String::new();
        while self.pos < self.chars.len() && self.chars[self.pos] != quote {
            s.push(self.chars[self.pos]);
            self.pos += 1;
        }
        if self.pos >= self.chars.len() {
            return Some(Tok::Error(format!("unterminated string literal (missing closing {quote})")));
        }
        self.pos += 1;
        Some(Tok::Str(s))
    }

    fn take_int(&mut self) -> Option<Tok> {
        let mut num = String::new();
        while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_digit() {
            num.push(self.chars[self.pos]);
            self.pos += 1;
        }
        match num.parse::<i64>() {
            Ok(n) => Some(Tok::Int(n)),
            Err(_) => Some(Tok::Error(format!(
                "integer literal `{num}` overflows 64-bit signed integer"
            ))),
        }
    }

    fn take_ident(&mut self) -> String {
        let mut name = String::new();
        while self.pos < self.chars.len()
            && (self.chars[self.pos].is_alphanumeric() || self.chars[self.pos] == '_')
        {
            name.push(self.chars[self.pos]);
            self.pos += 1;
        }
        name
    }
}

fn parse_or(tokens: &mut Tokenizer) -> Result<Expr, String> {
    let mut parts = vec![parse_and(tokens)?];
    while let Some(Tok::Op(op)) = tokens.peek() {
        if op == "||" {
            tokens.next();
            parts.push(parse_and(tokens)?);
        } else {
            break;
        }
    }
    fold_expr(parts, Expr::Or)
}

fn parse_and(tokens: &mut Tokenizer) -> Result<Expr, String> {
    let mut parts = vec![parse_atom(tokens)?];
    while let Some(Tok::Op(op)) = tokens.peek() {
        if op == "&&" {
            tokens.next();
            parts.push(parse_atom(tokens)?);
        } else {
            break;
        }
    }
    fold_expr(parts, Expr::And)
}

/// Collapse a parsed `&&` / `||` chain: a single part is the expression itself.
fn fold_expr(parts: Vec<Expr>, wrap: fn(Vec<Expr>) -> Expr) -> Result<Expr, String> {
    let mut parts = parts.into_iter();
    let first = parts
        .next()
        .ok_or_else(|| "expected an expression".to_string())?;
    Ok(match parts.next() {
        None => first,
        Some(second) => {
            let mut rest = vec![first, second];
            rest.extend(parts);
            wrap(rest)
        }
    })
}

fn parse_atom(tokens: &mut Tokenizer) -> Result<Expr, String> {
    match tokens.next() {
        Some(Tok::LParen) => parse_paren(tokens),
        Some(Tok::Ident(name)) if matches!(tokens.peek(), Some(Tok::LParen)) => {
            parse_predicate(name, tokens)
        }
        Some(tok) => parse_cmp(tok, tokens),
        None => Err("unexpected end of expression".to_string()),
    }
}

fn parse_predicate(name: String, tokens: &mut Tokenizer) -> Result<Expr, String> {
    match name.as_str() {
        "exists" | "not_exists" => parse_exists(name, tokens),
        "text_in" => parse_text_in(tokens),
        other => Err(format!("unknown predicate `{other}`")),
    }
}

fn parse_text_in(tokens: &mut Tokenizer) -> Result<Expr, String> {
    match tokens.next() {
        Some(Tok::LParen) => {}
        _ => return Err("expected `(` after text_in".to_string()),
    }
    let value = parse_text_in_value(tokens)?;
    expect_comma(tokens, "text_in VALUE")?;
    let set_src = parse_text_in_set(tokens)?;
    expect_comma(tokens, "text_in SET")?;
    let min = parse_text_in_bound(tokens, "MIN")?;
    expect_comma(tokens, "text_in MIN")?;
    let max = parse_text_in_bound(tokens, "MAX")?;
    expect_text_in_close(tokens)?;
    if min > max {
        return Err("text_in MIN must be <= MAX".to_string());
    }
    let set = super::text_in::parse_set(&set_src)?;
    Ok(Expr::TextIn(TextIn {
        value,
        set_src,
        set,
        min,
        max,
    }))
}

fn parse_text_in_value(tokens: &mut Tokenizer) -> Result<Operand, String> {
    match tokens.next() {
        Some(Tok::Dollar(name)) => Ok(Operand::Arg(name)),
        Some(Tok::Ident(name)) if name == "state" => {
            Err("text_in VALUE must be a `$arg` or a fact name, not `state`".to_string())
        }
        Some(Tok::Ident(name)) => Ok(Operand::Fact(name)),
        Some(Tok::Str(_)) | Some(Tok::Int(_)) | Some(Tok::Bool(_)) => {
            Err("text_in VALUE must be a `$arg` or a fact name, not a literal".to_string())
        }
        Some(Tok::RParen) => {
            Err("text_in requires four arguments: VALUE, SET, MIN, MAX".to_string())
        }
        _ => Err("text_in VALUE must be a `$arg` or a fact name".to_string()),
    }
}

fn parse_text_in_set(tokens: &mut Tokenizer) -> Result<String, String> {
    match tokens.next() {
        Some(Tok::Str(s)) => Ok(s),
        Some(Tok::RParen) => {
            Err("text_in requires four arguments: VALUE, SET, MIN, MAX".to_string())
        }
        _ => Err("text_in SET must be a string literal".to_string()),
    }
}

fn parse_text_in_bound(tokens: &mut Tokenizer, which: &str) -> Result<u32, String> {
    match tokens.next() {
        Some(Tok::Int(n)) if n > super::text_in::MAX_BOUND => Err(format!(
            "text_in {which} must be <= {}",
            super::text_in::MAX_BOUND
        )),
        Some(Tok::Int(n)) => Ok(n as u32),
        Some(Tok::Op(op)) if op == "-" => Err(format!(
            "text_in {which} must be a non-negative integer literal in 0..={}",
            super::text_in::MAX_BOUND
        )),
        Some(Tok::RParen) => {
            Err("text_in requires four arguments: VALUE, SET, MIN, MAX".to_string())
        }
        Some(Tok::Str(_)) | Some(Tok::Ident(_)) | Some(Tok::Bool(_)) | Some(Tok::Dollar(_)) => {
            Err(format!(
                "text_in {which} must be an integer literal in 0..={}",
                super::text_in::MAX_BOUND
            ))
        }
        _ => Err("text_in requires four arguments: VALUE, SET, MIN, MAX".to_string()),
    }
}

fn expect_comma(tokens: &mut Tokenizer, after: &str) -> Result<(), String> {
    match tokens.next() {
        Some(Tok::Op(op)) if op == "," => Ok(()),
        Some(Tok::RParen) => {
            Err("text_in requires four arguments: VALUE, SET, MIN, MAX".to_string())
        }
        _ => Err(format!("expected `,` after {after}")),
    }
}

fn expect_text_in_close(tokens: &mut Tokenizer) -> Result<(), String> {
    match tokens.next() {
        Some(Tok::RParen) => Ok(()),
        Some(Tok::Op(op)) if op == "," => {
            Err("text_in requires four arguments: VALUE, SET, MIN, MAX".to_string())
        }
        _ => Err("expected `)` after text_in(...)".to_string()),
    }
}

/// Parse a parenthesized sub-expression; the opening `(` is already consumed.
fn parse_paren(tokens: &mut Tokenizer) -> Result<Expr, String> {
    let inner = parse_or(tokens)?;
    match tokens.next() {
        Some(Tok::RParen) => Ok(inner),
        _ => Err("expected `)`".to_string()),
    }
}

/// Parse `exists ( FACT )` / `not_exists ( FACT )`; the keyword token is already consumed.
fn parse_exists(name: String, tokens: &mut Tokenizer) -> Result<Expr, String> {
    match tokens.next() {
        Some(Tok::LParen) => {}
        _ => return Err(format!("expected `(` after {name}")),
    }
    let fact = match tokens.next() {
        Some(Tok::Ident(f)) => f,
        _ => return Err(format!("expected fact name inside {name}(...)")),
    };
    match tokens.next() {
        Some(Tok::RParen) => {}
        _ => return Err(format!("expected `)` after {name}(...)")),
    }
    if name == "exists" {
        Ok(Expr::Exists(fact))
    } else {
        Ok(Expr::NotExists(fact))
    }
}

/// Parse a comparison; the leading operand token is already consumed.
fn parse_cmp(tok: Tok, tokens: &mut Tokenizer) -> Result<Expr, String> {
    let left = with_arith(operand_from(tok)?, tokens)?;
    let op = match tokens.next() {
        Some(Tok::Op(op)) if matches!(op.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">=") => op,
        other => return Err(comparison_operator_error(other)),
    };
    let right_tok = tokens
        .next()
        .ok_or_else(|| "expected an operand after the comparison operator".to_string())?;
    let right = with_arith(operand_from(right_tok)?, tokens)?;
    Ok(Expr::Cmp(Cmp {
        left,
        op: cmp_op_from_str(&op)?,
        right,
    }))
}

/// Map a comparison operator token to its `CmpOp`.
fn cmp_op_from_str(op: &str) -> Result<CmpOp, String> {
    match op {
        "==" => Ok(CmpOp::Eq),
        "!=" => Ok(CmpOp::Ne),
        "<" => Ok(CmpOp::Lt),
        "<=" => Ok(CmpOp::Le),
        ">" => Ok(CmpOp::Gt),
        ">=" => Ok(CmpOp::Ge),
        _ => Err(format!("unsupported comparison operator `{op}`")),
    }
}

fn operand_from(tok: Tok) -> Result<Operand, String> {
    match tok {
        Tok::Ident(name) if name == "state" => Ok(Operand::State),
        Tok::Ident(name) if name == "effect_status" => Ok(Operand::EffectStatus),
        Tok::Ident(name) if name == "effect_kind" => Ok(Operand::EffectKind),
        Tok::Ident(name) if name == "effect_outcome" => Ok(Operand::EffectOutcome),
        Tok::Ident(name) => Ok(Operand::Fact(name)),
        Tok::Dollar(name) => Ok(Operand::Arg(name)),
        Tok::Str(s) => Ok(Operand::Literal(Value::String(s))),
        Tok::Int(n) => Ok(Operand::Literal(Value::Number(n.into()))),
        Tok::Bool(b) => Ok(Operand::Literal(Value::Bool(b))),
        Tok::Error(msg) => Err(msg),
        _ => Err("expected an operand".to_string()),
    }
}

/// Consume an optional trailing arithmetic term without changing `base ± INT` encoding.
fn with_arith(operand: Operand, tokens: &mut Tokenizer) -> Result<Operand, String> {
    let Some(Tok::Op(op)) = tokens.peek() else {
        return Ok(operand);
    };
    match op.as_str() {
        "+" | "-" => finish_add_sub(operand, tokens),
        "*" | "/" => finish_scale(operand, tokens),
        _ => Ok(operand),
    }
}

fn finish_add_sub(operand: Operand, tokens: &mut Tokenizer) -> Result<Operand, String> {
    let Some(Tok::Op(op)) = tokens.next() else {
        return Err("expected `+` or `-`".to_string());
    };
    let add = op == "+";
    match tokens.next() {
        Some(Tok::Int(n)) => Ok(Operand::Arith {
            base: Box::new(operand),
            add,
            amount: n,
        }),
        Some(tok) => match operand_from(tok) {
            Ok(right @ (Operand::Fact(_) | Operand::Arg(_))) => Ok(Operand::BinArith {
                left: Box::new(operand),
                add,
                right: Box::new(right),
            }),
            _ => Err(format!("expected an integer after `{op}`")),
        },
        None => Err(format!("expected an integer after `{op}`")),
    }
}

fn finish_scale(operand: Operand, tokens: &mut Tokenizer) -> Result<Operand, String> {
    let Some(Tok::Op(op)) = tokens.next() else {
        return Err("expected `*` or `/`".to_string());
    };
    match tokens.next() {
        Some(Tok::Int(n)) if n > 0 => {
            if op == "*" {
                Ok(Operand::Mul {
                    base: Box::new(operand),
                    amount: n,
                })
            } else {
                Ok(Operand::Div {
                    base: Box::new(operand),
                    amount: n,
                })
            }
        }
        Some(Tok::Int(0)) if op == "/" => {
            Err("division by zero: divisor must be a positive integer constant".into())
        }
        Some(Tok::Int(_)) => Err(format!("`{op}` requires a positive integer constant")),
        _ => Err(format!(
            "`{op}` requires a positive integer constant; supported forms: {GUARD_FORMS}"
        )),
    }
}

fn comparison_operator_error(tok: Option<Tok>) -> String {
    match tok {
        Some(Tok::Error(msg)) => msg,
        Some(Tok::Op(op)) if is_unsupported_arith_op(&op) => unsupported_operator(&op),
        Some(tok) => {
            format!("expected a comparison operator, got {tok:?}; supported forms: {GUARD_FORMS}")
        }
        None => format!(
            "expected a comparison operator (for boolean facts/args write `flag == true` or `flag == false`); supported forms: {GUARD_FORMS}"
        ),
    }
}

fn trailing_token_error(tok: &Tok, pos: usize) -> String {
    match tok {
        Tok::Error(msg) => msg.clone(),
        Tok::Op(op) if is_unsupported_arith_op(op) => unsupported_operator(op),
        _ => format!("unexpected token {tok:?} at position {pos}; supported forms: {GUARD_FORMS}"),
    }
}

fn is_unsupported_arith_op(op: &str) -> bool {
    matches!(op, "*" | "/" | "%" | "")
}

fn unsupported_operator(op: &str) -> String {
    let shown = if op.is_empty() { "<unknown>" } else { op };
    format!("unsupported operator `{shown}` in a guard; supported forms: {GUARD_FORMS}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed_cmp(src: &str) -> Result<Cmp, String> {
        match parse(src)? {
            Expr::Cmp(cmp) => Ok(cmp),
            other => Err(format!("{src}: {other:?}")),
        }
    }

    #[test]
    fn constant_add_keeps_arith_encoding() {
        assert_eq!(
            parsed_cmp("n + 1 == 6").map(|cmp| cmp.left),
            Ok(Operand::Arith {
                base: Box::new(Operand::Fact("n".into())),
                add: true,
                amount: 1,
            })
        );
    }

    #[test]
    fn operand_add_is_bin_arith_not_arith() {
        let right = parsed_cmp("$remain_after == remaining_qty - $units").map(|cmp| cmp.right);
        assert!(
            matches!(right, Ok(Operand::BinArith { add: false, .. })),
            "{right:?}"
        );
        assert_ne!(
            right,
            Ok(Operand::Arith {
                base: Box::new(Operand::Fact("remaining_qty".into())),
                add: false,
                amount: 0,
            })
        );
    }

    #[test]
    fn mul_and_div_are_distinct_from_arith() {
        let mul = parsed_cmp("$amount == units * 175").map(|cmp| cmp.right);
        assert!(
            matches!(mul, Ok(Operand::Mul { amount: 175, .. })),
            "{mul:?}"
        );
        let div = parsed_cmp("$deposit_cents == amount / 50").map(|cmp| cmp.right);
        assert!(
            matches!(div, Ok(Operand::Div { amount: 50, .. })),
            "{div:?}"
        );
    }

    #[test]
    fn assignments_at_fold_state_equality() {
        let mut facts = std::collections::BTreeMap::new();
        facts.insert("region_a".into(), Value::String("a1".into()));
        let fold = |src: &str, state: &str| {
            parse(src)
                .ok()
                .and_then(|expr| eval_with_assignments_at(&expr, &facts, state))
        };
        assert_eq!(
            fold(r#"state == "idle" && region_a == "a1""#, "idle"),
            Some(true)
        );
        assert_eq!(
            fold(r#"state == "idle" && region_a == "a1""#, "other"),
            Some(false)
        );
        assert_eq!(
            parse(r#"state == "idle" && region_a == "a1""#)
                .ok()
                .and_then(|expr| eval_with_assignments(&expr, &facts)),
            None
        );
        assert_eq!(fold(r#"state == "idle" && flag == true"#, "idle"), None);
        assert_eq!(
            fold(r#"state == "idle" && flag == true"#, "other"),
            Some(false)
        );
    }

    #[test]
    fn partial_eval_folds_bound_arguments() {
        let facts = std::collections::BTreeMap::new();
        let mut args = std::collections::BTreeMap::new();
        args.insert("x".into(), Value::from(3));
        let fold = |args: &std::collections::BTreeMap<String, Value>| {
            parse("$x >= 0 || $x < 0")
                .ok()
                .and_then(|expr| eval_partial(&expr, &facts, args, Some("idle")))
        };
        assert_eq!(fold(&args), Some(true));
        args.insert("x".into(), Value::from(-1));
        assert_eq!(fold(&args), Some(true));
        let unbound = std::collections::BTreeMap::new();
        assert_eq!(fold(&unbound), None);
    }

    #[test]
    fn text_in_round_trip_is_stable() {
        let src = "text_in($id,'A-Za-z0-9._-',1,64)";
        let expr = parse(src).expect("parse");
        let printed = format_expr(&expr);
        assert_eq!(printed, src);
        assert_eq!(parse(&printed).expect("reparse"), expr);
        let again = format_expr(&parse(&printed).expect("reparse"));
        assert_eq!(again, printed);
    }

    #[test]
    fn text_in_partial_eval_stays_unknown() {
        let mut args = std::collections::BTreeMap::new();
        args.insert("id".into(), Value::String("ok".into()));
        let facts = std::collections::BTreeMap::new();
        let expr = parse("text_in($id,'A-Za-z0-9._-',1,64)").expect("parse");
        assert_eq!(eval_partial(&expr, &facts, &args, Some("idle")), None);
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    #[kani::proof]
    fn prove_checked_i64_arith_no_silent_wrap() {
        let op_sel: u8 = kani::any();
        kani::assume(op_sel < 4);
        let left: i64 = kani::any();
        let right: i64 = kani::any();

        match op_sel {
            0 => {
                let res = checked_i64_binary(ArithOpKind::Add, left, right);
                let wide = (left as i128) + (right as i128);
                if wide >= i64::MIN as i128 && wide <= i64::MAX as i128 {
                    assert_eq!(res, Ok(wide as i64));
                    kani::cover!(left > 0 && right > 0, "valid positive add");
                } else {
                    assert!(res.is_err());
                    kani::cover!(true, "add overflow refused");
                }
            }
            1 => {
                let res = checked_i64_binary(ArithOpKind::Sub, left, right);
                let wide = (left as i128) - (right as i128);
                if wide >= i64::MIN as i128 && wide <= i64::MAX as i128 {
                    assert_eq!(res, Ok(wide as i64));
                    kani::cover!(left < 0 && right > 0, "valid sub");
                } else {
                    assert!(res.is_err());
                    kani::cover!(true, "sub overflow refused");
                }
            }
            2 => {
                let mul_sel: u8 = kani::any();
                kani::assume(mul_sel < 5);
                match mul_sel {
                    0 => assert_eq!(checked_i64_binary(ArithOpKind::Mul, left, 0), Ok(0)),
                    1 => assert_eq!(checked_i64_binary(ArithOpKind::Mul, left, 1), Ok(left)),
                    2 => {
                        let res = checked_i64_binary(ArithOpKind::Mul, left, -1);
                        if left == i64::MIN {
                            assert!(res.is_err());
                            kani::cover!(true, "i64 MIN mul -1 overflow refused");
                        } else {
                            assert_eq!(res, Ok(-left));
                        }
                    }
                    3 => {
                        let res = checked_i64_binary(ArithOpKind::Mul, left, 2);
                        let wide = (left as i128) * 2;
                        if wide >= i64::MIN as i128 && wide <= i64::MAX as i128 {
                            assert_eq!(res, Ok(wide as i64));
                        } else {
                            assert!(res.is_err());
                            kani::cover!(true, "mul by 2 overflow refused");
                        }
                    }
                    _ => {
                        let res = checked_i64_binary(ArithOpKind::Mul, left, 250);
                        let wide = (left as i128) * 250;
                        if wide >= i64::MIN as i128 && wide <= i64::MAX as i128 {
                            assert_eq!(res, Ok(wide as i64));
                            kani::cover!(left > 1, "valid mul by 250");
                        } else {
                            assert!(res.is_err());
                            kani::cover!(true, "mul by 250 overflow refused");
                        }
                    }
                }
            }
            _ => {
                let div_sel: u8 = kani::any();
                kani::assume(div_sel < 5);
                match div_sel {
                    0 => {
                        assert!(checked_i64_binary(ArithOpKind::DivPos, left, 0).is_err());
                        kani::cover!(true, "div zero refused");
                    }
                    1 => {
                        assert!(checked_i64_binary(ArithOpKind::DivPos, left, -3).is_err());
                        kani::cover!(true, "div negative refused");
                    }
                    2 => assert_eq!(checked_i64_binary(ArithOpKind::DivPos, left, 1), Ok(left)),
                    3 => {
                        let res = checked_i64_binary(ArithOpKind::DivPos, left, 2);
                        assert_eq!(res, Ok(((left as i128) / 2) as i64));
                        kani::cover!(left < -1, "negative toward-zero div by 2");
                    }
                    _ => {
                        kani::assume((-1024..=1024).contains(&left));
                        let res = checked_i64_binary(ArithOpKind::DivPos, left, 3);
                        assert_eq!(res, Ok(((left as i128) / 3) as i64));
                        kani::cover!(left == -7, "odd toward-zero div -7 / 3 == -2");
                    }
                }
            }
        }
    }
}
