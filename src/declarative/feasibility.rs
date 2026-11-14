//! Sound impossibility proofs for positive success obligations.
//!
//! The in-house engine returns [`Feasibility`]: `ProvenImpossible` with an
//! explicit scope and basis, or `Unknown`. It does not emit witness candidates;
//! a later backend may. Authored behavior is never satisfied by a proof.
//! Generated product-context obligations may be discharged only when the proof
//! is inside the sound fragment. String ordering over unbounded argument
//! domains is outside that fragment.

use super::def::{CommandDef, Definition, OutcomeDef};
use super::expr::{self, CmpOp, Expr, Operand};
use super::plan::{ImpossibilityRecordV1, SemanticCoordinate};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const SOURCE_REASON: &str =
    "successful commit is impossible at this source: no ordered outcome can be selected given retained product assignments";
const OUTCOME_REASON: &str = "successful commit is impossible";
const REACH_REASON: &str = "successful commit is impossible";
const ARG_CASE_LIMIT: usize = 256;
const FLOW_STEP_LIMIT: usize = 10_000;

/// Three-result feasibility judgment. Witness candidates are not evidence until
/// the kernel replays them; this engine leaves that variant unused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Feasibility {
    #[allow(dead_code)]
    WitnessCandidate {
        generated: bool,
    },
    ProvenImpossible {
        scope: String,
        basis: String,
    },
    Unknown {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Abs {
    Exact(Value),
    Top,
}

struct FactFlow {
    reachable: BTreeMap<String, BTreeMap<String, Abs>>,
    complete: bool,
}

pub fn impossible_success_records(def: &Definition) -> Vec<ImpossibilityRecordV1> {
    let flow = analyze_flow(def);
    let mut records = Vec::new();
    for (command, cmd) in &def.commands {
        records.extend(impossible_command_sources(def, command, cmd));
        records.extend(impossible_outcomes(def, command, cmd, &flow));
    }
    records.extend(impossible_states(def, &flow));
    records.sort_by(|left, right| left.obligation_id.cmp(&right.obligation_id));
    records
}

fn impossible_command_sources(
    def: &Definition,
    command: &str,
    cmd: &CommandDef,
) -> Vec<ImpossibilityRecordV1> {
    let mut records = Vec::new();
    for source in &cmd.from {
        let Feasibility::ProvenImpossible { scope, basis } = command_feasibility(cmd, def, source)
        else {
            continue;
        };
        let id = format!("command/{command}/from/{source}/executable");
        records.push(success_record(
            id,
            "OBL-CMD-001",
            "command_source",
            SOURCE_REASON,
            source,
            SemanticCoordinate {
                command: Some(command.to_string()),
                source: Some(source.clone()),
                ..SemanticCoordinate::default()
            },
            def,
            scope,
            basis,
        ));
    }
    records
}

fn impossible_outcomes(
    def: &Definition,
    command: &str,
    cmd: &CommandDef,
    flow: &FactFlow,
) -> Vec<ImpossibilityRecordV1> {
    let mut records = Vec::new();
    let proving = cmd
        .from
        .first()
        .cloned()
        .unwrap_or_else(|| command.to_string());
    for index in 0..arm_count(cmd) {
        let Feasibility::ProvenImpossible { scope, basis } =
            outcome_feasibility(cmd, def, index, flow)
        else {
            continue;
        };
        let reason = format!("{OUTCOME_REASON}: {basis}");
        records.push(success_record(
            format!("outcome/{command}/{index}/executable"),
            "OBL-OUT-001",
            "ordered_outcome",
            &reason,
            &proving,
            SemanticCoordinate {
                command: Some(command.to_string()),
                outcome_index: Some(index),
                ..SemanticCoordinate::default()
            },
            def,
            scope,
            basis,
        ));
    }
    records
}

fn impossible_states(def: &Definition, flow: &FactFlow) -> Vec<ImpossibilityRecordV1> {
    let mut records = Vec::new();
    for (name, state) in &def.states {
        if state.initial {
            continue;
        }
        let Feasibility::ProvenImpossible { scope, basis } = state_feasibility(def, name, flow)
        else {
            continue;
        };
        records.extend(reachability_records(
            def,
            name,
            state.terminal,
            &scope,
            &basis,
        ));
    }
    records
}

fn reachability_records(
    def: &Definition,
    name: &str,
    terminal: bool,
    scope: &str,
    basis: &str,
) -> Vec<ImpossibilityRecordV1> {
    let reason = format!("{REACH_REASON}: {basis}");
    let coordinate = SemanticCoordinate {
        state: Some(name.to_string()),
        ..SemanticCoordinate::default()
    };
    let mut records = vec![success_record(
        format!("state/{name}/executable"),
        "OBL-STATE-001",
        "reachability",
        &reason,
        name,
        coordinate.clone(),
        def,
        scope.to_string(),
        basis.to_string(),
    )];
    if terminal {
        records.push(success_record(
            format!("terminal/{name}/reachable"),
            "OBL-TERM-001",
            "reachability",
            &reason,
            name,
            coordinate,
            def,
            scope.to_string(),
            basis.to_string(),
        ));
    }
    records
}

fn success_record(
    obligation_id: String,
    kind: &str,
    class: &str,
    reason: &str,
    proving_source: &str,
    coordinate: SemanticCoordinate,
    def: &Definition,
    scope: String,
    basis: String,
) -> ImpossibilityRecordV1 {
    ImpossibilityRecordV1 {
        obligation_id: obligation_id.clone(),
        kind: kind.to_string(),
        class: class.to_string(),
        discharged: discharge_allowed(def, &obligation_id, class),
        reason: reason.to_string(),
        proving_source: proving_source.to_string(),
        coordinate,
        basis,
        scope,
        trust: String::new(),
        certificate_digest: String::new(),
        solvers: Vec::new(),
    }
}

fn discharge_allowed(def: &Definition, obligation_id: &str, class: &str) -> bool {
    if obligation_id.starts_with("authored/") {
        return false;
    }
    match class {
        "command_source" => true,
        "ordered_outcome" | "reachability" => !def.origin.is_empty(),
        _ => false,
    }
}

fn command_feasibility(cmd: &CommandDef, def: &Definition, source: &str) -> Feasibility {
    match command_success(cmd, def, source) {
        Some(false) => Feasibility::ProvenImpossible {
            scope: format!("source={source}"),
            basis: "guard_false".to_string(),
        },
        Some(true) => Feasibility::Unknown {
            reason: "an arm may be selected".to_string(),
        },
        None => Feasibility::Unknown {
            reason: "guard evaluation is unknown".to_string(),
        },
    }
}

fn outcome_feasibility(
    cmd: &CommandDef,
    def: &Definition,
    index: usize,
    flow: &FactFlow,
) -> Feasibility {
    match outcome_impossible(cmd, def, index, flow) {
        Some(basis) => Feasibility::ProvenImpossible {
            scope: format!("arm={index}"),
            basis,
        },
        None => Feasibility::Unknown {
            reason: "outcome not proved impossible".to_string(),
        },
    }
}

fn state_feasibility(def: &Definition, name: &str, flow: &FactFlow) -> Feasibility {
    match state_impossible(def, name, flow) {
        Some(basis) => Feasibility::ProvenImpossible {
            scope: format!("state={name}"),
            basis,
        },
        None => Feasibility::Unknown {
            reason: "state not proved unreachable".to_string(),
        },
    }
}

fn command_success(cmd: &CommandDef, def: &Definition, source: &str) -> Option<bool> {
    let facts = known_facts_at(def, source);
    let args = BTreeMap::new();
    let mut unknown = false;
    for when in arm_guards(cmd) {
        match fold_guard(when, &facts, &args, source) {
            Some(true) => return Some(true),
            Some(false) => {}
            None => unknown = true,
        }
    }
    if unknown {
        None
    } else {
        Some(false)
    }
}

fn outcome_impossible(
    cmd: &CommandDef,
    def: &Definition,
    index: usize,
    flow: &FactFlow,
) -> Option<String> {
    if cmd.from.is_empty() {
        return Some("guard_false".to_string());
    }
    let dest_basis = outcome_dest(cmd, index).and_then(|dest| state_impossible(def, &dest, flow));
    let mut basis = None;
    let mut saw_reachable = false;
    let mut unknown = false;
    for source in &cmd.from {
        if flow.complete && !flow.reachable.contains_key(source) {
            continue;
        }
        saw_reachable = true;
        let extra = flow_facts_at(flow, source);
        match unselectable_at(cmd, def, index, source, &extra) {
            Some(proof) => basis = Some(proof),
            None => unknown = true,
        }
    }
    if saw_reachable && !unknown {
        return basis;
    }
    if dest_basis.is_some() {
        return dest_basis;
    }
    if !saw_reachable {
        return Some("fact_flow".to_string());
    }
    None
}

fn outcome_dest(cmd: &CommandDef, index: usize) -> Option<String> {
    command_outcomes(cmd)
        .get(index)
        .map(|outcome| outcome.to.clone())
}

fn state_impossible(def: &Definition, name: &str, flow: &FactFlow) -> Option<String> {
    if let Some(id) = invariant_false_at(def, name) {
        return Some(format!("invariant={id}"));
    }
    if flow.complete && !flow.reachable.contains_key(name) {
        return Some("fact_flow".to_string());
    }
    None
}

fn unselectable_at(
    cmd: &CommandDef,
    def: &Definition,
    index: usize,
    source: &str,
    extra: &BTreeMap<String, Value>,
) -> Option<String> {
    let mut facts = known_facts_at(def, source);
    for (name, value) in extra {
        facts.entry(name.clone()).or_insert_with(|| value.clone());
    }
    let cases = arg_cases(cmd, index).unwrap_or_else(|| vec![BTreeMap::new()]);
    let mut basis = None;
    for args in cases {
        let proof = valuation_unselectable(cmd, index, &facts, &args, source)?;
        basis = Some(proof);
    }
    basis
}

fn valuation_unselectable(
    cmd: &CommandDef,
    index: usize,
    facts: &BTreeMap<String, Value>,
    args: &BTreeMap<String, Value>,
    source: &str,
) -> Option<String> {
    let mut unknown = false;
    for (position, when) in arm_guards(cmd).into_iter().enumerate() {
        if position > index {
            break;
        }
        match fold_guard(when, facts, args, source) {
            Some(true) if position == index => return None,
            Some(true) => return Some(format!("shadowed_by={position}")),
            Some(false) => {}
            None => unknown = true,
        }
    }
    if unknown {
        None
    } else {
        Some("guard_false".to_string())
    }
}

fn arm_count(cmd: &CommandDef) -> usize {
    cmd.outcomes.as_ref().map(Vec::len).unwrap_or(1)
}

fn arm_guards(cmd: &CommandDef) -> Vec<Option<&str>> {
    match cmd.outcomes.as_ref() {
        Some(outcomes) => outcomes
            .iter()
            .map(|outcome| outcome.when.as_deref())
            .collect(),
        None => vec![cmd.when.as_deref()],
    }
}

fn fold_guard(
    when: Option<&str>,
    facts: &BTreeMap<String, Value>,
    args: &BTreeMap<String, Value>,
    source: &str,
) -> Option<bool> {
    match when {
        None => Some(true),
        Some(text) => expr::eval_partial(&expr::parse(text).ok()?, facts, args, Some(source)),
    }
}

fn known_facts_at(def: &Definition, source: &str) -> BTreeMap<String, Value> {
    let mut facts = BTreeMap::new();
    for inv in def.invariants.values() {
        if !invariant_applies(inv.when.as_deref(), source) {
            continue;
        }
        let Ok(expr) = expr::parse(&inv.assert) else {
            continue;
        };
        collect_eq_literals(&expr, &mut facts);
    }
    facts
}

fn invariant_applies(when: Option<&str>, source: &str) -> bool {
    match when {
        None => true,
        Some(text) => {
            fold_guard(Some(text), &BTreeMap::new(), &BTreeMap::new(), source) == Some(true)
        }
    }
}

fn invariant_false_at(def: &Definition, source: &str) -> Option<String> {
    let facts = known_facts_at(def, source);
    for (name, inv) in &def.invariants {
        if !invariant_applies(inv.when.as_deref(), source) {
            continue;
        }
        if fold_guard(Some(&inv.assert), &facts, &BTreeMap::new(), source) == Some(false) {
            return Some(name.clone());
        }
    }
    None
}

fn collect_eq_literals(expr: &Expr, out: &mut BTreeMap<String, Value>) {
    match expr {
        Expr::And(parts) => {
            for part in parts {
                collect_eq_literals(part, out);
            }
        }
        Expr::Cmp(cmp) if cmp.op == CmpOp::Eq => match (&cmp.left, &cmp.right) {
            (Operand::Fact(name), Operand::Literal(value))
            | (Operand::Literal(value), Operand::Fact(name)) => {
                out.insert(name.clone(), value.clone());
            }
            _ => {}
        },
        _ => {}
    }
}

fn arg_cases(cmd: &CommandDef, index: usize) -> Option<Vec<BTreeMap<String, Value>>> {
    let Some(declared) = cmd.args.as_ref() else {
        return Some(vec![BTreeMap::new()]);
    };
    let mut names = BTreeSet::new();
    for (position, when) in arm_guards(cmd).into_iter().enumerate() {
        if position > index {
            break;
        }
        if let Some(text) = when {
            if let Ok(expr) = expr::parse(text) {
                collect_arg_names(&expr, &mut names);
            }
        }
    }
    let mut slots = Vec::new();
    for name in names {
        let ty = declared.get(&name).map(String::as_str).unwrap_or("string");
        let values = arg_domain(cmd, index, &name, ty)?;
        slots.push((name, values));
    }
    cartesian(slots)
}

fn arg_domain(cmd: &CommandDef, index: usize, name: &str, ty: &str) -> Option<Vec<Value>> {
    let mut flags = DomainFlags::default();
    let mut ints = BTreeSet::new();
    let mut strings = BTreeSet::new();
    for (position, when) in arm_guards(cmd).into_iter().enumerate() {
        if position > index {
            break;
        }
        if let Some(text) = when {
            if let Ok(expr) = expr::parse(text) {
                walk_arg_domain(&expr, name, &mut flags, &mut ints, &mut strings);
            }
        }
    }
    if flags.indirect {
        return None;
    }
    match ty {
        "bool" => Some(vec![Value::Bool(false), Value::Bool(true)]),
        "int" => Some(int_representatives(ints)),
        "string" if flags.ordered => None,
        "string" => Some(string_equality_representatives(strings)),
        _ => None,
    }
}

#[derive(Default)]
struct DomainFlags {
    indirect: bool,
    ordered: bool,
}

fn walk_arg_domain(
    expr: &Expr,
    name: &str,
    flags: &mut DomainFlags,
    ints: &mut BTreeSet<i64>,
    strings: &mut BTreeSet<String>,
) {
    match expr {
        Expr::And(parts) | Expr::Or(parts) => {
            for part in parts {
                walk_arg_domain(part, name, flags, ints, strings);
            }
        }
        Expr::Cmp(cmp) => collect_arg_cut(cmp, name, flags, ints, strings),
        Expr::Exists(_) | Expr::NotExists(_) => {}
        Expr::TextIn(pred) => {
            if operand_has_arg(&pred.value, name) {
                flags.indirect = true;
            }
        }
    }
}

fn collect_arg_cut(
    cmp: &expr::Cmp,
    name: &str,
    flags: &mut DomainFlags,
    ints: &mut BTreeSet<i64>,
    strings: &mut BTreeSet<String>,
) {
    match (&cmp.left, &cmp.right) {
        (Operand::Arg(arg), Operand::Literal(value))
        | (Operand::Literal(value), Operand::Arg(arg))
            if arg == name =>
        {
            if !matches!(cmp.op, CmpOp::Eq | CmpOp::Ne) {
                flags.ordered = true;
            }
            push_literal_cut(value, ints, strings);
        }
        _ if operand_has_arg(&cmp.left, name) || operand_has_arg(&cmp.right, name) => {
            flags.indirect = true;
        }
        _ => {}
    }
}

fn push_literal_cut(value: &Value, ints: &mut BTreeSet<i64>, strings: &mut BTreeSet<String>) {
    if let Some(n) = value.as_i64() {
        ints.insert(n);
    } else if let Some(text) = value.as_str() {
        strings.insert(text.to_string());
    }
}

fn operand_has_arg(operand: &Operand, name: &str) -> bool {
    match operand {
        Operand::Arg(arg) => arg == name,
        Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => {
            operand_has_arg(base, name)
        }
        Operand::BinArith { left, right, .. } => {
            operand_has_arg(left, name) || operand_has_arg(right, name)
        }
        _ => false,
    }
}

fn collect_arg_names(expr: &Expr, out: &mut BTreeSet<String>) {
    match expr {
        Expr::And(parts) | Expr::Or(parts) => {
            for part in parts {
                collect_arg_names(part, out);
            }
        }
        Expr::Cmp(cmp) => {
            collect_arg_from_operand(&cmp.left, out);
            collect_arg_from_operand(&cmp.right, out);
        }
        Expr::Exists(_) | Expr::NotExists(_) => {}
        Expr::TextIn(pred) => collect_arg_from_operand(&pred.value, out),
    }
}

fn collect_arg_from_operand(operand: &Operand, out: &mut BTreeSet<String>) {
    match operand {
        Operand::Arg(name) => {
            out.insert(name.clone());
        }
        Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => {
            collect_arg_from_operand(base, out);
        }
        Operand::BinArith { left, right, .. } => {
            collect_arg_from_operand(left, out);
            collect_arg_from_operand(right, out);
        }
        _ => {}
    }
}

fn int_representatives(cuts: BTreeSet<i64>) -> Vec<Value> {
    let mut values = BTreeSet::new();
    if cuts.is_empty() {
        values.insert(0);
    }
    for n in &cuts {
        values.insert(*n);
        if let Some(prev) = n.checked_sub(1) {
            values.insert(prev);
        }
        if let Some(next) = n.checked_add(1) {
            values.insert(next);
        }
    }
    values.into_iter().map(Value::from).collect()
}

fn string_equality_representatives(cuts: BTreeSet<String>) -> Vec<Value> {
    let mut values: Vec<Value> = cuts
        .iter()
        .map(|text| Value::String(text.clone()))
        .collect();
    let other = "\u{1}other";
    if !cuts.iter().any(|text| text == other) {
        values.push(Value::String(other.to_string()));
    }
    values
}

fn cartesian(slots: Vec<(String, Vec<Value>)>) -> Option<Vec<BTreeMap<String, Value>>> {
    let mut acc = vec![BTreeMap::new()];
    for (name, values) in slots {
        let mut next = Vec::new();
        for row in acc {
            for value in &values {
                if next.len() >= ARG_CASE_LIMIT {
                    return None;
                }
                let mut item = row.clone();
                item.insert(name.clone(), value.clone());
                next.push(item);
            }
        }
        acc = next;
    }
    Some(acc)
}

fn analyze_flow(def: &Definition) -> FactFlow {
    let Some(initial) = def
        .states
        .iter()
        .find_map(|(name, state)| state.initial.then_some(name.clone()))
    else {
        return FactFlow {
            reachable: BTreeMap::new(),
            complete: false,
        };
    };
    let mut reachable = BTreeMap::new();
    reachable.insert(initial.clone(), initial_abs(def));
    let mut work = VecDeque::from([initial]);
    let mut steps = 0usize;
    while let Some(state) = work.pop_front() {
        steps += 1;
        if steps > FLOW_STEP_LIMIT {
            return FactFlow {
                reachable,
                complete: false,
            };
        }
        let Some(facts) = reachable.get(&state).cloned() else {
            continue;
        };
        push_successors(def, &state, &facts, &mut reachable, &mut work);
    }
    FactFlow {
        reachable,
        complete: true,
    }
}

fn push_successors(
    def: &Definition,
    state: &str,
    facts: &BTreeMap<String, Abs>,
    reachable: &mut BTreeMap<String, BTreeMap<String, Abs>>,
    work: &mut VecDeque<String>,
) {
    for cmd in def.commands.values() {
        if !cmd.from.iter().any(|source| source == state) {
            continue;
        }
        for (index, outcome) in command_outcomes(cmd).into_iter().enumerate() {
            if !may_take_arm(cmd, def, index, state, facts) {
                continue;
            }
            let next = apply_then(facts, &outcome, cmd);
            if must_violate(def, &outcome.to, &next) {
                continue;
            }
            if join_into(reachable, &outcome.to, next) {
                work.push_back(outcome.to);
            }
        }
    }
}

fn command_outcomes(cmd: &CommandDef) -> Vec<OutcomeDef> {
    cmd.outcomes.clone().unwrap_or_else(|| {
        vec![OutcomeDef {
            when: cmd.when.clone(),
            to: cmd.to.clone().unwrap_or_default(),
            then: cmd.then.clone(),
        }]
    })
}

fn may_take_arm(
    cmd: &CommandDef,
    def: &Definition,
    index: usize,
    source: &str,
    abs: &BTreeMap<String, Abs>,
) -> bool {
    let extra = concrete_abs(abs);
    unselectable_at(cmd, def, index, source, &extra).is_none()
}

fn apply_then(
    facts: &BTreeMap<String, Abs>,
    outcome: &OutcomeDef,
    cmd: &CommandDef,
) -> BTreeMap<String, Abs> {
    let mut next = facts.clone();
    let Some(then) = &outcome.then else {
        return next;
    };
    let cases = arg_cases(cmd, usize::MAX).unwrap_or_else(|| vec![BTreeMap::new()]);
    let cases = if cases.is_empty() {
        vec![BTreeMap::new()]
    } else {
        cases
    };
    for (fact, raw) in then {
        next.insert(fact.clone(), assigned_abs(facts.get(fact), raw, &cases));
    }
    next
}

fn assigned_abs(previous: Option<&Abs>, raw: &str, cases: &[BTreeMap<String, Value>]) -> Abs {
    if let Some(arg) = raw.strip_prefix('$') {
        return join_arg_values(arg, cases);
    }
    if let Some(amount) = raw
        .strip_prefix('+')
        .and_then(|text| text.parse::<i64>().ok())
    {
        return add_counter(previous, amount);
    }
    parse_then_literal(raw).map(Abs::Exact).unwrap_or(Abs::Top)
}

fn join_arg_values(arg: &str, cases: &[BTreeMap<String, Value>]) -> Abs {
    let mut joined: Option<Abs> = None;
    for args in cases {
        let next = match args.get(arg) {
            Some(value) => Abs::Exact(value.clone()),
            None => Abs::Top,
        };
        joined = Some(match joined {
            None => next,
            Some(prev) => join_abs(prev, next),
        });
    }
    joined.unwrap_or(Abs::Top)
}

fn add_counter(previous: Option<&Abs>, amount: i64) -> Abs {
    match previous {
        Some(Abs::Exact(value)) => value
            .as_i64()
            .and_then(|n| n.checked_add(amount))
            .map(Value::from)
            .map(Abs::Exact)
            .unwrap_or(Abs::Top),
        _ => Abs::Top,
    }
}

fn parse_then_literal(value: &str) -> Option<Value> {
    if value == "true" {
        return Some(Value::Bool(true));
    }
    if value == "false" {
        return Some(Value::Bool(false));
    }
    if let Ok(n) = value.parse::<i64>() {
        return Some(Value::from(n));
    }
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        return Some(Value::String(value[1..value.len() - 1].to_string()));
    }
    None
}

fn must_violate(def: &Definition, dest: &str, abs: &BTreeMap<String, Abs>) -> bool {
    let mut facts = known_facts_at(def, dest);
    for (name, value) in concrete_abs(abs) {
        facts.entry(name).or_insert(value);
    }
    for inv in def.invariants.values() {
        if !invariant_applies(inv.when.as_deref(), dest) {
            continue;
        }
        if fold_guard(Some(&inv.assert), &facts, &BTreeMap::new(), dest) == Some(false) {
            return true;
        }
    }
    false
}

fn join_into(
    reachable: &mut BTreeMap<String, BTreeMap<String, Abs>>,
    state: &str,
    facts: BTreeMap<String, Abs>,
) -> bool {
    let Some(old) = reachable.get_mut(state) else {
        reachable.insert(state.to_string(), facts);
        return true;
    };
    let incoming: BTreeSet<_> = facts.keys().cloned().collect();
    let mut grew = join_present(old, facts);
    grew |= widen_absent(old, &incoming);
    grew
}

fn join_present(old: &mut BTreeMap<String, Abs>, facts: BTreeMap<String, Abs>) -> bool {
    let mut grew = false;
    for (name, value) in facts {
        match old.get(&name) {
            None => {
                old.insert(name, value);
                grew = true;
            }
            Some(Abs::Exact(prev)) if Abs::Exact(prev.clone()) != value => {
                old.insert(name, Abs::Top);
                grew = true;
            }
            _ => {}
        }
    }
    grew
}

fn widen_absent(old: &mut BTreeMap<String, Abs>, incoming: &BTreeSet<String>) -> bool {
    let missing: Vec<_> = old
        .keys()
        .filter(|name| !incoming.contains(*name))
        .cloned()
        .collect();
    let mut grew = false;
    for name in missing {
        if old.get(&name) != Some(&Abs::Top) {
            old.insert(name, Abs::Top);
            grew = true;
        }
    }
    grew
}

fn join_abs(left: Abs, right: Abs) -> Abs {
    match (left, right) {
        (Abs::Exact(a), Abs::Exact(b)) if a == b => Abs::Exact(a),
        _ => Abs::Top,
    }
}

fn concrete_abs(abs: &BTreeMap<String, Abs>) -> BTreeMap<String, Value> {
    abs.iter()
        .filter_map(|(name, value)| match value {
            Abs::Exact(exact) => Some((name.clone(), exact.clone())),
            Abs::Top => None,
        })
        .collect()
}

fn flow_facts_at(flow: &FactFlow, source: &str) -> BTreeMap<String, Value> {
    flow.reachable
        .get(source)
        .map(concrete_abs)
        .unwrap_or_default()
}

fn initial_abs(def: &Definition) -> BTreeMap<String, Abs> {
    let mut facts = BTreeMap::new();
    for (name, fact) in &def.facts {
        if let Some(initial) = &fact.initial {
            facts.insert(name.clone(), Abs::Exact(initial.clone()));
        }
    }
    facts
}
