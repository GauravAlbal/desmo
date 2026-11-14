//! Replay SMT models through the compiled kernel machine.

use super::encode::Sort;
use super::machine::{lower_definition, Lowered};
use super::query::SmtQuery;
use super::solve::ModelMap;
use crate::declarative::compile::CompiledMachine;
use crate::effect::EffectStatus;
use crate::machine::{Decision, Machine};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct CommandStep {
    pub name: String,
    pub args: BTreeMap<String, Value>,
}

pub fn state_object(state: &str, facts: &BTreeMap<String, Value>) -> Value {
    let mut map = Map::new();
    map.insert("state".into(), Value::String(state.to_string()));
    for (name, value) in facts {
        map.insert(name.clone(), value.clone());
    }
    Value::Object(map)
}

pub fn replay_command(
    machine: &CompiledMachine,
    state: &Value,
    step: &CommandStep,
) -> Result<Value, String> {
    let command = json!({
        "name": step.name,
        "args": Value::Object(step.args.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
    });
    let decision = machine
        .decide(state, None, &EffectStatus::None, &command)
        .map_err(|err| err.message)?;
    match decision {
        Decision::Emit { event, .. } => {
            let applied = machine.apply(state, &event).map_err(|err| err.message)?;
            Ok(applied.state)
        }
        Decision::Noop { .. } => Err("kernel returned noop".to_string()),
    }
}

pub fn replay_trace(machine: &CompiledMachine, steps: &[CommandStep]) -> Result<Value, String> {
    let mut state = machine.initial_state();
    for step in steps {
        state = replay_command(machine, &state, step)?;
    }
    Ok(state)
}

pub fn state_name(state: &Value) -> Option<&str> {
    state.get("state").and_then(Value::as_str)
}

pub fn replay_visits(
    machine: &CompiledMachine,
    steps: &[CommandStep],
    want: &str,
) -> Result<Value, String> {
    let mut state = machine.initial_state();
    if state_name(&state) == Some(want) {
        return Ok(state);
    }
    for step in steps {
        state = replay_command(machine, &state, step)?;
        if state_name(&state) == Some(want) {
            return Ok(state);
        }
    }
    Err(format!("kernel trace did not visit {want}"))
}

pub fn replay_command_from(
    machine: &CompiledMachine,
    steps: &[CommandStep],
    command: &str,
    source: &str,
) -> Result<Value, String> {
    let mut state = machine.initial_state();
    for step in steps {
        if step.name == command && state_name(&state) == Some(source) {
            return replay_command(machine, &state, step);
        }
        state = replay_command(machine, &state, step)?;
    }
    Err(format!("kernel trace did not run {command} from {source}"))
}

pub fn replay_takes(
    machine: &CompiledMachine,
    steps: &[CommandStep],
    command: &str,
    index: usize,
) -> Result<Value, String> {
    let mut state = machine.initial_state();
    for step in steps {
        if step.name == command {
            let got = selected_outcome(machine, &state, step)?;
            if got == index {
                return replay_command(machine, &state, step);
            }
        }
        state = replay_command(machine, &state, step)?;
    }
    Err(format!("kernel trace did not take {command}/{index}"))
}

pub fn selected_outcome(
    machine: &CompiledMachine,
    state: &Value,
    step: &CommandStep,
) -> Result<usize, String> {
    let command = json!({
        "name": step.name,
        "args": Value::Object(step.args.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
    });
    let decision = machine
        .decide(state, None, &EffectStatus::None, &command)
        .map_err(|err| err.message)?;
    match decision {
        Decision::Emit { event, .. } => event
            .payload
            .get("outcome")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .ok_or_else(|| "event payload has no outcome index".to_string()),
        Decision::Noop { .. } => Err("kernel returned noop".to_string()),
    }
}

pub fn model_to_state(
    query: &SmtQuery,
    model: &ModelMap,
    frame: &str,
) -> Result<(String, BTreeMap<String, Value>), String> {
    let st = model_int(model, &format!("|{frame}st|"))?;
    let state = query
        .names
        .state_name(st)
        .ok_or_else(|| format!("state id {st} is not in the partition"))?
        .to_string();
    let mut facts = BTreeMap::new();
    for (name, sort) in &query.names.facts {
        let set = model_bool(model, &format!("|{frame}fs:{name}|"))?;
        if !set {
            continue;
        }
        facts.insert(
            name.clone(),
            model_value(model, &format!("|{frame}fv:{name}|"), *sort, query)?,
        );
    }
    Ok((state, facts))
}

pub fn model_to_args(
    query: &SmtQuery,
    model: &ModelMap,
    frame: &str,
) -> Result<BTreeMap<String, Value>, String> {
    let mut args = BTreeMap::new();
    for (name, sort) in &query.names.args {
        args.insert(
            name.clone(),
            model_value(model, &format!("|{frame}av:{name}|"), *sort, query)?,
        );
    }
    Ok(args)
}

pub fn bmc_trace(
    lowered: &Lowered,
    query: &SmtQuery,
    model: &ModelMap,
    depth: usize,
) -> Result<Vec<CommandStep>, String> {
    let mut steps = Vec::new();
    for time in 0..depth {
        let frame = format!("t{time}:");
        let cmd = model_int(model, &format!("|{frame}cmd|"))?;
        if cmd < 0 {
            continue;
        }
        let idx = usize::try_from(cmd).map_err(|_| "command index overflow".to_string())?;
        let name = lowered
            .commands
            .get(idx)
            .cloned()
            .ok_or_else(|| format!("command index {idx} missing"))?;
        let mut arg_query = query.clone();
        arg_query.names.args = lowered.command_args(&name);
        steps.push(CommandStep {
            args: model_to_args(&arg_query, model, &frame)?,
            name,
        });
    }
    Ok(steps)
}

pub fn local_step_from_model(
    lowered: &Lowered,
    query: &SmtQuery,
    model: &ModelMap,
    command: &str,
) -> Result<(Value, CommandStep), String> {
    let (state, facts) = model_to_state(query, model, "")?;
    let mut arg_query = query.clone();
    arg_query.names.args = lowered.command_args(command);
    let step = CommandStep {
        name: command.to_string(),
        args: model_to_args(&arg_query, model, "")?,
    };
    Ok((state_object(&state, &facts), step))
}

fn model_value(model: &ModelMap, key: &str, sort: Sort, query: &SmtQuery) -> Result<Value, String> {
    match sort {
        Sort::Int => Ok(Value::Number(model_int(model, key)?.into())),
        Sort::Bool => Ok(Value::Bool(model_bool(model, key)?)),
        Sort::Str => {
            let id = model_int(model, key)?;
            Ok(Value::String(query.names.strings.decode_or_remainder(id)))
        }
    }
}

fn model_int(model: &ModelMap, key: &str) -> Result<i64, String> {
    model
        .ints
        .get(key)
        .copied()
        .ok_or_else(|| format!("model missing int {key}"))
}

fn model_bool(model: &ModelMap, key: &str) -> Result<bool, String> {
    model
        .bools
        .get(key)
        .copied()
        .ok_or_else(|| format!("model missing bool {key}"))
}

pub fn lowered_of(def: &crate::declarative::def::Definition) -> Result<Lowered, String> {
    lower_definition(def).map_err(|err| err.reason)
}

pub fn transition_from_model(
    def: &crate::declarative::def::Definition,
    query: &SmtQuery,
    model: &ModelMap,
) -> Result<(Value, CommandStep), String> {
    let lowered = lowered_of(def)?;
    let cmd = model_int(model, "|cmd|")?;
    let idx = usize::try_from(cmd).map_err(|_| "command index overflow".to_string())?;
    let name = lowered
        .commands
        .get(idx)
        .cloned()
        .ok_or_else(|| format!("command index {idx} missing"))?;
    local_step_from_model(&lowered, query, model, &name)
}
