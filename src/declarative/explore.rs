//! Generic FIFO exploration over authoritative declarative machine semantics.
//!
//! Candidate commands come from the definition plus an input-only case corpus.
//! Answers (target, outcome, terminality) come from the machine evaluator.

use super::analyze::{analyze, machine_graph, MachineGraph, StructuralAnalysis};
use super::compile::CompiledMachine;
use super::def::Definition;
use super::expr::{self, EvalCtx, Expr};
use crate::canonical::{canonical_json, digest_json};
use crate::effect::EffectStatus;
use crate::machine::{Decision, Machine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

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
    /// Shortest witness paths for declared inline `[reach]` predicates.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reach_witnesses: BTreeMap<String, Vec<WitnessStep>>,
    /// Number of candidate transitions pruned by explicit `--bound` predicates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary_pruned: Option<usize>,
}

/// Optional exploration controls modeled on Stateright's boundary, fail-fast, and multi-thread BFS.
#[derive(Clone, Debug, Default)]
pub struct ExploreOptions {
    pub boundary: Vec<(String, Expr)>,
    pub fail_fast: bool,
    pub threads: usize,
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
    explore_with_options(machine, corpus, bounds, &ExploreOptions::default(), false).0
}

/// Explore with explicit boundary predicates, `--fail-fast`, and optional parallel worker threads.
pub fn explore_with_options(
    machine: &CompiledMachine,
    corpus: &CaseCorpus,
    bounds: Bounds,
    options: &ExploreOptions,
    capture_invariant_refusals: bool,
) -> (Exploration, Vec<InvariantRefusalWitness>) {
    let def = machine.definition();
    let graph = machine_graph(def);
    let analysis = analyze(&graph);
    let mut refusals = BTreeMap::new();
    let refusal_target = if capture_invariant_refusals {
        Some(&mut refusals)
    } else {
        None
    };
    let (run, _) = walk(
        machine,
        def,
        &graph,
        &analysis,
        corpus,
        bounds,
        options,
        refusal_target,
        false,
    );
    (run, refusals.into_values().collect())
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
    let options = ExploreOptions::default();
    let (run, traced) = walk(machine, def, &graph, &analysis, corpus, bounds, &options, None, true);
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
    explore_with_options(machine, corpus, bounds, &ExploreOptions::default(), true)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    machine: &CompiledMachine,
    def: &Definition,
    graph: &MachineGraph,
    analysis: &StructuralAnalysis,
    corpus: &CaseCorpus,
    bounds: Bounds,
    options: &ExploreOptions,
    mut invariant_refusals: Option<&mut BTreeMap<String, InvariantRefusalWitness>>,
    build_graph: bool,
) -> (Exploration, Option<RuntimeGraph>) {
    let compiled_reach = compile_reach(def);
    let effect_outcomes = if machine_uses_effect_guards(def) {
        Some(harvest_effect_outcomes(def))
    } else {
        None
    };
    let uses_effect_guards = effect_outcomes.is_some();

    let mut queue = VecDeque::new();
    let mut seen = HashSet::<(u64, u64)>::new();
    let initial = machine.initial_state();
    let initial_effect = EffectStatus::None;

    let mut nodes = 0usize;
    let mut edges = 0usize;
    let mut boundary_pruned = 0usize;
    let mut complete = true;
    let mut incomplete_reason = None;
    let mut executed_commands = BTreeSet::new();
    let mut executed_command_sources = BTreeSet::new();
    let mut executed_outcomes = BTreeSet::new();
    let mut reached_labels = BTreeSet::new();
    let mut reached_terminals = BTreeSet::new();
    let mut refusal_classes = BTreeSet::new();
    let mut witnesses = WitnessBook::default();
    let mut reach_witnesses = BTreeMap::new();

    if state_within_boundary(&initial, &initial_effect, &options.boundary) {
        seen.insert(structural_fingerprint(&initial, &initial_effect, uses_effect_guards));
        queue.push_back(Node {
            state: initial.clone(),
            effect: initial_effect.clone(),
            path: Vec::new(),
            depth: 0,
        });
        record_label(
            &mut witnesses,
            &mut reached_labels,
            &mut reached_terminals,
            &initial,
            &[],
            false,
        );
        check_reach_witnesses(
            &compiled_reach,
            &initial,
            &initial_effect,
            &[],
            &mut reach_witnesses,
        );
    } else {
        boundary_pruned += 1;
    }

    let mut traced = build_graph.then(|| GraphBuilder::start(&initial, &analysis.terminal_states));
    let all_candidates = candidates_for(def, corpus);
    let threads = options.threads.max(1);

    let mut ctx = FoldCtx {
        def,
        bounds,
        options,
        uses_effect_guards,
        compiled_reach: &compiled_reach,
        invariant_refusals: invariant_refusals.as_deref_mut(),
        traced: traced.as_mut(),
        seen: &mut seen,
        queue: &mut queue,
        nodes: &mut nodes,
        edges: &mut edges,
        boundary_pruned: &mut boundary_pruned,
        complete: &mut complete,
        incomplete_reason: &mut incomplete_reason,
        executed_commands: &mut executed_commands,
        executed_command_sources: &mut executed_command_sources,
        executed_outcomes: &mut executed_outcomes,
        reached_labels: &mut reached_labels,
        reached_terminals: &mut reached_terminals,
        refusal_classes: &mut refusal_classes,
        witnesses: &mut witnesses,
        reach_witnesses: &mut reach_witnesses,
        aborted_fail_fast: false,
    };

    if threads <= 1 || build_graph {
        while let Some(node) = ctx.queue.pop_front() {
            if *ctx.nodes >= ctx.bounds.max_nodes {
                *ctx.complete = false;
                *ctx.incomplete_reason = Some("max_nodes".to_string());
                // Push back implicitly accounted for via !queue.is_empty() or break
                break;
            }
            *ctx.nodes += 1;
            let expanded = expand_single_node(
                machine,
                def,
                &node,
                effect_outcomes.as_deref(),
                &all_candidates,
                build_graph,
            );
            fold_expanded_node(&node, expanded, &mut ctx);
            if ctx.aborted_fail_fast {
                break;
            }
        }
    } else {
        while !ctx.queue.is_empty() && !ctx.aborted_fail_fast {
            let remaining = ctx.bounds.max_nodes.saturating_sub(*ctx.nodes);
            if remaining == 0 {
                *ctx.complete = false;
                *ctx.incomplete_reason = Some("max_nodes".to_string());
                break;
            }
            let take = ctx.queue.len().min(remaining);
            let batch: Vec<Node> = ctx.queue.drain(..take).collect();
            *ctx.nodes += batch.len();
            let expanded_batch = expand_level_parallel(
                machine,
                def,
                &batch,
                effect_outcomes.as_deref(),
                &all_candidates,
                threads,
            );
            for (node, expanded) in batch.into_iter().zip(expanded_batch) {
                fold_expanded_node(&node, expanded, &mut ctx);
                if ctx.aborted_fail_fast {
                    break;
                }
            }
        }
    }

    if !ctx.aborted_fail_fast && !queue.is_empty() && complete {
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
        reach_witnesses,
        boundary_pruned: (!options.boundary.is_empty()).then_some(boundary_pruned),
    };
    (exploration, traced.map(GraphBuilder::finish))
}

struct ExpandedEffectStep {
    digest: String,
    recorded_effect: EffectStatus,
}

struct ExpandedCandidate<'a> {
    candidate: &'a Candidate,
    from_mismatch: bool,
    step: Option<Step>,
}

struct ExpandedNode<'a> {
    effect_steps: Vec<ExpandedEffectStep>,
    candidate_steps: Vec<ExpandedCandidate<'a>>,
}

struct FoldCtx<'a, 'b> {
    def: &'a Definition,
    bounds: Bounds,
    options: &'a ExploreOptions,
    uses_effect_guards: bool,
    compiled_reach: &'a [(String, Expr)],
    invariant_refusals: Option<&'b mut BTreeMap<String, InvariantRefusalWitness>>,
    traced: Option<&'b mut GraphBuilder>,
    seen: &'b mut HashSet<(u64, u64)>,
    queue: &'b mut VecDeque<Node>,
    nodes: &'b mut usize,
    edges: &'b mut usize,
    boundary_pruned: &'b mut usize,
    complete: &'b mut bool,
    incomplete_reason: &'b mut Option<String>,
    executed_commands: &'b mut BTreeSet<String>,
    executed_command_sources: &'b mut BTreeSet<CommandSourceRef>,
    executed_outcomes: &'b mut BTreeSet<OutcomeRef>,
    reached_labels: &'b mut BTreeSet<String>,
    reached_terminals: &'b mut BTreeSet<String>,
    refusal_classes: &'b mut BTreeSet<String>,
    witnesses: &'b mut WitnessBook,
    reach_witnesses: &'b mut BTreeMap<String, Vec<WitnessStep>>,
    aborted_fail_fast: bool,
}

fn expand_single_node<'a>(
    machine: &CompiledMachine,
    def: &Definition,
    node: &Node,
    effect_outcomes: Option<&[String]>,
    all_candidates: &'a [Candidate],
    build_graph: bool,
) -> ExpandedNode<'a> {
    let mut effect_steps = Vec::new();
    if let (Some(outcomes), EffectStatus::Pending(intent)) = (effect_outcomes, &node.effect) {
        for digest in outcomes {
            effect_steps.push(ExpandedEffectStep {
                digest: digest.clone(),
                recorded_effect: EffectStatus::Recorded(intent.clone(), digest.clone(), None),
            });
        }
    }
    let cur_state_name = node.state.get("state").and_then(Value::as_str);
    let mut candidate_steps = Vec::with_capacity(all_candidates.len());
    for candidate in all_candidates {
        if !build_graph {
            if let (Some(st), Some(cdef)) =
                (cur_state_name, def.commands.get(&candidate.command_name))
            {
                if !cdef.from.iter().any(|s| s == st) {
                    candidate_steps.push(ExpandedCandidate {
                        candidate,
                        from_mismatch: true,
                        step: None,
                    });
                    continue;
                }
            }
        }
        let step = step_machine(machine, &node.state, &node.effect, &candidate.command);
        candidate_steps.push(ExpandedCandidate {
            candidate,
            from_mismatch: false,
            step: Some(step),
        });
    }
    ExpandedNode {
        effect_steps,
        candidate_steps,
    }
}

fn expand_level_parallel<'a>(
    machine: &CompiledMachine,
    def: &Definition,
    batch: &[Node],
    effect_outcomes: Option<&[String]>,
    all_candidates: &'a [Candidate],
    threads: usize,
) -> Vec<ExpandedNode<'a>> {
    let workers = threads.min(batch.len()).max(1);
    if workers == 1 {
        return batch
            .iter()
            .map(|n| expand_single_node(machine, def, n, effect_outcomes, all_candidates, false))
            .collect();
    }
    let chunk_size = batch.len().div_ceil(workers);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for chunk in batch.chunks(chunk_size) {
            handles.push(scope.spawn(move || {
                chunk
                    .iter()
                    .map(|node| {
                        expand_single_node(
                            machine,
                            def,
                            node,
                            effect_outcomes,
                            all_candidates,
                            false,
                        )
                    })
                    .collect::<Vec<_>>()
            }));
        }
        let mut out = Vec::with_capacity(batch.len());
        for handle in handles {
            out.extend(handle.join().expect("parallel explore worker panicked"));
        }
        out
    })
}

fn fold_expanded_node(node: &Node, expanded: ExpandedNode<'_>, ctx: &mut FoldCtx<'_, '_>) {
    if let Some(gb) = ctx.traced.as_deref_mut() {
        gb.mark_expanded(&node.state);
    }
    for eff in expanded.effect_steps {
        if !state_within_boundary(&node.state, &eff.recorded_effect, &ctx.options.boundary) {
            *ctx.boundary_pruned += 1;
            continue;
        }
        let key = structural_fingerprint(&node.state, &eff.recorded_effect, true);
        if !ctx.seen.insert(key) {
            continue;
        }
        if node.depth + 1 >= ctx.bounds.max_depth {
            *ctx.complete = false;
            *ctx.incomplete_reason = Some("max_depth".to_string());
            continue;
        }
        let mut path = node.path.clone();
        path.push(WitnessStep {
            command: format!("record_effect:{}", eff.digest),
            case: None,
            outcome_index: None,
        });
        check_reach_witnesses(
            ctx.compiled_reach,
            &node.state,
            &eff.recorded_effect,
            &path,
            ctx.reach_witnesses,
        );
        ctx.queue.push_back(Node {
            state: node.state.clone(),
            effect: eff.recorded_effect,
            path,
            depth: node.depth + 1,
        });
    }
    for entry in expanded.candidate_steps {
        if entry.from_mismatch {
            ctx.refusal_classes
                .insert(crate::diagnostic::DiagnosticCode::IkIllegalState.as_str());
            continue;
        }
        let Some(step) = entry.step else {
            continue;
        };
        let candidate = entry.candidate;
        match step {
            Step::Commit {
                next,
                terminal,
                outcome_index,
                planned_effect,
            } => {
                if ctx.uses_effect_guards
                    && planned_effect.is_some()
                    && matches!(node.effect, EffectStatus::Pending(_))
                {
                    ctx.refusal_classes
                        .insert(crate::diagnostic::DiagnosticCode::IkEffectStale.as_str());
                    if let Some(gb) = ctx.traced.as_deref_mut() {
                        gb.record_loop(&node.state, candidate, "refused:IK_EFFECT_STALE");
                    }
                    continue;
                }
                *ctx.edges += 1;
                if let Some(gb) = ctx.traced.as_deref_mut() {
                    gb.record_commit(&node.state, &next, terminal, candidate);
                }
                record_executed(
                    ctx.executed_commands,
                    ctx.executed_command_sources,
                    ctx.executed_outcomes,
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
                record_command(ctx.witnesses, &candidate.command_name, &path);
                record_outcome(ctx.witnesses, &candidate.command_name, outcome_index, &path);
                record_label(
                    ctx.witnesses,
                    ctx.reached_labels,
                    ctx.reached_terminals,
                    &next,
                    &path,
                    terminal,
                );
                let next_effect = if ctx.uses_effect_guards {
                    match planned_effect {
                        Some(intent) => EffectStatus::Pending(intent),
                        None => node.effect.clone(),
                    }
                } else {
                    EffectStatus::None
                };
                check_reach_witnesses(
                    ctx.compiled_reach,
                    &next,
                    &next_effect,
                    &path,
                    ctx.reach_witnesses,
                );
                if !state_within_boundary(&next, &next_effect, &ctx.options.boundary) {
                    *ctx.boundary_pruned += 1;
                    continue;
                }
                let key = structural_fingerprint(&next, &next_effect, ctx.uses_effect_guards);
                if !ctx.seen.insert(key) {
                    continue;
                }
                if terminal {
                    if let Some(gb) = ctx.traced.as_deref_mut() {
                        gb.close_terminal(&next);
                    }
                    continue;
                }
                if node.depth + 1 >= ctx.bounds.max_depth {
                    *ctx.complete = false;
                    *ctx.incomplete_reason = Some("max_depth".to_string());
                    continue;
                }
                ctx.queue.push_back(Node {
                    state: next,
                    effect: next_effect,
                    path,
                    depth: node.depth + 1,
                });
            }
            Step::Noop => {
                if let Some(gb) = ctx.traced.as_deref_mut() {
                    gb.record_loop(&node.state, candidate, "noop");
                }
            }
            Step::Refuse {
                diagnostic,
                outcome_index,
            } => {
                ctx.refusal_classes.insert(diagnostic.code.as_str());
                let prev_len = ctx.invariant_refusals.as_ref().map_or(0, |m| m.len());
                record_invariant_refusal(
                    ctx.def,
                    ctx.invariant_refusals.as_deref_mut(),
                    &node.path,
                    candidate,
                    &diagnostic,
                    outcome_index,
                );
                if ctx.options.fail_fast
                    && ctx
                        .invariant_refusals
                        .as_ref()
                        .is_some_and(|m| m.len() > prev_len)
                {
                    *ctx.complete = false;
                    *ctx.incomplete_reason = Some("fail_fast".to_string());
                    ctx.aborted_fail_fast = true;
                    return;
                }
                if let Some(gb) = ctx.traced.as_deref_mut() {
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

pub(crate) fn record_invariant_refusal(
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

pub(crate) fn machine_effect_outcomes(def: &Definition) -> Option<Vec<String>> {
    if machine_uses_effect_guards(def) {
        Some(harvest_effect_outcomes(def))
    } else {
        None
    }
}

/// Zero-allocation 128-bit structural fingerprint for visited-state deduplication during BFS.
/// Uses two independent 64-bit FNV-1a streams with explicit JSON variant tags.
pub(crate) fn structural_fingerprint(
    state: &Value,
    effect: &EffectStatus,
    track_effect: bool,
) -> (u64, u64) {
    struct Hasher128 {
        h1: u64,
        h2: u64,
    }
    impl Hasher128 {
        fn new() -> Self {
            Self {
                h1: 0xcbf29ce484222325,
                h2: 0xaf63bd4c8601b7df,
            }
        }
        #[inline]
        fn byte(&mut self, b: u8) {
            self.h1 ^= b as u64;
            self.h1 = self.h1.wrapping_mul(0x100000001b3);
            self.h2 ^= (b as u64).wrapping_add(0x9e);
            self.h2 = self.h2.wrapping_mul(0x100000001b3).rotate_left(5);
        }
        #[inline]
        fn bytes(&mut self, data: &[u8]) {
            self.u64(data.len() as u64);
            for &b in data {
                self.byte(b);
            }
        }
        #[inline]
        fn u64(&mut self, v: u64) {
            for b in v.to_le_bytes() {
                self.byte(b);
            }
        }
        fn value(&mut self, v: &Value) {
            match v {
                Value::Null => self.byte(0),
                Value::Bool(b) => {
                    self.byte(1);
                    self.byte(*b as u8);
                }
                Value::Number(n) => {
                    self.byte(2);
                    if let Some(i) = n.as_i64() {
                        self.byte(0);
                        self.u64(i as u64);
                    } else if let Some(u) = n.as_u64() {
                        self.byte(1);
                        self.u64(u);
                    } else {
                        self.byte(2);
                        self.bytes(n.to_string().as_bytes());
                    }
                }
                Value::String(s) => {
                    self.byte(3);
                    self.bytes(s.as_bytes());
                }
                Value::Array(arr) => {
                    self.byte(4);
                    self.u64(arr.len() as u64);
                    for item in arr {
                        self.value(item);
                    }
                }
                Value::Object(map) => {
                    self.byte(5);
                    self.u64(map.len() as u64);
                    for (k, val) in map {
                        self.bytes(k.as_bytes());
                        self.value(val);
                    }
                }
            }
        }
    }
    let mut h = Hasher128::new();
    h.value(state);
    if track_effect {
        match effect {
            EffectStatus::None => h.byte(10),
            EffectStatus::Pending(i) => {
                h.byte(11);
                h.bytes(i.effect_kind.as_bytes());
            }
            EffectStatus::Recorded(i, o, _) => {
                h.byte(12);
                h.bytes(i.effect_kind.as_bytes());
                h.bytes(o.as_bytes());
            }
        }
    }
    (h.h1, h.h2)
}

pub(crate) fn compile_reach(def: &Definition) -> Vec<(String, Expr)> {
    def.reach
        .iter()
        .filter_map(|(name, src)| expr::parse(src).ok().map(|ast| (name.clone(), ast)))
        .collect()
}

pub(crate) fn eval_state_expr(expr: &Expr, state: &Value, effect: &EffectStatus) -> Result<bool, String> {
    let label = state.get("state").and_then(Value::as_str).unwrap_or("");
    let mut facts = BTreeMap::new();
    if let Some(obj) = state.as_object() {
        for (k, v) in obj {
            if k != "state" {
                facts.insert(k.clone(), v.clone());
            }
        }
    }
    let args = BTreeMap::new();
    let ctx = EvalCtx {
        facts: &facts,
        args: &args,
        state: label,
        effect: Some(effect),
    };
    expr::eval(expr, &ctx)
}

pub(crate) fn state_within_boundary(
    state: &Value,
    effect: &EffectStatus,
    boundary: &[(String, Expr)],
) -> bool {
    boundary
        .iter()
        .all(|(_, ast)| eval_state_expr(ast, state, effect).unwrap_or(false))
}

pub(crate) fn check_reach_witnesses(
    reach: &[(String, Expr)],
    state: &Value,
    effect: &EffectStatus,
    path: &[WitnessStep],
    witnesses: &mut BTreeMap<String, Vec<WitnessStep>>,
) {
    for (name, ast) in reach {
        if witnesses.contains_key(name) {
            continue;
        }
        if eval_state_expr(ast, state, effect).unwrap_or(false) {
            witnesses.insert(name.clone(), path.to_vec());
        }
    }
}

pub(crate) struct Candidate {
    pub(crate) command_name: String,
    pub(crate) case: Option<String>,
    pub(crate) command: Value,
}

pub(crate) fn candidates_for(def: &Definition, corpus: &CaseCorpus) -> Vec<Candidate> {
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

pub(crate) enum Step {
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

pub(crate) fn step_machine(
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
