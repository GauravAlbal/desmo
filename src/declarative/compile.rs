//! Compiles a validated [`Definition`] into a [`CompiledMachine`] implementing the
//! uniform [`crate::machine::Machine`] contract. The compiled machine is pure:
//! [`Machine::decide`] validates the command, resolves the first matching outcome and
//! frozen arguments, and emits a versioned event; [`Machine::apply`] folds that event
//! into post-state, terminality, and effect intent without re-evaluating guards, and
//! never performs IO.

use super::def::{self, CommandDef, Definition, FactDef, OutcomeDef, StaticErrors};
use super::evaluate::{self, GuardSpec, InvariantSpec};
use super::expr::{self, EvalCtx, Expr};
use crate::canonical::digest_json;
use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::effect::EffectIntentV1;
use crate::identity::{AggregateId, EffectId, MachineIdentity, MachineSpecHash, MachineVersion};
use crate::machine::{ApplyOutcome, Decision, DomainEvent, Machine};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Compiled declarative machine.
pub struct CompiledMachine {
    identity: MachineIdentity,
    definition: Definition,
    initial_state: Value,
    commands: BTreeMap<String, CompiledCommand>,
    invariants: Vec<CompiledInvariant>,
    terminal_states: Vec<String>,
    fact_types: BTreeMap<String, FactInfo>,
}

pub(crate) struct FactInfo {
    ty: String,
    policy: String,
}

pub(crate) struct CompiledCommand {
    from: Vec<String>,
    outcomes: Vec<CompiledOutcome>,
    args: BTreeMap<String, String>,
    effect: Option<CompiledEffect>,
}

pub(crate) struct CompiledOutcome {
    when: Option<Expr>,
    when_src: Option<String>,
    when_rule: String,
    to: String,
    then: Vec<(String, ThenValue)>,
}

enum ThenValue {
    Literal(Value),
    Arg(String),
    CounterIncr(i64),
}

struct CompiledEffect {
    kind: String,
    id: ValueSource,
    payload_digest: ValueSource,
    idempotency_key: ValueSource,
}

enum ValueSource {
    Literal(Value),
    Arg(String),
}

pub(crate) struct CompiledInvariant {
    name: String,
    when: Option<Expr>,
    when_src: Option<String>,
    when_rule: String,
    assert: Expr,
    assert_rule: String,
}

/// Parse a definition from TOML text and run static checks without compiling.
pub fn parse_definition(toml_text: &str) -> Result<Definition, StaticErrors> {
    Ok(super::region::parse_source(toml_text)?.definition)
}

/// Parse a definition from TOML text and compile it, running strict static checks.
pub fn parse_and_compile(toml_text: &str) -> Result<CompiledMachine, StaticErrors> {
    compile(parse_definition(toml_text)?)
}

/// Load a definition from a file and compile it.
pub fn load_def(path: &std::path::Path) -> Result<CompiledMachine, StaticErrors> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| vec![format!("cannot read {}: {e}", path.display())])?;
    parse_and_compile(&text)
}

/// Load a checked definition from a file without compiling.
pub fn load_definition(path: &std::path::Path) -> Result<Definition, StaticErrors> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| vec![format!("cannot read {}: {e}", path.display())])?;
    parse_definition(&text)
}

/// Compile a definition, running strict static checks first.
pub fn compile(definition: Definition) -> Result<CompiledMachine, StaticErrors> {
    let errors = def::check(&definition);
    if !errors.is_empty() {
        return Err(errors);
    }

    let spec_hash = MachineSpecHash(digest_json(&definition));
    let identity = MachineIdentity {
        machine_id: definition.machine_id.clone(),
        version: MachineVersion(definition.machine_version),
        spec_hash,
    };

    // Initial state: state name + facts with initial values.
    let initial_name = initial_state_name(&definition)?;
    let mut facts = BTreeMap::new();
    for (name, f) in &definition.facts {
        if let Some(initial) = &f.initial {
            facts.insert(name.clone(), initial.clone());
        }
    }
    let initial_state = state_value(&initial_name, &facts);

    let mut commands = BTreeMap::new();
    for (name, cmd) in &definition.commands {
        commands.insert(name.clone(), compile_command(name, cmd, &definition.facts)?);
    }

    let invariants = compile_invariants(&definition)?;

    let terminal_states = definition
        .states
        .iter()
        .filter(|(_, s)| s.terminal)
        .map(|(name, _)| name.clone())
        .collect();

    let fact_types = definition
        .facts
        .iter()
        .map(|(name, f)| {
            (
                name.clone(),
                FactInfo {
                    ty: f.ty.clone(),
                    policy: f.policy.clone(),
                },
            )
        })
        .collect();

    Ok(CompiledMachine {
        identity,
        definition,
        initial_state,
        commands,
        invariants,
        terminal_states,
        fact_types,
    })
}

fn initial_state_name(definition: &Definition) -> Result<String, StaticErrors> {
    definition
        .states
        .iter()
        .find(|(_, state)| state.initial)
        .map(|(name, _)| name.clone())
        .ok_or_else(|| vec!["exactly one initial state must be declared (found 0)".to_string()])
}

fn compile_invariants(definition: &Definition) -> Result<Vec<CompiledInvariant>, StaticErrors> {
    let mut compiled = Vec::with_capacity(definition.invariants.len());
    for (name, inv) in &definition.invariants {
        let when_rule = format!("invariants.{name}.when");
        let assert_rule = format!("invariants.{name}.assert");
        compiled.push(CompiledInvariant {
            name: name.clone(),
            when: parse_checked_optional(inv.when.as_deref(), &when_rule)?,
            when_src: inv.when.clone(),
            when_rule,
            assert: parse_checked_expr(&inv.assert, &assert_rule)?,
            assert_rule,
        });
    }
    Ok(compiled)
}

fn parse_checked_expr(source: &str, rule: &str) -> Result<Expr, StaticErrors> {
    expr::parse(source).map_err(|error| vec![format!("{rule} parse error: {error}")])
}

fn parse_checked_optional(source: Option<&str>, rule: &str) -> Result<Option<Expr>, StaticErrors> {
    match source {
        Some(text) => parse_checked_expr(text, rule).map(Some),
        None => Ok(None),
    }
}

fn compile_command(
    name: &str,
    cmd: &CommandDef,
    facts: &BTreeMap<String, FactDef>,
) -> Result<CompiledCommand, StaticErrors> {
    let outcomes = match &cmd.outcomes {
        Some(outcomes) => outcomes
            .iter()
            .enumerate()
            .map(|(i, outcome)| compile_outcome(name, i, outcome, facts))
            .collect::<Result<Vec<_>, _>>()?,
        None => vec![compile_sugar_outcome(name, cmd, facts)?],
    };
    let effect = cmd.effect.as_ref().map(|e| CompiledEffect {
        kind: e.kind.clone(),
        id: value_source(&e.id),
        payload_digest: e
            .payload_digest
            .as_deref()
            .map(value_source)
            .unwrap_or(ValueSource::Literal(Value::String(String::new()))),
        idempotency_key: e
            .idempotency_key
            .as_deref()
            .map(value_source)
            .unwrap_or(ValueSource::Literal(Value::String(String::new()))),
    });
    Ok(CompiledCommand {
        from: cmd.from.clone(),
        outcomes,
        args: cmd.args.clone().unwrap_or_default(),
        effect,
    })
}

fn compile_sugar_outcome(
    name: &str,
    cmd: &CommandDef,
    facts: &BTreeMap<String, FactDef>,
) -> Result<CompiledOutcome, StaticErrors> {
    let when_rule = format!("commands.{name}.when");
    Ok(CompiledOutcome {
        when: parse_checked_optional(cmd.when.as_deref(), &when_rule)?,
        when_src: cmd.when.clone(),
        when_rule,
        to: cmd
            .to
            .clone()
            .ok_or_else(|| vec![format!("command `{name}` must declare `to` or `outcomes`")])?,
        then: compile_optional_then(cmd.then.as_ref(), facts)?,
    })
}

fn compile_outcome(
    cmd_name: &str,
    i: usize,
    outcome: &OutcomeDef,
    facts: &BTreeMap<String, FactDef>,
) -> Result<CompiledOutcome, StaticErrors> {
    let when_rule = format!("commands.{cmd_name}.outcomes[{i}].when");
    Ok(CompiledOutcome {
        when: parse_checked_optional(outcome.when.as_deref(), &when_rule)?,
        when_src: outcome.when.clone(),
        when_rule,
        to: outcome.to.clone(),
        then: compile_optional_then(outcome.then.as_ref(), facts)?,
    })
}

fn compile_optional_then(
    then: Option<&BTreeMap<String, String>>,
    facts: &BTreeMap<String, FactDef>,
) -> Result<Vec<(String, ThenValue)>, StaticErrors> {
    match then {
        Some(then) => compile_then(then, facts),
        None => Ok(Vec::new()),
    }
}

fn compile_then(
    then: &BTreeMap<String, String>,
    facts: &BTreeMap<String, FactDef>,
) -> Result<Vec<(String, ThenValue)>, StaticErrors> {
    let mut compiled = Vec::with_capacity(then.len());
    for (fact, value) in then {
        compiled.push((fact.clone(), compile_then_value(fact, value, facts)?));
    }
    Ok(compiled)
}

fn compile_then_value(
    fact: &str,
    value: &str,
    facts: &BTreeMap<String, FactDef>,
) -> Result<ThenValue, StaticErrors> {
    if facts.get(fact).map(|f| f.policy.as_str()) == Some("counter") {
        let n = value
            .strip_prefix('+')
            .and_then(|n| n.parse::<i64>().ok())
            .ok_or_else(|| {
                vec![format!(
                    "counter fact `{fact}` accepts only `+N` increments, got `{value}`"
                )]
            })?;
        return Ok(ThenValue::CounterIncr(n));
    }
    if let Some(arg) = value.strip_prefix('$') {
        return Ok(ThenValue::Arg(arg.to_string()));
    }
    parse_literal(value).map(ThenValue::Literal).ok_or_else(|| {
        vec![format!(
            "`then` value `{value}` for `{fact}` is not a literal or `$arg` (use '\"text\"' for strings, \"1\" for ints, \"true\"/\"false\" for bools, \"+1\" on counters, or \"$arg\")"
        )]
    })
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

fn value_source(value: &str) -> ValueSource {
    if let Some(value) = value.strip_prefix('$') {
        ValueSource::Arg(value.to_string())
    } else {
        ValueSource::Literal(Value::String(value.to_string()))
    }
}

fn state_value(state: &str, facts: &BTreeMap<String, Value>) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("state".to_string(), Value::String(state.to_string()));
    for (name, value) in facts {
        map.insert(name.clone(), value.clone());
    }
    Value::Object(map)
}

impl CompiledMachine {
    /// The deterministic spec hash of this machine's definition.
    pub fn spec_hash(&self) -> &MachineSpecHash {
        &self.identity.spec_hash
    }

    /// The validated definition this machine was compiled from.
    pub fn definition(&self) -> &Definition {
        &self.definition
    }

    pub(crate) fn select_command_outcomes(
        &self,
        name: &str,
        cmd: &CompiledCommand,
        current_state: &str,
        source_legal: bool,
        facts: &BTreeMap<String, Value>,
        args: &BTreeMap<String, Value>,
    ) -> evaluate::OutcomeSelection {
        let specs: Vec<GuardSpec<'_>> = cmd
            .outcomes
            .iter()
            .map(|outcome| GuardSpec {
                when: outcome.when.as_ref(),
                when_src: outcome.when_src.as_deref(),
                when_rule: outcome.when_rule.as_str(),
                target: outcome.to.as_str(),
            })
            .collect();
        let ctx = EvalCtx {
            facts,
            args,
            state: current_state,
        };
        let _ = name;
        evaluate::select_outcomes(current_state, source_legal, &specs, &ctx)
    }

    pub(crate) fn check_invariants(
        &self,
        facts: &BTreeMap<String, Value>,
        dest_state: &str,
    ) -> evaluate::InvariantCheck {
        let specs: Vec<InvariantSpec<'_>> = self
            .invariants
            .iter()
            .map(|inv| InvariantSpec {
                name: inv.name.as_str(),
                when: inv.when.as_ref(),
                when_src: inv.when_src.as_deref(),
                when_rule: inv.when_rule.as_str(),
                assert: &inv.assert,
                assert_rule: inv.assert_rule.as_str(),
            })
            .collect();
        let empty = BTreeMap::new();
        let ctx = EvalCtx {
            facts,
            args: &empty,
            state: dest_state,
        };
        evaluate::evaluate_invariants(&specs, &ctx)
    }
}

impl Machine for CompiledMachine {
    fn identity(&self) -> &MachineIdentity {
        &self.identity
    }

    fn initial_state(&self) -> Value {
        self.initial_state.clone()
    }

    fn validate_initial(&self) -> Result<(), Diagnostic> {
        let (state, facts) = decode_state(&self.initial_state, &self.fact_types)?;
        match self.check_invariants(&facts, &state).error {
            Some(diagnostic) => Err(diagnostic),
            None => Ok(()),
        }
    }

    fn decide(
        &self,
        state: &Value,
        _observation: Option<&Value>,
        _effect: &crate::effect::EffectStatus,
        command: &Value,
    ) -> Result<Decision, Diagnostic> {
        let prepared = prepare_command(self, state, command)?;
        if !prepared.source_legal {
            return Err(Diagnostic::new(
                DiagnosticCode::IkIllegalState,
                Some(prepared.name.clone()),
                format!(
                    "command `{}` is not legal from state `{}`",
                    prepared.name, prepared.current_state
                ),
            ));
        }
        let selection = self.select_command_outcomes(
            &prepared.name,
            prepared.cmd,
            &prepared.current_state,
            true,
            &prepared.current_facts,
            &prepared.args,
        );
        if let Some(error) = selection.error {
            return Err(error);
        }
        let Some(index) = selection.selected else {
            return Err(Diagnostic::new(
                DiagnosticCode::IkInvariantFailed,
                Some(format!("{}.when", prepared.name)),
                format!(
                    "no outcome of command `{}` applies in state `{}`",
                    prepared.name, prepared.current_state
                ),
            ));
        };
        Ok(Decision::emit(DomainEvent::new(
            prepared.name,
            2,
            json!({
                "source": prepared.current_state,
                "outcome": index,
                "args": prepared.args
            }),
        )))
    }

    fn apply(&self, state: &Value, event: &DomainEvent) -> Result<ApplyOutcome, Diagnostic> {
        let reduced = self.reduce_event(state, event)?;
        #[cfg(feature = "test-support")]
        let skip = crate::law_mutant::is(crate::law_mutant::LawMutant::SkipInvariantValidation);
        #[cfg(not(feature = "test-support"))]
        let skip = false;
        if let Some(error) = reduced.invariants.error {
            if !skip {
                return Err(error);
            }
        }
        let mut result = ApplyOutcome::new(reduced.state).with_terminal(reduced.terminal);
        if let Some(effect) = reduced.effect {
            result = result.with_effect(effect);
        }
        Ok(result)
    }
}

pub(crate) struct PreparedCommand<'a> {
    pub name: String,
    pub args: BTreeMap<String, Value>,
    pub cmd: &'a CompiledCommand,
    pub current_state: String,
    pub current_facts: BTreeMap<String, Value>,
    pub source_legal: bool,
}

pub(crate) struct ReducedEvent {
    #[allow(dead_code)]
    pub dest: String,
    #[allow(dead_code)]
    pub facts: BTreeMap<String, Value>,
    pub state: Value,
    pub terminal: bool,
    pub effect: Option<EffectIntentV1>,
    pub invariants: evaluate::InvariantCheck,
}

impl CompiledMachine {
    pub(crate) fn prepare(
        &self,
        state: &Value,
        command: &Value,
    ) -> Result<PreparedCommand<'_>, Diagnostic> {
        prepare_command(self, state, command)
    }

    pub(crate) fn reduce_event(
        &self,
        state: &Value,
        event: &DomainEvent,
    ) -> Result<ReducedEvent, Diagnostic> {
        if event.event_version != 2 {
            return Err(unknown_event_schema(event));
        }
        let Some(cmd) = self.commands.get(&event.event_type) else {
            return Err(unknown_event_schema(event));
        };
        let (current_state, current_facts) = decode_state(state, &self.fact_types)?;
        let (source, index, args) = decode_event_payload(&event.event_type, cmd, event)?;
        if source != current_state || !cmd.from.contains(&source) {
            return Err(Diagnostic::internal(format!(
                "event `{}` source `{source}` does not match current state `{current_state}`",
                event.event_type
            )));
        }
        let outcome = &cmd.outcomes[index];
        let mut new_facts = current_facts;
        for (fact, value) in &outcome.then {
            apply_assignment(&event.event_type, fact, value, &args, &mut new_facts)?;
        }
        let invariants = self.check_invariants(&new_facts, &outcome.to);
        let terminal = self.terminal_states.iter().any(|s| s == &outcome.to);
        let effect = cmd.effect.as_ref().map(|effect| EffectIntentV1 {
            effect_id: EffectId(resolve_source(&effect.id, &args)),
            aggregate_id: AggregateId(String::new()),
            generation: 0,
            effect_kind: effect.kind.clone(),
            payload_digest: resolve_source(&effect.payload_digest, &args),
            idempotency_key: resolve_source(&effect.idempotency_key, &args),
            payload: None,
        });
        Ok(ReducedEvent {
            dest: outcome.to.clone(),
            state: state_value(&outcome.to, &new_facts),
            terminal,
            effect,
            invariants,
            facts: new_facts,
        })
    }
}

fn prepare_command<'a>(
    machine: &'a CompiledMachine,
    state: &Value,
    command: &Value,
) -> Result<PreparedCommand<'a>, Diagnostic> {
    let (current_state, current_facts) = decode_state(state, &machine.fact_types)?;
    let (name, args) = decode_command(command)?;
    let Some(cmd) = machine.commands.get(&name) else {
        return Err(Diagnostic::new(
            DiagnosticCode::IkCommandInvalid,
            Some("command.unknown"),
            format!("unknown command `{name}`"),
        ));
    };
    validate_args(&name, &cmd.args, &args)?;
    let source_legal = cmd.from.contains(&current_state);
    Ok(PreparedCommand {
        name,
        args,
        cmd,
        current_state,
        current_facts,
        source_legal,
    })
}

fn decode_state(
    state: &Value,
    fact_types: &BTreeMap<String, FactInfo>,
) -> Result<(String, BTreeMap<String, Value>), Diagnostic> {
    let obj = state
        .as_object()
        .ok_or_else(|| Diagnostic::internal("state is not a JSON object"))?;
    let name = obj
        .get("state")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Diagnostic::internal("state object has no string `state` field"))?
        .to_string();
    let mut facts = BTreeMap::new();
    for (fact, info) in fact_types {
        if let Some(value) = obj.get(fact) {
            if !matches_type(value, &info.ty) {
                return Err(Diagnostic::internal(format!(
                    "state fact `{fact}` has the wrong type"
                )));
            }
            facts.insert(fact.clone(), value.clone());
        } else if info.policy == "counter" {
            return Err(Diagnostic::internal(format!(
                "state fact `{fact}` (counter) is missing"
            )));
        }
    }
    Ok((name, facts))
}

fn matches_type(value: &Value, ty: &str) -> bool {
    match ty {
        "int" => value.as_i64().is_some(),
        "string" => value.is_string(),
        "bool" => value.is_boolean(),
        _ => false,
    }
}

fn decode_command(command: &Value) -> Result<(String, BTreeMap<String, Value>), Diagnostic> {
    let obj = command.as_object().ok_or_else(|| {
        Diagnostic::new(
            DiagnosticCode::IkCommandInvalid,
            crate::diagnostic::NO_RULE,
            "command must be a JSON object",
        )
    })?;
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCode::IkCommandInvalid,
                crate::diagnostic::NO_RULE,
                "command object requires a string `name`",
            )
        })?
        .to_string();
    let args = match obj.get("args") {
        None => BTreeMap::new(),
        Some(Value::Object(map)) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Some(_) => {
            return Err(Diagnostic::new(
                DiagnosticCode::IkCommandInvalid,
                crate::diagnostic::NO_RULE,
                "command `args` must be an object",
            ))
        }
    };
    Ok((name, args))
}

fn validate_args(
    name: &str,
    declared: &BTreeMap<String, String>,
    args: &BTreeMap<String, Value>,
) -> Result<(), Diagnostic> {
    for (arg, ty) in declared {
        match args.get(arg) {
            None => {
                return Err(Diagnostic::new(
                    DiagnosticCode::IkCommandInvalid,
                    Some(format!("{name}.args")),
                    format!("command `{name}` requires argument `${arg}`"),
                ))
            }
            Some(v) => {
                let ok = match ty.as_str() {
                    "int" => v.as_i64().is_some(),
                    "string" => v.is_string(),
                    "bool" => v.is_boolean(),
                    _ => false,
                };
                if !ok {
                    return Err(Diagnostic::new(
                        DiagnosticCode::IkCommandInvalid,
                        Some(format!("{name}.args")),
                        format!("command `{name}` argument `${arg}` has the wrong type"),
                    ));
                }
            }
        }
    }
    for arg in args.keys() {
        if !declared.contains_key(arg) {
            return Err(Diagnostic::new(
                DiagnosticCode::IkCommandInvalid,
                Some(format!("{name}.args")),
                format!("command `{name}` received unexpected argument `${arg}`"),
            ));
        }
    }
    Ok(())
}

/// Stable refusal for an event this machine does not recognize (unknown command
/// name or event version).
fn unknown_event_schema(event: &DomainEvent) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::IkEventSchemaUnknown,
        Some("event.schema"),
        format!(
            "declarative machine does not recognize event `{}` version {}",
            event.event_type, event.event_version
        ),
    )
}

/// Decode the closed event payload `{source, outcome, args}` frozen by
/// [`Machine::decide`]. The source is checked by [`Machine::apply`].
fn decode_event_payload(
    name: &str,
    cmd: &CompiledCommand,
    event: &DomainEvent,
) -> Result<(String, usize, BTreeMap<String, Value>), Diagnostic> {
    let obj = event.payload.as_object().ok_or_else(|| {
        Diagnostic::internal(format!("event `{name}` payload is not a JSON object"))
    })?;
    if obj.len() != 3
        || !obj.contains_key("source")
        || !obj.contains_key("outcome")
        || !obj.contains_key("args")
    {
        return Err(Diagnostic::internal(format!(
            "event `{name}` payload is not closed to `source`, `outcome`, and `args`"
        )));
    }
    let source = obj
        .get("source")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            Diagnostic::internal(format!("event `{name}` payload `source` is not a string"))
        })?
        .to_string();
    let index = obj.get("outcome").and_then(Value::as_u64).ok_or_else(|| {
        Diagnostic::internal(format!(
            "event `{name}` payload `outcome` is not an integer"
        ))
    })? as usize;
    if index >= cmd.outcomes.len() {
        return Err(Diagnostic::internal(format!(
            "event `{name}` payload `outcome` index {index} is out of bounds"
        )));
    }
    let args = match obj.get("args") {
        Some(Value::Object(map)) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => {
            return Err(Diagnostic::internal(format!(
                "event `{name}` payload `args` is not an object"
            )))
        }
    };
    validate_payload_args(name, &cmd.args, &args)?;
    Ok((source, index, args))
}

/// Fold-path argument validation mirroring [`validate_args`]. Failures are internal
/// because decide already validated the live command before freezing the event.
fn validate_payload_args(
    name: &str,
    declared: &BTreeMap<String, String>,
    args: &BTreeMap<String, Value>,
) -> Result<(), Diagnostic> {
    for (arg, ty) in declared {
        match args.get(arg) {
            None => {
                return Err(Diagnostic::internal(format!(
                    "event `{name}` payload is missing argument `${arg}`"
                )))
            }
            Some(v) => {
                let ok = match ty.as_str() {
                    "int" => v.as_i64().is_some(),
                    "string" => v.is_string(),
                    "bool" => v.is_boolean(),
                    _ => false,
                };
                if !ok {
                    return Err(Diagnostic::internal(format!(
                        "event `{name}` payload argument `${arg}` has the wrong type"
                    )));
                }
            }
        }
    }
    for arg in args.keys() {
        if !declared.contains_key(arg) {
            return Err(Diagnostic::internal(format!(
                "event `{name}` payload has unexpected argument `${arg}`"
            )));
        }
    }
    Ok(())
}

fn apply_assignment(
    name: &str,
    fact: &str,
    value: &ThenValue,
    args: &BTreeMap<String, Value>,
    facts: &mut BTreeMap<String, Value>,
) -> Result<(), Diagnostic> {
    let resolved = match value {
        ThenValue::Literal(v) => v.clone(),
        ThenValue::Arg(arg) => args.get(arg).cloned().ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCode::IkCommandInvalid,
                Some(format!("{name}.args")),
                format!("command `{name}` references missing argument `${arg}`"),
            )
        })?,
        ThenValue::CounterIncr(n) => {
            let current = facts
                .get(fact)
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Diagnostic::internal(format!("counter fact `{fact}` missing")))?;
            let next = current.checked_add(*n).ok_or_else(|| {
                Diagnostic::new(
                    DiagnosticCode::IkInvariantFailed,
                    Some(format!("{name}.then.{fact}")),
                    "integer arithmetic overflow".to_string(),
                )
            })?;
            Value::Number(next.into())
        }
    };
    facts.insert(fact.to_string(), resolved);
    Ok(())
}

fn resolve_source(source: &ValueSource, args: &BTreeMap<String, Value>) -> String {
    match source {
        ValueSource::Literal(v) => v
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| serde_json::to_string(v).unwrap_or_default()),
        ValueSource::Arg(name) => args
            .get(name)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default(),
    }
}

/// Convenience: build the create-command value recorded as the genesis event.
pub fn create_command(aggregate_id: &str, identity: &MachineIdentity) -> Value {
    json!({
        "op": "create",
        "aggregate_id": aggregate_id,
        "machine": identity,
    })
}
