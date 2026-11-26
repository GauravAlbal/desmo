//! Generic FIFO exploration over authoritative declarative machine semantics.
//!
//! Candidate commands come from the definition plus an input-only case corpus.
//! Answers (target, outcome, terminality) come from the machine evaluator.

use super::analyze::{analyze, machine_graph, MachineGraph, StructuralAnalysis};
use super::compile::CompiledMachine;
use super::def::Definition;
use crate::canonical::{canonical_json, digest_json};
use crate::effect::EffectStatus;
use crate::machine::{Decision, Machine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Explicit exploration bound. Exceeding it is incomplete, not silent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    pub max_nodes: usize,
    pub max_depth: usize,
}

/// Input-only cases. No expected targets or diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CaseCorpus {
    pub commands: BTreeMap<String, Vec<InputCase>>,
}

/// One concrete argument assignment for a named command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputCase {
    pub name: String,
    pub args: BTreeMap<String, Value>,
}

/// One step of an executable witness.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessStep {
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_index: Option<usize>,
}

/// Versioned exploration result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Exploration {
    pub schema_version: u64,
    pub machine_id: String,
    pub spec_hash: String,
    pub bound: Bounds,
    pub case_corpus_digest: String,
    pub complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete_reason: Option<String>,
    pub nodes: usize,
    pub edges: usize,
    pub coverage: Coverage,
    pub witnesses: WitnessBook,
}

/// Structural vs bounded executable coverage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    pub declared_states: Vec<String>,
    pub structurally_reachable_states: Vec<String>,
    pub executably_reached_states: Vec<String>,
    pub executably_reached_semantic_states: usize,
    pub declared_commands: Vec<String>,
    pub executed_commands: Vec<String>,
    /// Successful folds keyed by command and source state.
    #[serde(default)]
    pub executed_command_sources: Vec<CommandSourceRef>,
    pub declared_outcomes: Vec<OutcomeRef>,
    pub executed_outcomes: Vec<OutcomeRef>,
    pub unreached_outcomes: Vec<UnreachedOutcome>,
    pub terminal_states: Vec<String>,
    pub reached_terminal_states: Vec<String>,
    pub refusal_classes: Vec<String>,
}

/// Declared or executed (command, outcome_index) pair.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OutcomeRef {
    pub command: String,
    pub outcome_index: usize,
}

/// Successful fold of `command` from `source`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CommandSourceRef {
    pub command: String,
    pub source: String,
}

/// A structurally present outcome the case corpus did not execute.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnreachedOutcome {
    pub command: String,
    pub outcome_index: usize,
    pub source: String,
    pub target: String,
    pub structural: String,
    pub executable: String,
}

/// First-discovered (hence shortest) witnesses.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessBook {
    pub by_state_label: BTreeMap<String, Vec<WitnessStep>>,
    pub by_terminal: BTreeMap<String, Vec<WitnessStep>>,
    pub by_command: BTreeMap<String, Vec<WitnessStep>>,
    pub by_outcome: BTreeMap<String, Vec<WitnessStep>>,
}

/// First shortest path to a proposed transition blocked by a model invariant.
/// This is separate from the stable exploration/counterexample wire formats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvariantRefusalWitness {
    pub diagnostic: crate::diagnostic::Diagnostic,
    pub steps: Vec<WitnessStep>,
}

/// One concrete state the explorer recorded.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub facts: Value,
    pub terminal: bool,
}

/// One evaluated command. Refusals and no-ops are self-loops, not commits.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GraphEdge {
    pub from: String,
    pub command: String,
    pub case: Option<String>,
    pub outcome: String,
    pub to: String,
    pub args: Value,
}

/// A declared input-domain limit. Missing corpus cases are restrictions, not edges.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GraphRestriction {
    pub command: String,
    pub limit: String,
}

/// Finite graph produced by the same walk as [`explore`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RuntimeGraph {
    pub initial: String,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub frontier: Vec<String>,
    pub restrictions: Vec<GraphRestriction>,
}

struct GraphBuilder {
    initial: String,
    nodes: BTreeMap<String, GraphNode>,
    edges: Vec<GraphEdge>,
    expanded: BTreeSet<String>,
    closed: BTreeSet<String>,
}

impl GraphBuilder {
    fn start(state: &Value, terminals: &[String]) -> Self {
        let mut builder = Self {
            initial: String::new(),
            nodes: BTreeMap::new(),
            edges: Vec::new(),
            expanded: BTreeSet::new(),
            closed: BTreeSet::new(),
        };
        let id = builder.ensure(state, label_is_terminal(state, terminals));
        builder.initial = id;
        builder
    }

    fn ensure(&mut self, state: &Value, terminal: bool) -> String {
        let id = node_id(state);
        let node = self.nodes.entry(id.clone()).or_insert_with(|| GraphNode {
            id: id.clone(),
            label: state_label(state),
            facts: fact_object(state),
            terminal,
        });
        if terminal {
            node.terminal = true;
        }
        id
    }

    fn mark_expanded(&mut self, state: &Value) {
        self.expanded.insert(node_id(state));
    }

    fn close_terminal(&mut self, state: &Value) {
        self.closed.insert(node_id(state));
    }

    fn record_commit(&mut self, from: &Value, to: &Value, terminal: bool, candidate: &Candidate) {
        self.push_edge(from, to, terminal, candidate, "committed");
    }

    fn record_loop(&mut self, state: &Value, candidate: &Candidate, outcome: &str) {
        self.ensure(state, false);
        self.push_edge(state, state, false, candidate, outcome);
    }

    fn push_edge(
        &mut self,
        from: &Value,
        to: &Value,
        terminal: bool,
        candidate: &Candidate,
        outcome: &str,
    ) {
        let from_id = self.ensure(from, false);
        let to_id = self.ensure(to, terminal);
        self.edges.push(GraphEdge {
            from: from_id,
            command: candidate.command_name.clone(),
            case: candidate.case.clone(),
            outcome: outcome.to_string(),
            to: to_id,
            args: command_args(&candidate.command),
        });
    }

    fn finish(self) -> RuntimeGraph {
        let frontier = self
            .nodes
            .keys()
            .filter(|id| !self.expanded.contains(*id) && !self.closed.contains(*id))
            .cloned()
            .collect();
        RuntimeGraph {
            initial: self.initial,
            nodes: self.nodes.into_values().collect(),
            edges: self.edges,
            frontier,
            restrictions: Vec::new(),
        }
    }
}

fn node_id(state: &Value) -> String {
    format!("n{}", digest_json(state))
}

fn state_label(state: &Value) -> String {
    state
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn fact_object(state: &Value) -> Value {
    let mut facts = serde_json::Map::new();
    let Some(object) = state.as_object() else {
        return Value::Object(facts);
    };
    for (key, value) in object {
        if key != "state" {
            facts.insert(key.clone(), value.clone());
        }
    }
    Value::Object(facts)
}

fn command_args(command: &Value) -> Value {
    command.get("args").cloned().unwrap_or_else(|| json!({}))
}

fn label_is_terminal(state: &Value, terminals: &[String]) -> bool {
    let Some(label) = state.get("state").and_then(Value::as_str) else {
        return false;
    };
    terminals.iter().any(|name| name == label)
}

fn input_restrictions(def: &Definition, corpus: &CaseCorpus) -> Vec<GraphRestriction> {
    let mut out = Vec::new();
    for (name, cmd) in &def.commands {
        if !command_needs_args(cmd) {
            continue;
        }
        out.push(restriction_for(name, corpus.commands.get(name)));
    }
    out
}

fn command_needs_args(cmd: &super::def::CommandDef) -> bool {
    cmd.args.as_ref().is_some_and(|args| !args.is_empty())
}

fn restriction_for(name: &str, cases: Option<&Vec<InputCase>>) -> GraphRestriction {
    let Some(cases) = cases.filter(|rows| !rows.is_empty()) else {
        return GraphRestriction {
            command: name.to_string(),
            limit: "no_corpus_case".to_string(),
        };
    };
    let listed = cases
        .iter()
        .map(|row| row.name.as_str())
        .collect::<Vec<_>>()
        .join(",");
    GraphRestriction {
        command: name.to_string(),
        limit: format!("corpus_cases:{listed}"),
    }
}

impl CaseCorpus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &mut self,
        command: impl Into<String>,
        name: impl Into<String>,
        args: BTreeMap<String, Value>,
    ) {
        self.commands
            .entry(command.into())
            .or_default()
            .push(InputCase {
                name: name.into(),
                args,
            });
    }

    pub fn digest(&self) -> String {
        digest_json(self)
    }
}

/// Explore from the real initial state using the real evaluator.
pub fn explore(machine: &CompiledMachine, corpus: &CaseCorpus, bounds: Bounds) -> Exploration {
    let def = machine.definition();
    let graph = machine_graph(def);
    let analysis = analyze(&graph);
    walk(machine, def, &graph, &analysis, corpus, bounds, None, false).0
}

/// The same traversal, retaining the finite graph the explorer actually walked.
pub fn explore_graph(
    machine: &CompiledMachine,
    corpus: &CaseCorpus,
    bounds: Bounds,
) -> (Exploration, RuntimeGraph) {
    let def = machine.definition();
    let graph = machine_graph(def);
    let analysis = analyze(&graph);
    let (run, traced) = walk(machine, def, &graph, &analysis, corpus, bounds, None, true);
    let mut traced = traced.expect("graph builder requested");
    traced.restrictions = input_restrictions(def, corpus);
    (run, traced)
}

/// The same traversal, additionally retaining first invariant-refusal paths.
pub fn explore_with_invariant_witnesses(
    machine: &CompiledMachine,
    corpus: &CaseCorpus,
    bounds: Bounds,
) -> (Exploration, Vec<InvariantRefusalWitness>) {
    let def = machine.definition();
    let graph = machine_graph(def);
    let analysis = analyze(&graph);
    let mut refusals = BTreeMap::new();
    let (run, _) = walk(
        machine,
        def,
        &graph,
        &analysis,
        corpus,
        bounds,
        Some(&mut refusals),
        false,
    );
    (run, refusals.into_values().collect())
}

#[allow(clippy::too_many_arguments)]
fn walk(
    machine: &CompiledMachine,
    def: &Definition,
    graph: &MachineGraph,
    analysis: &StructuralAnalysis,
    corpus: &CaseCorpus,
    bounds: Bounds,
    mut invariant_refusals: Option<&mut BTreeMap<String, InvariantRefusalWitness>>,
    build_graph: bool,
) -> (Exploration, Option<RuntimeGraph>) {
    let uses_effect_guards = machine_uses_effect_guards(def);
    let effect_outcomes = if uses_effect_guards {
        harvest_effect_outcomes(def)
    } else {
        Vec::new()
    };

    let mut queue = VecDeque::new();
    let mut seen = std::collections::HashSet::<[u8; 32]>::new();
    let initial = machine.initial_state();
    let initial_effect = EffectStatus::None;
    let initial_key = node_seen_key(&initial, &initial_effect, uses_effect_guards);
    seen.insert(initial_key);
    queue.push_back(Node {
        state: initial.clone(),
        effect: initial_effect,
        path: Vec::new(),
        depth: 0,
    });

    let mut nodes = 0usize;
    let mut edges = 0usize;
    let mut complete = true;
    let mut incomplete_reason = None;
    let mut executed_commands = BTreeSet::new();
    let mut executed_command_sources = BTreeSet::new();
    let mut executed_outcomes = BTreeSet::new();
    let mut reached_labels = BTreeSet::new();
    let mut reached_terminals = BTreeSet::new();
    let mut refusal_classes = BTreeSet::new();
    let mut witnesses = WitnessBook::default();

    record_label(
        &mut witnesses,
        &mut reached_labels,
        &mut reached_terminals,
        &initial,
        &[],
        false,
    );
    let mut traced = build_graph.then(|| GraphBuilder::start(&initial, &analysis.terminal_states));
    let all_candidates = candidates(def, corpus);

    while let Some(node) = queue.pop_front() {
        nodes += 1;
        if nodes > bounds.max_nodes {
            complete = false;
            incomplete_reason = Some("max_nodes".to_string());
            break;
        }
        if let Some(gb) = traced.as_mut() {
            gb.mark_expanded(&node.state);
        }
        if uses_effect_guards {
            if let EffectStatus::Pending(intent) = &node.effect {
                for digest in &effect_outcomes {
                    let recorded_effect =
                        EffectStatus::Recorded(intent.clone(), digest.clone(), None);
                    let key = node_seen_key(&node.state, &recorded_effect, true);
                    if !seen.insert(key) {
                        continue;
                    }
                    if node.depth + 1 >= bounds.max_depth {
                        complete = false;
                        incomplete_reason = Some("max_depth".to_string());
                        continue;
                    }
                    let mut path = node.path.clone();
                    path.push(WitnessStep {
                        command: format!("record_effect:{digest}"),
                        case: None,
                        outcome_index: None,
                    });
                    queue.push_back(Node {
                        state: node.state.clone(),
                        effect: recorded_effect,
                        path,
                        depth: node.depth + 1,
                    });
                }
                continue;
            }
        }
        let cur_state_name = node.state.get("state").and_then(Value::as_str);
        for candidate in &all_candidates {
            if !build_graph {
                if let (Some(st), Some(cdef)) =
                    (cur_state_name, def.commands.get(&candidate.command_name))
                {
                    if !cdef.from.iter().any(|s| s == st) {
                        refusal_classes.insert(crate::diagnostic::DiagnosticCode::IkIllegalState.as_str());
                        continue;
                    }
                }
            }
            match classify(machine, &node.state, &node.effect, &candidate.command) {
                Step::Commit {
                    next,
                    terminal,
                    outcome_index,
                    planned_effect,
                } => {
                    edges += 1;
                    if let Some(gb) = traced.as_mut() {
                        gb.record_commit(&node.state, &next, terminal, candidate);
                    }
                    record_executed(
                        &mut executed_commands,
                        &mut executed_command_sources,
                        &mut executed_outcomes,
                        &node.state,
                        &candidate.command_name,
                        outcome_index,
                    );
                    let mut path = node.path.clone();
                    path.push(WitnessStep {
                        command: candidate.command_name.clone(),
                        case: candidate.case.clone(),
                        outcome_index: Some(outcome_index),
                    });
                    record_command(&mut witnesses, &candidate.command_name, &path);
                    record_outcome(
                        &mut witnesses,
                        &candidate.command_name,
                        outcome_index,
                        &path,
                    );
                    record_label(
                        &mut witnesses,
                        &mut reached_labels,
                        &mut reached_terminals,
                        &next,
                        &path,
                        terminal,
                    );
                    let next_effect = if uses_effect_guards {
                        match planned_effect {
                            Some(intent) => EffectStatus::Pending(intent),
                            None => node.effect.clone(),
                        }
                    } else {
                        EffectStatus::None
                    };
                    let key = node_seen_key(&next, &next_effect, uses_effect_guards);
                    if !seen.insert(key) {
                        continue;
                    }
                    if terminal {
                        if let Some(gb) = traced.as_mut() {
                            gb.close_terminal(&next);
                        }
                        continue;
                    }
                    if node.depth + 1 >= bounds.max_depth {
                        complete = false;
                        incomplete_reason = Some("max_depth".to_string());
                        continue;
                    }
                    queue.push_back(Node {
                        state: next,
                        effect: next_effect,
                        path,
                        depth: node.depth + 1,
                    });
                }
                Step::Noop => {
                    if let Some(gb) = traced.as_mut() {
                        gb.record_loop(&node.state, candidate, "noop");
                    }
                }
                Step::Refuse {
                    diagnostic,
                    outcome_index,
                } => {
                    refusal_classes.insert(diagnostic.code.as_str());
                    record_invariant_refusal(
                        def,
                        invariant_refusals.as_deref_mut(),
                        &node.path,
                        candidate,
                        &diagnostic,
                        outcome_index,
                    );
                    if let Some(gb) = traced.as_mut() {
                        gb.record_loop(
                            &node.state,
                            candidate,
                            &format!("refused:{}", diagnostic.code.as_str()),
                        );
                    }
                }
            }
        }
    }
    if !queue.is_empty() && complete {
        complete = false;
        incomplete_reason = Some("max_nodes".to_string());
    }

    let exploration = Exploration {
        schema_version: 1,
        machine_id: def.machine_id.clone(),
        spec_hash: machine.spec_hash().0.clone(),
        bound: bounds,
        case_corpus_digest: corpus.digest(),
        complete,
        incomplete_reason,
        nodes,
        edges,
        coverage: coverage(
            graph,
            analysis,
            Executed {
                labels: &reached_labels,
                semantic_states: seen.len(),
                commands: &executed_commands,
                command_sources: &executed_command_sources,
                outcomes: &executed_outcomes,
                terminals: &reached_terminals,
                refusals: &refusal_classes,
            },
        ),
        witnesses,
    };
    (exploration, traced.map(GraphBuilder::finish))
}

fn record_invariant_refusal(
    def: &Definition,
    refusals: Option<&mut BTreeMap<String, InvariantRefusalWitness>>,
    path: &[WitnessStep],
    candidate: &Candidate,
    diagnostic: &crate::diagnostic::Diagnostic,
    outcome_index: Option<usize>,
) {
    let Some(refusals) = refusals else {
        return;
    };
    if outcome_index.is_none()
        || diagnostic.code != crate::diagnostic::DiagnosticCode::IkInvariantFailed
    {
        return;
    }
    let Some(rule) = diagnostic.rule.as_deref() else {
        return;
    };
    let declared = def.invariants.contains_key(rule)
        || rule
            .strip_prefix("invariants.")
            .and_then(|path| {
                path.strip_suffix(".when")
                    .or_else(|| path.strip_suffix(".assert"))
            })
            .is_some_and(|name| def.invariants.contains_key(name));
    if !declared {
        return;
    }
    refusals.entry(rule.to_string()).or_insert_with(|| {
        let mut steps = path.to_vec();
        steps.push(WitnessStep {
            command: candidate.command_name.clone(),
            case: candidate.case.clone(),
            outcome_index,
        });
        InvariantRefusalWitness {
            diagnostic: diagnostic.clone(),
            steps,
        }
    });
}

struct Node {
    state: Value,
    effect: EffectStatus,
    path: Vec<WitnessStep>,
    depth: usize,
}

fn machine_uses_effect_guards(def: &Definition) -> bool {
    for cmd in def.commands.values() {
        if let Some(w) = &cmd.when {
            if super::expr::parse(w).is_ok_and(|e| super::expr::uses_effect_operands(&e)) {
                return true;
            }
        }
        if let Some(outcomes) = &cmd.outcomes {
            for o in outcomes {
                if let Some(w) = &o.when {
                    if super::expr::parse(w).is_ok_and(|e| super::expr::uses_effect_operands(&e)) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn harvest_effect_outcomes(def: &Definition) -> Vec<String> {
    let mut set = BTreeSet::new();
    for cmd in def.commands.values() {
        for w in cmd
            .when
            .iter()
            .chain(cmd.outcomes.iter().flatten().filter_map(|o| o.when.as_ref()))
        {
            if let Ok(ast) = super::expr::parse(w) {
                collect_effect_outcome_literals(&ast, &mut set);
            }
        }
    }
    if set.is_empty() {
        set.insert("succeeded".to_string());
        set.insert("failed".to_string());
    }
    set.into_iter().collect()
}

fn collect_effect_outcome_literals(ast: &super::expr::Expr, out: &mut BTreeSet<String>) {
    use super::expr::{Expr, Operand};
    match ast {
        Expr::Or(parts) | Expr::And(parts) => {
            for p in parts {
                collect_effect_outcome_literals(p, out);
            }
        }
        Expr::Cmp(cmp) => match (&cmp.left, &cmp.right) {
            (Operand::EffectOutcome, Operand::Literal(Value::String(s)))
            | (Operand::Literal(Value::String(s)), Operand::EffectOutcome) => {
                out.insert(s.clone());
            }
            _ => {}
        },
        _ => {}
    }
}

fn node_seen_key(state: &Value, effect: &EffectStatus, track_effect: bool) -> [u8; 32] {
    if !track_effect {
        return crate::canonical::sha256_value_raw(state);
    }
    let eff = match effect {
        EffectStatus::None => json!({"s": "none"}),
        EffectStatus::Pending(i) => json!({"s": "pending", "k": i.effect_kind}),
        EffectStatus::Recorded(i, o, _) => json!({"s": "recorded", "k": i.effect_kind, "o": o}),
    };
    crate::canonical::sha256_value_raw(&json!([state, eff]))
}

struct Candidate {
    command_name: String,
    case: Option<String>,
    command: Value,
}

fn candidates(def: &Definition, corpus: &CaseCorpus) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (name, cmd) in &def.commands {
        let needs_args = cmd
            .args
            .as_ref()
            .map(|args| !args.is_empty())
            .unwrap_or(false);
        if needs_args {
            let Some(cases) = corpus.commands.get(name) else {
                continue;
            };
            for case in cases {
                out.push(Candidate {
                    command_name: name.clone(),
                    case: Some(case.name.clone()),
                    command: json!({ "name": name, "args": case.args }),
                });
            }
        } else {
            out.push(Candidate {
                command_name: name.clone(),
                case: None,
                command: json!({ "name": name }),
            });
        }
    }
    out
}

enum Step {
    Commit {
        next: Value,
        terminal: bool,
        outcome_index: usize,
        planned_effect: Option<crate::effect::EffectIntentV1>,
    },
    Noop,
    Refuse {
        diagnostic: crate::diagnostic::Diagnostic,
        outcome_index: Option<usize>,
    },
}

fn classify(
    machine: &dyn Machine,
    state: &Value,
    effect: &EffectStatus,
    command: &Value,
) -> Step {
    let decision = match machine.decide(state, None, effect, command) {
        Ok(decision) => decision,
        Err(diagnostic) => {
            return Step::Refuse {
                diagnostic,
                outcome_index: None,
            };
        }
    };
    let Decision::Emit { event, .. } = decision else {
        return Step::Noop;
    };
    let outcome_index = event
        .payload
        .get("outcome")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    match machine.apply(state, &event) {
        Ok(applied) => Step::Commit {
            next: applied.state,
            terminal: applied.terminal,
            outcome_index,
            planned_effect: applied.effect,
        },
        Err(diagnostic) => Step::Refuse {
            diagnostic,
            outcome_index: Some(outcome_index),
        },
    }
}

fn record_label(
    witnesses: &mut WitnessBook,
    labels: &mut BTreeSet<String>,
    terminals: &mut BTreeSet<String>,
    state: &Value,
    path: &[WitnessStep],
    terminal: bool,
) {
    let Some(label) = state.get("state").and_then(Value::as_str) else {
        return;
    };
    if labels.insert(label.to_string()) {
        witnesses
            .by_state_label
            .insert(label.to_string(), path.to_vec());
    }
    if terminal && terminals.insert(label.to_string()) {
        witnesses
            .by_terminal
            .insert(label.to_string(), path.to_vec());
    }
}

fn record_executed(
    commands: &mut BTreeSet<String>,
    command_sources: &mut BTreeSet<CommandSourceRef>,
    outcomes: &mut BTreeSet<OutcomeRef>,
    source_state: &Value,
    command_name: &str,
    outcome_index: usize,
) {
    commands.insert(command_name.to_string());
    if let Some(source) = source_state.get("state").and_then(Value::as_str) {
        command_sources.insert(CommandSourceRef {
            command: command_name.to_string(),
            source: source.to_string(),
        });
    }
    outcomes.insert(OutcomeRef {
        command: command_name.to_string(),
        outcome_index,
    });
}

fn record_command(witnesses: &mut WitnessBook, command: &str, path: &[WitnessStep]) {
    witnesses
        .by_command
        .entry(command.to_string())
        .or_insert_with(|| path.to_vec());
}
fn record_outcome(
    witnesses: &mut WitnessBook,
    command: &str,
    outcome_index: usize,
    path: &[WitnessStep],
) {
    let key = format!("{command}.{outcome_index}");
    witnesses
        .by_outcome
        .entry(key)
        .or_insert_with(|| path.to_vec());
}

struct Executed<'a> {
    labels: &'a BTreeSet<String>,
    semantic_states: usize,
    commands: &'a BTreeSet<String>,
    command_sources: &'a BTreeSet<CommandSourceRef>,
    outcomes: &'a BTreeSet<OutcomeRef>,
    terminals: &'a BTreeSet<String>,
    refusals: &'a BTreeSet<String>,
}

fn coverage(
    graph: &MachineGraph,
    analysis: &StructuralAnalysis,
    executed: Executed<'_>,
) -> Coverage {
    let declared_states: Vec<String> = graph.states.iter().map(|s| s.name.clone()).collect();
    let structurally_reachable_states: Vec<String> =
        analysis.shortest_paths.keys().cloned().collect();
    let declared_commands: Vec<String> = analysis.commands.keys().cloned().collect();
    let mut declared_outcomes = BTreeSet::new();
    let mut unreached = Vec::new();
    for edge in &graph.edges {
        let refer = OutcomeRef {
            command: edge.command.clone(),
            outcome_index: edge.outcome_index,
        };
        declared_outcomes.insert(refer.clone());
        if !executed.outcomes.contains(&refer) {
            unreached.push(UnreachedOutcome {
                command: edge.command.clone(),
                outcome_index: edge.outcome_index,
                source: edge.source.clone(),
                target: edge.target.clone(),
                structural: "structurally_present".to_string(),
                executable: "executably_unreached".to_string(),
            });
        }
    }
    Coverage {
        declared_states,
        structurally_reachable_states,
        executably_reached_states: executed.labels.iter().cloned().collect(),
        executably_reached_semantic_states: executed.semantic_states,
        declared_commands,
        executed_commands: executed.commands.iter().cloned().collect(),
        executed_command_sources: executed.command_sources.iter().cloned().collect(),
        declared_outcomes: declared_outcomes.into_iter().collect(),
        executed_outcomes: executed.outcomes.iter().cloned().collect(),
        unreached_outcomes: unreached,
        terminal_states: analysis.terminal_states.clone(),
        reached_terminal_states: executed.terminals.iter().cloned().collect(),
        refusal_classes: executed.refusals.iter().cloned().collect(),
    }
}

/// Canonical serialization of a witness path.
pub fn witness_json(path: &[WitnessStep]) -> String {
    canonical_json(path)
}
