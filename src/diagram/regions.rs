//! Region lanes from the authored Chart, not the flattened product graph.

use super::{RegionHop, RegionLane, RegionView, StrokeClass};
use crate::declarative::chart::{analyze_chart, ChartEdge, JointChart};
use crate::declarative::Definition;
use std::collections::{BTreeMap, BTreeSet};

pub fn from_def(def: &Definition) -> Option<RegionView> {
    if !def.chart.hierarchical {
        return None;
    }
    let region_names: Vec<String> = def.chart.regions.keys().cloned().collect();
    if region_names.is_empty() {
        return None;
    }
    let joints = joint_rails(&def.chart.joints);
    let analytics = analyze_chart(&def.chart);
    let mut lanes = Vec::new();
    for name in region_names {
        let region = def.chart.regions.get(&name)?;
        let parent = region.groups.values().next();
        let children = parent.map(|g| g.children.clone()).unwrap_or_default();
        let parent_name = parent.map(|g| g.parent.clone());
        let states = leaf_states(region, &children);
        let hops = expand_chart_edges(&region.edges);
        let parent_hops = parent
            .map(|g| expand_chart_edges(&g.parent_edges))
            .unwrap_or_default();
        let terminals: Vec<String> = analytics
            .ends
            .iter()
            .filter(|(region_name, _)| region_name == &name)
            .map(|(_, state)| state.clone())
            .collect();
        lanes.push(RegionLane {
            parent_name,
            children,
            name,
            states,
            terminals,
            hops,
            parent_hops,
        });
    }
    Some(RegionView { lanes, joints })
}

pub fn facts(def: &Definition, view: &RegionView) -> Vec<String> {
    let region_n = view.lanes.len();
    let leaf_n: usize = view.lanes.iter().map(|lane| lane.states.len()).sum();
    let cmd_n = command_count(def);
    let mut chunks = vec![
        format!(
            "{region_n} {}",
            if region_n == 1 { "region" } else { "regions" }
        ),
        format!(
            "{leaf_n} leaf {}",
            if leaf_n == 1 { "state" } else { "states" }
        ),
        if view.joints.is_empty() {
            format!("{cmd_n} {}", if cmd_n == 1 { "command" } else { "commands" })
        } else {
            format!(
                "{cmd_n} commands ({} {})",
                view.joints.len(),
                if view.joints.len() == 1 {
                    "joint"
                } else {
                    "joints"
                }
            )
        },
    ];
    chunks.push(region_reach(def, view));
    if let Some(size) = parallel_size_fact(view) {
        chunks.push(size);
    }
    if let Some(cycle) = closed_cycle_note(view) {
        chunks.push(cycle);
    }
    chunks.push(product_reach(def));
    chunks
}

fn parallel_size_fact(view: &RegionView) -> Option<String> {
    if view.lanes.len() < 2 {
        return None;
    }
    let sizes: Vec<usize> = view.lanes.iter().map(|lane| lane.states.len()).collect();
    let mut product = 1usize;
    for size in &sizes {
        product = product.saturating_mul(*size);
    }
    if product == 0 || product >= crate::declarative::region::PRODUCT_BOUND {
        return None;
    }
    let joined = sizes
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("+");
    Some(format!(
        "{} parallel regions: {joined} states (a flat chart would need {product})",
        view.lanes.len()
    ))
}

fn closed_cycle_note(view: &RegionView) -> Option<String> {
    let lane = view.lanes.first()?;
    if view.lanes.len() != 1 {
        return None;
    }
    let children = if lane.children.is_empty() {
        lane.states.clone()
    } else {
        lane.children.clone()
    };
    if children.len() < 3 {
        return None;
    }
    let mut path = vec![children[0].clone()];
    let mut cur = children[0].clone();
    for _ in 0..children.len() {
        let next = lane.hops.iter().find(|hop| {
            hop.from == cur && children.iter().any(|c| c == &hop.to) && hop.to != hop.from
        })?;
        path.push(next.to.clone());
        cur = next.to.clone();
        if cur == children[0] {
            break;
        }
    }
    if path.last() != Some(&children[0]) || path.len() != children.len() + 1 {
        return None;
    }
    Some(format!("1 loop-closed cycle: {}", path.join(" → ")))
}

fn command_count(def: &Definition) -> usize {
    let mut names = BTreeSet::new();
    for key in def.chart.authored_keys() {
        names.insert(key.command);
    }
    names.len()
}

fn region_reach(def: &Definition, view: &RegionView) -> String {
    let mut ended = Vec::new();
    let mut open = Vec::new();
    for lane in &view.lanes {
        if lane_has_end(def, lane) {
            ended.push(lane.name.as_str());
        } else {
            open.push(lane.name.as_str());
        }
    }
    if open.is_empty() {
        format!("ends in {} (per region)", ended.join(", "))
    } else if ended.is_empty() {
        format!("{}: no end state (per region)", open.join(", "))
    } else {
        format!(
            "ends in {} · {}: no end state (per region)",
            ended.join(", "),
            open.join(", ")
        )
    }
}

fn lane_has_end(_def: &Definition, lane: &RegionLane) -> bool {
    !lane.terminals.is_empty()
}

fn product_reach(def: &Definition) -> String {
    let terminals = def.states.values().filter(|st| st.terminal).count();
    if terminals == 0 {
        format!(
            "product: {} states, 0 ends (structural)",
            def.states.len()
        )
    } else {
        format!(
            "product: {} states, {} {} (structural)",
            def.states.len(),
            terminals,
            if terminals == 1 { "end" } else { "ends" }
        )
    }
}

fn leaf_states(
    region: &crate::declarative::chart::RegionChart,
    children: &[String],
) -> Vec<String> {
    let mut states: BTreeSet<String> = region.states.keys().cloned().collect();
    let mut ordered = Vec::new();
    if let Some((initial, _)) = region.states.iter().find(|(_, s)| s.initial) {
        if states.remove(initial) {
            ordered.push(initial.clone());
        }
    }
    for child in children {
        if states.remove(child) {
            ordered.push(child.clone());
        }
    }
    let mut rest: Vec<String> = states.into_iter().collect();
    rest.sort();
    ordered.extend(rest);
    ordered
}

fn expand_chart_edges(edges: &[ChartEdge]) -> Vec<RegionHop> {
    edges
        .iter()
        .map(|edge| RegionHop {
            from: edge.from.clone(),
            to: edge.to.clone(),
            command: edge.command.clone(),
            guard: edge.rule.clone().unwrap_or_default(),
            stroke: StrokeClass::Declared,
            cross_reads: Vec::new(),
        })
        .collect()
}

fn joint_rails(joints: &[JointChart]) -> Vec<super::JointRail> {
    joints
        .iter()
        .map(|joint| {
            let mut changes: Vec<String> = joint.legs.iter().map(|(r, _, _)| r.clone()).collect();
            changes.sort();
            changes.dedup();
            let mut grouped: BTreeMap<String, (BTreeSet<String>, String)> = BTreeMap::new();
            for (region, from, to) in &joint.legs {
                let entry = grouped.entry(region.clone()).or_insert_with(|| {
                    (BTreeSet::new(), to.clone())
                });
                entry.0.insert(from.clone());
                entry.1 = to.clone();
            }
            let legs = grouped
                .into_iter()
                .map(|(region, (froms, to))| {
                    (
                        region,
                        froms.into_iter().collect::<Vec<_>>().join(", "),
                        to,
                    )
                })
                .collect();
            super::JointRail {
                command: joint.command.clone(),
                changes,
                guard: joint.rule.clone().unwrap_or_default(),
                legs,
            }
        })
        .collect()
}
