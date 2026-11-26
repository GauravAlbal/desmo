//! Structural analytics for a lifecycle diagram.
//!
//! Reachability, finish distances, and loops ignore guard values: they describe
//! the declared graph, not what will happen at runtime. Every claim the rest of
//! the diagram prints is computed here so a reader never has to walk the machine.

use crate::declarative::analyze::{machine_graph, GraphEdge, MachineGraph};
use crate::declarative::expr::{self, Expr, Operand};
use crate::declarative::Definition;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const TIME_MARKERS: &[&str] = &["_ms", "until", "deadline", "time", "span"];
const COUNT_MARKERS: &[&str] = &["count", "overdue", "qty", "bound", "limit", "n_"];

/// Guard class derived from the definition text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GuardKind {
    None,
    StoredFact,
    Membership,
    Count,
    Time,
    Grammar,
}

/// One command that is enabled from a state on the declared graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnabledCmd {
    pub command: String,
    pub to: String,
    pub guarded: bool,
    pub guard_kind: GuardKind,
    pub effect: bool,
}

/// A strongly-connected component that contains a cycle, named by a resident command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoopInfo {
    pub states: Vec<String>,
    pub command: String,
    pub label: String,
}

/// Structural facts about one definition. Guard values are not executed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Analytics {
    pub initial: String,
    pub terminals: Vec<String>,
    pub unreachable: Vec<String>,
    pub dead_ends: Vec<String>,
    pub traps: Vec<String>,
    pub loops_only_traps: Vec<String>,
    pub cannot_finish: Vec<String>,
    pub every_state_reaches_end: bool,
    pub loops: Vec<LoopInfo>,
    pub shortest_finish: BTreeMap<String, Option<u32>>,
    pub shortest_from_initial: Option<u32>,
    pub longest_finish: Option<u32>,
    pub enabled: BTreeMap<String, Vec<EnabledCmd>>,
    pub guarded_commands: Vec<String>,
    pub effect_commands: Vec<String>,
    pub invariant_count: usize,
    pub command_count: usize,
    pub state_count: usize,
    pub unreachable_commands: Vec<String>,
}

/// Reachability and cohort delta between two versions of the same machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeAnalytics {
    pub states_added: Vec<String>,
    pub states_removed: Vec<String>,
    pub commands_added: Vec<String>,
    pub commands_removed: Vec<String>,
    pub guards_changed: Vec<String>,
    pub facts_added: Vec<String>,
    pub newly_reachable: Vec<String>,
    pub no_longer_reachable: Vec<String>,
    pub new_dead_ends: Vec<String>,
    pub finish_changed: Vec<String>,
    pub every_state_still_reaches_end: bool,
    pub old_every_reaches_end: bool,
    pub reachability_line: String,
}

/// Compute structural analytics for one definition.
pub fn analyze_def(def: &Definition) -> Analytics {
    let graph = machine_graph(def);
    analyze_graph(def, &graph)
}

fn analyze_graph(def: &Definition, graph: &MachineGraph) -> Analytics {
    let initial = graph
        .states
        .iter()
        .find(|state| state.initial)
        .map(|state| state.name.clone())
        .unwrap_or_default();
    let terminals: Vec<String> = graph
        .states
        .iter()
        .filter(|state| state.terminal)
        .map(|state| state.name.clone())
        .collect();
    let forward = bfs_from(graph, &initial);
    let finish = finish_distances(graph, &terminals);
    let unreachable: Vec<String> = graph
        .states
        .iter()
        .map(|state| state.name.clone())
        .filter(|name| !forward.contains(name))
        .collect();
    let cannot_finish: Vec<String> = graph
        .states
        .iter()
        .map(|state| state.name.clone())
        .filter(|name| finish.get(name).copied().flatten().is_none())
        .collect();
    let dead_ends = dead_end_states(graph, &unreachable);
    let (traps, loops_only_traps) = trap_states(graph, &cannot_finish, &dead_ends, &unreachable);
    let loops = named_loops(graph);
    let enabled = enabled_map(graph);
    let guarded_commands = guarded_of(def);
    let effect_commands = effect_of(def);
    let unreachable_commands = commands_from_unreachable(graph, &unreachable);
    let shortest_from_initial = finish.get(&initial).copied().flatten();
    let longest_finish = longest_on_condensation(graph, &terminals);
    Analytics {
        initial,
        terminals,
        unreachable,
        dead_ends,
        traps,
        loops_only_traps,
        cannot_finish: cannot_finish.clone(),
        every_state_reaches_end: cannot_finish.is_empty(),
        loops,
        shortest_finish: finish,
        shortest_from_initial,
        longest_finish,
        enabled,
        guarded_commands,
        effect_commands,
        invariant_count: def.invariants.len(),
        command_count: def.commands.len(),
        state_count: def.states.len(),
        unreachable_commands,
    }
}

/// Compare two definitions. Differences are structural; guard values are not executed.
pub fn analyze_change(prior: &Definition, next: &Definition) -> ChangeAnalytics {
    let old = analyze_def(prior);
    let new = analyze_def(next);
    let states_added = added(prior.states.keys(), next.states.keys());
    let states_removed = added(next.states.keys(), prior.states.keys());
    let commands_added = added(prior.commands.keys(), next.commands.keys());
    let commands_removed = added(next.commands.keys(), prior.commands.keys());
    let facts_added = added(prior.facts.keys(), next.facts.keys());
    let guards_changed = guards_changed(prior, next);
    let newly_reachable = added(new.unreachable.iter(), old.unreachable.iter());
    let no_longer_reachable = added(old.unreachable.iter(), new.unreachable.iter());
    let new_dead_ends = added(old.dead_ends.iter(), new.dead_ends.iter());
    let finish_changed = finish_delta(&old, &new);
    let reachability_line = reachability_line(
        &newly_reachable,
        &no_longer_reachable,
        &new_dead_ends,
        &finish_changed,
        new.every_state_reaches_end,
    );
    ChangeAnalytics {
        states_added,
        states_removed,
        commands_added,
        commands_removed,
        guards_changed,
        facts_added,
        newly_reachable,
        no_longer_reachable,
        new_dead_ends,
        finish_changed,
        every_state_still_reaches_end: new.every_state_reaches_end,
        old_every_reaches_end: old.every_state_reaches_end,
        reachability_line,
    }
}

/// Classify one guard string. Unknown syntax falls back to name heuristics.
pub fn classify_guard(text: &str) -> GuardKind {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return GuardKind::None;
    }
    match expr::parse(trimmed) {
        Ok(expr) => pick_kind(kinds_in(&expr)),
        Err(_) => heuristic_kind(trimmed),
    }
}

fn added<'a, I, J>(old: I, new: J) -> Vec<String>
where
    I: Iterator<Item = &'a String>,
    J: Iterator<Item = &'a String>,
{
    let prior: BTreeSet<&str> = old.map(String::as_str).collect();
    new.map(String::as_str)
        .filter(|name| !prior.contains(name))
        .map(str::to_string)
        .collect()
}

fn guards_changed(prior: &Definition, next: &Definition) -> Vec<String> {
    let mut out = Vec::new();
    for (name, cmd) in &next.commands {
        let Some(old) = prior.commands.get(name) else {
            continue;
        };
        if old.when != cmd.when {
            out.push(name.clone());
            continue;
        }
        let old_out = old
            .outcomes
            .as_ref()
            .map(|rows| rows.iter().map(|row| row.when.clone()).collect::<Vec<_>>());
        let new_out = cmd
            .outcomes
            .as_ref()
            .map(|rows| rows.iter().map(|row| row.when.clone()).collect::<Vec<_>>());
        if old_out != new_out {
            out.push(name.clone());
        }
    }
    out
}

fn finish_delta(old: &Analytics, new: &Analytics) -> Vec<String> {
    let mut names: BTreeSet<&str> = BTreeSet::new();
    names.extend(old.shortest_finish.keys().map(String::as_str));
    names.extend(new.shortest_finish.keys().map(String::as_str));
    names
        .into_iter()
        .filter(|name| old.shortest_finish.get(*name) != new.shortest_finish.get(*name))
        .map(str::to_string)
        .collect()
}

fn reachability_line(
    newly_reachable: &[String],
    no_longer_reachable: &[String],
    new_dead_ends: &[String],
    finish_changed: &[String],
    every_end: bool,
) -> String {
    if newly_reachable.is_empty()
        && no_longer_reachable.is_empty()
        && new_dead_ends.is_empty()
        && finish_changed.is_empty()
    {
        return "Reachability: unchanged (structural)".into();
    }
    let mut parts = Vec::new();
    if !newly_reachable.is_empty() {
        parts.push(format!("newly reachable {}", newly_reachable.join(", ")));
    }
    if !no_longer_reachable.is_empty() {
        parts.push(format!(
            "no longer reachable {}",
            no_longer_reachable.join(", ")
        ));
    }
    if !new_dead_ends.is_empty() {
        parts.push(format!("new dead ends {}", new_dead_ends.join(", ")));
    }
    if !finish_changed.is_empty() {
        parts.push(format!(
            "shortest finish changed at {}",
            finish_changed.join(", ")
        ));
    }
    if !every_end {
        parts.push("not every state reaches an end".into());
    }
    format!("Reachability: {} (structural)", parts.join("; "))
}

fn bfs_from(graph: &MachineGraph, start: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    if start.is_empty() {
        return seen;
    }
    seen.insert(start.to_string());
    queue.push_back(start.to_string());
    while let Some(node) = queue.pop_front() {
        for edge in &graph.edges {
            if edge.source != node {
                continue;
            }
            if seen.insert(edge.target.clone()) {
                queue.push_back(edge.target.clone());
            }
        }
    }
    seen
}

fn finish_distances(graph: &MachineGraph, terminals: &[String]) -> BTreeMap<String, Option<u32>> {
    let mut reverse: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for state in &graph.states {
        reverse.entry(state.name.as_str()).or_default();
    }
    for edge in &graph.edges {
        reverse
            .entry(edge.target.as_str())
            .or_default()
            .push(edge.source.as_str());
    }
    let mut dist: BTreeMap<String, u32> = BTreeMap::new();
    let mut queue = VecDeque::new();
    for terminal in terminals {
        dist.insert(terminal.clone(), 0);
        queue.push_back(terminal.clone());
    }
    while let Some(node) = queue.pop_front() {
        let next = dist.get(&node).copied().unwrap_or(0) + 1;
        let Some(preds) = reverse.get(node.as_str()) else {
            continue;
        };
        for pred in preds {
            if dist.contains_key(*pred) {
                continue;
            }
            dist.insert((*pred).to_string(), next);
            queue.push_back((*pred).to_string());
        }
    }
    graph
        .states
        .iter()
        .map(|state| (state.name.clone(), dist.get(&state.name).copied()))
        .collect()
}

fn trap_states(
    graph: &MachineGraph,
    cannot_finish: &[String],
    dead_ends: &[String],
    unreachable: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut outgoing: BTreeMap<&str, Vec<&GraphEdge>> = BTreeMap::new();
    for edge in &graph.edges {
        outgoing.entry(edge.source.as_str()).or_default().push(edge);
    }
    let mut traps = Vec::new();
    let mut loops_only = Vec::new();
    for name in cannot_finish {
        if dead_ends.iter().any(|dead| dead == name) || unreachable.iter().any(|u| u == name) {
            continue;
        }
        let Some(edges) = outgoing.get(name.as_str()) else {
            continue;
        };
        if edges.is_empty() {
            continue;
        }
        traps.push(name.clone());
        if edges.iter().all(|edge| edge.source == edge.target) {
            loops_only.push(name.clone());
        }
    }
    (traps, loops_only)
}

fn dead_end_states(graph: &MachineGraph, unreachable: &[String]) -> Vec<String> {
    let mut outgoing: BTreeSet<&str> = BTreeSet::new();
    for edge in &graph.edges {
        outgoing.insert(edge.source.as_str());
    }
    graph
        .states
        .iter()
        .filter(|state| {
            !state.terminal
                && !outgoing.contains(state.name.as_str())
                && !unreachable.iter().any(|name| name == &state.name)
        })
        .map(|state| state.name.clone())
        .collect()
}

fn commands_from_unreachable(graph: &MachineGraph, unreachable: &[String]) -> Vec<String> {
    let bad: BTreeSet<&str> = unreachable.iter().map(String::as_str).collect();
    let mut by_cmd: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for edge in &graph.edges {
        by_cmd
            .entry(edge.command.as_str())
            .or_default()
            .insert(edge.source.as_str());
    }
    let mut out = Vec::new();
    for (command, sources) in by_cmd {
        if !sources.is_empty() && sources.iter().all(|src| bad.contains(src)) {
            out.push(command.to_string());
        }
    }
    out
}

fn named_loops(graph: &MachineGraph) -> Vec<LoopInfo> {
    let sccs = sccs_iterative(graph);
    let self_loops = self_loop_commands(graph);
    let mut loops = Vec::new();
    for scc in sccs {
        let cyclic = scc.len() > 1 || self_loops.contains_key(&scc[0]);
        if !cyclic {
            continue;
        }
        let command = loop_command(graph, &scc, &self_loops);
        let host = scc[0].clone();
        let label = format!("{host} ↺ {command}");
        loops.push(LoopInfo {
            states: scc,
            command,
            label,
        });
    }
    loops
}

fn self_loop_commands(graph: &MachineGraph) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for edge in &graph.edges {
        if edge.source == edge.target {
            out.entry(edge.source.clone())
                .or_insert_with(|| edge.command.clone());
        }
    }
    out
}

fn loop_command(
    graph: &MachineGraph,
    scc: &[String],
    self_loops: &BTreeMap<String, String>,
) -> String {
    if let Some(name) = scc.first().and_then(|state| self_loops.get(state)) {
        return name.clone();
    }
    let members: BTreeSet<&str> = scc.iter().map(String::as_str).collect();
    for edge in &graph.edges {
        if members.contains(edge.source.as_str()) && members.contains(edge.target.as_str()) {
            return edge.command.clone();
        }
    }
    String::from("loop")
}

/// Iterative Tarjan. Recursion is bounded by the explicit stack, not the call stack.
pub fn sccs_iterative(graph: &MachineGraph) -> Vec<Vec<String>> {
    let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for state in &graph.states {
        adj.entry(state.name.clone()).or_default();
    }
    for edge in &graph.edges {
        adj.entry(edge.source.clone())
            .or_default()
            .push(edge.target.clone());
    }
    let mut index = 0usize;
    let mut indices: BTreeMap<String, usize> = BTreeMap::new();
    let mut lowlink: BTreeMap<String, usize> = BTreeMap::new();
    let mut stack: Vec<String> = Vec::new();
    let mut on_stack: BTreeSet<String> = BTreeSet::new();
    let mut sccs: Vec<Vec<String>> = Vec::new();
    for state in &graph.states {
        if indices.contains_key(&state.name) {
            continue;
        }
        walk_scc(
            &state.name,
            &adj,
            &mut index,
            &mut indices,
            &mut lowlink,
            &mut stack,
            &mut on_stack,
            &mut sccs,
        );
    }
    sccs.sort();
    sccs
}

struct Frame {
    node: String,
    edge_i: usize,
}

fn walk_scc(
    start: &str,
    adj: &BTreeMap<String, Vec<String>>,
    index: &mut usize,
    indices: &mut BTreeMap<String, usize>,
    lowlink: &mut BTreeMap<String, usize>,
    stack: &mut Vec<String>,
    on_stack: &mut BTreeSet<String>,
    sccs: &mut Vec<Vec<String>>,
) {
    let mut frames = vec![Frame {
        node: start.to_string(),
        edge_i: 0,
    }];
    while !frames.is_empty() {
        let node = frames
            .last()
            .map(|frame| frame.node.clone())
            .unwrap_or_default();
        if !indices.contains_key(&node) {
            indices.insert(node.clone(), *index);
            lowlink.insert(node.clone(), *index);
            *index += 1;
            stack.push(node.clone());
            on_stack.insert(node.clone());
        }
        let succs = adj.get(&node).cloned().unwrap_or_default();
        let edge_i = frames.last().map(|frame| frame.edge_i).unwrap_or(0);
        if edge_i < succs.len() {
            if let Some(top) = frames.last_mut() {
                top.edge_i += 1;
            }
            push_successor(
                &succs[edge_i],
                indices,
                on_stack,
                lowlink,
                &node,
                &mut frames,
            );
            continue;
        }
        frames.pop();
        pop_scc(&node, indices, lowlink, stack, on_stack, sccs);
        min_parent(
            lowlink,
            &node,
            frames.last().map(|frame| frame.node.as_str()),
        );
    }
}

fn push_successor(
    w: &str,
    indices: &BTreeMap<String, usize>,
    on_stack: &BTreeSet<String>,
    lowlink: &mut BTreeMap<String, usize>,
    parent: &str,
    frames: &mut Vec<Frame>,
) {
    if !indices.contains_key(w) {
        frames.push(Frame {
            node: w.to_string(),
            edge_i: 0,
        });
        return;
    }
    if on_stack.contains(w) {
        min_with(lowlink, parent, indices.get(w).copied());
    }
}

fn pop_scc(
    finished: &str,
    indices: &BTreeMap<String, usize>,
    lowlink: &BTreeMap<String, usize>,
    stack: &mut Vec<String>,
    on_stack: &mut BTreeSet<String>,
    sccs: &mut Vec<Vec<String>>,
) {
    if lowlink.get(finished) != indices.get(finished) {
        return;
    }
    let mut scc = Vec::new();
    while let Some(name) = stack.pop() {
        on_stack.remove(&name);
        let stop = name == finished;
        scc.push(name);
        if stop {
            break;
        }
    }
    scc.sort();
    sccs.push(scc);
}

fn min_parent(lowlink: &mut BTreeMap<String, usize>, child: &str, parent: Option<&str>) {
    let Some(parent) = parent else {
        return;
    };
    min_with(lowlink, parent, lowlink.get(child).copied());
}

fn min_with(lowlink: &mut BTreeMap<String, usize>, node: &str, other: Option<usize>) {
    let Some(value) = other else {
        return;
    };
    if let Some(entry) = lowlink.get_mut(node) {
        if value < *entry {
            *entry = value;
        }
    }
}

fn longest_on_condensation(graph: &MachineGraph, terminals: &[String]) -> Option<u32> {
    let sccs = sccs_iterative(graph);
    if sccs.is_empty() {
        return None;
    }
    let mut id_of: BTreeMap<&str, usize> = BTreeMap::new();
    for (id, scc) in sccs.iter().enumerate() {
        for name in scc {
            id_of.insert(name.as_str(), id);
        }
    }
    let mut dag: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); sccs.len()];
    for edge in &graph.edges {
        let Some(&from) = id_of.get(edge.source.as_str()) else {
            continue;
        };
        let Some(&to) = id_of.get(edge.target.as_str()) else {
            continue;
        };
        if from != to {
            dag[from].insert(to);
        }
    }
    let mut is_term = vec![false; sccs.len()];
    for terminal in terminals {
        if let Some(&id) = id_of.get(terminal.as_str()) {
            is_term[id] = true;
        }
    }
    let mut best: Option<u32> = None;
    for id in 0..sccs.len() {
        if let Some(len) = dag_longest(id, &dag, &is_term, &mut vec![None; sccs.len()]) {
            best = Some(best.map_or(len, |cur| cur.max(len)));
        }
    }
    best
}

fn dag_longest(
    id: usize,
    dag: &[BTreeSet<usize>],
    is_term: &[bool],
    memo: &mut [Option<Option<u32>>],
) -> Option<u32> {
    if let Some(cached) = memo[id] {
        return cached;
    }
    let mut best: Option<u32> = if is_term[id] { Some(0) } else { None };
    for &next in &dag[id] {
        if let Some(len) = dag_longest(next, dag, is_term, memo) {
            let cand = len + 1;
            best = Some(best.map_or(cand, |cur| cur.max(cand)));
        }
    }
    memo[id] = Some(best);
    best
}

fn enabled_map(graph: &MachineGraph) -> BTreeMap<String, Vec<EnabledCmd>> {
    let mut out: BTreeMap<String, Vec<EnabledCmd>> = BTreeMap::new();
    for state in &graph.states {
        out.insert(state.name.clone(), Vec::new());
    }
    for edge in &graph.edges {
        let guarded = edge.guard.is_some();
        let kind = edge
            .guard
            .as_deref()
            .map(classify_guard)
            .unwrap_or(GuardKind::None);
        let cmd = EnabledCmd {
            command: edge.command.clone(),
            to: edge.target.clone(),
            guarded,
            guard_kind: kind,
            effect: edge.effect.is_some(),
        };
        if let Some(list) = out.get_mut(&edge.source) {
            if !list
                .iter()
                .any(|item| item.command == cmd.command && item.to == cmd.to)
            {
                list.push(cmd);
            }
        }
    }
    out
}

fn guarded_of(def: &Definition) -> Vec<String> {
    def.commands
        .iter()
        .filter(|(_, cmd)| {
            cmd.when.is_some()
                || cmd
                    .outcomes
                    .as_ref()
                    .is_some_and(|rows| rows.iter().any(|row| row.when.is_some()))
        })
        .map(|(name, _)| name.clone())
        .collect()
}

fn effect_of(def: &Definition) -> Vec<String> {
    def.commands
        .iter()
        .filter(|(_, cmd)| cmd.effect.is_some())
        .map(|(name, _)| name.clone())
        .collect()
}

fn kinds_in(expr: &Expr) -> BTreeSet<GuardKind> {
    let mut out = BTreeSet::new();
    collect_kinds(expr, &mut out);
    out
}

fn collect_kinds(expr: &Expr, out: &mut BTreeSet<GuardKind>) {
    match expr {
        Expr::Or(parts) | Expr::And(parts) => {
            for part in parts {
                collect_kinds(part, out);
            }
        }
        Expr::TextIn(_) => {
            out.insert(GuardKind::Grammar);
        }
        Expr::Exists(_) | Expr::NotExists(_) => {
            out.insert(GuardKind::Membership);
        }
        Expr::Cmp(cmp) => {
            out.insert(kind_from_names(&names_in(&cmp.left), &names_in(&cmp.right)));
        }
    }
}

fn names_in(op: &Operand) -> Vec<String> {
    match op {
        Operand::Fact(name) | Operand::Arg(name) => vec![name.clone()],
        Operand::State => vec!["state".into()],
        Operand::EffectStatus => vec!["effect_status".into()],
        Operand::EffectKind => vec!["effect_kind".into()],
        Operand::EffectOutcome => vec!["effect_outcome".into()],
        Operand::Literal(_) => Vec::new(),
        Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => {
            names_in(base)
        }
        Operand::BinArith { left, right, .. } => {
            let mut names = names_in(left);
            names.extend(names_in(right));
            names
        }
    }
}

fn kind_from_names(left: &[String], right: &[String]) -> GuardKind {
    let mut names = Vec::new();
    names.extend(left.iter().cloned());
    names.extend(right.iter().cloned());
    if names.iter().any(|name| timed(name)) {
        return GuardKind::Time;
    }
    if names.iter().any(|name| counted(name)) {
        return GuardKind::Count;
    }
    if !left.is_empty() || !right.is_empty() {
        return GuardKind::StoredFact;
    }
    GuardKind::None
}

fn timed(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    TIME_MARKERS.iter().any(|mark| lower.contains(mark))
}

fn counted(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    COUNT_MARKERS.iter().any(|mark| lower.contains(mark))
}

fn heuristic_kind(text: &str) -> GuardKind {
    let lower = text.to_ascii_lowercase();
    if lower.contains("text_in") {
        return GuardKind::Grammar;
    }
    if TIME_MARKERS.iter().any(|mark| lower.contains(mark)) {
        return GuardKind::Time;
    }
    if COUNT_MARKERS.iter().any(|mark| lower.contains(mark)) {
        return GuardKind::Count;
    }
    if lower.contains("exists") {
        return GuardKind::Membership;
    }
    GuardKind::StoredFact
}

fn pick_kind(kinds: BTreeSet<GuardKind>) -> GuardKind {
    kinds.into_iter().max().unwrap_or(GuardKind::None)
}

/// Main-line stations: shortest declared walk from initial to a terminal.
pub fn main_line(graph: &MachineGraph, analytics: &Analytics) -> Vec<String> {
    let Some(target) = analytics.terminals.first() else {
        return vec![analytics.initial.clone()];
    };
    shortest_walk(graph, &analytics.initial, target)
}

fn shortest_walk(graph: &MachineGraph, start: &str, goal: &str) -> Vec<String> {
    let mut prev: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    seen.insert(start.to_string());
    queue.push_back(start.to_string());
    while let Some(node) = queue.pop_front() {
        if node == goal {
            break;
        }
        let mut edges: Vec<&GraphEdge> = graph
            .edges
            .iter()
            .filter(|edge| edge.source == node && edge.source != edge.target)
            .collect();
        edges.sort_by(|a, b| (&a.command, &a.target).cmp(&(&b.command, &b.target)));
        for edge in edges {
            if !seen.insert(edge.target.clone()) {
                continue;
            }
            prev.insert(edge.target.clone(), (node.clone(), edge.command.clone()));
            queue.push_back(edge.target.clone());
        }
    }
    let mut path = vec![goal.to_string()];
    let mut at = goal.to_string();
    while at != start {
        match prev.get(&at) {
            Some((from, _)) => {
                path.push(from.clone());
                at = from.clone();
            }
            None => break,
        }
    }
    path.reverse();
    if path.first().map(String::as_str) != Some(start) {
        path.insert(0, start.to_string());
    }
    path
}

/// Stable left-to-right station order: main line, then remaining by finish distance.
pub fn station_order(graph: &MachineGraph, analytics: &Analytics) -> Vec<String> {
    let mut order = main_line(graph, analytics);
    let mut seen: BTreeSet<String> = order.iter().cloned().collect();
    let mut rest: Vec<(Option<u32>, String)> = graph
        .states
        .iter()
        .map(|state| state.name.clone())
        .filter(|name| seen.insert(name.clone()))
        .map(|name| {
            let dist = analytics.shortest_finish.get(&name).copied().flatten();
            (dist, name)
        })
        .collect();
    rest.sort_by(|a, b| match (a.0, b.0) {
        (Some(x), Some(y)) => (x, &a.1).cmp(&(y, &b.1)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.1.cmp(&b.1),
    });
    order.extend(rest.into_iter().map(|(_, name)| name));
    order
}

pub fn graph_of(def: &Definition) -> MachineGraph {
    machine_graph(def)
}
