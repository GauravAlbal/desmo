//! Lower a schema-1 [`Definition`] into the transition list the encoder uses.

use super::encode::{Sort, StringPart};
use super::{Names, Unsupported};
use crate::declarative::def::{CommandDef, Definition, FactDef, OutcomeDef};
use crate::declarative::expr::{self, Expr};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub enum ThenOp {
    Literal(Value),
    Arg(String),
    CounterIncr(i64),
}

#[derive(Clone, Debug)]
pub struct Transition {
    pub command: String,
    pub command_index: usize,
    pub source: String,
    pub outcome_index: usize,
    pub dest: String,
    pub when: Option<Expr>,
    pub earlier: Vec<Option<Expr>>,
    pub then: Vec<(String, ThenOp)>,
    pub args: BTreeMap<String, Sort>,
}

#[derive(Clone, Debug)]
pub struct Lowered {
    pub def: Definition,
    pub names: Names,
    pub commands: Vec<String>,
    pub transitions: Vec<Transition>,
    pub invariants: Vec<Invariant>,
    pub initial_state: String,
    pub initial_facts: BTreeMap<String, Value>,
    pub terminals: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub struct Invariant {
    pub name: String,
    pub when: Option<Expr>,
    pub assert: Expr,
}

pub fn lower_definition(def: &Definition) -> Result<Lowered, Unsupported> {
    let states = sorted_keys(&def.states);
    let initial_state = initial_name(def)?;
    let mut strings = BTreeSet::new();
    for name in &states {
        strings.insert(name.clone());
    }
    collect_def_strings(def, &mut strings);
    let facts = fact_sorts(def);
    let args = all_arg_sorts(def);
    let names = Names {
        facts,
        args,
        strings: StringPart::new(strings),
        states,
    };
    let commands: Vec<String> = sorted_keys(&def.commands);
    let mut transitions = Vec::new();
    for (cmd_index, command) in commands.iter().enumerate() {
        let cmd = match def.commands.get(command) {
            Some(cmd) => cmd,
            None => continue,
        };
        push_command(command, cmd_index, cmd, def, &mut transitions)?;
    }
    Ok(Lowered {
        names,
        commands,
        transitions,
        invariants: lower_invariants(def)?,
        initial_facts: initial_facts(def),
        terminals: def
            .states
            .iter()
            .filter(|(_, state)| state.terminal)
            .map(|(name, _)| name.clone())
            .collect(),
        initial_state,
        def: def.clone(),
    })
}

fn initial_name(def: &Definition) -> Result<String, Unsupported> {
    def.states
        .iter()
        .find(|(_, state)| state.initial)
        .map(|(name, _)| name.clone())
        .ok_or_else(|| Unsupported::new("definition has no initial state"))
}

fn fact_sorts(def: &Definition) -> BTreeMap<String, Sort> {
    def.facts
        .iter()
        .filter_map(|(name, fact)| parse_sort(&fact.ty).map(|sort| (name.clone(), sort)))
        .collect()
}

fn all_arg_sorts(def: &Definition) -> BTreeMap<String, Sort> {
    let mut out = BTreeMap::new();
    for cmd in def.commands.values() {
        let Some(args) = cmd.args.as_ref() else {
            continue;
        };
        for (name, ty) in args {
            if let Some(sort) = parse_sort(ty) {
                out.insert(name.clone(), sort);
            }
        }
    }
    out
}

pub fn parse_sort(ty: &str) -> Option<Sort> {
    match ty {
        "int" => Some(Sort::Int),
        "bool" => Some(Sort::Bool),
        "string" => Some(Sort::Str),
        _ => None,
    }
}

fn sorted_keys<V>(map: &BTreeMap<String, V>) -> Vec<String> {
    map.keys().cloned().collect()
}

fn initial_facts(def: &Definition) -> BTreeMap<String, Value> {
    def.facts
        .iter()
        .filter_map(|(name, fact)| fact.initial.clone().map(|value| (name.clone(), value)))
        .collect()
}

fn lower_invariants(def: &Definition) -> Result<Vec<Invariant>, Unsupported> {
    let mut out = Vec::new();
    for (name, inv) in &def.invariants {
        out.push(Invariant {
            name: name.clone(),
            when: parse_optional(inv.when.as_deref())?,
            assert: parse_required(&inv.assert)?,
        });
    }
    Ok(out)
}

fn push_command(
    name: &str,
    command_index: usize,
    cmd: &CommandDef,
    def: &Definition,
    out: &mut Vec<Transition>,
) -> Result<(), Unsupported> {
    let outcomes = lowered_outcomes(name, cmd, &def.facts)?;
    let args = arg_sorts(cmd);
    for source in &cmd.from {
        for (outcome_index, outcome) in outcomes.iter().enumerate() {
            let earlier = outcomes
                .iter()
                .take(outcome_index)
                .map(|row| row.when.clone())
                .collect();
            out.push(Transition {
                command: name.to_string(),
                command_index,
                source: source.clone(),
                outcome_index,
                dest: outcome.to.clone(),
                when: outcome.when.clone(),
                earlier,
                then: outcome.then.clone(),
                args: args.clone(),
            });
        }
    }
    Ok(())
}

struct OutcomeRow {
    when: Option<Expr>,
    to: String,
    then: Vec<(String, ThenOp)>,
}

fn lowered_outcomes(
    name: &str,
    cmd: &CommandDef,
    facts: &BTreeMap<String, FactDef>,
) -> Result<Vec<OutcomeRow>, Unsupported> {
    match &cmd.outcomes {
        Some(list) => list
            .iter()
            .enumerate()
            .map(|(i, outcome)| outcome_row(name, i, outcome, facts))
            .collect(),
        None => {
            let to = cmd
                .to
                .clone()
                .ok_or_else(|| Unsupported::new(format!("command `{name}` has no destination")))?;
            Ok(vec![OutcomeRow {
                when: parse_optional(cmd.when.as_deref())?,
                then: compile_then(cmd.then.as_ref(), facts)?,
                to,
            }])
        }
    }
}

fn outcome_row(
    name: &str,
    index: usize,
    outcome: &OutcomeDef,
    facts: &BTreeMap<String, FactDef>,
) -> Result<OutcomeRow, Unsupported> {
    let _ = (name, index);
    Ok(OutcomeRow {
        when: parse_optional(outcome.when.as_deref())?,
        to: outcome.to.clone(),
        then: compile_then(outcome.then.as_ref(), facts)?,
    })
}

fn arg_sorts(cmd: &CommandDef) -> BTreeMap<String, Sort> {
    cmd.args
        .as_ref()
        .map(|args| {
            args.iter()
                .filter_map(|(name, ty)| parse_sort(ty).map(|sort| (name.clone(), sort)))
                .collect()
        })
        .unwrap_or_default()
}

fn compile_then(
    then: Option<&BTreeMap<String, String>>,
    facts: &BTreeMap<String, FactDef>,
) -> Result<Vec<(String, ThenOp)>, Unsupported> {
    let Some(then) = then else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (fact, value) in then {
        out.push((fact.clone(), then_op(fact, value, facts)?));
    }
    Ok(out)
}

fn then_op(
    fact: &str,
    value: &str,
    facts: &BTreeMap<String, FactDef>,
) -> Result<ThenOp, Unsupported> {
    if facts.get(fact).map(|f| f.policy.as_str()) == Some("once") {
        return Err(Unsupported::new(format!(
            "`policy = \"once\"` assignment on `{fact}` is outside the exact SMT fragment"
        )));
    }
    if facts.get(fact).map(|f| f.policy.as_str()) == Some("counter") {
        let n = value
            .strip_prefix('+')
            .and_then(|n| n.parse::<i64>().ok())
            .ok_or_else(|| Unsupported::new(format!("counter `{fact}` increment is not `+N`")))?;
        return Ok(ThenOp::CounterIncr(n));
    }
    if let Some(arg) = value.strip_prefix('$') {
        return Ok(ThenOp::Arg(arg.to_string()));
    }
    if let Some(lit) = parse_literal(value) {
        return Ok(ThenOp::Literal(lit));
    }
    if matches!(
        crate::declarative::expr::parse_then_arith(value),
        Ok(Some(_))
    ) {
        return Err(Unsupported::new(
            "arithmetic then expression is outside the exact SMT fragment",
        ));
    }
    Err(Unsupported::new(format!(
        "`then` value `{value}` is not a literal or `$arg`"
    )))
}

fn parse_literal(value: &str) -> Option<Value> {
    if value == "true" {
        return Some(Value::Bool(true));
    }
    if value == "false" {
        return Some(Value::Bool(false));
    }
    if let Ok(n) = value.parse::<i64>() {
        return Some(Value::Number(n.into()));
    }
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        return Some(Value::String(value[1..value.len() - 1].to_string()));
    }
    None
}

fn parse_optional(src: Option<&str>) -> Result<Option<Expr>, Unsupported> {
    match src {
        Some(text) => parse_required(text).map(Some),
        None => Ok(None),
    }
}

fn parse_required(src: &str) -> Result<Expr, Unsupported> {
    expr::parse(src).map_err(|err| Unsupported::new(format!("guard parse: {err}")))
}

fn collect_def_strings(def: &Definition, out: &mut BTreeSet<String>) {
    for fact in def.facts.values() {
        if let Some(Value::String(s)) = &fact.initial {
            out.insert(s.clone());
        }
    }
    for cmd in def.commands.values() {
        collect_cmd_strings(cmd, out);
    }
    for inv in def.invariants.values() {
        collect_text(inv.when.as_deref(), out);
        collect_text(Some(&inv.assert), out);
    }
}

fn collect_cmd_strings(cmd: &CommandDef, out: &mut BTreeSet<String>) {
    collect_text(cmd.when.as_deref(), out);
    collect_then_strings(cmd.then.as_ref(), out);
    let Some(outcomes) = cmd.outcomes.as_ref() else {
        return;
    };
    for outcome in outcomes {
        collect_text(outcome.when.as_deref(), out);
        collect_then_strings(outcome.then.as_ref(), out);
    }
}

fn collect_then_strings(then: Option<&BTreeMap<String, String>>, out: &mut BTreeSet<String>) {
    let Some(then) = then else {
        return;
    };
    for value in then.values() {
        if let Some(Value::String(s)) = parse_literal(value) {
            out.insert(s);
        }
    }
}

fn collect_text(src: Option<&str>, out: &mut BTreeSet<String>) {
    let Some(src) = src else {
        return;
    };
    if let Ok(expr) = expr::parse(src) {
        out.extend(super::encode::collect_strings(&expr));
    }
}

impl Lowered {
    pub fn command_args(&self, command: &str) -> BTreeMap<String, Sort> {
        self.def
            .commands
            .get(command)
            .map(arg_sorts)
            .unwrap_or_default()
    }

    pub fn outcomes_for<'a>(&'a self, command: &str, source: &str) -> Vec<&'a Transition> {
        self.transitions
            .iter()
            .filter(|row| row.command == command && row.source == source)
            .collect()
    }

    pub fn is_counter(&self, fact: &str) -> bool {
        self.def
            .facts
            .get(fact)
            .map(|f| f.policy.as_str() == "counter")
            .unwrap_or(false)
    }

    pub fn is_terminal(&self, state: &str) -> bool {
        self.terminals.contains(state)
    }
}
