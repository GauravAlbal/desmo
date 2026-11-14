//! Deterministic river layout. No crates. Insertion order never matters:
//! edges and nodes are sorted by stable identity before any walk.

use crate::declarative::chart::{Chart, ChartEdge, RegionChart};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// One authored hop as drawn: command, source, target.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DrawnEdge {
    pub command: String,
    pub from: String,
    pub to: String,
}

impl DrawnEdge {
    pub fn from_chart(edge: &ChartEdge) -> Self {
        Self {
            command: edge.command.clone(),
            from: edge.from.clone(),
            to: edge.to.clone(),
        }
    }
}

/// Pure layout of one connected leaf graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiverLayout {
    pub spine: Vec<String>,
    pub layers: Vec<Vec<String>>,
    pub paired: Vec<(DrawnEdge, DrawnEdge)>,
    pub drawn: Vec<DrawnEdge>,
}

/// Layout every region of a chart; joints counted once as `* → *`.
pub fn layout_chart(chart: &Chart) -> Vec<DrawnEdge> {
    let mut drawn = Vec::new();
    for region in chart.regions.values() {
        drawn.extend(region_drawn(region));
    }
    for joint in &chart.joints {
        drawn.push(DrawnEdge {
            command: joint.command.clone(),
            from: "*".into(),
            to: "*".into(),
        });
    }
    drawn.sort();
    drawn.dedup();
    drawn
}

fn region_drawn(region: &RegionChart) -> Vec<DrawnEdge> {
    let mut drawn: Vec<DrawnEdge> = region.edges.iter().map(DrawnEdge::from_chart).collect();
    for group in region.groups.values() {
        drawn.extend(group.parent_edges.iter().map(DrawnEdge::from_chart));
    }
    drawn.sort();
    drawn.dedup();
    drawn
}

/// Layout a leaf graph. `nodes` and `edges` may arrive in any order.
pub fn layout_graph(
    nodes: Vec<String>,
    edges: Vec<DrawnEdge>,
    initial: Option<&str>,
) -> RiverLayout {
    let mut nodes = nodes;
    nodes.sort();
    nodes.dedup();
    let mut edges = edges;
    edges.sort();
    edges.dedup();
    let start = pick_start(&nodes, initial);
    let directed: Vec<DrawnEdge> = edges
        .iter()
        .filter(|e| e.from != e.to)
        .cloned()
        .collect();
    let back = feedback_arcs(&nodes, &directed, start.as_deref());
    let forward: Vec<DrawnEdge> = directed
        .iter()
        .filter(|e| !back.contains(*e))
        .cloned()
        .collect();
    let ranks = longest_path_ranks(&nodes, &forward, start.as_deref());
    let layers = barycenter_layers(&nodes, &forward, &ranks);
    let spine: Vec<String> = layers.iter().flatten().cloned().collect();
    let paired = pair_cycles(&directed, &spine);
    RiverLayout {
        spine,
        layers,
        paired,
        drawn: edges,
    }
}

fn pick_start(nodes: &[String], initial: Option<&str>) -> Option<String> {
    if let Some(name) = initial {
        if nodes.iter().any(|n| n == name) {
            return Some(name.to_string());
        }
    }
    nodes.first().cloned()
}

/// Back-edges = edges to a DFS ancestor from `start`. Children in name order.
fn feedback_arcs(nodes: &[String], edges: &[DrawnEdge], start: Option<&str>) -> BTreeSet<DrawnEdge> {
    let adj = adjacency(nodes, edges);
    let mut back = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut stack = Vec::new();
    if let Some(root) = start {
        dfs_back(root, &adj, &mut seen, &mut stack, &mut back);
    }
    for node in nodes {
        dfs_back(node, &adj, &mut seen, &mut stack, &mut back);
    }
    back
}

fn adjacency(nodes: &[String], edges: &[DrawnEdge]) -> BTreeMap<String, Vec<DrawnEdge>> {
    let mut adj: BTreeMap<String, Vec<DrawnEdge>> = BTreeMap::new();
    for node in nodes {
        adj.entry(node.clone()).or_default();
    }
    for edge in edges {
        adj.entry(edge.from.clone()).or_default().push(edge.clone());
    }
    for list in adj.values_mut() {
        list.sort();
    }
    adj
}

fn dfs_back(
    node: &str,
    adj: &BTreeMap<String, Vec<DrawnEdge>>,
    seen: &mut BTreeSet<String>,
    stack: &mut Vec<String>,
    back: &mut BTreeSet<DrawnEdge>,
) {
    if !seen.insert(node.to_string()) {
        return;
    }
    stack.push(node.to_string());
    if let Some(outs) = adj.get(node) {
        for edge in outs {
            if stack.iter().any(|n| n == &edge.to) {
                back.insert(edge.clone());
            } else {
                dfs_back(&edge.to, adj, seen, stack, back);
            }
        }
    }
    stack.pop();
}

fn longest_path_ranks(
    nodes: &[String],
    forward: &[DrawnEdge],
    start: Option<&str>,
) -> BTreeMap<String, usize> {
    let mut rank = init_ranks(nodes, start);
    let mut incoming = incoming_degree(nodes, forward);
    let adj = adjacency(nodes, forward);
    let mut queue = zero_in_queue(&incoming, start);
    relax_longest(&mut rank, &mut incoming, &adj, &mut queue);
    rank
}

fn init_ranks(nodes: &[String], start: Option<&str>) -> BTreeMap<String, usize> {
    let mut rank: BTreeMap<String, usize> = BTreeMap::new();
    for node in nodes {
        rank.insert(node.clone(), 0);
    }
    if let Some(root) = start {
        rank.insert(root.to_string(), 0);
    }
    rank
}

fn incoming_degree(nodes: &[String], forward: &[DrawnEdge]) -> BTreeMap<String, usize> {
    let mut incoming: BTreeMap<String, usize> = nodes.iter().map(|n| (n.clone(), 0)).collect();
    for edge in forward {
        *incoming.entry(edge.to.clone()).or_insert(0) += 1;
    }
    incoming
}

fn zero_in_queue(incoming: &BTreeMap<String, usize>, start: Option<&str>) -> VecDeque<String> {
    let mut queue: VecDeque<String> = incoming
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(k, _)| k.clone())
        .collect();
    if let Some(root) = start {
        if !queue.iter().any(|n| n == root) {
            queue.push_front(root.to_string());
        }
    }
    queue
}

fn relax_longest(
    rank: &mut BTreeMap<String, usize>,
    incoming: &mut BTreeMap<String, usize>,
    adj: &BTreeMap<String, Vec<DrawnEdge>>,
    queue: &mut VecDeque<String>,
) {
    let mut seen = BTreeSet::new();
    while let Some(node) = queue.pop_front() {
        if !seen.insert(node.clone()) {
            continue;
        }
        let base = *rank.get(&node).unwrap_or(&0);
        relax_outs(base, &node, rank, incoming, adj, queue);
    }
}

fn relax_outs(
    base: usize,
    node: &str,
    rank: &mut BTreeMap<String, usize>,
    incoming: &mut BTreeMap<String, usize>,
    adj: &BTreeMap<String, Vec<DrawnEdge>>,
    queue: &mut VecDeque<String>,
) {
    let Some(outs) = adj.get(node) else {
        return;
    };
    for edge in outs {
        let next = base + 1;
        let cur = rank.entry(edge.to.clone()).or_insert(0);
        if next > *cur {
            *cur = next;
        }
        close_in_edge(&edge.to, incoming, queue);
    }
}

fn close_in_edge(to: &str, incoming: &mut BTreeMap<String, usize>, queue: &mut VecDeque<String>) {
    let Some(left) = incoming.get_mut(to) else {
        return;
    };
    *left = left.saturating_sub(1);
    if *left == 0 {
        queue.push_back(to.to_string());
    }
}

fn barycenter_layers(
    nodes: &[String],
    forward: &[DrawnEdge],
    ranks: &BTreeMap<String, usize>,
) -> Vec<Vec<String>> {
    let max = ranks.values().copied().max().unwrap_or(0);
    let mut layers: Vec<Vec<String>> = vec![Vec::new(); max + 1];
    for node in nodes {
        let r = *ranks.get(node).unwrap_or(&0);
        layers[r].push(node.clone());
    }
    for layer in &mut layers {
        layer.sort();
    }
    let preds = predecessors(forward);
    for i in 1..layers.len() {
        let prev = layers[i - 1].clone();
        layers[i].sort_by(|a, b| {
            barycenter(a, &prev, &preds)
                .partial_cmp(&barycenter(b, &prev, &preds))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(b))
        });
    }
    layers.retain(|layer| !layer.is_empty());
    layers
}

fn predecessors(forward: &[DrawnEdge]) -> BTreeMap<String, Vec<String>> {
    let mut preds: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in forward {
        preds.entry(edge.to.clone()).or_default().push(edge.from.clone());
    }
    for list in preds.values_mut() {
        list.sort();
        list.dedup();
    }
    preds
}

fn barycenter(node: &str, prev: &[String], preds: &BTreeMap<String, Vec<String>>) -> f64 {
    let Some(list) = preds.get(node) else {
        return prev.len() as f64;
    };
    let mut sum = 0.0;
    let mut n = 0.0;
    for parent in list {
        if let Some(idx) = prev.iter().position(|p| p == parent) {
            sum += idx as f64;
            n += 1.0;
        }
    }
    if n == 0.0 {
        prev.len() as f64
    } else {
        sum / n
    }
}

fn pair_cycles(edges: &[DrawnEdge], spine: &[String]) -> Vec<(DrawnEdge, DrawnEdge)> {
    let mut used = BTreeSet::new();
    let mut paired = Vec::new();
    for edge in edges {
        if used.contains(edge) {
            continue;
        }
        let rev = edges.iter().find(|other| {
            other.from == edge.to && other.to == edge.from && *other != edge && !used.contains(*other)
        });
        if let Some(rev) = rev {
            if adjacent_on_spine(spine, &edge.from, &edge.to) {
                used.insert(edge.clone());
                used.insert(rev.clone());
                let (a, b) = if edge <= rev {
                    (edge.clone(), rev.clone())
                } else {
                    (rev.clone(), edge.clone())
                };
                paired.push((a, b));
                continue;
            }
        }
    }
    paired
}

fn adjacent_on_spine(spine: &[String], a: &str, b: &str) -> bool {
    let Some(i) = spine.iter().position(|n| n == a) else {
        return false;
    };
    let Some(j) = spine.iter().position(|n| n == b) else {
        return false;
    };
    i.abs_diff(j) == 1
}

/// Count command labels expected in the text projection.
pub fn command_counts(drawn: &[DrawnEdge]) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for edge in drawn {
        *counts.entry(edge.command.clone()).or_insert(0) += 1;
    }
    counts
}
