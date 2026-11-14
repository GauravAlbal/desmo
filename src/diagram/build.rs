//! Definition + evidence -> `ik.diagram.v1`.

use super::analysis::{
    analyze_change, analyze_def, graph_of, station_order, Analytics, ChangeAnalytics, GuardKind,
};
use super::{
    class_label, ChangeView, CohortRow, Diagram, DiagramKind, Emphasis, Hop,
    RecoveryView, Station, StrokeClass, TableRow, TimelineStep, MAX_FACTS, MAX_LEGEND,
    MAX_LEGEND_LINES,
    MAX_LEVEL0, MAX_SPURS, MAX_STATES,
};
use crate::declarative::analyze::MachineGraph;
use crate::declarative::Definition;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn build(
    def: Option<&Definition>,
    prior: Option<&Definition>,
    findings: &[Value],
    kind: DiagramKind,
    subject: &str,
    operation_id: Option<&str>,
    level: u8,
    focus: Option<&str>,
    ascii: bool,
    sidecar: Option<&std::path::Path>,
) -> Diagram {
    if kind == DiagramKind::Recovery {
        return recovery(def, findings, subject, operation_id, ascii);
    }
    let Some(def) = def else {
        return empty(kind, ascii);
    };
    let analytics = analyze_def(def);
    let graph = graph_of(def);
    let change = prior.map(|old| analyze_change(old, def));
    let mut stations = stations_of(&graph, &analytics, prior, change.as_ref());
    let mut hops = hops_of(def, &graph, prior, change.as_ref());
    let observed_at = annotate_observed(&mut hops, findings, &analytics.initial);
    annotate_explored(&mut hops, findings);
    annotate_refused(&mut hops, findings, &analytics.initial);
    mark_main_line(&mut hops, &stations);
    let you_are_here = you_are_here(kind, observed_at, &analytics);
    collapse(&mut stations, &mut hops, level);
    if let Some(name) = focus {
        apply_focus(&mut stations, &mut hops, name);
    }
    trim_spurs(&mut hops);
    let mut change_view = change
        .as_ref()
        .map(|delta| change_view(def, prior, delta, &analytics, findings));
    if let Some(view) = change_view.as_mut() {
        super::risk::attach_ledger(view, def, prior, findings, sidecar);
    }
    let stuck: Vec<String> = stations
        .iter()
        .filter(|st| st.stuck || st.unreachable)
        .map(|st| st.id.clone())
        .collect();
    let emphasis = emphasis_of(kind, &stuck, change_view.as_ref());
    let next = next_chips(&analytics, you_are_here.as_deref());
    let shortest_from_here = you_are_here
        .as_ref()
        .and_then(|name| analytics.shortest_finish.get(name).copied().flatten());
    let regions = super::regions::from_def(def);
    let mut facts_chunks = facts_chunks(&analytics, kind, change.as_ref());
    if let Some(view) = &regions {
        facts_chunks = super::regions::facts(def, view);
    }
    if hops.iter().all(|hop| hop.stroke == StrokeClass::Declared) {
        if let Some(last) = facts_chunks.last_mut() {
            last.push_str(" · all rules declared");
        }
    }
    let facts_line = facts_chunks.join(" · ");
    let rows = match &regions {
        Some(view) => region_rows(view),
        None => rows_of(&hops),
    };
    let legend = legend_of(
        &hops,
        &stations,
        you_are_here.is_some(),
        kind,
        change_view.as_ref(),
    );
    let loops: Vec<String> = analytics
        .loops
        .iter()
        .map(|item| item.label.clone())
        .collect();
    let unreachable: Vec<String> = analytics.unreachable.clone();
    let mut diagram = Diagram {
        kind,
        level,
        ascii,
        facts_line,
        facts_chunks,
        stations,
        hops,
        you_are_here,
        next,
        loops,
        stuck,
        unreachable,
        shortest_from_here,
        rows,
        legend,
        emphasis,
        change: change_view,
        recovery: None,
        regions,
        focus: focus.map(str::to_string),
        safe_next: None,
        rules: Vec::new(),
    };
    diagram.rules = super::rules::entries(&diagram, def, prior);
    diagram
}

fn empty(kind: DiagramKind, ascii: bool) -> Diagram {
    Diagram {
        kind,
        level: 1,
        ascii,
        facts_line: String::new(),
        facts_chunks: Vec::new(),
        stations: Vec::new(),
        hops: Vec::new(),
        you_are_here: None,
        next: Vec::new(),
        loops: Vec::new(),
        stuck: Vec::new(),
        unreachable: Vec::new(),
        shortest_from_here: None,
        rows: Vec::new(),
        legend: Vec::new(),
        emphasis: Emphasis::YouAreHere,
        change: None,
        recovery: None,
        regions: None,
        focus: None,
        safe_next: None,
        rules: Vec::new(),
    }
}

fn stations_of(
    graph: &MachineGraph,
    analytics: &Analytics,
    prior: Option<&Definition>,
    change: Option<&ChangeAnalytics>,
) -> Vec<Station> {
    let added: BTreeSet<&str> = change
        .map(|delta| delta.states_added.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let removed: Vec<String> = change
        .map(|delta| delta.states_removed.clone())
        .unwrap_or_default();
    let stuck: BTreeSet<&str> = analytics.dead_ends.iter().map(String::as_str).collect();
    let traps: BTreeSet<&str> = analytics.traps.iter().map(String::as_str).collect();
    let loops_only: BTreeSet<&str> = analytics
        .loops_only_traps
        .iter()
        .map(String::as_str)
        .collect();
    let unreachable: BTreeSet<&str> = analytics.unreachable.iter().map(String::as_str).collect();
    let mut order = station_order(graph, analytics);
    for name in &removed {
        if !order.iter().any(|id| id == name) {
            order.push(name.clone());
        }
    }
    let by_name: BTreeMap<&str, &crate::declarative::analyze::GraphState> = graph
        .states
        .iter()
        .map(|st| (st.name.as_str(), st))
        .collect();
    let mut stations = Vec::new();
    for id in order {
        let state = by_name.get(id.as_str());
        let prior_term = prior
            .and_then(|old| old.states.get(&id))
            .map(|st| st.terminal)
            .unwrap_or(false);
        let terminal = state.map(|st| st.terminal).unwrap_or(prior_term);
        let initial = state.map(|st| st.initial).unwrap_or(false);
        stations.push(Station {
            id: id.clone(),
            initial,
            terminal,
            distance: analytics.shortest_finish.get(&id).copied().flatten(),
            is_new: added.contains(id.as_str()),
            is_removed: removed.iter().any(|name| name == &id),
            stuck: stuck.contains(id.as_str()),
            trap: traps.contains(id.as_str()),
            trap_loops_only: loops_only.contains(id.as_str()),
            unreachable: unreachable.contains(id.as_str()),
            collapsed: false,
            collapsed_names: Vec::new(),
        });
    }
    stations
}

fn hops_of(
    def: &Definition,
    graph: &MachineGraph,
    prior: Option<&Definition>,
    change: Option<&ChangeAnalytics>,
) -> Vec<Hop> {
    let added: BTreeSet<&str> = change
        .map(|delta| delta.commands_added.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let mut hops: Vec<Hop> = graph
        .edges
        .iter()
        .map(|edge| Hop {
            from: edge.source.clone(),
            to: edge.target.clone(),
            command: edge.command.clone(),
            stroke: StrokeClass::Declared,
            step: None,
            guarded: edge.guard.is_some(),
            effect: edge.effect.is_some(),
            is_new: added.contains(edge.command.as_str()),
            is_removed: false,
            guard_kind: edge
                .guard
                .as_deref()
                .map(super::classify_guard)
                .unwrap_or(GuardKind::None),
            stub: false,
            refused_reason: None,
            main_line: false,
            guard_text: edge.guard.clone().unwrap_or_default(),
            arg_names: arg_names_of(def, &edge.command),
        })
        .collect();
    if let Some(old) = prior {
        let old_graph = graph_of(old);
        let current: BTreeSet<(&str, &str, &str)> = graph
            .edges
            .iter()
            .map(|e| (e.source.as_str(), e.command.as_str(), e.target.as_str()))
            .collect();
        for edge in &old_graph.edges {
            let key = (
                edge.source.as_str(),
                edge.command.as_str(),
                edge.target.as_str(),
            );
            if current.contains(&key) {
                continue;
            }
            hops.push(Hop {
                from: edge.source.clone(),
                to: edge.target.clone(),
                command: edge.command.clone(),
                stroke: StrokeClass::Declared,
                step: None,
                guarded: edge.guard.is_some(),
                effect: edge.effect.is_some(),
                is_new: false,
                is_removed: true,
                guard_kind: edge
                    .guard
                    .as_deref()
                    .map(super::classify_guard)
                    .unwrap_or(GuardKind::None),
                stub: false,
                refused_reason: None,
                main_line: false,
                guard_text: edge.guard.clone().unwrap_or_default(),
                arg_names: arg_names_of(old, &edge.command),
            });
        }
    }
    hops.sort_by(|a, b| {
        (&a.from, &a.command, &a.to, a.stub).cmp(&(&b.from, &b.command, &b.to, b.stub))
    });
    hops
}

struct ObservedHop {
    command: String,
    from: Option<String>,
    to: Option<String>,
}

fn annotate_observed(hops: &mut [Hop], findings: &[Value], initial: &str) -> Option<String> {
    let mut step = 0u32;
    let mut last = None;
    for finding in findings {
        let Some(event) = observed_commit(finding, initial) else {
            continue;
        };
        step += 1;
        mark_observed_hops(hops, &event, step);
        last = event.to.or(last);
    }
    last
}

fn observed_commit(finding: &Value, initial: &str) -> Option<ObservedHop> {
    if !is_observed_commit(finding) {
        return None;
    }
    Some(ObservedHop {
        command: command_of(finding)?,
        from: from_state(finding, initial),
        to: finding["after"]["state"].as_str().map(str::to_string),
    })
}

fn is_observed_commit(finding: &Value) -> bool {
    matches!(
        (
            finding["evidence_class"].as_str(),
            finding["status"].as_str()
        ),
        (Some("observed_trace"), Some("committed"))
    )
}

fn mark_observed_hops(hops: &mut [Hop], event: &ObservedHop, step: u32) {
    for hop in hops.iter_mut() {
        if hop_matches_event(hop, event) {
            hop.stroke = StrokeClass::Observed;
            hop.step = Some(step);
        }
    }
}

fn hop_matches_event(hop: &Hop, event: &ObservedHop) -> bool {
    !hop.stub
        && hop.command == event.command
        && endpoint_ok(event.from.as_deref(), &hop.from)
        && endpoint_ok(event.to.as_deref(), &hop.to)
}

fn endpoint_ok(want: Option<&str>, got: &str) -> bool {
    match want {
        Some(want) => want == got,
        None => true,
    }
}

fn annotate_explored(hops: &mut [Hop], findings: &[Value]) {
    let mut commands = BTreeSet::new();
    for finding in findings {
        if finding["evidence_class"].as_str() != Some("bounded_paths") {
            continue;
        }
        if finding["status"].as_str() != Some("witness_found") {
            continue;
        }
        if let Some(command) = command_of(finding) {
            commands.insert(command);
        }
    }
    for hop in hops.iter_mut() {
        if hop.stub || hop.stroke != StrokeClass::Declared {
            continue;
        }
        if commands.contains(&hop.command) {
            hop.stroke = StrokeClass::Explored;
        }
    }
}

fn annotate_refused(hops: &mut Vec<Hop>, findings: &[Value], initial: &str) {
    for finding in findings {
        let class = finding["evidence_class"].as_str().unwrap_or("");
        let status = finding["status"].as_str().unwrap_or("");
        let refused =
            status == "refused" && (class == "observed_trace" || class == "continuity_observation");
        if !refused {
            continue;
        }
        let Some(command) = command_of(finding) else {
            continue;
        };
        let from = from_state(finding, initial).unwrap_or_else(|| initial.to_string());
        hops.push(Hop {
            from,
            to: String::new(),
            command,
            stroke: StrokeClass::Refused,
            step: None,
            guarded: false,
            effect: false,
            is_new: false,
            is_removed: false,
            guard_kind: GuardKind::None,
            stub: true,
            refused_reason: refused_reason(finding),
            main_line: false,
            guard_text: String::new(),
            arg_names: Vec::new(),
        });
    }
}

fn arg_names_of(def: &Definition, command: &str) -> Vec<String> {
    def.commands
        .get(command)
        .and_then(|cmd| cmd.args.as_ref())
        .map(|args| args.keys().cloned().collect())
        .unwrap_or_default()
}

fn command_of(finding: &Value) -> Option<String> {
    if let Some(name) = non_empty(finding["source_coordinate"]["command"].as_str()) {
        return Some(name.to_string());
    }
    if let Some(name) = non_empty(finding["present"]["command"].as_str()) {
        return Some(name.to_string());
    }
    let attempted = finding["attempted"].as_str().unwrap_or("");
    let name = attempted.split_whitespace().next().unwrap_or("");
    if name.is_empty() || name.starts_with('{') {
        None
    } else {
        Some(name.to_string())
    }
}

fn from_state(finding: &Value, initial: &str) -> Option<String> {
    if let Some(name) = non_empty(finding["before"]["state"].as_str()) {
        return Some(name.to_string());
    }
    if finding["before"].is_null() {
        return Some(initial.to_string());
    }
    None
}

fn refused_reason(finding: &Value) -> Option<String> {
    if let Some(code) = non_empty(finding["present"]["code"].as_str()) {
        return Some(code.to_string());
    }
    let scope = finding["scope"].as_str().unwrap_or("");
    let Some(rest) = scope.split("refused ").nth(1) else {
        return None;
    };
    let code = rest.split(';').next().unwrap_or(rest).trim();
    if code.is_empty() {
        None
    } else {
        Some(code.to_string())
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|text| !text.is_empty())
}

fn mark_main_line(hops: &mut [Hop], stations: &[Station]) {
    let line: Vec<&str> = stations
        .iter()
        .filter(|st| !st.collapsed && !st.is_removed)
        .map(|st| st.id.as_str())
        .collect();
    for hop in hops.iter_mut() {
        if hop.stub || hop.from == hop.to {
            continue;
        }
        if let Some(i) = line.iter().position(|id| *id == hop.from) {
            if line.get(i + 1) == Some(&hop.to.as_str()) {
                hop.main_line = true;
            }
        }
    }
}

fn you_are_here(
    kind: DiagramKind,
    observed_at: Option<String>,
    analytics: &Analytics,
) -> Option<String> {
    if kind != DiagramKind::Delivery && kind != DiagramKind::Change {
        return observed_at;
    }
    if observed_at.is_some() {
        return observed_at;
    }
    if kind == DiagramKind::Delivery {
        Some(analytics.initial.clone())
    } else {
        None
    }
}

fn collapse(stations: &mut Vec<Station>, hops: &mut Vec<Hop>, level: u8) {
    let cap = if level == 0 { MAX_LEVEL0 } else { MAX_STATES };
    if stations.len() <= cap {
        return;
    }
    let keep = cap.saturating_sub(1);
    let collapsed: Vec<String> = stations.iter().skip(keep).map(|st| st.id.clone()).collect();
    if collapsed.is_empty() {
        return;
    }
    stations.truncate(keep);
    stations.push(Station {
        id: "collapsed_tail".into(),
        initial: false,
        terminal: false,
        distance: None,
        is_new: false,
        is_removed: false,
        stuck: false,
        trap: false,
        trap_loops_only: false,
        unreachable: false,
        collapsed: true,
        collapsed_names: collapsed.clone(),
    });
    let set: BTreeSet<&str> = collapsed.iter().map(String::as_str).collect();
    for hop in hops.iter_mut() {
        if set.contains(hop.from.as_str()) {
            hop.from = "collapsed_tail".into();
        }
        if set.contains(hop.to.as_str()) {
            hop.to = "collapsed_tail".into();
        }
    }
    hops.retain(|hop| !(hop.from == "collapsed_tail" && hop.to == "collapsed_tail" && !hop.stub));
}

fn apply_focus(stations: &mut Vec<Station>, hops: &mut Vec<Hop>, focus: &str) {
    let mut keep: BTreeSet<String> = BTreeSet::new();
    keep.insert(focus.to_string());
    for hop in hops.iter() {
        if hop.from == focus || hop.to == focus {
            keep.insert(hop.from.clone());
            if !hop.to.is_empty() {
                keep.insert(hop.to.clone());
            }
        }
    }
    stations.retain(|st| keep.contains(&st.id));
    hops.retain(|hop| keep.contains(&hop.from) || hop.to == focus);
}

fn trim_spurs(hops: &mut Vec<Hop>) {
    let mut spurs = 0usize;
    hops.retain(|hop| {
        if hop.main_line || hop.stub || hop.from == hop.to {
            return true;
        }
        if spurs < MAX_SPURS {
            spurs += 1;
            true
        } else {
            false
        }
    });
}

fn next_chips(analytics: &Analytics, here: Option<&str>) -> Vec<String> {
    let Some(here) = here else {
        return Vec::new();
    };
    analytics
        .enabled
        .get(here)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|cmd| {
            if cmd.guarded {
                format!("{} (guarded)", cmd.command)
            } else {
                cmd.command
            }
        })
        .collect()
}

fn facts_chunks(
    analytics: &Analytics,
    kind: DiagramKind,
    change: Option<&ChangeAnalytics>,
) -> Vec<String> {
    let mut chunks = Vec::new();
    chunks.push(format!(
        "{}, {}",
        counted(analytics.state_count, "state", "states"),
        counted(analytics.command_count, "command", "commands"),
    ));
    chunks.push(counted(analytics.terminals.len(), "end", "ends"));
    chunks.extend(reachability_chunks(analytics));
    let _ = (kind, change);
    if !analytics.loops.is_empty() && chunks.len() < MAX_FACTS && analytics.every_state_reaches_end
    {
        chunks.push(counted(analytics.loops.len(), "loop", "loops"));
    }
    if chunks.len() < MAX_FACTS {
        if let Some(n) = analytics.shortest_from_initial {
            chunks.push(format!(
                "shortest finish: {n} {}",
                if n == 1 { "step" } else { "steps" }
            ));
        }
    }
    chunks.truncate(MAX_FACTS);
    chunks
}

fn counted(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn reachability_chunks(analytics: &Analytics) -> Vec<String> {
    if analytics.every_state_reaches_end {
        return vec!["every state reaches an end (structural)".into()];
    }
    let unreachable = analytics.unreachable.len();
    let dead = analytics
        .dead_ends
        .iter()
        .filter(|name| !analytics.unreachable.contains(*name))
        .count();
    let trap = analytics
        .traps
        .iter()
        .filter(|name| !analytics.unreachable.contains(*name))
        .count();
    let loops_only = analytics
        .loops_only_traps
        .iter()
        .filter(|name| !analytics.unreachable.contains(*name))
        .count();
    let cannot = dead + trap;
    let mut chunks = Vec::new();
    if cannot > 0 {
        chunks.push(format!(
            "{cannot} cannot reach an end ({})",
            cannot_detail(dead, trap, loops_only)
        ));
    }
    if unreachable > 0 {
        chunks.push(counted(unreachable, "unreachable", "unreachable"));
    }
    if chunks.is_empty() {
        chunks.push(format!(
            "{} cannot reach an end (structural)",
            counted(analytics.cannot_finish.len(), "state", "states")
        ));
    }
    chunks
}

fn cannot_detail(dead: usize, trap: usize, loops_only: usize) -> String {
    let mut bits = Vec::new();
    if dead > 0 {
        bits.push(counted(dead, "dead end", "dead ends"));
    }
    if loops_only > 0 {
        bits.push(counted(loops_only, "loops-only trap", "loops-only traps"));
    }
    let other_trap = trap.saturating_sub(loops_only);
    if other_trap > 0 {
        bits.push(counted(other_trap, "trap", "traps"));
    }
    bits.join(", ")
}

fn change_view(
    def: &Definition,
    prior: Option<&Definition>,
    delta: &ChangeAnalytics,
    analytics: &Analytics,
    findings: &[Value],
) -> ChangeView {
    let prior_version = prior.map(|old| old.machine_version).unwrap_or(0);
    let continuity = continuity_note(findings, prior_version);
    let cohorts = cohort_rows(def, prior, analytics, findings, delta);
    ChangeView {
        prior_version,
        new_version: def.machine_version,
        added_commands: delta.commands_added.clone(),
        removed_commands: delta.commands_removed.clone(),
        added_states: delta.states_added.clone(),
        removed_states: delta.states_removed.clone(),
        guards_changed: delta.guards_changed.clone(),
        reachability_line: delta.reachability_line.clone(),
        old_every_reaches_end: delta.old_every_reaches_end,
        new_dead_ends: delta.new_dead_ends.clone(),
        finish_changed: delta.finish_changed.clone(),
        cohorts,
        continuity_note: continuity,
        risk: Vec::new(),
        no_change: Vec::new(),
        divergence: Vec::new(),
        observed_note: String::new(),
        live_instances: String::new(),
        callers: String::new(),
        rule_effect: String::new(),
        compat: prior
            .map(|old| crate::declarative::compat::drain_compat(old, def))
            .unwrap_or_default(),
        machine_id: def.machine_id.clone(),
    }
}

fn continuity_note(findings: &[Value], prior_version: u64) -> String {
    let rows: Vec<&Value> = findings
        .iter()
        .filter(|row| row["evidence_class"].as_str() == Some("continuity_observation"))
        .collect();
    if rows.is_empty() {
        return "continuity not demonstrated".into();
    }
    let mut parts = Vec::new();
    for row in rows {
        let command = row["present"]["command"].as_str().unwrap_or("");
        if command.is_empty() {
            continue;
        }
        if row["status"].as_str() == Some("refused") {
            let reason = row["present"]["code"].as_str().unwrap_or("refusal");
            parts.push(format!("{command}: refused ({reason})"));
        } else {
            parts.push(format!("{command}: committed"));
        }
    }
    format!(
        "Existing instances (version {prior_version}): {}",
        parts.join(" | ")
    )
}

fn cohort_rows(
    def: &Definition,
    prior: Option<&Definition>,
    analytics: &Analytics,
    findings: &[Value],
    delta: &ChangeAnalytics,
) -> Vec<CohortRow> {
    let old_enabled = prior.map(analyze_def);
    let mut rows = Vec::new();
    let mut names: Vec<String> = def.states.keys().cloned().collect();
    if let Some(old) = prior {
        for name in old.states.keys() {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
    }
    names.sort();
    for name in names {
        let existing = existing_cell(&name, old_enabled.as_ref(), findings, delta);
        let new_cell = new_cell(&name, analytics);
        rows.push(CohortRow {
            states: vec![name],
            existing,
            new_instances: new_cell,
        });
    }
    collapse_cohorts(rows)
}

fn existing_cell(
    state: &str,
    old: Option<&Analytics>,
    findings: &[Value],
    delta: &ChangeAnalytics,
) -> String {
    let Some(old) = old else {
        return "⊘".into();
    };
    if !old.enabled.contains_key(state) {
        return "⊘".into();
    }
    let mut bits = Vec::new();
    if let Some(cmds) = old.enabled.get(state) {
        for cmd in cmds {
            bits.push(cmd.command.clone());
        }
    }
    for command in refused_commands(findings, state) {
        if bits
            .iter()
            .all(|item| item.trim_start_matches('✗') != command)
        {
            bits.push(format!("✗{command}"));
        }
    }
    if bits.is_empty() {
        if delta.old_every_reaches_end {
            "↻".into()
        } else {
            "⊘".into()
        }
    } else {
        bits.join(" ")
    }
}

fn refused_commands(findings: &[Value], state: &str) -> Vec<String> {
    let mut out = Vec::new();
    for finding in findings {
        if finding["evidence_class"].as_str() != Some("continuity_observation") {
            continue;
        }
        if finding["status"].as_str() != Some("refused") {
            continue;
        }
        let from = finding["before"]["state"].as_str();
        if from.is_some() && from != Some(state) {
            continue;
        }
        if let Some(command) = non_empty(finding["present"]["command"].as_str()) {
            out.push(command.to_string());
        }
    }
    out
}

fn new_cell(state: &str, analytics: &Analytics) -> String {
    let Some(cmds) = analytics.enabled.get(state) else {
        return "⊘".into();
    };
    if cmds.is_empty() {
        return "⊘".into();
    }
    cmds.iter()
        .map(|cmd| cmd.command.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn collapse_cohorts(rows: Vec<CohortRow>) -> Vec<CohortRow> {
    rows.into_iter().take(7).collect()
}

fn region_rows(view: &super::RegionView) -> Vec<TableRow> {
    let mut rows = Vec::new();
    for lane in &view.lanes {
        for hop in lane.hops.iter().chain(lane.parent_hops.iter()) {
            rows.push(TableRow {
                command: hop.command.clone(),
                from: hop.from.clone(),
                to: hop.to.clone(),
                guard: hop.guard.clone(),
                effect: lane.name.clone(),
                class: "region".into(),
            });
        }
    }
    for joint in &view.joints {
        rows.push(TableRow {
            command: joint.command.clone(),
            from: joint.changes.join(", "),
            to: String::new(),
            guard: joint.guard.clone(),
            effect: String::new(),
            class: "joint".into(),
        });
    }
    rows
}

fn rows_of(hops: &[Hop]) -> Vec<TableRow> {
    hops.iter()
        .map(|hop| {
            let to = if hop.stub {
                match &hop.refused_reason {
                    Some(reason) => format!("(refused: {reason})"),
                    None => "(refused)".into(),
                }
            } else {
                hop.to.clone()
            };
            let effect = if hop.effect { "effect" } else { "" };
            TableRow {
                command: hop.command.clone(),
                from: hop.from.clone(),
                to,
                guard: hop.guard_text.clone(),
                effect: effect.to_string(),
                class: class_label(hop.stroke, hop.is_new, hop.is_removed),
            }
        })
        .collect()
}

fn legend_of(
    hops: &[Hop],
    stations: &[Station],
    here: bool,
    kind: DiagramKind,
    change: Option<&ChangeView>,
) -> Vec<&'static str> {
    let mut legend = Vec::new();
    for item in legend_candidates(hops, stations, here, kind) {
        push_legend(&mut legend, item);
    }
    let cap = MAX_LEGEND * MAX_LEGEND_LINES;
    for item in glyph_legend(change) {
        if legend.len() < cap && !legend.contains(&item) {
            legend.push(item);
        }
    }
    legend
}

fn glyph_legend(change: Option<&ChangeView>) -> Vec<&'static str> {
    let Some(view) = change else {
        return Vec::new();
    };
    let mut text = String::new();
    for row in &view.cohorts {
        text.push_str(&row.existing);
        text.push_str(&row.new_instances);
    }
    let mut items = Vec::new();
    if text.contains('✓') {
        items.push("✓ yes");
    }
    if text.contains('✗') {
        items.push("✗ no/refused");
    }
    if text.contains('↻') {
        items.push("↻ drains under old rules");
    }
    if text.contains('⊘') {
        items.push("⊘ n/a");
    }
    items
}

fn legend_candidates(
    hops: &[Hop],
    stations: &[Station],
    here: bool,
    kind: DiagramKind,
) -> Vec<&'static str> {
    let mut items = Vec::new();
    if kind == DiagramKind::Change
        && (hops.iter().any(|h| h.is_new) || stations.iter().any(|s| s.is_new))
    {
        items.push("✚ new");
    }
    if kind == DiagramKind::Delivery && here && stations.iter().all(|s| !s.stuck && !s.unreachable)
    {
        items.push("▼ HERE");
    }
    if hops.iter().any(|h| h.is_removed) || stations.iter().any(|s| s.is_removed) {
        items.push("✖ removed");
    }
    if stations.iter().any(|s| s.distance.is_some()) {
        items.push("·N steps to end");
    }
    if stations.iter().any(|s| s.terminal) {
        items.push("◉ end");
    }
    if hops.iter().any(|h| h.guarded) {
        items.push("⎇ guarded");
    }
    items.extend(stroke_legend(hops));
    items
}

fn stroke_legend(hops: &[Hop]) -> Vec<&'static str> {
    let mut items = Vec::new();
    if hops.iter().any(|h| h.stroke == StrokeClass::Observed) {
        items.push("━━▶ observed");
    }
    if hops.iter().any(|h| h.stroke == StrokeClass::Explored) {
        items.push("┄┄▶ explored");
    }
    if hops
        .iter()
        .any(|h| h.stroke == StrokeClass::Declared && !h.stub)
    {
        items.push("──▶ declared");
    }
    if hops.iter().any(|h| h.stroke == StrokeClass::Refused) {
        items.push("──✕ refused");
    }
    items
}

fn push_legend(legend: &mut Vec<&'static str>, item: &'static str) {
    if legend.len() < MAX_LEGEND && !legend.contains(&item) {
        legend.push(item);
    }
}

fn emphasis_of(kind: DiagramKind, stuck: &[String], change: Option<&ChangeView>) -> Emphasis {
    match kind {
        DiagramKind::Recovery => Emphasis::YouAreHere,
        DiagramKind::Change => {
            if change.is_some_and(|view| {
                !view.added_commands.is_empty() || !view.added_states.is_empty()
            }) {
                Emphasis::New
            } else {
                Emphasis::New
            }
        }
        DiagramKind::Delivery => {
            if stuck.is_empty() {
                Emphasis::YouAreHere
            } else {
                Emphasis::Stuck
            }
        }
    }
}

fn recovery(
    def: Option<&Definition>,
    findings: &[Value],
    subject: &str,
    operation_id: Option<&str>,
    ascii: bool,
) -> Diagram {
    let named = distinct_operation_ids(findings);
    if operation_id.is_none() && named.len() > 1 {
        return recovery_ambiguous(
            format!(
                "ambiguous: evidence names {} operations ({}); select one with --operation <id>",
                named.len(),
                named.join(", ")
            ),
            ascii,
        );
    }
    let attributed = attributed_findings(findings, subject, operation_id.or(named.first().map(String::as_str)));
    let command = recovery_command(&attributed);
    let signals = scan_recovery(&attributed);
    let timeline = recovery_timeline(&command, &signals);
    let you_are_here = recovery_here(&signals).map(str::to_string);
    let (resolved_outcome, resolved_how) = recovery_resolution(&signals);
    let fork = vec![
        (
            "committed".into(),
            "look up by saved operation id; never resend".into(),
        ),
        (
            "not committed".into(),
            "look up by saved operation id; never resend".into(),
        ),
        (
            "unknown".into(),
            "look up by saved operation id; never resend".into(),
        ),
    ];
    let view = RecoveryView {
        timeline,
        command,
        you_are_here: you_are_here.clone(),
        fork,
        subject: subject.to_string(),
        operation_id: operation_id.map(str::to_string),
        resolved_outcome,
        resolved_how,
    };
    let facts_chunks = vec![
        "events the evidence supports".into(),
        "lookup by operation id".into(),
        "never resend".into(),
    ];
    let facts_line = facts_chunks.join(" · ");
    let rows = view
        .timeline
        .iter()
        .map(|step| TableRow {
            command: step.id.to_string(),
            from: String::new(),
            to: String::new(),
            guard: String::new(),
            effect: String::new(),
            class: recovery_class(step),
        })
        .collect();
    let legend = recovery_legend(&view);
    Diagram {
        kind: DiagramKind::Recovery,
        level: 1,
        ascii,
        facts_line,
        facts_chunks,
        stations: recovery_stations(def),
        hops: Vec::new(),
        you_are_here,
        next: Vec::new(),
        loops: Vec::new(),
        stuck: Vec::new(),
        unreachable: Vec::new(),
        shortest_from_here: None,
        rows,
        legend,
        emphasis: Emphasis::YouAreHere,
        change: None,
        recovery: Some(view),
        focus: None,
        regions: None,
        safe_next: Some(recovery_safe_next(&signals)),
        rules: Vec::new(),
    }
}

fn recovery_safe_next(signals: &RecoverySignals) -> String {
    if signals.found_by_lookup || signals.historical {
        "do not resend; the earlier result stands".into()
    } else {
        "look up by saved operation id; never resend".into()
    }
}

struct RecoverySignals {
    reply_lost: bool,
    reply_received: bool,
    found_by_lookup: bool,
    historical: bool,
    sent: bool,
    plan: bool,
    saved: bool,
}

pub(super) fn distinct_operation_ids(findings: &[Value]) -> Vec<String> {
    let mut ids = BTreeSet::new();
    for finding in findings {
        if let Some(id) = finding_operation_id(finding) {
            ids.insert(id);
        }
    }
    ids.into_iter().collect()
}

fn attempted_object(finding: &Value) -> Option<Value> {
    match &finding["attempted"] {
        Value::String(text) => serde_json::from_str(text).ok(),
        Value::Object(_) => Some(finding["attempted"].clone()),
        _ => None,
    }
}

fn finding_operation_id(finding: &Value) -> Option<String> {
    attempted_object(finding)
        .as_ref()
        .and_then(|value| value.get("operation_id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .or_else(|| {
            finding["operation_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(str::to_string)
        })
}

fn finding_command(finding: &Value) -> Option<String> {
    attempted_object(finding)
        .as_ref()
        .and_then(|value| value.get("command"))
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            finding["source_coordinate"]["command"]
                .as_str()
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            finding["command"]
                .as_str()
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
}

fn finding_subject(finding: &Value) -> Option<String> {
    finding["subject"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            finding["aggregate"]
                .as_str()
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
}

fn attributed_findings(
    findings: &[Value],
    subject: &str,
    operation_id: Option<&str>,
) -> Vec<Value> {
    findings
        .iter()
        .filter(|finding| {
            if let (Some(selected), Some(named)) = (operation_id, finding_operation_id(finding)) {
                if named != selected {
                    return false;
                }
            }
            if !subject.is_empty() {
                if let Some(named) = finding_subject(finding) {
                    if named != subject {
                        return false;
                    }
                }
            }
            true
        })
        .cloned()
        .collect()
}

fn recovery_command(findings: &[Value]) -> String {
    findings
        .iter()
        .find_map(finding_command)
        .unwrap_or_else(|| "command not identified in evidence".into())
}

fn recovery_ambiguous(message: String, ascii: bool) -> Diagram {
    let mut diagram = empty(DiagramKind::Recovery, ascii);
    diagram.facts_line = message.clone();
    diagram.facts_chunks = vec![message];
    diagram
}

fn scan_recovery(findings: &[Value]) -> RecoverySignals {
    let mut signals = RecoverySignals {
        reply_lost: false,
        reply_received: false,
        found_by_lookup: false,
        historical: false,
        sent: false,
        plan: false,
        saved: false,
    };
    for finding in findings {
        match finding["status"].as_str().unwrap_or("") {
            "reply_dropped" => {
                signals.reply_lost = true;
                signals.sent = true;
            }
            "sent" => signals.sent = true,
            "committed" => signals.reply_received = true,
            "committed_exact" | "found" => signals.found_by_lookup = true,
            "historical" => {
                signals.historical = true;
                signals.found_by_lookup = true;
            }
            "planned" | "plan" => signals.plan = true,
            "saved" => signals.saved = true,
            _ => {}
        }
    }
    signals
}

fn recovery_stations(def: Option<&Definition>) -> Vec<Station> {
    let Some(def) = def else {
        return Vec::new();
    };
    def.states
        .iter()
        .map(|(name, state)| Station {
            id: name.clone(),
            initial: state.initial,
            terminal: state.terminal,
            distance: None,
            is_new: false,
            is_removed: false,
            stuck: false,
            trap: false,
            trap_loops_only: false,
            unreachable: false,
            collapsed: false,
            collapsed_names: Vec::new(),
        })
        .collect()
}

const OBSERVED: &str = "observed";
const LOOKUP: &str = "found by lookup";

fn timeline_step(id: &'static str, label: &str, how: &'static str, note: &str) -> TimelineStep {
    TimelineStep {
        id,
        label: label.to_string(),
        how,
        note: note.to_string(),
    }
}

/// Only events the evidence supports. A plain command has no plan or save stage, so
/// those appear only when the evidence names them.
fn recovery_timeline(command: &str, signals: &RecoverySignals) -> Vec<TimelineStep> {
    let mut steps = Vec::new();
    if signals.plan {
        steps.push(timeline_step("plan", &format!("plan {command}"), OBSERVED, ""));
    }
    if signals.saved {
        steps.push(timeline_step("saved", "saved", OBSERVED, ""));
    }
    if signals.sent || signals.reply_lost || signals.reply_received {
        steps.push(timeline_step("sent", "request sent", OBSERVED, ""));
    }
    if signals.reply_lost {
        steps.push(timeline_step(
            "lost",
            "reply lost",
            OBSERVED,
            "the caller never saw the result",
        ));
    }
    steps.extend(commit_step(signals));
    if signals.historical {
        steps.push(timeline_step(
            "history",
            "asked again",
            LOOKUP,
            "the earlier result stands; it is not a current handle",
        ));
    }
    steps
}

fn commit_step(signals: &RecoverySignals) -> Option<TimelineStep> {
    if signals.reply_received && !signals.reply_lost {
        Some(timeline_step(
            "committed",
            "committed",
            OBSERVED,
            "the reply reported the commit",
        ))
    } else if signals.found_by_lookup {
        Some(timeline_step(
            "committed",
            "committed",
            LOOKUP,
            "found under the same operation id; not committed again",
        ))
    } else {
        None
    }
}

fn recovery_here(signals: &RecoverySignals) -> Option<&'static str> {
    match (
        signals.historical,
        signals.found_by_lookup,
        signals.reply_received && !signals.reply_lost,
        signals.reply_lost,
    ) {
        (true, _, _, _) => Some("history"),
        (_, true, _, _) | (_, _, true, _) => Some("committed"),
        (_, _, _, true) => Some("lost"),
        _ => None,
    }
}

fn recovery_resolution(signals: &RecoverySignals) -> (Option<String>, Option<String>) {
    if signals.historical || signals.found_by_lookup {
        (Some("committed".into()), Some("found by lookup".into()))
    } else {
        (None, None)
    }
}

fn recovery_class(step: &TimelineStep) -> String {
    if step.how == LOOKUP {
        "lookup".into()
    } else {
        "observed".into()
    }
}

fn recovery_legend(view: &RecoveryView) -> Vec<&'static str> {
    let mut legend = vec!["▼ HERE"];
    if view.timeline.iter().any(|step| step.how == OBSERVED) {
        push_legend(&mut legend, "━ observed");
    }
    if view.timeline.iter().any(|step| step.how == LOOKUP) {
        push_legend(&mut legend, "┄ lookup");
    }
    if view.timeline.iter().any(|step| step.id == "lost") {
        push_legend(&mut legend, "✂ reply lost");
    }
    legend.truncate(MAX_LEGEND);
    legend
}
