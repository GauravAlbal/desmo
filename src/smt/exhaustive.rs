//! Kernel-driven finite-domain BFS. The compiled machine is the stepper.

use super::encode::{conjunct, smt_int, Sort};
use super::machine::{lower_definition, Lowered};
use super::query::Target;
use super::replay::{replay_command, CommandStep};
use super::{fragment_error, Names};
use crate::canonical::digest_json;
use crate::declarative::compile::CompiledMachine;
use crate::declarative::def::{CommandDef, Definition};
use crate::declarative::expr::{self, Expr, Operand};
use crate::effect::EffectStatus;
use crate::machine::{Decision, Machine};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const CASE_LIMIT: usize = 2048;
const NODE_LIMIT: usize = 50_000;
const DEPTH_LIMIT: usize = 64;
const REMAINDER: &str = "\u{10FFFF}#other";

#[derive(Clone, Debug)]
pub struct DomainScope {
    pub strings: Vec<String>,
    pub ints: Vec<i64>,
    pub remainder: String,
    pub closed: bool,
    pub closed_reason: Option<String>,
}

impl DomainScope {
    pub fn sentence(&self) -> String {
        let ints: Vec<String> = self.ints.iter().map(|n| n.to_string()).collect();
        format!(
            "strings=[{}]; ints=[{}]; bools=[false,true]; remainder={}",
            self.strings.join(","),
            ints.join(","),
            self.remainder
        )
    }
}

#[derive(Clone, Debug)]
pub struct ExhaustiveResult {
    pub complete: bool,
    pub incomplete_reason: Option<String>,
    pub nodes: usize,
    pub reachable_digest: String,
    pub scope: DomainScope,
    pub reached_outcomes: BTreeSet<(String, usize)>,
    pub reached_command_from: BTreeSet<(String, String)>,
    pub reached_states: BTreeSet<String>,
    pub witness: Option<Vec<CommandStep>>,
    pub invariant_smt: String,
    pub projected_smt: String,
}

thread_local! {
    static UNIVERSE: RefCell<Option<(String, Universe)>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug)]
struct Universe {
    complete: bool,
    incomplete_reason: Option<String>,
    nodes: usize,
    reachable_digest: String,
    scope: DomainScope,
    reached_outcomes: BTreeSet<(String, usize)>,
    reached_command_from: BTreeSet<(String, String)>,
    reached_states: BTreeSet<String>,
    outcome_path: BTreeMap<(String, usize), Vec<CommandStep>>,
    command_from_path: BTreeMap<(String, String), Vec<CommandStep>>,
    state_path: BTreeMap<String, Vec<CommandStep>>,
    invariant_smt: String,
    projected_smt: String,
}

pub fn search(
    def: &Definition,
    machine: &CompiledMachine,
    target: &Target,
) -> Result<ExhaustiveResult, String> {
    let uni = universe(def, machine)?;
    Ok(project(&uni, target))
}

fn universe(def: &Definition, machine: &CompiledMachine) -> Result<Universe, String> {
    let key = machine.spec_hash().to_string();
    if let Some(hit) = UNIVERSE.with(|slot| {
        slot.borrow()
            .as_ref()
            .filter(|(stored, _)| stored == &key)
            .map(|(_, uni)| uni.clone())
    }) {
        return Ok(hit);
    }
    let built = build_universe(def, machine)?;
    UNIVERSE.with(|slot| {
        *slot.borrow_mut() = Some((key, built.clone()));
    });
    Ok(built)
}

fn project(uni: &Universe, target: &Target) -> ExhaustiveResult {
    let witness = match target {
        Target::State(name) | Target::Terminal(name) => uni.state_path.get(name).cloned(),
        Target::Outcome { command, index } => {
            uni.outcome_path.get(&(command.clone(), *index)).cloned()
        }
        Target::CommandFrom { command, source } => uni
            .command_from_path
            .get(&(command.clone(), source.clone()))
            .cloned(),
    };
    ExhaustiveResult {
        complete: uni.complete,
        incomplete_reason: uni.incomplete_reason.clone(),
        nodes: uni.nodes,
        reachable_digest: uni.reachable_digest.clone(),
        scope: uni.scope.clone(),
        reached_outcomes: uni.reached_outcomes.clone(),
        reached_command_from: uni.reached_command_from.clone(),
        reached_states: uni.reached_states.clone(),
        invariant_smt: uni.invariant_smt.clone(),
        projected_smt: uni.projected_smt.clone(),
        witness,
    }
}

fn build_universe(def: &Definition, machine: &CompiledMachine) -> Result<Universe, String> {
    let lowered = lower_definition(def).map_err(|err| err.reason)?;
    let scope = domain_scope(def, &lowered);
    let corpus = command_cases(def, &scope)?;
    walk(machine, &lowered, &scope, &corpus)
}

fn walk(
    machine: &CompiledMachine,
    lowered: &Lowered,
    scope: &DomainScope,
    corpus: &BTreeMap<String, Vec<BTreeMap<String, Value>>>,
) -> Result<Universe, String> {
    let mut queue = VecDeque::new();
    let mut seen = BTreeSet::new();
    let mut configs = Vec::new();
    let initial = machine.initial_state();
    seen.insert(digest_json(&initial));
    configs.push(initial.clone());
    queue.push_back(Node {
        state: initial,
        path: Vec::new(),
        depth: 0,
    });
    let mut reached_outcomes = BTreeSet::new();
    let mut reached_command_from = BTreeSet::new();
    let mut reached_states = BTreeSet::new();
    let mut outcome_path = BTreeMap::new();
    let mut command_from_path = BTreeMap::new();
    let mut state_path = BTreeMap::new();
    note_state(
        &queue[0].state,
        &queue[0].path,
        &mut reached_states,
        &mut state_path,
    );
    let mut complete = scope.closed;
    let mut incomplete_reason = scope.closed_reason.clone();
    while let Some(node) = queue.pop_front() {
        if node.depth >= DEPTH_LIMIT {
            complete = false;
            incomplete_reason = Some(format!("depth limit {DEPTH_LIMIT}"));
            continue;
        }
        if seen.len() >= NODE_LIMIT {
            complete = false;
            incomplete_reason = Some(format!("node limit {NODE_LIMIT}"));
            break;
        }
        expand_node(
            machine,
            lowered,
            corpus,
            node,
            &mut queue,
            &mut seen,
            &mut configs,
            &mut reached_outcomes,
            &mut reached_command_from,
            &mut reached_states,
            &mut outcome_path,
            &mut command_from_path,
            &mut state_path,
        )?;
    }
    Ok(Universe {
        complete,
        incomplete_reason,
        nodes: seen.len(),
        reachable_digest: digest_json(&configs),
        invariant_smt: invariant_from(&configs, &lowered.names, false)?,
        projected_smt: invariant_from(&configs, &lowered.names, true)?,
        scope: scope.clone(),
        reached_outcomes,
        reached_command_from,
        reached_states,
        outcome_path,
        command_from_path,
        state_path,
    })
}

struct Node {
    state: Value,
    path: Vec<CommandStep>,
    depth: usize,
}

fn expand_node(
    machine: &CompiledMachine,
    lowered: &Lowered,
    corpus: &BTreeMap<String, Vec<BTreeMap<String, Value>>>,
    node: Node,
    queue: &mut VecDeque<Node>,
    seen: &mut BTreeSet<String>,
    configs: &mut Vec<Value>,
    reached_outcomes: &mut BTreeSet<(String, usize)>,
    reached_command_from: &mut BTreeSet<(String, String)>,
    reached_states: &mut BTreeSet<String>,
    outcome_path: &mut BTreeMap<(String, usize), Vec<CommandStep>>,
    command_from_path: &mut BTreeMap<(String, String), Vec<CommandStep>>,
    state_path: &mut BTreeMap<String, Vec<CommandStep>>,
) -> Result<(), String> {
    let source = node
        .state
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    for command in &lowered.commands {
        let empty = Vec::new();
        let cases = corpus.get(command).unwrap_or(&empty);
        for args in cases {
            let step = CommandStep {
                name: command.clone(),
                args: args.clone(),
            };
            let Some((next, index)) = try_step(machine, &node.state, &step)? else {
                continue;
            };
            let path = path_plus(&node.path, &step);
            reached_outcomes.insert((command.clone(), index));
            outcome_path
                .entry((command.clone(), index))
                .or_insert_with(|| path.clone());
            reached_command_from.insert((command.clone(), source.clone()));
            command_from_path
                .entry((command.clone(), source.clone()))
                .or_insert_with(|| path.clone());
            note_state(&next, &path, reached_states, state_path);
            if !seen.insert(digest_json(&next)) {
                continue;
            }
            configs.push(next.clone());
            queue.push_back(Node {
                state: next,
                path,
                depth: node.depth + 1,
            });
        }
    }
    Ok(())
}

fn try_step(
    machine: &CompiledMachine,
    state: &Value,
    step: &CommandStep,
) -> Result<Option<(Value, usize)>, String> {
    let command = json!({
        "name": step.name,
        "args": Value::Object(step.args.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
    });
    let decision = match machine.decide(state, None, &EffectStatus::None, &command) {
        Ok(decision) => decision,
        Err(_) => return Ok(None),
    };
    match decision {
        Decision::Emit { event, .. } => {
            let index = event
                .payload
                .get("outcome")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .ok_or_else(|| "event payload has no outcome index".to_string())?;
            let applied = match machine.apply(state, &event) {
                Ok(applied) => applied,
                Err(_) => return Ok(None),
            };
            Ok(Some((applied.state, index)))
        }
        Decision::Noop { .. } => Ok(None),
    }
}

fn path_plus(path: &[CommandStep], step: &CommandStep) -> Vec<CommandStep> {
    let mut out = path.to_vec();
    out.push(step.clone());
    out
}

fn note_state(
    state: &Value,
    path: &[CommandStep],
    reached: &mut BTreeSet<String>,
    state_path: &mut BTreeMap<String, Vec<CommandStep>>,
) {
    let Some(name) = state.get("state").and_then(Value::as_str) else {
        return;
    };
    reached.insert(name.to_string());
    state_path
        .entry(name.to_string())
        .or_insert_with(|| path.to_vec());
}

fn domain_scope(def: &Definition, lowered: &Lowered) -> DomainScope {
    let mut strings: Vec<String> = lowered.names.strings.all().to_vec();
    if !strings.iter().any(|s| s == REMAINDER) {
        strings.push(REMAINDER.to_string());
    }
    strings.sort();
    strings.dedup();
    let mut ints = collect_ints(def);
    ints.insert(i64::MIN);
    ints.insert(i64::MAX);
    let (closed, closed_reason) = match domain_limit(def, &lowered.names) {
        Some(reason) => (false, Some(reason)),
        None => (true, None),
    };
    DomainScope {
        strings,
        ints: ints.into_iter().collect(),
        remainder: REMAINDER.to_string(),
        closed,
        closed_reason,
    }
}

fn domain_limit(def: &Definition, names: &Names) -> Option<String> {
    if let Some(reason) = ordering_reason(def, names) {
        return Some(reason);
    }
    if names.facts.values().any(|sort| *sort == Sort::Int)
        || names.args.values().any(|sort| *sort == Sort::Int)
    {
        return Some(
            "integer arguments or facts are sampled, not enumerated over all i64".to_string(),
        );
    }
    None
}

fn ordering_reason(def: &Definition, names: &Names) -> Option<String> {
    for cmd in def.commands.values() {
        for text in command_guards(cmd) {
            let expr = expr::parse(text).ok()?;
            if let Some(err) = fragment_error(&expr, names) {
                return Some(err.reason);
            }
        }
    }
    for inv in def.invariants.values() {
        let expr = expr::parse(&inv.assert).ok()?;
        if let Some(err) = fragment_error(&expr, names) {
            return Some(err.reason);
        }
    }
    None
}

fn command_guards(cmd: &CommandDef) -> Vec<&str> {
    let mut out = Vec::new();
    if let Some(when) = cmd.when.as_deref() {
        out.push(when);
    }
    if let Some(outcomes) = &cmd.outcomes {
        for outcome in outcomes {
            if let Some(when) = outcome.when.as_deref() {
                out.push(when);
            }
        }
    }
    out
}

fn collect_ints(def: &Definition) -> BTreeSet<i64> {
    let mut out = BTreeSet::from([0, 1, -1, 2]);
    for fact in def.facts.values() {
        push_int(fact.initial.as_ref(), &mut out);
    }
    for cmd in def.commands.values() {
        collect_cmd_ints(cmd, &mut out);
    }
    for inv in def.invariants.values() {
        collect_text_ints(inv.when.as_deref(), &mut out);
        collect_text_ints(Some(&inv.assert), &mut out);
    }
    with_neighbors(out)
}

fn with_neighbors(base: BTreeSet<i64>) -> BTreeSet<i64> {
    let mut out = base.clone();
    for n in base {
        out.insert(n.saturating_add(1));
        out.insert(n.saturating_sub(1));
    }
    out
}

fn collect_cmd_ints(cmd: &CommandDef, out: &mut BTreeSet<i64>) {
    collect_text_ints(cmd.when.as_deref(), out);
    collect_then_ints(cmd.then.as_ref(), out);
    let Some(outcomes) = &cmd.outcomes else {
        return;
    };
    for outcome in outcomes {
        collect_text_ints(outcome.when.as_deref(), out);
        collect_then_ints(outcome.then.as_ref(), out);
    }
}

fn collect_then_ints(then: Option<&BTreeMap<String, String>>, out: &mut BTreeSet<i64>) {
    let Some(then) = then else {
        return;
    };
    for value in then.values() {
        if let Ok(n) = value.parse::<i64>() {
            out.insert(n);
        }
        if let Some(n) = value
            .strip_prefix('+')
            .and_then(|rest| rest.parse::<i64>().ok())
        {
            out.insert(n);
        }
    }
}

fn collect_text_ints(src: Option<&str>, out: &mut BTreeSet<i64>) {
    let Some(src) = src else {
        return;
    };
    let Ok(expr) = expr::parse(src) else {
        return;
    };
    walk_ints(&expr, out);
}

fn walk_ints(expr: &Expr, out: &mut BTreeSet<i64>) {
    match expr {
        Expr::And(parts) | Expr::Or(parts) => {
            for part in parts {
                walk_ints(part, out);
            }
        }
        Expr::Cmp(cmp) => {
            walk_operand_ints(&cmp.left, out);
            walk_operand_ints(&cmp.right, out);
        }
        Expr::Exists(_) | Expr::NotExists(_) => {}
        Expr::TextIn(_) => {}
    }
}

fn walk_operand_ints(op: &Operand, out: &mut BTreeSet<i64>) {
    match op {
        Operand::Literal(value) => push_int(Some(value), out),
        Operand::Arith { amount, base, .. } => {
            out.insert(*amount);
            walk_operand_ints(base, out);
        }
        Operand::Mul { amount, base } | Operand::Div { amount, base } => {
            out.insert(*amount);
            walk_operand_ints(base, out);
        }
        Operand::BinArith { left, right, .. } => {
            walk_operand_ints(left, out);
            walk_operand_ints(right, out);
        }
        Operand::Fact(_) | Operand::Arg(_) | Operand::State => {}
    }
}

fn push_int(value: Option<&Value>, out: &mut BTreeSet<i64>) {
    if let Some(n) = value.and_then(Value::as_i64) {
        out.insert(n);
    }
}

fn command_cases(
    def: &Definition,
    scope: &DomainScope,
) -> Result<BTreeMap<String, Vec<BTreeMap<String, Value>>>, String> {
    let mut out = BTreeMap::new();
    for (name, cmd) in &def.commands {
        match arg_rows(cmd, scope) {
            Some(rows) => {
                out.insert(name.clone(), rows);
            }
            None => {
                return Err(format!(
                    "argument cartesian for `{name}` exceeds {CASE_LIMIT}"
                ));
            }
        }
    }
    Ok(out)
}

fn arg_rows(cmd: &CommandDef, scope: &DomainScope) -> Option<Vec<BTreeMap<String, Value>>> {
    let Some(args) = cmd.args.as_ref() else {
        return Some(vec![BTreeMap::new()]);
    };
    if args.is_empty() {
        return Some(vec![BTreeMap::new()]);
    }
    let mut rows = vec![BTreeMap::new()];
    for (name, ty) in args {
        let values = values_for(ty, scope);
        let mut next = Vec::new();
        for row in &rows {
            for value in &values {
                if next.len() >= CASE_LIMIT {
                    return None;
                }
                let mut copy = row.clone();
                copy.insert(name.clone(), value.clone());
                next.push(copy);
            }
        }
        rows = next;
    }
    Some(rows)
}

fn values_for(ty: &str, scope: &DomainScope) -> Vec<Value> {
    match ty {
        "bool" => vec![Value::Bool(false), Value::Bool(true)],
        "int" => scope.ints.iter().copied().map(Value::from).collect(),
        "string" => scope
            .strings
            .iter()
            .map(|s| Value::String(s.clone()))
            .collect(),
        _ => vec![Value::Null],
    }
}

fn invariant_from(configs: &[Value], names: &Names, skip_ints: bool) -> Result<String, String> {
    if configs.is_empty() {
        return Ok("false".to_string());
    }
    let mut seen = BTreeSet::new();
    let mut parts = Vec::new();
    for config in configs {
        let formula = config_formula(config, names, skip_ints)?;
        if seen.insert(formula.clone()) {
            parts.push(formula);
        }
    }
    Ok(super::encode::disjunct(&parts))
}

fn config_formula(state: &Value, names: &Names, skip_ints: bool) -> Result<String, String> {
    let label = state
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| "reachable configuration missing state".to_string())?;
    let id = names
        .state_id(label)
        .ok_or_else(|| format!("state `{label}` missing from partition"))?;
    let mut parts = vec![format!("(= |st| {})", smt_int(id))];
    for (name, sort) in &names.facts {
        if skip_ints && *sort == Sort::Int {
            continue;
        }
        match state.get(name) {
            Some(value) => {
                parts.push(format!("(= |fs:{name}| true)"));
                parts.push(format!(
                    "(= |fv:{name}| {})",
                    value_smt(value, *sort, names)?
                ));
            }
            None => parts.push(format!("(= |fs:{name}| false)")),
        }
    }
    Ok(conjunct(&parts))
}

fn value_smt(value: &Value, sort: Sort, names: &Names) -> Result<String, String> {
    match sort {
        Sort::Int => value
            .as_i64()
            .map(smt_int)
            .ok_or_else(|| "configuration int is not i64".to_string()),
        Sort::Bool => match value.as_bool() {
            Some(true) => Ok("true".to_string()),
            Some(false) => Ok("false".to_string()),
            None => Err("configuration bool is not boolean".to_string()),
        },
        Sort::Str => {
            let text = value
                .as_str()
                .ok_or_else(|| "configuration string is not a string".to_string())?;
            let id = names
                .strings
                .try_id(text)
                .unwrap_or_else(|| names.strings.remainder());
            Ok(smt_int(id))
        }
    }
}

pub fn suggested_cases(steps: &[CommandStep]) -> Value {
    let mut commands: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (i, step) in steps.iter().enumerate() {
        let args: Map<String, Value> = step
            .args
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        commands.entry(step.name.clone()).or_default().push(json!({
            "name": format!("suggested-{}-{}", i, step.name),
            "args": Value::Object(args),
        }));
    }
    json!({ "commands": commands })
}

pub fn format_trace(steps: &[CommandStep]) -> String {
    if steps.is_empty() {
        return "empty trace from initial state".to_string();
    }
    steps
        .iter()
        .map(|step| format!("{}{}", step.name, format_args(&step.args)))
        .collect::<Vec<_>>()
        .join(" -> ")
}

fn format_args(args: &BTreeMap<String, Value>) -> String {
    if args.is_empty() {
        return String::new();
    }
    let body: Vec<String> = args.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("({})", body.join(","))
}

#[allow(dead_code)]
pub fn replay_found(machine: &CompiledMachine, steps: &[CommandStep]) -> Result<Value, String> {
    if steps.is_empty() {
        return Ok(machine.initial_state());
    }
    replay_command_path(machine, steps)
}

fn replay_command_path(machine: &CompiledMachine, steps: &[CommandStep]) -> Result<Value, String> {
    let mut state = machine.initial_state();
    for step in steps {
        state = replay_command(machine, &state, step)?;
    }
    Ok(state)
}
