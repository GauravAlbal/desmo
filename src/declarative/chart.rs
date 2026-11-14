//! Authored structure. The flat [`Definition`] is the compile target; `Chart` is
//! the source of truth for groups, regions, joints, and diagram projection.

use super::def::{CommandDef, Definition, FactDef};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Authored machine structure after parse, independent of product lowering.
#[derive(Clone, Debug, Default)]
pub struct Chart {
    pub machine_id: String,
    pub machine_version: u64,
    /// True when the source used schema-2 regions/groups/joints.
    pub hierarchical: bool,
    pub regions: BTreeMap<String, RegionChart>,
    pub joints: Vec<JointChart>,
    pub facts: BTreeMap<String, FactDef>,
}

/// One authored region.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegionChart {
    pub states: BTreeMap<String, ChartState>,
    pub groups: BTreeMap<String, GroupChart>,
    pub edges: Vec<ChartEdge>,
}

/// Leaf state flags as authored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChartState {
    pub initial: bool,
    pub terminal: bool,
}

/// Compound parent: children plus parent handlers counted once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupChart {
    pub parent: String,
    pub children: Vec<String>,
    pub parent_edges: Vec<ChartEdge>,
}

/// One authored edge. Parent handlers appear once, not per child.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartEdge {
    pub command: String,
    pub from: String,
    pub to: String,
    pub rule: Option<String>,
    pub provenance: EdgeProvenance,
}

/// Where an authored edge came from.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeProvenance {
    Child { region: String },
    Parent { region: String, group: String },
    Joint { command: String },
    Flat { region: String },
}

/// Cross-region command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JointChart {
    pub command: String,
    pub rule: Option<String>,
    /// (region, from-leaf, to-leaf)
    pub legs: Vec<(String, String, String)>,
}

/// Fail-closed chart/flat disagreement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartError {
    pub code: &'static str,
    pub message: String,
}

impl ChartError {
    fn inconsistent(edge: &str, detail: impl Into<String>) -> Self {
        Self {
            code: "IK_CHART_INCONSISTENT",
            message: format!("{edge}: {}", detail.into()),
        }
    }
}

impl std::fmt::Display for ChartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// Identity used for conservation and round-trip.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EdgeKey {
    pub command: String,
    pub from: String,
    pub to: String,
    pub region: String,
    pub layer: String,
    pub rule: Option<String>,
}

impl ChartEdge {
    pub fn key(&self) -> EdgeKey {
        let (region, layer) = match &self.provenance {
            EdgeProvenance::Child { region } => (region.clone(), "child".into()),
            EdgeProvenance::Parent { region, .. } => (region.clone(), "parent".into()),
            EdgeProvenance::Joint { command } => (command.clone(), "joint".into()),
            EdgeProvenance::Flat { region } => (region.clone(), "flat".into()),
        };
        EdgeKey {
            command: self.command.clone(),
            from: self.from.clone(),
            to: self.to.clone(),
            region,
            layer,
            rule: self.rule.clone(),
        }
    }
}

impl Chart {
    /// Schema-1 constructor: one region, no groups, edges from the flat commands.
    pub fn from_flat(def: &Definition) -> Self {
        let region = def.machine_id.clone();
        let mut states = BTreeMap::new();
        for (name, state) in &def.states {
            states.insert(
                name.clone(),
                ChartState {
                    initial: state.initial,
                    terminal: state.terminal,
                },
            );
        }
        let mut edges = Vec::new();
        for (command, cmd) in &def.commands {
            edges.extend(flat_edges(command, cmd, &region));
        }
        let mut regions = BTreeMap::new();
        regions.insert(
            region,
            RegionChart {
                states,
                groups: BTreeMap::new(),
                edges,
            },
        );
        Self {
            machine_id: def.machine_id.clone(),
            machine_version: def.machine_version,
            hierarchical: false,
            regions,
            joints: Vec::new(),
            facts: def.facts.clone(),
        }
    }

    /// All authored edges including parent handlers (once) and joint legs as joint keys.
    pub fn authored_keys(&self) -> Vec<EdgeKey> {
        let mut keys = Vec::new();
        for region in self.regions.values() {
            for edge in &region.edges {
                keys.push(edge.key());
            }
            for group in region.groups.values() {
                for edge in &group.parent_edges {
                    keys.push(edge.key());
                }
            }
        }
        for joint in &self.joints {
            keys.push(EdgeKey {
                command: joint.command.clone(),
                from: "*".into(),
                to: "*".into(),
                region: String::new(),
                layer: "joint".into(),
                rule: joint.rule.clone(),
            });
        }
        keys.sort();
        keys
    }
}

fn dest_pairs(cmd: &CommandDef) -> Vec<(Option<String>, String)> {
    if let Some(outcomes) = &cmd.outcomes {
        return outcomes
            .iter()
            .map(|o| (o.when.clone(), o.to.clone()))
            .collect();
    }
    if let Some(to) = &cmd.to {
        return vec![(cmd.when.clone(), to.clone())];
    }
    Vec::new()
}

fn skip_pinned_from(rule: Option<&str>, from: &str) -> bool {
    let Some(rule) = rule else {
        return false;
    };
    match pinned_state(rule) {
        Some(pinned) => pinned != from,
        None => false,
    }
}

fn flat_edges(command: &str, cmd: &CommandDef, region: &str) -> Vec<ChartEdge> {
    let dests = dest_pairs(cmd);
    let mut out = Vec::new();
    for from in &cmd.from {
        for (rule, to) in &dests {
            if skip_pinned_from(rule.as_deref(), from) {
                continue;
            }
            out.push(ChartEdge {
                command: command.to_string(),
                from: from.clone(),
                to: to.clone(),
                rule: rule.clone(),
                provenance: EdgeProvenance::Flat {
                    region: region.to_string(),
                },
            });
        }
    }
    out
}

/// Every authored edge must lower to ≥1 flat transition; every flat transition
/// maps to exactly one authored edge; keys round-trip.
pub fn check_consistent(chart: &Chart, def: &Definition) -> Result<(), ChartError> {
    let authored = chart.authored_keys();
    let lowered = lowered_keys(chart, def);
    if !chart.hierarchical {
        for key in &authored {
            if !lowered.iter().any(|item| item == key) {
                return Err(ChartError::inconsistent(
                    &format!("{} {}→{}", key.command, key.from, key.to),
                    "authored edge has no flat transition",
                ));
            }
        }
    }
    let mut seen: BTreeMap<FlatHop, EdgeKey> = BTreeMap::new();
    for hop in flat_hops(def) {
        if !hop_is_live(def, &hop) {
            continue;
        }
        let mut matches: Vec<EdgeKey> = authored
            .iter()
            .filter(|key| hop_matches_key(chart, def, &hop, key))
            .cloned()
            .collect();
        matches.sort();
        matches.dedup();
        if matches.is_empty() {
            return Err(ChartError::inconsistent(
                &format!("{} {}→{}", hop.command, hop.from, hop.to),
                "flat transition maps to 0 authored edges",
            ));
        }
        if matches.len() != 1 {
            return Err(ChartError::inconsistent(
                &format!("{} {}→{}", hop.command, hop.from, hop.to),
                format!(
                    "flat transition maps to {} authored edges",
                    matches.len()
                ),
            ));
        }
        seen.insert(hop, matches[0].clone());
    }
    let mut recovered: Vec<EdgeKey> = seen.into_values().collect();
    recovered.sort();
    recovered.dedup();
    let mut expect = if chart.hierarchical {
        lowered
    } else {
        authored
    };
    expect.sort();
    expect.dedup();
    if recovered != expect {
        let missing: Vec<String> = expect
            .iter()
            .filter(|key| !recovered.contains(key))
            .map(|key| format!("{} {}→{} ({})", key.command, key.from, key.to, key.layer))
            .collect();
        return Err(ChartError::inconsistent(
            "round-trip",
            format!("missing {}", missing.join("; ")),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct FlatHop {
    command: String,
    from: String,
    to: String,
    rule: Option<String>,
}

fn flat_hops(def: &Definition) -> Vec<FlatHop> {
    let mut hops = Vec::new();
    for (command, cmd) in &def.commands {
        let dests: Vec<(Option<String>, String)> = if let Some(outcomes) = &cmd.outcomes {
            outcomes
                .iter()
                .map(|o| (o.when.clone(), o.to.clone()))
                .collect()
        } else if let Some(to) = &cmd.to {
            vec![(cmd.when.clone(), to.clone())]
        } else {
            Vec::new()
        };
        for from in &cmd.from {
            for (rule, to) in &dests {
                hops.push(FlatHop {
                    command: command.clone(),
                    from: from.clone(),
                    to: to.clone(),
                    rule: rule.clone(),
                });
            }
        }
    }
    hops
}

fn lowered_keys(chart: &Chart, def: &Definition) -> Vec<EdgeKey> {
    let mut keys = Vec::new();
    for hop in flat_hops(def) {
        for key in chart.authored_keys() {
            if hop_matches_key(chart, def, &hop, &key) {
                keys.push(key);
            }
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

fn hop_is_live(def: &Definition, hop: &FlatHop) -> bool {
    let Some(rule) = hop.rule.as_deref() else {
        return true;
    };
    let Some(pinned) = pinned_state(rule) else {
        return true;
    };
    if hop.from == pinned {
        return true;
    }
    if let Some(map) = def.origin.products.get(&hop.from) {
        return map.values().any(|leaf| leaf == &pinned);
    }
    false
}

fn pinned_state(rule: &str) -> Option<String> {
    let rest = rule.strip_prefix("state == ")?;
    if let Some(body) = rest.strip_prefix('"') {
        let ident: String = body.chars().take_while(|ch| *ch != '"').collect();
        if ident.is_empty() {
            None
        } else {
            Some(ident)
        }
    } else {
        let ident: String = rest
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .collect();
        if ident.is_empty() {
            None
        } else {
            Some(ident)
        }
    }
}

fn hop_matches_key(chart: &Chart, def: &Definition, hop: &FlatHop, key: &EdgeKey) -> bool {
    if hop.command != key.command {
        return false;
    }
    if key.layer == "flat" {
        return hop.from == key.from && hop.to == key.to && hop.rule == key.rule;
    }
    if key.layer == "joint" {
        return true;
    }
    let from_leaf = product_leaf(def, &hop.from, &key.region);
    let to_leaf = product_leaf(def, &hop.to, &key.region);
    let Some(from_leaf) = from_leaf else {
        return false;
    };
    let Some(to_leaf) = to_leaf else {
        return false;
    };
    if key.layer == "child" {
        return from_leaf == key.from
            && to_leaf == key.to
            && child_takes_rule(chart, key, &hop.rule);
    }
    if key.layer == "parent" {
        if child_takes_hop(chart, key, &from_leaf, &to_leaf, &hop.rule) {
            return false;
        }
        let children = group_children(chart, &key.region, &key.from);
        return children.iter().any(|c| c == &from_leaf) && to_leaf == key.to;
    }
    false
}

fn child_takes_hop(
    chart: &Chart,
    parent: &EdgeKey,
    from: &str,
    to: &str,
    hop_rule: &Option<String>,
) -> bool {
    chart.regions.values().any(|region| {
        region.edges.iter().any(|edge| {
            edge.command == parent.command
                && edge.from == from
                && edge.to == to
                && matches!(
                    &edge.provenance,
                    EdgeProvenance::Child { region } if region == &parent.region
                )
                && child_rule_in_hop(edge.rule.as_deref(), hop_rule.as_deref())
        })
    })
}

fn child_takes_rule(chart: &Chart, key: &EdgeKey, hop_rule: &Option<String>) -> bool {
    child_rule_in_hop(authored_rule(chart, key).as_deref(), hop_rule.as_deref())
}

fn child_rule_in_hop(authored: Option<&str>, hop: Option<&str>) -> bool {
    match (authored, hop) {
        (None, _) => true,
        (Some(rule), Some(hop)) => hop.contains(&format!("({rule})")) || hop.contains(rule),
        (Some(_), None) => false,
    }
}

fn authored_rule(chart: &Chart, key: &EdgeKey) -> Option<String> {
    for region in chart.regions.values() {
        for edge in region.edges.iter().chain(
            region
                .groups
                .values()
                .flat_map(|g| g.parent_edges.iter()),
        ) {
            if edge.key() == *key {
                return edge.rule.clone();
            }
        }
    }
    None
}

fn group_children(chart: &Chart, region: &str, parent: &str) -> Vec<String> {
    chart
        .regions
        .get(region)
        .and_then(|item| item.groups.get(parent))
        .map(|group| group.children.clone())
        .unwrap_or_default()
}

fn product_leaf(def: &Definition, product: &str, region: &str) -> Option<String> {
    if let Some(map) = def.origin.products.get(product) {
        return map.get(region).cloned();
    }
    Some(product.to_string())
}

/// Reachability on the authored chart (not the product graph).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChartAnalytics {
    pub ends: Vec<(String, String)>,
    pub dead_ends: Vec<(String, String)>,
    pub traps: Vec<(String, String)>,
    pub cycles: Vec<Vec<String>>,
}

/// Hierarchical analytics. A leaf is an end iff it is declared terminal or has
/// no authored exit (parent handlers count as exits for every child).
pub fn analyze_chart(chart: &Chart) -> ChartAnalytics {
    let mut analytics = ChartAnalytics::default();
    for (region_name, region) in &chart.regions {
        let graph = region_graph(region);
        for (state, flags) in &region.states {
            let outs = graph.get(state).map(Vec::len).unwrap_or(0);
            if flags.terminal || outs == 0 {
                analytics.ends.push((region_name.clone(), state.clone()));
            }
            if outs == 0 && !flags.terminal {
                analytics.dead_ends.push((region_name.clone(), state.clone()));
            }
        }
        analytics.cycles.extend(region_cycles(&graph));
        analytics.traps.extend(region_traps(region_name, &graph, region));
    }
    analytics
}

fn region_graph(region: &RegionChart) -> BTreeMap<String, Vec<(String, String)>> {
    let mut graph: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for (name, _) in &region.states {
        graph.entry(name.clone()).or_default();
    }
    for edge in &region.edges {
        graph
            .entry(edge.from.clone())
            .or_default()
            .push((edge.command.clone(), edge.to.clone()));
    }
    for group in region.groups.values() {
        for edge in &group.parent_edges {
            for child in &group.children {
                graph
                    .entry(child.clone())
                    .or_default()
                    .push((edge.command.clone(), edge.to.clone()));
            }
        }
    }
    graph
}

fn region_cycles(graph: &BTreeMap<String, Vec<(String, String)>>) -> Vec<Vec<String>> {
    let mut cycles = Vec::new();
    let nodes: Vec<String> = graph.keys().cloned().collect();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut low: BTreeMap<String, usize> = BTreeMap::new();
    let mut stack: Vec<String> = Vec::new();
    let mut on = BTreeSet::new();
    let mut next = 0usize;
    for node in &nodes {
        if !index.contains_key(node) {
            scc(
                node,
                graph,
                &mut index,
                &mut low,
                &mut stack,
                &mut on,
                &mut next,
                &mut cycles,
            );
        }
    }
    cycles
}

fn scc(
    node: &str,
    graph: &BTreeMap<String, Vec<(String, String)>>,
    index: &mut BTreeMap<String, usize>,
    low: &mut BTreeMap<String, usize>,
    stack: &mut Vec<String>,
    on: &mut BTreeSet<String>,
    next: &mut usize,
    cycles: &mut Vec<Vec<String>>,
) {
    index.insert(node.to_string(), *next);
    low.insert(node.to_string(), *next);
    *next += 1;
    stack.push(node.to_string());
    on.insert(node.to_string());
    if let Some(edges) = graph.get(node) {
        for (_, to) in edges {
            if !index.contains_key(to) {
                scc(to, graph, index, low, stack, on, next, cycles);
                min_low(low, node, *low.get(to).unwrap_or(&0));
            } else if on.contains(to) {
                min_low(low, node, *index.get(to).unwrap_or(&0));
            }
        }
    }
    if low.get(node) == index.get(node) {
        let mut comp = pop_component(stack, on, node);
        if comp.len() > 1 {
            comp.reverse();
            cycles.push(comp);
        }
    }
}

fn min_low(low: &mut BTreeMap<String, usize>, node: &str, candidate: usize) {
    if let Some(cur) = low.get_mut(node) {
        *cur = (*cur).min(candidate);
    }
}

fn pop_component(stack: &mut Vec<String>, on: &mut BTreeSet<String>, node: &str) -> Vec<String> {
    let mut comp = Vec::new();
    while let Some(item) = stack.pop() {
        on.remove(&item);
        let done = item == node;
        comp.push(item);
        if done {
            break;
        }
    }
    comp
}

fn region_traps(
    region: &str,
    graph: &BTreeMap<String, Vec<(String, String)>>,
    region_chart: &RegionChart,
) -> Vec<(String, String)> {
    let ends: BTreeSet<String> = region_chart
        .states
        .iter()
        .filter(|(name, flags)| {
            flags.terminal || graph.get(*name).map(Vec::len).unwrap_or(0) == 0
        })
        .map(|(name, _)| name.clone())
        .collect();
    let mut traps = Vec::new();
    for (state, _) in &region_chart.states {
        if ends.contains(state) {
            continue;
        }
        if !reaches_end(state, graph, &ends) {
            traps.push((region.to_string(), state.clone()));
        }
    }
    traps
}

fn reaches_end(
    start: &str,
    graph: &BTreeMap<String, Vec<(String, String)>>,
    ends: &BTreeSet<String>,
) -> bool {
    let mut seen = BTreeSet::new();
    let mut q = VecDeque::from([start.to_string()]);
    while let Some(cur) = q.pop_front() {
        if ends.contains(&cur) {
            return true;
        }
        if !seen.insert(cur.clone()) {
            continue;
        }
        if let Some(edges) = graph.get(&cur) {
            for (_, to) in edges {
                q.push_back(to.clone());
            }
        }
    }
    false
}

/// Build a hierarchical chart from authored maps (used by schema-2 lowering).
pub fn hierarchical_chart(
    machine_id: String,
    machine_version: u64,
    facts: BTreeMap<String, FactDef>,
    regions: BTreeMap<String, RegionChart>,
    joints: Vec<JointChart>,
) -> Chart {
    Chart {
        machine_id,
        machine_version,
        hierarchical: true,
        regions,
        joints,
        facts,
    }
}

/// Helper for tests: drop one authored child edge.
pub fn drop_first_edge(chart: &mut Chart) -> Option<EdgeKey> {
    for region in chart.regions.values_mut() {
        if let Some(edge) = region.edges.first() {
            let key = edge.key();
            region.edges.remove(0);
            return Some(key);
        }
        for group in region.groups.values_mut() {
            if let Some(edge) = group.parent_edges.first() {
                let key = edge.key();
                group.parent_edges.remove(0);
                return Some(key);
            }
        }
    }
    None
}

