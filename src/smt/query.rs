//! SMT-LIB2 queries: pinned eval, local selectability, BMC, and Horn reachability.

use super::encode::{
    arg_val, conjunct, encode_expr, fact_set, fact_val, in_i64_range, in_string_id_range, smt_int,
    state_sym, Sort, I64_MAX_SMT, I64_MIN_SMT,
};
use super::machine::{lower_definition, Lowered, ThenOp, Transition};
use super::{Names, Unsupported};
use crate::declarative::def::Definition;
use crate::declarative::expr::Expr;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryKind {
    GuardEval,
    Selectability,
    Bmc,
    Horn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    State(String),
    Terminal(String),
    Outcome { command: String, index: usize },
    CommandFrom { command: String, source: String },
}

#[derive(Clone, Debug)]
pub struct SmtQuery {
    pub text: String,
    pub kind: QueryKind,
    pub names: Names,
    pub get_values: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Pinned {
    pub state: String,
    pub facts: BTreeMap<String, Option<Value>>,
    pub args: BTreeMap<String, Value>,
}

pub fn encode_guard_eval(expr: &Expr, pinned: &Pinned) -> Result<SmtQuery, Unsupported> {
    let names = names_for_expr(expr, pinned);
    if let Some(err) = super::fragment_error(expr, &names) {
        return Err(err);
    }
    let term = encode_expr(expr, &names)?;
    let mut lines = vec![
        "; ik.smt.guard-eval".to_string(),
        "(set-logic QF_LIA)".to_string(),
        "(set-option :produce-models true)".to_string(),
    ];
    lines.extend(declare_names(&names, "", true));
    lines.push(format!("(assert {})", pin_formula(&names, pinned)?));
    lines.push(format!("(declare-const |g:err| Bool)"));
    lines.push(format!("(declare-const |g:val| Bool)"));
    lines.push(format!("(assert (= |g:err| {}))", term.error));
    lines.push(format!("(assert (= |g:val| {}))", term.value));
    lines.push("(check-sat)".to_string());
    lines.push("(get-value (|g:err| |g:val|))".to_string());
    Ok(SmtQuery {
        text: join_lines(&lines),
        kind: QueryKind::GuardEval,
        names,
        get_values: vec!["|g:err|".into(), "|g:val|".into()],
    })
}

pub fn encode_selectability(
    def: &Definition,
    command: &str,
    source: &str,
    outcome: usize,
) -> Result<SmtQuery, Unsupported> {
    let lowered = lower_definition(def)?;
    let trans = lowered
        .outcomes_for(command, source)
        .into_iter()
        .find(|row| row.outcome_index == outcome)
        .ok_or_else(|| Unsupported::new("no such command/source/outcome"))?;
    let mut names = lowered.names.clone();
    names.args = trans.args.clone();
    check_transition_fragment(trans, &names)?;
    let selectable = selectable_formula(trans, &names, "")?;
    let mut lines = vec![
        format!("; ik.smt.selectability command={command} source={source} outcome={outcome}"),
        "(set-logic QF_LIA)".to_string(),
        "(set-option :produce-models true)".to_string(),
    ];
    lines.extend(declare_names(&names, "", true));
    lines.extend(range_asserts(&names, "", true));
    lines.push(format!("(assert {selectable})"));
    lines.push("(check-sat)".to_string());
    let gets = value_symbols(&names, "", true);
    lines.push(format!("(get-value ({}))", gets.join(" ")));
    Ok(SmtQuery {
        text: join_lines(&lines),
        kind: QueryKind::Selectability,
        names,
        get_values: gets,
    })
}

pub fn encode_selectability_any(
    def: &Definition,
    command: &str,
    outcome: usize,
) -> Result<SmtQuery, Unsupported> {
    let lowered = lower_definition(def)?;
    let mut names = lowered.names.clone();
    if let Some(row) = lowered
        .transitions
        .iter()
        .find(|row| row.command == command && row.outcome_index == outcome)
    {
        names.args = row.args.clone();
    }
    let selectable = selectable_any_formula(&lowered, command, outcome)?;
    let mut lines = vec![
        format!("; ik.smt.selectability-any command={command} outcome={outcome}"),
        "(set-logic QF_LIA)".to_string(),
        "(set-option :produce-models true)".to_string(),
    ];
    lines.extend(declare_names(&names, "", true));
    lines.extend(range_asserts(&names, "", true));
    lines.push(format!("(assert {selectable})"));
    lines.push("(check-sat)".to_string());
    Ok(SmtQuery {
        text: join_lines(&lines),
        kind: QueryKind::Selectability,
        names,
        get_values: Vec::new(),
    })
}

pub fn encode_bmc(
    def: &Definition,
    target: &Target,
    depth: usize,
) -> Result<SmtQuery, Unsupported> {
    let lowered = lower_definition(def)?;
    check_machine_fragment(&lowered)?;
    let mut lines = vec![
        format!("; ik.smt.bmc depth={depth} target={target:?}"),
        "(set-logic QF_LIA)".to_string(),
        "(set-option :produce-models true)".to_string(),
    ];
    for time in 0..=depth {
        lines.extend(declare_names(&lowered.names, &frame(time), time < depth));
        lines.extend(range_asserts(&lowered.names, &frame(time), time < depth));
        lines.push(format!("(declare-const |{}cmd| Int)", frame(time)));
        lines.push(format!("(declare-const |{}out| Int)", frame(time)));
    }
    lines.push(format!("(assert {})", init_formula(&lowered, &frame(0))?));
    for time in 0..depth {
        let step = step_formula(&lowered, &frame(time), &frame(time + 1))?;
        lines.push(format!("(assert {step})"));
    }
    lines.push(format!("(assert {})", bmc_target(&lowered, target, depth)?));
    lines.push("(check-sat)".to_string());
    let mut gets = Vec::new();
    for time in 0..=depth {
        gets.extend(value_symbols(&lowered.names, &frame(time), time < depth));
        gets.push(format!("|{}cmd|", frame(time)));
        gets.push(format!("|{}out|", frame(time)));
    }
    lines.push(format!("(get-value ({}))", gets.join(" ")));
    Ok(SmtQuery {
        text: join_lines(&lines),
        kind: QueryKind::Bmc,
        names: lowered.names,
        get_values: gets,
    })
}

pub fn encode_horn(def: &Definition, target: &Target) -> Result<SmtQuery, Unsupported> {
    let lowered = lower_definition(def)?;
    check_machine_fragment(&lowered)?;
    let pre = reach_args(&lowered.names, "");
    let post = reach_args(&lowered.names, "p:");
    let mut lines = vec![
        format!("; ik.smt.horn target={target:?}"),
        "(set-logic HORN)".to_string(),
        format!("(declare-fun Reach ({}) Bool)", reach_sig(&lowered.names)),
    ];
    let init_bind = config_binders(&lowered.names, "");
    let init_ranges = range_conjunct(&lowered.names, "", false);
    let init = init_formula(&lowered, "")?;
    lines.push(format!(
        "(assert (forall ({init_bind}) (=> (and {init_ranges} {init}) (Reach {pre}))))"
    ));
    let step_bind = horn_step_binders(&lowered);
    let pre_ranges = range_conjunct(&lowered.names, "", false);
    let post_ranges = range_conjunct(&lowered.names, "p:", false);
    let arg_ranges = arg_range_conjunct(&lowered);
    let step = step_formula(&lowered, "", "p:")?;
    lines.push(format!(
        "(assert (forall ({step_bind}) (=> (and {pre_ranges} {post_ranges} {arg_ranges} (Reach {pre}) {step}) (Reach {post}))))"
    ));
    lines.push(horn_query(&lowered, target)?);
    lines.push("(check-sat)".to_string());
    lines.push("(get-model)".to_string());
    Ok(SmtQuery {
        text: join_lines(&lines),
        kind: QueryKind::Horn,
        names: lowered.names,
        get_values: Vec::new(),
    })
}

pub fn dump_query(stem: &str, query: &SmtQuery) {
    let Ok(dir) = std::env::var("SMT_QUERY_DIR") else {
        return;
    };
    let path = std::path::Path::new(&dir);
    let _ = std::fs::create_dir_all(path);
    let safe: String = stem
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    let dest = path.join(format!("{safe}.smt2"));
    if query.text.len() > 1_500_000 {
        let note = format!(
            "; omitted {}-byte query (above dump cap); kind={:?}\n",
            query.text.len(),
            query.kind
        );
        let _ = std::fs::write(dest, note);
        return;
    }
    let _ = std::fs::write(dest, &query.text);
}

fn names_for_expr(expr: &Expr, pinned: &Pinned) -> Names {
    let mut facts = BTreeMap::new();
    let mut args = BTreeMap::new();
    let mut strings = super::encode::collect_strings(expr);
    strings.insert(pinned.state.clone());
    note_value_strings(&pinned.facts, &mut strings);
    for value in pinned.args.values() {
        if let Some(s) = value.as_str() {
            strings.insert(s.to_string());
        }
    }
    collect_named_sorts(expr, &pinned.facts, &pinned.args, &mut facts, &mut args);
    for (name, value) in &pinned.facts {
        facts
            .entry(name.clone())
            .or_insert_with(|| value_sort(value.as_ref()));
    }
    for (name, value) in &pinned.args {
        args.entry(name.clone())
            .or_insert_with(|| value_sort(Some(value)));
    }
    Names {
        facts,
        args,
        strings: super::encode::StringPart::new(strings),
        states: vec![pinned.state.clone()],
    }
}

fn note_value_strings(
    facts: &BTreeMap<String, Option<Value>>,
    strings: &mut std::collections::BTreeSet<String>,
) {
    for value in facts.values().flatten() {
        if let Some(s) = value.as_str() {
            strings.insert(s.to_string());
        }
    }
}

fn collect_named_sorts(
    expr: &Expr,
    facts: &BTreeMap<String, Option<Value>>,
    args: &BTreeMap<String, Value>,
    fact_sorts: &mut BTreeMap<String, Sort>,
    arg_sorts: &mut BTreeMap<String, Sort>,
) {
    match expr {
        Expr::And(parts) | Expr::Or(parts) => {
            for part in parts {
                collect_named_sorts(part, facts, args, fact_sorts, arg_sorts);
            }
        }
        Expr::Cmp(cmp) => {
            note_operand(&cmp.left, facts, args, fact_sorts, arg_sorts);
            note_operand(&cmp.right, facts, args, fact_sorts, arg_sorts);
        }
        Expr::Exists(name) | Expr::NotExists(name) => {
            fact_sorts
                .entry(name.clone())
                .or_insert_with(|| value_sort(facts.get(name).and_then(|v| v.as_ref())));
        }
        Expr::TextIn(pred) => note_operand(&pred.value, facts, args, fact_sorts, arg_sorts),
    }
}

fn note_operand(
    op: &crate::declarative::expr::Operand,
    facts: &BTreeMap<String, Option<Value>>,
    args: &BTreeMap<String, Value>,
    fact_sorts: &mut BTreeMap<String, Sort>,
    arg_sorts: &mut BTreeMap<String, Sort>,
) {
    use crate::declarative::expr::Operand;
    match op {
        Operand::Fact(name) => {
            fact_sorts
                .entry(name.clone())
                .or_insert_with(|| value_sort(facts.get(name).and_then(|v| v.as_ref())));
        }
        Operand::Arg(name) => {
            arg_sorts
                .entry(name.clone())
                .or_insert_with(|| value_sort(args.get(name)));
        }
        Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => {
            note_operand(base, facts, args, fact_sorts, arg_sorts);
        }
        Operand::BinArith { left, right, .. } => {
            note_operand(left, facts, args, fact_sorts, arg_sorts);
            note_operand(right, facts, args, fact_sorts, arg_sorts);
        }
        Operand::State
        | Operand::EffectStatus
        | Operand::EffectKind
        | Operand::EffectOutcome
        | Operand::Literal(_) => {}
    }
}

fn value_sort(value: Option<&Value>) -> Sort {
    match value {
        Some(v) if v.as_i64().is_some() => Sort::Int,
        Some(v) if v.as_bool().is_some() => Sort::Bool,
        Some(v) if v.as_str().is_some() => Sort::Str,
        _ => Sort::Int,
    }
}

fn declare_names(names: &Names, frame: &str, with_args: bool) -> Vec<String> {
    let mut lines = vec![format!("(declare-const |{frame}st| Int)")];
    for (name, sort) in &names.facts {
        lines.push(format!(
            "(declare-const |{frame}fv:{name}| {})",
            sort_name(*sort)
        ));
        lines.push(format!("(declare-const |{frame}fs:{name}| Bool)"));
    }
    if with_args {
        for (name, sort) in &names.args {
            lines.push(format!(
                "(declare-const |{frame}av:{name}| {})",
                sort_name(*sort)
            ));
        }
    }
    lines
}

fn range_asserts(names: &Names, frame: &str, with_args: bool) -> Vec<String> {
    range_terms(names, frame, with_args)
        .into_iter()
        .map(|term| format!("(assert {term})"))
        .collect()
}

fn range_conjunct(names: &Names, frame: &str, with_args: bool) -> String {
    let terms = range_terms(names, frame, with_args);
    if terms.is_empty() {
        "true".to_string()
    } else {
        conjunct(&terms)
    }
}

fn range_terms(names: &Names, frame: &str, with_args: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let nstates = names.states.len();
    lines.push(format!(
        "(and (>= |{frame}st| 0) (< |{frame}st| {nstates}))"
    ));
    for (name, sort) in &names.facts {
        match sort {
            Sort::Int => {
                let v = format!("|{frame}fv:{name}|");
                lines.push(in_i64_range(&v));
            }
            Sort::Str => {
                let v = format!("|{frame}fv:{name}|");
                lines.push(in_string_id_range(&v));
            }
            Sort::Bool => {}
        }
    }
    if with_args {
        lines.extend(arg_range_terms(names, frame));
    }
    lines
}

fn arg_range_conjunct(lowered: &Lowered) -> String {
    let mut names = lowered.names.clone();
    names.args = BTreeMap::new();
    for trans in &lowered.transitions {
        for (name, sort) in &trans.args {
            names.args.insert(name.clone(), *sort);
        }
    }
    let terms = arg_range_terms(&names, "");
    if terms.is_empty() {
        "true".to_string()
    } else {
        conjunct(&terms)
    }
}

fn arg_range_terms(names: &Names, frame: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for (name, sort) in &names.args {
        match sort {
            Sort::Int => {
                let v = format!("|{frame}av:{name}|");
                lines.push(in_i64_range(&v));
            }
            Sort::Str => {
                let v = format!("|{frame}av:{name}|");
                lines.push(in_string_id_range(&v));
            }
            Sort::Bool => {}
        }
    }
    lines
}

fn value_symbols(names: &Names, frame: &str, with_args: bool) -> Vec<String> {
    let mut out = vec![format!("|{frame}st|")];
    for name in names.facts.keys() {
        out.push(format!("|{frame}fv:{name}|"));
        out.push(format!("|{frame}fs:{name}|"));
    }
    if with_args {
        for name in names.args.keys() {
            out.push(format!("|{frame}av:{name}|"));
        }
    }
    out
}

fn sort_name(sort: Sort) -> &'static str {
    match sort {
        Sort::Int | Sort::Str => "Int",
        Sort::Bool => "Bool",
    }
}

fn frame(time: usize) -> String {
    format!("t{time}:")
}

fn pin_formula(names: &Names, pinned: &Pinned) -> Result<String, Unsupported> {
    let mut parts = Vec::new();
    let st = names
        .state_id(&pinned.state)
        .ok_or_else(|| Unsupported::new("pinned state is not in the partition"))?;
    parts.push(format!("(= {} {})", state_sym(), smt_int(st)));
    for (name, sort) in &names.facts {
        match pinned.facts.get(name) {
            Some(Some(value)) => {
                parts.push(format!("(= {} true)", fact_set(name)));
                parts.push(format!(
                    "(= {} {})",
                    fact_val(name),
                    lit_smt(value, *sort, names)?
                ));
            }
            _ => parts.push(format!("(= {} false)", fact_set(name))),
        }
    }
    for (name, sort) in &names.args {
        let value = pinned
            .args
            .get(name)
            .ok_or_else(|| Unsupported::new(format!("pinned arg `{name}` missing")))?;
        parts.push(format!(
            "(= {} {})",
            arg_val(name),
            lit_smt(value, *sort, names)?
        ));
    }
    Ok(conjunct(&parts))
}

fn lit_smt(value: &Value, sort: Sort, names: &Names) -> Result<String, Unsupported> {
    match sort {
        Sort::Int => value
            .as_i64()
            .map(smt_int)
            .ok_or_else(|| Unsupported::new("pinned int is not i64")),
        Sort::Bool => match value.as_bool() {
            Some(true) => Ok("true".into()),
            Some(false) => Ok("false".into()),
            None => Err(Unsupported::new("pinned bool is not bool")),
        },
        Sort::Str => value
            .as_str()
            .and_then(|s| names.strings.try_id(s).map(smt_int))
            .ok_or_else(|| Unsupported::new("pinned string is not in the collected partition")),
    }
}

fn selectable_formula(
    trans: &Transition,
    names: &Names,
    frame: &str,
) -> Result<String, Unsupported> {
    let src = names
        .state_id(&trans.source)
        .ok_or_else(|| Unsupported::new("source state missing"))?;
    let mut parts = vec![format!("(= |{frame}st| {})", smt_int(src))];
    for earlier in &trans.earlier {
        parts.push(guard_is(earlier, names, frame, false)?);
    }
    parts.push(guard_is(&trans.when, names, frame, true)?);
    Ok(conjunct(&parts))
}

fn guard_is(
    when: &Option<Expr>,
    names: &Names,
    frame: &str,
    want_true: bool,
) -> Result<String, Unsupported> {
    let Some(expr) = when else {
        return Ok(if want_true {
            "true".to_string()
        } else {
            "false".to_string()
        });
    };
    let term = encode_expr(expr, names)?;
    let value = apply_frame(&term.value, frame);
    let error = apply_frame(&term.error, frame);
    if want_true {
        Ok(format!("(and (not {error}) {value})"))
    } else {
        Ok(format!("(and (not {error}) (not {value}))"))
    }
}

fn apply_frame(text: &str, frame: &str) -> String {
    apply_state_frame(&retarget(text, "|av:", frame), frame)
}

fn apply_state_frame(text: &str, frame: &str) -> String {
    retarget(
        &retarget(&retarget(text, "|fv:", frame), "|fs:", frame),
        "|st|",
        frame,
    )
}

fn retarget(text: &str, needle: &str, frame: &str) -> String {
    if frame.is_empty() {
        return text.to_string();
    }
    if needle == "|st|" {
        return text.replace("|st|", &format!("|{frame}st|"));
    }
    text.replace(needle, &format!("|{frame}{}", &needle[1..]))
}

fn init_formula(lowered: &Lowered, frame: &str) -> Result<String, Unsupported> {
    let st = lowered
        .names
        .state_id(&lowered.initial_state)
        .ok_or_else(|| Unsupported::new("initial state missing"))?;
    let mut parts = vec![format!("(= |{frame}st| {})", smt_int(st))];
    for (name, sort) in &lowered.names.facts {
        match lowered.initial_facts.get(name) {
            Some(value) => {
                parts.push(format!("(= |{frame}fs:{name}| true)"));
                parts.push(format!(
                    "(= |{frame}fv:{name}| {})",
                    lit_smt(value, *sort, &lowered.names)?
                ));
            }
            None if lowered.is_counter(name) => {
                parts.push(format!("(= |{frame}fs:{name}| true)"));
                parts.push(format!("(= |{frame}fv:{name}| 0)"));
            }
            None => parts.push(format!("(= |{frame}fs:{name}| false)")),
        }
    }
    Ok(conjunct(&parts))
}

fn step_formula(lowered: &Lowered, pre: &str, post: &str) -> Result<String, Unsupported> {
    let mut disj = Vec::new();
    for trans in &lowered.transitions {
        disj.push(transition_formula(lowered, trans, pre, post)?);
    }
    let step = super::encode::disjunct(&disj);
    let inv = invariants_on(lowered, post)?;
    Ok(format!("(and {step} {inv})"))
}

fn transition_formula(
    lowered: &Lowered,
    trans: &Transition,
    pre: &str,
    post: &str,
) -> Result<String, Unsupported> {
    let dest = lowered
        .names
        .state_id(&trans.dest)
        .ok_or_else(|| Unsupported::new("dest state missing"))?;
    let mut parts = vec![
        format!("(= |{pre}cmd| {})", smt_int(trans.command_index as i64)),
        format!("(= |{pre}out| {})", smt_int(trans.outcome_index as i64)),
        selectable_formula(trans, &arg_names(&lowered.names, trans), pre)?,
        format!("(= |{post}st| {})", smt_int(dest)),
        format!("(= |{post}cmd| {})", smt_int(trans.command_index as i64)),
        format!("(= |{post}out| {})", smt_int(trans.outcome_index as i64)),
    ];
    parts.extend(assignment_formulas(lowered, trans, pre, post)?);
    Ok(conjunct(&parts))
}

fn arg_names(names: &Names, trans: &Transition) -> Names {
    let mut out = names.clone();
    out.args = trans.args.clone();
    out
}

fn assignment_formulas(
    lowered: &Lowered,
    trans: &Transition,
    pre: &str,
    post: &str,
) -> Result<Vec<String>, Unsupported> {
    let written: BTreeMap<&str, &ThenOp> = trans
        .then
        .iter()
        .map(|(name, op)| (name.as_str(), op))
        .collect();
    let mut parts = Vec::new();
    for (name, sort) in &lowered.names.facts {
        match written.get(name.as_str()) {
            Some(op) => parts.push(write_fact(name, *sort, op, lowered, pre, post)?),
            None => {
                parts.push(format!("(= |{post}fv:{name}| |{pre}fv:{name}|)"));
                parts.push(format!("(= |{post}fs:{name}| |{pre}fs:{name}|)"));
            }
        }
    }
    Ok(parts)
}

fn write_fact(
    name: &str,
    sort: Sort,
    op: &ThenOp,
    lowered: &Lowered,
    pre: &str,
    post: &str,
) -> Result<String, Unsupported> {
    let value = match op {
        ThenOp::Literal(v) => lit_smt(v, sort, &lowered.names)?,
        ThenOp::Arg(arg) => format!("|{pre}av:{arg}|"),
        ThenOp::CounterIncr(n) => {
            let sum = format!("(+ |{pre}fv:{name}| {})", smt_int(*n));
            let ov = format!("(or (< {sum} {I64_MIN_SMT}) (> {sum} {I64_MAX_SMT}))");
            let assign =
                format!("(and (not {ov}) (= |{post}fv:{name}| {sum}) (= |{post}fs:{name}| true))");
            return Ok(assign);
        }
    };
    Ok(format!(
        "(and (= |{post}fv:{name}| {value}) (= |{post}fs:{name}| true))"
    ))
}

fn invariants_on(lowered: &Lowered, post: &str) -> Result<String, Unsupported> {
    let mut parts = Vec::new();
    for inv in &lowered.invariants {
        parts.push(invariant_ok(inv, &lowered.names, post)?);
    }
    Ok(conjunct(&parts))
}

fn invariant_ok(
    inv: &super::machine::Invariant,
    names: &Names,
    frame: &str,
) -> Result<String, Unsupported> {
    let assert = encode_expr(&inv.assert, names)?;
    let a_err = apply_state_frame(&assert.error, frame);
    let a_val = apply_state_frame(&assert.value, frame);
    let Some(when) = &inv.when else {
        return Ok(format!("(and (not {a_err}) {a_val})"));
    };
    let gate = encode_expr(when, names)?;
    let g_err = apply_state_frame(&gate.error, frame);
    let g_val = apply_state_frame(&gate.value, frame);
    Ok(format!(
        "(and (not {g_err}) (or (not {g_val}) (and (not {a_err}) {a_val})))"
    ))
}

fn bmc_target(lowered: &Lowered, target: &Target, depth: usize) -> Result<String, Unsupported> {
    let mut parts = Vec::new();
    match target {
        Target::State(name) | Target::Terminal(name) => {
            let id = lowered
                .names
                .state_id(name)
                .ok_or_else(|| Unsupported::new("target state missing"))?;
            for time in 0..=depth {
                parts.push(format!("(= |{}st| {})", frame(time), smt_int(id)));
            }
        }
        Target::Outcome { command, index } => {
            let cmd = command_index(lowered, command)?;
            for time in 0..depth {
                parts.push(format!(
                    "(and (= |{}cmd| {}) (= |{}out| {}))",
                    frame(time),
                    smt_int(cmd),
                    frame(time),
                    smt_int(*index as i64)
                ));
            }
        }
        Target::CommandFrom { command, source } => {
            let cmd = command_index(lowered, command)?;
            let st = lowered
                .names
                .state_id(source)
                .ok_or_else(|| Unsupported::new("source state missing"))?;
            for time in 0..depth {
                parts.push(format!(
                    "(and (= |{}st| {}) (= |{}cmd| {}))",
                    frame(time),
                    smt_int(st),
                    frame(time),
                    smt_int(cmd)
                ));
            }
        }
    }
    Ok(super::encode::disjunct(&parts))
}

fn horn_query(lowered: &Lowered, target: &Target) -> Result<String, Unsupported> {
    match target {
        Target::State(name) | Target::Terminal(name) => {
            let id = lowered
                .names
                .state_id(name)
                .ok_or_else(|| Unsupported::new("target state missing"))?;
            Ok(format!(
                "(assert (forall ({}) (=> (and (Reach {}) (= |st| {})) false)))",
                config_binders(&lowered.names, ""),
                reach_args(&lowered.names, ""),
                smt_int(id)
            ))
        }
        Target::Outcome { command, index } => {
            horn_selectable(lowered, &selectable_any_formula(lowered, command, *index)?)
        }
        Target::CommandFrom { command, source } => {
            horn_selectable(lowered, &selectable_from_formula(lowered, command, source)?)
        }
    }
}

fn horn_selectable(lowered: &Lowered, sel: &str) -> Result<String, Unsupported> {
    let mut bind = config_binders(&lowered.names, "");
    let args = arg_binders(lowered);
    if !args.is_empty() {
        bind.push(' ');
        bind.push_str(&args);
    }
    Ok(format!(
        "(assert (forall ({bind}) (=> (and (Reach {}) {sel}) false)))",
        reach_args(&lowered.names, "")
    ))
}

fn selectable_from_formula(
    lowered: &Lowered,
    command: &str,
    source: &str,
) -> Result<String, Unsupported> {
    let mut disj = Vec::new();
    for trans in &lowered.transitions {
        if trans.command == command && trans.source == source {
            check_transition_fragment(trans, &arg_names(&lowered.names, trans))?;
            disj.push(selectable_formula(
                trans,
                &arg_names(&lowered.names, trans),
                "",
            )?);
        }
    }
    if disj.is_empty() {
        return Ok("false".to_string());
    }
    Ok(super::encode::disjunct(&disj))
}

fn selectable_any_formula(
    lowered: &Lowered,
    command: &str,
    outcome: usize,
) -> Result<String, Unsupported> {
    let mut disj = Vec::new();
    for trans in &lowered.transitions {
        if trans.command == command && trans.outcome_index == outcome {
            check_transition_fragment(trans, &arg_names(&lowered.names, trans))?;
            disj.push(selectable_formula(
                trans,
                &arg_names(&lowered.names, trans),
                "",
            )?);
        }
    }
    if disj.is_empty() {
        return Err(Unsupported::new("no such command/outcome"));
    }
    Ok(super::encode::disjunct(&disj))
}

fn command_index(lowered: &Lowered, command: &str) -> Result<i64, Unsupported> {
    lowered
        .commands
        .iter()
        .position(|name| name == command)
        .map(|i| i as i64)
        .ok_or_else(|| Unsupported::new(format!("unknown command `{command}`")))
}

fn check_machine_fragment(lowered: &Lowered) -> Result<(), Unsupported> {
    for trans in &lowered.transitions {
        check_transition_fragment(trans, &arg_names(&lowered.names, trans))?;
    }
    for inv in &lowered.invariants {
        if let Some(err) = super::fragment_error(&inv.assert, &lowered.names) {
            return Err(err);
        }
        if let Some(when) = &inv.when {
            if let Some(err) = super::fragment_error(when, &lowered.names) {
                return Err(err);
            }
        }
    }
    Ok(())
}

fn check_transition_fragment(trans: &Transition, names: &Names) -> Result<(), Unsupported> {
    if let Some(when) = &trans.when {
        if let Some(err) = super::fragment_error(when, names) {
            return Err(err);
        }
    }
    for earlier in trans.earlier.iter().flatten() {
        if let Some(err) = super::fragment_error(earlier, names) {
            return Err(err);
        }
    }
    Ok(())
}

fn reach_sig(names: &Names) -> String {
    let mut sorts = vec!["Int".to_string()];
    for sort in names.facts.values() {
        sorts.push(sort_name(*sort).to_string());
        sorts.push("Bool".to_string());
    }
    sorts.join(" ")
}

fn reach_args(names: &Names, frame: &str) -> String {
    let mut args = vec![format!("|{frame}st|")];
    for name in names.facts.keys() {
        args.push(format!("|{frame}fv:{name}|"));
        args.push(format!("|{frame}fs:{name}|"));
    }
    args.join(" ")
}

fn config_binders(names: &Names, frame: &str) -> String {
    let mut out = vec![format!("(|{frame}st| Int)")];
    for (name, sort) in &names.facts {
        out.push(format!("(|{frame}fv:{name}| {})", sort_name(*sort)));
        out.push(format!("(|{frame}fs:{name}| Bool)"));
    }
    out.join(" ")
}

fn horn_step_binders(lowered: &Lowered) -> String {
    let mut parts = vec![
        config_binders(&lowered.names, ""),
        "(|cmd| Int) (|out| Int)".to_string(),
    ];
    let args = arg_binders(lowered);
    if !args.is_empty() {
        parts.push(args);
    }
    parts.push(config_binders(&lowered.names, "p:"));
    parts.push("(|p:cmd| Int) (|p:out| Int)".to_string());
    parts.join(" ")
}

fn arg_binders(lowered: &Lowered) -> String {
    let mut seen = BTreeMap::new();
    for trans in &lowered.transitions {
        for (name, sort) in &trans.args {
            seen.insert(name.clone(), *sort);
        }
    }
    let mut out = Vec::new();
    for (name, sort) in seen {
        out.push(format!("(|av:{name}| {})", sort_name(sort)));
    }
    out.join(" ")
}

fn join_lines(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// `sat` on these queries means the named condition failed.
pub fn encode_invariant_init(def: &Definition, invariant: &str) -> Result<SmtQuery, Unsupported> {
    let lowered = lower_definition(def)?;
    check_machine_fragment(&lowered)?;
    let inv = apply_state_frame(invariant, "");
    let mut lines = invariant_header("init");
    lines.extend(declare_names(&lowered.names, "", false));
    lines.extend(range_asserts(&lowered.names, "", false));
    lines.push(format!("(assert {})", init_formula(&lowered, "")?));
    lines.push(format!("(assert (not {inv}))"));
    lines.push("(check-sat)".to_string());
    let gets = value_symbols(&lowered.names, "", false);
    Ok(smt_query(lines, lowered.names, gets))
}

pub fn encode_invariant_step(def: &Definition, invariant: &str) -> Result<SmtQuery, Unsupported> {
    let lowered = lower_definition(def)?;
    check_machine_fragment(&lowered)?;
    let pre = apply_state_frame(invariant, "");
    let post = apply_state_frame(invariant, "p:");
    let mut lines = invariant_header("step");
    lines.extend(declare_names(&lowered.names, "", true));
    lines.push("(declare-const |cmd| Int)".to_string());
    lines.push("(declare-const |out| Int)".to_string());
    lines.extend(declare_names(&lowered.names, "p:", false));
    lines.push("(declare-const |p:cmd| Int)".to_string());
    lines.push("(declare-const |p:out| Int)".to_string());
    lines.extend(range_asserts(&lowered.names, "", true));
    lines.extend(range_asserts(&lowered.names, "p:", false));
    lines.push(format!("(assert {pre})"));
    lines.push(format!("(assert {})", step_formula(&lowered, "", "p:")?));
    lines.push(format!("(assert (not {post}))"));
    lines.push("(check-sat)".to_string());
    let mut gets = value_symbols(&lowered.names, "", true);
    gets.push("|cmd|".into());
    gets.push("|out|".into());
    gets.extend(value_symbols(&lowered.names, "p:", false));
    gets.push("|p:cmd|".into());
    gets.push("|p:out|".into());
    Ok(smt_query(lines, lowered.names, gets))
}

pub fn encode_invariant_exclude(
    def: &Definition,
    invariant: &str,
    target: &Target,
) -> Result<SmtQuery, Unsupported> {
    let lowered = lower_definition(def)?;
    check_machine_fragment(&lowered)?;
    let inv = apply_state_frame(invariant, "");
    let with_args = matches!(target, Target::Outcome { .. } | Target::CommandFrom { .. });
    let mut lines = invariant_header("exclude");
    lines.extend(declare_names(&lowered.names, "", with_args));
    lines.extend(range_asserts(&lowered.names, "", with_args));
    lines.push(format!("(assert {inv})"));
    lines.push(format!("(assert {})", exclude_formula(&lowered, target)?));
    lines.push("(check-sat)".to_string());
    let gets = value_symbols(&lowered.names, "", with_args);
    Ok(smt_query(lines, lowered.names, gets))
}

fn exclude_formula(lowered: &Lowered, target: &Target) -> Result<String, Unsupported> {
    match target {
        Target::State(name) | Target::Terminal(name) => {
            let id = lowered
                .names
                .state_id(name)
                .ok_or_else(|| Unsupported::new("target state missing"))?;
            Ok(format!("(= |st| {})", smt_int(id)))
        }
        Target::Outcome { command, index } => selectable_any_formula(lowered, command, *index),
        Target::CommandFrom { command, source } => {
            selectable_from_formula(lowered, command, source)
        }
    }
}

fn invariant_header(which: &str) -> Vec<String> {
    vec![
        format!("; ik.smt.invariant-{which}"),
        "(set-logic QF_LIA)".to_string(),
        "(set-option :produce-models true)".to_string(),
    ]
}

fn smt_query(mut lines: Vec<String>, names: Names, gets: Vec<String>) -> SmtQuery {
    if !gets.is_empty() {
        lines.push(format!("(get-value ({}))", gets.join(" ")));
    }
    SmtQuery {
        text: join_lines(&lines),
        kind: QueryKind::Selectability,
        names,
        get_values: gets,
    }
}
#[cfg(test)]
mod guard_language_qualification {
    use super::*;
    use crate::declarative::expr;
    use crate::smt::{discover_solvers, run_smt2, SolverReply};
    use std::time::Duration;

    #[test]
    #[ignore = "requires native Z3; dedicated qualification"]
    fn parsed_add_guard_matches_i64_tag_oracle_and_false_controls() {
        let solver = discover_solvers().0.expect("Z3 required for guard qualification");
        let expr = expr::parse("$n + 1 > 0").expect("parse production guard grammar");
        let pinned = Pinned {
            state: "idle".into(),
            facts: BTreeMap::new(),
            args: BTreeMap::new(),
        };
        let names = names_for_expr(&expr, &pinned);
        let term = encode_expr(&expr, &names).expect("native expression encoder");
        let mut base = vec![
            "(set-logic QF_LIA)".to_string(),
            "(set-option :produce-models true)".to_string(),
        ];
        base.extend(declare_names(&names, "", true));
        base.extend(range_asserts(&names, "", true));
        base.push("(declare-const |g:err| Bool)".into());
        base.push("(declare-const |g:val| Bool)".into());
        let mut lines = base.clone();
        lines.push(format!("(assert (= |g:err| {}))", term.error));
        lines.push(format!("(assert (= |g:val| {}))", term.value));
        let n = "|av:n|";
        let mismatch = format!(
            "(or (not (= |g:err| (= {n} 9223372036854775807))) \
             (and (not |g:err|) \
               (not (= |g:val| (and (not (= {n} 9223372036854775807)) (>= {n} 0))))))"
        );
        lines.push(format!("(assert {mismatch})"));
        lines.push("(check-sat)".into());
        lines.push(format!("(get-value ({n}))"));
        let result = run_smt2(&solver, &join_lines(&lines), Duration::from_secs(10))
            .expect("run tag disagreement query");
        assert_eq!(result.reply, SolverReply::Unsat, "{}", result.raw);

        let mut control = base;
        control.push(format!("(assert (= {n} 9223372036854775807))"));
        control.push(format!("(assert (= |g:err| {}))", term.error));
        control.push(format!("(assert (= |g:val| {}))", term.value));
        control.push("(declare-const |oracle:err| Bool)".into());
        control.push("(assert (= |oracle:err| false))".into());
        control.push("(assert (not (= |g:err| |oracle:err|)))".into());
        control.push("(check-sat)".into());
        control.push(format!("(get-value ({n}))"));
        let result = run_smt2(&solver, &join_lines(&control), Duration::from_secs(10))
            .expect("run false-overflow-oracle control");
        assert_eq!(result.reply, SolverReply::Sat, "{}", result.raw);
        assert_eq!(result.model.ints.get(n), Some(&i64::MAX), "{}", result.raw);
    }

    #[test]
    #[ignore = "requires native Z3; dedicated qualification"]
    fn overflow_fallthrough_mutant_is_sat_at_max() {
        let solver = discover_solvers().0.expect("Z3 required for guard qualification");
        let def = crate::declarative::parse_definition(include_str!(
            "../../tests/fixtures/feasibility_truth/machines/overflow_guard_add_error.ik.toml"
        ))
        .expect("parse fixture");
        let lowered = lower_definition(&def).expect("native lowering");
        let later = lowered
            .outcomes_for("go", "idle")
            .into_iter()
            .find(|row| row.outcome_index == 1)
            .expect("later outcome");
        let earlier = later.earlier[0].as_ref().expect("earlier guard");
        let mut names = lowered.names.clone();
        names.args = later.args.clone();
        let first = encode_expr(earlier, &names).expect("native earlier guard");
        let next = encode_expr(later.when.as_ref().expect("later guard"), &names)
            .expect("native later guard");
        let n = "|av:n|";
        let mut lines = vec!["(set-logic QF_LIA)".into()];
        lines.extend(declare_names(&names, "", true));
        lines.extend(range_asserts(&names, "", true));
        lines.push(format!("(assert (= {n} 9223372036854775807))"));
        lines.push(format!("(assert {})", first.error));
        lines.push(format!("(assert (not {}))", next.error));
        lines.push(format!("(assert {})", next.value));
        lines.push("(check-sat)".into());
        lines.push(format!("(get-value ({n}))"));
        let result = run_smt2(&solver, &join_lines(&lines), Duration::from_secs(10))
            .expect("run error-as-false fallthrough mutant");
        assert_eq!(result.reply, SolverReply::Sat, "{}", result.raw);
        assert_eq!(result.model.ints.get(n), Some(&i64::MAX), "{}", result.raw);
    }
}
