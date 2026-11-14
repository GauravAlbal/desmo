//! Structural graph derived from a validated [`Definition`].
//!
//! This is an analysis view, not an execution IR. Reachability here is
//! structural: guards and command values are not executed.

use super::def::{CommandDef, Definition};
use crate::canonical::digest_json;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Deterministic graph of declared states and ordered outcomes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineGraph {
    pub schema_version: u64,
    pub machine_id: String,
    pub version: u64,
    pub spec_hash: String,
    pub states: Vec<GraphState>,
    pub edges: Vec<GraphEdge>,
}

/// One declared state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphState {
    pub name: String,
    pub initial: bool,
    pub terminal: bool,
}

/// One structural transition. Identity is source/command/outcome/target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub command: String,
    pub outcome_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

/// Structural analyses over a [`MachineGraph`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralAnalysis {
    pub schema_version: u64,
    pub initial: String,
    pub unreachable_states: Vec<String>,
    pub terminal_states: Vec<String>,
    pub structurally_reachable_terminals: Vec<String>,
    pub commands: BTreeMap<String, CommandShape>,
    pub guarded_branches: Vec<GraphEdge>,
    pub cycles: Vec<Vec<String>>,
    pub shortest_paths: BTreeMap<String, StructuralPath>,
}

/// Declared sources and destinations of one command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandShape {
    pub sources: Vec<String>,
    pub destinations: Vec<String>,
    pub outcome_count: usize,
}

/// Shortest structural walk from the initial state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralPath {
    pub states: Vec<String>,
    pub edges: Vec<String>,
}

/// Build a graph directly from a validated definition.
pub fn machine_graph(def: &Definition) -> MachineGraph {
    let states = def
        .states
        .iter()
        .map(|(name, state)| GraphState {
            name: name.clone(),
            initial: state.initial,
            terminal: state.terminal,
        })
        .collect();
    let mut edges = Vec::new();
    for (command, cmd) in &def.commands {
        let outcomes = declared_outcomes(cmd);
        let effect = cmd.effect.as_ref().map(|e| e.kind.clone());
        for source in &cmd.from {
            for (outcome_index, (guard, target)) in outcomes.iter().enumerate() {
                let id = format!("{source}/{command}/{outcome_index}/{target}");
                edges.push(GraphEdge {
                    id,
                    source: source.clone(),
                    command: command.clone(),
                    outcome_index,
                    guard: guard.clone(),
                    target: target.clone(),
                    effect: effect.clone(),
                });
            }
        }
    }
    edges.sort_by(|a, b| {
        (&a.source, &a.command, a.outcome_index, &a.target).cmp(&(
            &b.source,
            &b.command,
            b.outcome_index,
            &b.target,
        ))
    });
    MachineGraph {
        schema_version: 1,
        machine_id: def.machine_id.clone(),
        version: def.machine_version,
        spec_hash: digest_json(def),
        states,
        edges,
    }
}

/// Structural reachability, cycles, and shortest paths. Not data-dependent.
pub fn analyze(graph: &MachineGraph) -> StructuralAnalysis {
    let initial = graph
        .states
        .iter()
        .find(|s| s.initial)
        .map(|s| s.name.clone())
        .unwrap_or_default();
    let (reachable, shortest_paths) = structural_bfs(graph, &initial);
    let terminal_states: Vec<String> = graph
        .states
        .iter()
        .filter(|s| s.terminal)
        .map(|s| s.name.clone())
        .collect();
    let structurally_reachable_terminals = terminal_states
        .iter()
        .filter(|name| reachable.contains(*name))
        .cloned()
        .collect();
    let unreachable_states = graph
        .states
        .iter()
        .map(|s| s.name.clone())
        .filter(|name| !reachable.contains(name))
        .collect();
    StructuralAnalysis {
        schema_version: 1,
        initial,
        unreachable_states,
        terminal_states,
        structurally_reachable_terminals,
        commands: command_shapes(graph),
        guarded_branches: graph
            .edges
            .iter()
            .filter(|e| e.guard.is_some())
            .cloned()
            .collect(),
        cycles: cycles(graph),
        shortest_paths,
    }
}

fn declared_outcomes(cmd: &CommandDef) -> Vec<(Option<String>, String)> {
    if let Some(outcomes) = &cmd.outcomes {
        return outcomes
            .iter()
            .map(|outcome| (outcome.when.clone(), outcome.to.clone()))
            .collect();
    }
    vec![(cmd.when.clone(), cmd.to.clone().unwrap_or_default())]
}

fn command_shapes(graph: &MachineGraph) -> BTreeMap<String, CommandShape> {
    let mut shapes: BTreeMap<String, CommandShape> = BTreeMap::new();
    for edge in &graph.edges {
        let shape = shapes.entry(edge.command.clone()).or_insert(CommandShape {
            sources: Vec::new(),
            destinations: Vec::new(),
            outcome_count: 0,
        });
        if !shape.sources.contains(&edge.source) {
            shape.sources.push(edge.source.clone());
        }
        if !shape.destinations.contains(&edge.target) {
            shape.destinations.push(edge.target.clone());
        }
        shape.outcome_count = shape.outcome_count.max(edge.outcome_index + 1);
    }
    shapes
}

fn structural_bfs(
    graph: &MachineGraph,
    initial: &str,
) -> (BTreeSet<String>, BTreeMap<String, StructuralPath>) {
    let mut by_source: BTreeMap<&str, Vec<&GraphEdge>> = BTreeMap::new();
    for edge in &graph.edges {
        by_source.entry(&edge.source).or_default().push(edge);
    }
    let mut reachable = BTreeSet::new();
    let mut shortest_paths = BTreeMap::new();
    let mut queue = VecDeque::new();
    reachable.insert(initial.to_string());
    shortest_paths.insert(
        initial.to_string(),
        StructuralPath {
            states: vec![initial.to_string()],
            edges: Vec::new(),
        },
    );
    queue.push_back(initial.to_string());
    while let Some(state) = queue.pop_front() {
        let Some(edges) = by_source.get(state.as_str()) else {
            continue;
        };
        let parent = shortest_paths.get(&state).cloned().unwrap();
        for edge in edges {
            if !reachable.insert(edge.target.clone()) {
                continue;
            }
            let mut next = parent.clone();
            next.states.push(edge.target.clone());
            next.edges.push(edge.id.clone());
            shortest_paths.insert(edge.target.clone(), next);
            queue.push_back(edge.target.clone());
        }
    }
    (reachable, shortest_paths)
}

fn cycles(graph: &MachineGraph) -> Vec<Vec<String>> {
    let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for state in &graph.states {
        adj.entry(&state.name).or_default();
    }
    for edge in &graph.edges {
        adj.entry(&edge.source).or_default().push(&edge.target);
    }
    let mut dfs = Tarjan {
        adj: &adj,
        index: 0,
        indices: BTreeMap::new(),
        lowlink: BTreeMap::new(),
        stack: Vec::new(),
        on_stack: BTreeSet::new(),
        sccs: Vec::new(),
    };
    for state in &graph.states {
        if !dfs.indices.contains_key(state.name.as_str()) {
            dfs.visit(state.name.as_str());
        }
    }
    let mut self_loops = BTreeSet::new();
    for edge in &graph.edges {
        if edge.source == edge.target {
            self_loops.insert(edge.source.clone());
        }
    }
    let mut cycles: Vec<Vec<String>> = dfs
        .sccs
        .into_iter()
        .filter(|scc| scc.len() > 1 || scc.iter().any(|s| self_loops.contains(s)))
        .collect();
    cycles.sort();
    cycles
}

struct Tarjan<'a> {
    adj: &'a BTreeMap<&'a str, Vec<&'a str>>,
    index: usize,
    indices: BTreeMap<String, usize>,
    lowlink: BTreeMap<String, usize>,
    stack: Vec<String>,
    on_stack: BTreeSet<String>,
    sccs: Vec<Vec<String>>,
}

impl Tarjan<'_> {
    fn visit(&mut self, v: &str) {
        self.indices.insert(v.to_string(), self.index);
        self.lowlink.insert(v.to_string(), self.index);
        self.index += 1;
        self.stack.push(v.to_string());
        self.on_stack.insert(v.to_string());
        if let Some(nexts) = self.adj.get(v).cloned() {
            for w in nexts {
                if !self.indices.contains_key(w) {
                    self.visit(w);
                    let next = *self.lowlink.get(w).unwrap();
                    let cur = self.lowlink.get_mut(v).unwrap();
                    if next < *cur {
                        *cur = next;
                    }
                } else if self.on_stack.contains(w) {
                    let next = *self.indices.get(w).unwrap();
                    let cur = self.lowlink.get_mut(v).unwrap();
                    if next < *cur {
                        *cur = next;
                    }
                }
            }
        }
        if self.lowlink.get(v) == self.indices.get(v) {
            let mut scc = Vec::new();
            loop {
                let w = self.stack.pop().expect("tarjan stack");
                self.on_stack.remove(&w);
                let done = w == v;
                scc.push(w);
                if done {
                    break;
                }
            }
            scc.sort();
            self.sccs.push(scc);
        }
    }
}
