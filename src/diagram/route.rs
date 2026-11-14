//! Two-column river map. Primary projection of `ik.diagram.v1`.

use super::pretty::{self, PrettyRule};
use super::{
    display_width, ChangeView, Diagram, DiagramKind, Hop, RecoveryView, RegionHop, RegionLane,
    RegionView, Station, TimelineStep,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_COLS: usize = 120;
const RIGHT: usize = 30;
const BOX: usize = 38;

pub(super) fn compressed(diagram: &Diagram) -> String {
    let mut out = String::new();
    if !diagram.facts_line.is_empty() {
        for line in wrap_width(&diagram.facts_line, MAX_COLS) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.push_str("```text\n");
    let map = render(diagram);
    out.push_str(&map);
    if !map.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("```\n");
    let legend = legend_from(&map);
    for chunk in legend.chunks(super::MAX_LEGEND) {
        out.push_str("legend: ");
        out.push_str(&chunk.join(" · "));
        out.push('\n');
    }
    out
}

fn legend_from(map: &str) -> Vec<&'static str> {
    let mut items = Vec::new();
    if map.contains("YOU ARE HERE") {
        items.push("◀── YOU ARE HERE");
    }
    if map.contains(" END") && map.contains('✔') {
        items.push("✔ END");
    }
    if map.contains('↺') {
        items.push("↺ loop");
    }
    if map.contains('✂') {
        items.push("✂ reply lost");
    }
    if map.contains('✚') {
        items.push("✚ new");
    }
    if map.contains('↗') {
        items.push("↗ other exit");
    }
    if map.contains('✖') {
        items.push("✖ removed");
    }
    if map.contains('⟪') {
        items.push("⟪ ⟫ one command, unfolded");
    }
    let room = super::MAX_LEGEND * super::MAX_LEGEND_LINES;
    if items.len() + 2 <= room {
        items.push("● initial");
        items.push("○ other");
    }
    items
}

pub(super) fn render(diagram: &Diagram) -> String {
    if diagram.kind == DiagramKind::Recovery {
        return recovery_map(diagram);
    }
    if let Some(regions) = &diagram.regions {
        return region_maps(diagram, regions);
    }
    if diagram.kind == DiagramKind::Change {
        return change_map(diagram);
    }
    delivery_map(diagram)
}

fn delivery_map(diagram: &Diagram) -> String {
    let mut rules = Vec::new();
    let mut lines = river(diagram, None, &mut rules);
    lines.extend(rules_block(&rules));
    join_lines(lines, diagram.ascii)
}

fn change_map(diagram: &Diagram) -> String {
    let mut rules = Vec::new();
    let mut lines = river(diagram, Some(true), &mut rules);
    if let Some(view) = &diagram.change {
        lines.extend(risk_block(view));
    }
    lines.extend(rules_block(&rules));
    join_lines(lines, diagram.ascii)
}

fn river(
    diagram: &Diagram,
    new_line: Option<bool>,
    rules: &mut Vec<(String, PrettyRule)>,
) -> Vec<String> {
    let spine = spine_ids(diagram, new_line);
    let mut drawn = BTreeSet::new();
    let mut lines = emit_path(diagram, &spine, new_line, rules, &mut drawn);
    let extra = leftover(diagram, &drawn, new_line, false);
    if !extra.is_empty() {
        lines.push("also reachable:".into());
        lines.extend(emit_path(diagram, &extra, new_line, rules, &mut drawn));
    }
    let ghost = leftover(diagram, &drawn, new_line, true);
    if !ghost.is_empty() {
        lines.push("unreachable from start:".into());
        lines.extend(emit_path(diagram, &ghost, new_line, rules, &mut drawn));
    }
    super::rails::add_return_rails(lines).0
}

fn leftover(
    diagram: &Diagram,
    drawn: &BTreeSet<String>,
    new_line: Option<bool>,
    unreachable: bool,
) -> Vec<String> {
    diagram
        .stations
        .iter()
        .filter(|st| version_station(st, new_line))
        .filter(|st| !drawn.contains(&st.id))
        .filter(|st| st.unreachable == unreachable)
        .map(|st| st.id.clone())
        .collect()
}

fn emit_path(
    diagram: &Diagram,
    ids: &[String],
    new_line: Option<bool>,
    rules: &mut Vec<(String, PrettyRule)>,
    drawn: &mut BTreeSet<String>,
) -> Vec<String> {
    let mut lines = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        let next = ids.get(index + 1).map(String::as_str);
        if let Some(station) = station(diagram, id) {
            lines.extend(emit_state(diagram, station, next, new_line, rules));
            drawn.insert(id.clone());
            if let Some(to) = next {
                if let Some(hop) = hop_between(diagram, id, to, new_line) {
                    lines.extend(emit_step(diagram, hop, rules));
                }
            }
        }
    }
    lines
}

fn emit_state(
    diagram: &Diagram,
    station: &Station,
    next: Option<&str>,
    new_line: Option<bool>,
    rules: &mut Vec<(String, PrettyRule)>,
) -> Vec<String> {
    let left = state_left(station);
    let mut right = Vec::new();
    if diagram.you_are_here.as_deref() == Some(station.id.as_str()) {
        let steps = station.distance.unwrap_or(0);
        let unit = if steps == 1 { "step" } else { "steps" };
        right.push(format!("◀── YOU ARE HERE ({steps} {unit} to end)"));
    }
    let loops = named_hops(diagram, &station.id, new_line, HopKind::Loop, next);
    if !loops.is_empty() {
        let names = loops.iter().map(|h| h.command.as_str()).collect::<Vec<_>>();
        right.push(format!("↺ can also: {}", names.join(", ")));
    }
    if let Some(mark) = change_mark(diagram, station) {
        right.push(mark);
    } else if quiet_state(diagram, station, new_line) {
        right.push("(unchanged)".into());
    }
    if station.stuck && !station.unreachable {
        right.push("dead end".into());
    }
    if station.trap_loops_only {
        right.push("trap: loops only".into());
    } else if station.trap {
        right.push("trap".into());
    }
    let mut lines = vec![pair(&left, &right.join("  "))];
    lines.extend(change_split(diagram, station, next, rules));
    for hop in named_hops(diagram, &station.id, new_line, HopKind::Side, next) {
        lines.push(side_exit(diagram, hop, rules));
    }
    if diagram.kind == DiagramKind::Change {
        for hop in diagram.hops.iter().filter(|hop| {
            hop.from == station.id
                && hop.is_removed
                && !hop.stub
                && hop.from != hop.to
                && next != Some(hop.to.as_str())
        }) {
            lines.push(side_exit(diagram, hop, rules));
        }
    }
    lines
}

fn state_left(station: &Station) -> String {
    if station.terminal {
        format!("  ✔ [ {} ]  END", station.id)
    } else if station.initial {
        format!("  ● [ {} ]", station.id)
    } else {
        format!("  ○ [ {} ]", station.id)
    }
}

fn emit_step(diagram: &Diagram, hop: &Hop, rules: &mut Vec<(String, PrettyRule)>) -> Vec<String> {
    let right = if quiet_hop(diagram, hop) {
        "(unchanged)".into()
    } else {
        rule_bit(diagram, hop, rules)
    };
    let cmd = pair(&format!("      │  {}", hop.command), &right);
    vec!["      │".into(), cmd, "      ▼".into()]
}

fn side_exit(diagram: &Diagram, hop: &Hop, rules: &mut Vec<(String, PrettyRule)>) -> String {
    if hop.is_removed {
        let version = diagram
            .change
            .as_ref()
            .map(|view| view.new_version)
            .unwrap_or(0);
        return pair(
            "      ",
            &format!(
                "✖ {} → [ {} ]  REMOVED in v{version}",
                hop.command, hop.to
            ),
        );
    }
    let rule = rule_bit(diagram, hop, rules);
    if hop.from == hop.to {
        pair("      ", &format!("↺ {}{rule}", hop.command))
    } else {
        pair(
            "      ",
            &format!("↗ {} → [ {} ]{rule}", hop.command, hop.to),
        )
    }
}

fn change_split(
    diagram: &Diagram,
    station: &Station,
    next: Option<&str>,
    rules: &mut Vec<(String, PrettyRule)>,
) -> Vec<String> {
    let Some(view) = &diagram.change else {
        return Vec::new();
    };
    let added: Vec<&Hop> = diagram
        .hops
        .iter()
        .filter(|hop| hop.from == station.id && hop.is_new)
        .filter(|hop| hop.from == hop.to || next != Some(hop.to.as_str()))
        .collect();
    if added.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for hop in added {
        let rule = rule_bit(diagram, hop, rules);
        let loop_mark = if hop.from == hop.to { "↺ " } else { "" };
        let left = format!("      │  ✚ {loop_mark}{}", hop.command);
        let right = if rule.is_empty() {
            "NEW —".into()
        } else {
            format!("NEW — {rule}")
        };
        lines.push(pair(&left, &right));
        lines.push(format!(
            "      │    • New loans (v{}): can {}",
            view.new_version, hop.command
        ));
        match refused_line(view) {
            Some(text) => lines.push(format!(
                "      │    • Old loans (v{}): cannot; {text}",
                view.prior_version
            )),
            None => lines.push(format!(
                "      │    • Old loans (v{}): cannot; refusal not observed",
                view.prior_version
            )),
        }
    }
    lines
}

fn refused_line(view: &ChangeView) -> Option<String> {
    view.divergence.iter().find_map(|line| {
        if line.contains("REFUSED") {
            Some(line.trim_start_matches("✗ ").to_string())
        } else {
            None
        }
    })
}

#[derive(Clone, Copy)]
enum HopKind {
    Loop,
    Side,
}

fn named_hops<'a>(
    diagram: &'a Diagram,
    from: &str,
    new_line: Option<bool>,
    kind: HopKind,
    next: Option<&str>,
) -> Vec<&'a Hop> {
    diagram
        .hops
        .iter()
        .filter(|hop| hop.from == from && !hop.stub && version_hop(hop, new_line))
        .filter(|hop| match kind {
            HopKind::Loop => hop.from == hop.to && !hop.is_new,
            HopKind::Side => hop.from != hop.to && next != Some(hop.to.as_str()),
        })
        .collect()
}

fn quiet_state(diagram: &Diagram, station: &Station, new_line: Option<bool>) -> bool {
    if diagram.kind != DiagramKind::Change || station.is_new || station.is_removed {
        return false;
    }
    let hops: Vec<&Hop> = diagram
        .hops
        .iter()
        .filter(|hop| hop.from == station.id && !hop.stub)
        .collect();
    if hops
        .iter()
        .any(|hop| hop.is_new || hop.is_removed || rule_changed(diagram, hop))
    {
        return false;
    }
    hops.iter()
        .filter(|hop| version_hop(hop, new_line))
        .all(|hop| quiet_hop(diagram, hop))
}

fn change_mark(diagram: &Diagram, station: &Station) -> Option<String> {
    if diagram.kind != DiagramKind::Change {
        return None;
    }
    let lost: Vec<&str> = diagram
        .hops
        .iter()
        .filter(|hop| hop.from == station.id && hop.is_removed && !hop.stub)
        .map(|hop| hop.command.as_str())
        .collect();
    if lost.is_empty() {
        return None;
    }
    Some(format!("(changed: lost exit {})", lost.join(", ")))
}

fn quiet_hop(diagram: &Diagram, hop: &Hop) -> bool {
    diagram.kind == DiagramKind::Change && !hop.is_new && !hop.is_removed && !rule_changed(diagram, hop)
}

fn rule_changed(diagram: &Diagram, hop: &Hop) -> bool {
    diagram
        .change
        .as_ref()
        .is_some_and(|view| view.guards_changed.iter().any(|name| name == &hop.command))
}

fn rule_bit(_diagram: &Diagram, hop: &Hop, rules: &mut Vec<(String, PrettyRule)>) -> String {
    if hop.guard_text.is_empty() {
        return String::new();
    }
    let args: BTreeSet<String> = hop.arg_names.iter().cloned().collect();
    match pretty::diagram_guard(&hop.guard_text, &args) {
        Some(rule) if rule.inline => format!("if: {}", rule.pretty),
        Some(rule) => {
            let id = format!("r{}", rules.len() + 1);
            rules.push((id.clone(), rule));
            format!("if: [{id}]")
        }
        None => String::new(),
    }
}

pub(super) fn command_args(diagram: &Diagram, command: &str) -> BTreeSet<String> {
    diagram
        .hops
        .iter()
        .find(|hop| hop.command == command)
        .map(|hop| hop.arg_names.iter().cloned().collect())
        .unwrap_or_default()
}

fn pair(left: &str, right: &str) -> String {
    let right = right.trim();
    if right.is_empty() {
        return left.trim_end().to_string();
    }
    let width = display_width(left);
    if width >= RIGHT {
        format!("{left}  {right}")
    } else {
        format!("{left}{}{right}", " ".repeat(RIGHT - width))
    }
}

fn rules_block(rules: &[(String, PrettyRule)]) -> Vec<String> {
    if rules.is_empty() {
        return Vec::new();
    }
    let mut lines = vec!["Rules:".into()];
    for (id, rule) in rules {
        lines.push(format!("{id}  {}", rule.pretty));
        lines.push(format!("    verbatim: {}", rule.verbatim));
    }
    lines
}

fn spine_ids(diagram: &Diagram, new_line: Option<bool>) -> Vec<String> {
    let Some(start) = diagram.stations.iter().find(|st| st.initial && version_station(st, new_line)) else {
        return diagram
            .stations
            .iter()
            .filter(|st| version_station(st, new_line))
            .map(|st| st.id.clone())
            .collect();
    };
    let (dist, parent) = bfs(diagram, &start.id, new_line);
    let goal = diagram
        .stations
        .iter()
        .filter(|st| st.terminal && version_station(st, new_line))
        .filter(|st| dist.contains_key(st.id.as_str()))
        .min_by_key(|st| (dist[&st.id], diagram.stations.iter().position(|s| s.id == st.id).unwrap_or(99)))
        .map(|st| st.id.clone());
    let Some(goal) = goal else {
        return vec![start.id.clone()];
    };
    let mut path = vec![goal.clone()];
    let mut cur = goal;
    while cur != start.id {
        let Some(prev) = parent.get(&cur) else {
            break;
        };
        path.push(prev.clone());
        cur = prev.clone();
    }
    path.reverse();
    path
}

fn bfs(
    diagram: &Diagram,
    start: &str,
    new_line: Option<bool>,
) -> (BTreeMap<String, u32>, BTreeMap<String, String>) {
    let mut dist = BTreeMap::from([(start.to_string(), 0u32)]);
    let mut parent = BTreeMap::new();
    let mut q = VecDeque::from([start.to_string()]);
    while let Some(from) = q.pop_front() {
        let here = dist[&from];
        for hop in &diagram.hops {
            if hop.from != from || hop.from == hop.to || hop.stub || !version_hop(hop, new_line) {
                continue;
            }
            if dist.contains_key(&hop.to) {
                continue;
            }
            dist.insert(hop.to.clone(), here + 1);
            parent.insert(hop.to.clone(), from.clone());
            q.push_back(hop.to.clone());
        }
    }
    (dist, parent)
}

fn version_hop(hop: &Hop, new_line: Option<bool>) -> bool {
    match new_line {
        Some(true) => !hop.is_removed,
        Some(false) => !hop.is_new,
        None => true,
    }
}

fn version_station(station: &Station, new_line: Option<bool>) -> bool {
    match new_line {
        Some(true) => !station.is_removed,
        Some(false) => !station.is_new,
        None => true,
    }
}

fn station<'a>(diagram: &'a Diagram, id: &str) -> Option<&'a Station> {
    diagram.stations.iter().find(|st| st.id == id)
}

fn hop_between<'a>(
    diagram: &'a Diagram,
    from: &str,
    to: &str,
    new_line: Option<bool>,
) -> Option<&'a Hop> {
    diagram.hops.iter().find(|hop| {
        hop.from == from && hop.to == to && hop.from != hop.to && version_hop(hop, new_line)
    })
}

fn risk_block(view: &ChangeView) -> Vec<String> {
    let rows = [
        (
            "Instances in changed states",
            cohort_value(view),
            Some("(structural / checked)"),
        ),
        (
            "Shared facts touched",
            fact_value(view),
            Some("(structural / checked)"),
        ),
        ("Dead ends / unreachable", dead_value(view), None),
        ("Shortest finish changed", finish_value(view), None),
        ("Existing checks / cases", case_value(view), None),
        ("Effects & invariants", effect_value(view), None),
        (
            "Live v1 count & callers",
            "unknown  (not scanned / NOT ESTABLISHED)".into(),
            None,
        ),
    ];
    let label_w = rows.iter().map(|(label, _, _)| label.len()).max().unwrap_or(0);
    let mut lines = vec![String::new()];
    if let Some(summary) = change_summary(view) {
        lines.push(summary);
        lines.push(String::new());
    }
    lines.push("WHAT ELSE IS AT RISK (basis / status)".into());
    for (label, value, trail) in rows {
        lines.push(risk_line(label, label_w, &value, trail));
    }
    lines
}

fn change_summary(view: &ChangeView) -> Option<String> {
    if view.added_commands.is_empty() && view.removed_commands.is_empty() {
        return None;
    }
    let mut bits = Vec::new();
    for name in &view.added_commands {
        bits.push(format!("+{name}"));
    }
    for name in &view.removed_commands {
        bits.push(format!("✖ {name}"));
    }
    Some(format!("CHANGE: {}", bits.join("  ")))
}

fn risk_line(label: &str, width: usize, value: &str, trail: Option<&str>) -> String {
    let body = format!("  • {label:<width$} : {value}");
    let Some(trail) = trail else {
        return body;
    };
    let need = display_width(trail);
    let have = display_width(&body);
    if have + 1 + need >= MAX_COLS {
        format!("{body} {trail}")
    } else {
        format!("{body}{}{trail}", " ".repeat(MAX_COLS - have - need))
    }
}

fn cohort_value(view: &ChangeView) -> String {
    let mut parts = Vec::new();
    for row in &view.cohorts {
        if let Some(part) = cohort_part(view, row) {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join("; ")
    }
}

fn cohort_part(view: &ChangeView, row: &super::CohortRow) -> Option<String> {
    let v1: Vec<String> = tokens(&row.existing)
        .into_iter()
        .filter(|name| !name.starts_with('✗') && name != "⊘")
        .collect();
    let v2_raw = tokens(&row.new_instances);
    let added: Vec<&String> = view
        .added_commands
        .iter()
        .filter(|name| v2_raw.iter().any(|tok| tok == *name))
        .collect();
    let lost: Vec<&String> = view
        .removed_commands
        .iter()
        .filter(|name| v1.iter().any(|tok| tok == *name))
        .collect();
    let rule = view
        .guards_changed
        .iter()
        .any(|name| v1.iter().any(|tok| tok == name) || v2_raw.iter().any(|tok| tok == name));
    if added.is_empty() && lost.is_empty() && !rule {
        return None;
    }
    let v2 = if v2_raw.is_empty() || row.new_instances == "⊘" {
        "none".into()
    } else if !added.is_empty() {
        added
            .iter()
            .map(|name| format!("+{name}"))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        v2_raw.join(", ")
    };
    let v1_text = if v1.is_empty() {
        "none".into()
    } else {
        v1.join(", ")
    };
    Some(format!(
        "{} (v1: {v1_text} | v2: {v2})",
        row.states.join(", ")
    ))
}

fn tokens(cell: &str) -> Vec<String> {
    cell.split_whitespace()
        .filter(|tok| *tok != "⊘" && *tok != "↻")
        .map(str::to_string)
        .collect()
}

fn fact_value(view: &ChangeView) -> String {
    let mut by_fact: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in &view.risk {
        if !(row.impact.contains("written") || row.impact.contains("read")) {
            continue;
        }
        by_fact
            .entry(row.target.clone())
            .or_default()
            .insert(row.impact.clone());
    }
    if by_fact.is_empty() {
        return "none".into();
    }
    by_fact
        .into_iter()
        .map(|(fact, impacts)| format!("{fact} ({})", merge_impacts(&impacts)))
        .collect::<Vec<_>>()
        .join("; ")
}

fn merge_impacts(impacts: &BTreeSet<String>) -> String {
    let writers: Vec<&str> = impacts
        .iter()
        .filter_map(|text| text.strip_prefix("written by "))
        .collect();
    let readers: Vec<&str> = impacts
        .iter()
        .filter_map(|text| text.strip_prefix("read by "))
        .map(|text| text.trim_end_matches(" guard"))
        .collect();
    if writers.len() == 1 && readers.len() == 1 && writers[0] == readers[0] {
        return format!("written & read by {}", writers[0]);
    }
    impacts.iter().cloned().collect::<Vec<_>>().join("; ")
}

fn dead_value(view: &ChangeView) -> String {
    if view.new_dead_ends.is_empty() {
        "none".into()
    } else {
        view.new_dead_ends
            .iter()
            .map(|name| format!("{name} (new)"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn finish_value(view: &ChangeView) -> String {
    if view.finish_changed.is_empty() {
        "none".into()
    } else {
        view.finish_changed.join(", ")
    }
}

fn case_value(view: &ChangeView) -> String {
    let bits: Vec<String> = view
        .risk
        .iter()
        .filter(|row| row.impact.contains("names a changed"))
        .map(|row| row.target.clone())
        .collect();
    if bits.is_empty() {
        "none reference modified or removed commands".into()
    } else {
        bits.join(", ")
    }
}

fn effect_value(view: &ChangeView) -> String {
    if view.no_change.len() >= 2 {
        "no changes".into()
    } else if view.no_change.is_empty() {
        "see ledger".into()
    } else {
        format!("no change to {}", view.no_change.join(", "))
    }
}

fn recovery_map(diagram: &Diagram) -> String {
    let Some(view) = &diagram.recovery else {
        return String::from("(no evidenced steps)\n");
    };
    let mut lines = recovery_river(diagram, view);
    if view.resolved_outcome.is_some() {
        lines.push("Established: the earlier operation committed; this lookup did not commit again".into());
        lines.push("Uncertain: whether any handle from this lookup is current".into());
    } else {
        lines.extend(outcome_table(view));
    }
    if let Some(end) = action_line(diagram) {
        lines.push(end);
    }
    join_lines(lines, diagram.ascii)
}

fn recovery_river(diagram: &Diagram, view: &RecoveryView) -> Vec<String> {
    let from = diagram
        .stations
        .iter()
        .find(|st| st.initial)
        .map(|st| st.id.as_str())
        .unwrap_or("idle");
    let to = diagram
        .stations
        .iter()
        .find(|st| st.terminal)
        .map(|st| st.id.as_str())
        .unwrap_or("tagged");
    let here = here_if(diagram, from);
    let mut lines = vec![pair(&format!("  ● [ {from} ] ══▶ ⟪"), &here)];
    lines.extend(shaft(&view.command, ""));
    if view.timeline.is_empty() {
        lines.push("  (no recovery events in the evidence)".into());
        lines.extend(shaft("", ""));
    }
    for (index, step) in view.timeline.iter().enumerate() {
        lines.extend(timeline_lines(view, step, index));
    }
    lines.push(format!("  ⟫ ══▶ ✔ [ {to} ]  END"));
    lines
}

fn timeline_lines(view: &RecoveryView, step: &TimelineStep, index: usize) -> Vec<String> {
    const NUMBERED: [&str; 5] = ["①", "②", "③", "④", "⑤"];
    let glyph = NUMBERED.get(index).copied().unwrap_or("·");
    let mark = if step.id == "lost" { "✂" } else { "○" };
    let right = if view.you_are_here.as_deref() == Some(step.id) {
        format!("{}  ◀── YOU ARE HERE", step.how)
    } else {
        step.how.to_string()
    };
    let mut lines = vec![pair(&format!("  {mark} [ {glyph} {} ]", step.label), &right)];
    lines.extend(shaft("", ""));
    lines
}

fn here_if(diagram: &Diagram, id: &str) -> String {
    if diagram.you_are_here.as_deref() == Some(id) {
        "◀── YOU ARE HERE".into()
    } else {
        String::new()
    }
}

fn shaft(command: &str, rule: &str) -> Vec<String> {
    if command.is_empty() {
        vec!["      │".into(), "      ▼".into()]
    } else {
        vec![
            "      │".into(),
            pair(&format!("      │  {command}"), rule),
            "      ▼".into(),
        ]
    }
}

fn outcome_table(view: &RecoveryView) -> Vec<String> {
    if let Some((_, shared)) = shared_resolution(&view.fork) {
        return vec![format!("all outcomes: {shared}")];
    }
    view.fork
        .iter()
        .map(|(outcome, resolution)| format!("{outcome}: {resolution}"))
        .collect()
}

fn shared_resolution(fork: &[(String, String)]) -> Option<(String, String)> {
    let first = fork.first()?;
    if fork.iter().all(|row| row.1 == first.1) {
        let names = fork
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Some((names, first.1.clone()))
    } else {
        None
    }
}

fn action_line(diagram: &Diagram) -> Option<String> {
    diagram
        .safe_next
        .as_ref()
        .map(|text| format!("Safe next action: {text}"))
}

fn region_maps(diagram: &Diagram, view: &RegionView) -> String {
    let mut rules = Vec::new();
    let mut lines = match view.lanes.len() {
        0 => Vec::new(),
        1 => {
            let mut one = region_one(diagram, &view.lanes[0], &mut rules);
            one.extend(joint_bridge(view, 0));
            one
        }
        2 => region_l2(diagram, view, &mut rules),
        _ => {
            // stacked L1 when 3 or more lanes; two lanes stay side-by-side
            let mut stacked = region_l1(diagram, view, &mut rules);
            stacked.extend(joint_bridge(view, 0));
            stacked
        }
    };
    lines.extend(rules_block(&rules));
    join_lines(lines, diagram.ascii)
}

fn region_one(
    diagram: &Diagram,
    lane: &RegionLane,
    rules: &mut Vec<(String, PrettyRule)>,
) -> Vec<String> {
    box_wrap(&lane.name, 72, &lane_river(diagram, lane, rules, true))
}

fn region_l1(
    diagram: &Diagram,
    view: &RegionView,
    rules: &mut Vec<(String, PrettyRule)>,
) -> Vec<String> {
    let mut lines = Vec::new();
    for lane in &view.lanes {
        lines.extend(box_wrap(&lane.name, BOX, &lane_river(diagram, lane, rules, true)));
    }
    lines
}

fn region_l2(
    diagram: &Diagram,
    view: &RegionView,
    rules: &mut Vec<(String, PrettyRule)>,
) -> Vec<String> {
    let left = box_wrap(
        &view.lanes[0].name,
        BOX,
        &lane_river(diagram, &view.lanes[0], rules, false),
    );
    let right = box_wrap(
        &view.lanes[1].name,
        BOX,
        &lane_river(diagram, &view.lanes[1], rules, false),
    );
    let mut lines = zip_box(&left, &right);
    if let Some(last) = lines.last_mut() {
        *last = tee_pair(last, BOX);
    }
    lines.extend(joint_bridge(view, BOX));
    lines
}

fn tee_pair(line: &str, width: usize) -> String {
    let mut chars: Vec<char> = line.chars().collect();
    let left_mid = width / 2;
    let right_mid = width + 1 + width / 2;
    for idx in [left_mid, right_mid] {
        if idx < chars.len() {
            chars[idx] = '┬';
        }
    }
    chars.into_iter().collect()
}

fn joint_bridge(view: &RegionView, box_w: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let t1 = if box_w == 0 { 0 } else { box_w / 2 };
    let t2 = if box_w == 0 {
        0
    } else {
        box_w + 1 + box_w / 2
    };
    let span = if box_w == 0 {
        BOX
    } else {
        t2.saturating_sub(t1).saturating_add(1)
    };
    let name_w = view
        .joints
        .iter()
        .flat_map(|j| j.legs.iter().map(|(r, _, _)| r.len()))
        .max()
        .unwrap_or(0);
    for joint in &view.joints {
        let label = format!(" joint: {} ", joint.command);
        let inner = display_width(&label);
        let fill = span.saturating_sub(inner + 2);
        let left_fill = fill / 2;
        let right_fill = fill - left_fill;
        let bridge = format!(
            "╘{}{}{}╛",
            "═".repeat(left_fill),
            label,
            "═".repeat(right_fill)
        );
        lines.push(format!("{}{bridge}", " ".repeat(t1)));
        for (region, from, to) in &joint.legs {
            lines.push(format!(
                "    {region:<name_w$}: [ {from} ] → [ {to} ]"
            ));
        }
    }
    lines
}

fn lane_river(
    diagram: &Diagram,
    lane: &RegionLane,
    rules: &mut Vec<(String, PrettyRule)>,
    compound: bool,
) -> Vec<String> {
    if compound && !lane.children.is_empty() {
        return compound_river(diagram, lane, rules);
    }
    let mut lines = Vec::new();
    let spine = region_spine(lane);
    for (index, state) in spine.iter().enumerate() {
        let next = spine.get(index + 1).map(String::as_str);
        let prev = index.checked_sub(1).and_then(|i| spine.get(i)).map(String::as_str);
        let backs = back_note(lane, state, next, prev);
        lines.push(lane_state_line(lane, state, index));
        if !backs.is_empty() {
            lines.push(format!("    {backs}"));
        }
        if let Some(to) = next {
            if let Some(hop) = lane.hops.iter().find(|h| h.from == *state && h.to == to) {
                let back = lane
                    .hops
                    .iter()
                    .find(|h| h.from == to && h.to == *state)
                    .map(|h| h.command.as_str());
                lines.extend(hop_shaft(diagram, hop, back, rules));
            }
        }
    }
    lines
}

fn lane_state_line(lane: &RegionLane, state: &str, index: usize) -> String {
    let terminal = lane.terminals.iter().any(|name| name == state);
    let glyph = if terminal {
        "✔"
    } else if index == 0 {
        "●"
    } else {
        "○"
    };
    if terminal {
        format!("  {glyph} [ {state} ]  END")
    } else {
        format!("  {glyph} [ {state} ]")
    }
}

fn hop_shaft(
    diagram: &Diagram,
    hop: &RegionHop,
    back: Option<&str>,
    rules: &mut Vec<(String, PrettyRule)>,
) -> Vec<String> {
    let args = command_args(diagram, &hop.command);
    let extra = pretty::diagram_guard(&hop.guard, &args).map(|rule| {
        if rule.inline {
            format!("if: {}", rule.pretty)
        } else {
            let id = format!("r{}", rules.len() + 1);
            rules.push((id.clone(), rule));
            format!("if: [{id}]")
        }
    });
    let mut lines = vec!["      │".into()];
    if let Some(back) = back {
        lines.push(format!("      │  {} ▼    ▲ {back}", hop.command));
        return lines;
    }
    match extra {
        None => lines.push(format!("      │  {}", hop.command)),
        Some(rule) => {
            let same = format!("      │  {}  {rule}", hop.command);
            if display_width(&same) <= BOX - 2 {
                lines.push(same);
            } else {
                lines.push(format!("      │  {}", hop.command));
                lines.push(format!("         {rule}"));
            }
        }
    }
    lines.push("      ▼".into());
    lines
}

fn region_spine(lane: &RegionLane) -> Vec<String> {
    let nodes: Vec<String> = lane
        .states
        .iter()
        .filter(|state| !lane.parent_hops.iter().any(|hop| hop.to == **state))
        .cloned()
        .collect();
    let edges: Vec<crate::diagram::DrawnEdge> = lane
        .hops
        .iter()
        .filter(|hop| nodes.iter().any(|n| n == &hop.from) && nodes.iter().any(|n| n == &hop.to))
        .map(|hop| crate::diagram::DrawnEdge {
            command: hop.command.clone(),
            from: hop.from.clone(),
            to: hop.to.clone(),
        })
        .collect();
    let initial = nodes.first().cloned();
    crate::diagram::layout_graph(nodes, edges, initial.as_deref()).spine
}

fn compound_spine(lane: &RegionLane) -> Vec<String> {
    let nodes = lane.children.clone();
    let edges: Vec<crate::diagram::DrawnEdge> = lane
        .hops
        .iter()
        .filter(|hop| nodes.iter().any(|n| n == &hop.from) && nodes.iter().any(|n| n == &hop.to))
        .map(|hop| crate::diagram::DrawnEdge {
            command: hop.command.clone(),
            from: hop.from.clone(),
            to: hop.to.clone(),
        })
        .collect();
    let initial = nodes.first().cloned();
    let spine = crate::diagram::layout_graph(nodes.clone(), edges, initial.as_deref()).spine;
    if spine.is_empty() {
        nodes
    } else {
        spine
    }
}

fn back_note(lane: &RegionLane, state: &str, next: Option<&str>, prev: Option<&str>) -> String {
    let hops: Vec<&RegionHop> = lane
        .hops
        .iter()
        .filter(|hop| hop.from == *state && next != Some(hop.to.as_str()) && prev != Some(hop.to.as_str()))
        .collect();
    if hops.is_empty() {
        return String::new();
    }
    let bits: Vec<String> = hops
        .iter()
        .map(|hop| {
            if hop.from == hop.to {
                format!("↺ {}", hop.command)
            } else {
                format!("↺ {} → [ {} ]", hop.command, hop.to)
            }
        })
        .collect();
    format!("  ({})", bits.join(", "))
}

fn compound_river(
    diagram: &Diagram,
    lane: &RegionLane,
    rules: &mut Vec<(String, PrettyRule)>,
) -> Vec<String> {
    let parent = lane.parent_name.as_deref().unwrap_or("parent");
    let (body, gutter) = super::rails::add_return_rails(compound_child_body(diagram, lane, rules));
    let mut lines = dashed_group(parent, &body, 3 + gutter, 2);
    lines.extend(compound_parent_exit(lane, parent, gutter));
    lines.extend(compound_outside(lane, parent));
    lines
}

fn compound_child_body(
    diagram: &Diagram,
    lane: &RegionLane,
    rules: &mut Vec<(String, PrettyRule)>,
) -> Vec<String> {
    let mut body = Vec::new();
    let children = compound_spine(lane);
    for (index, state) in children.iter().enumerate() {
        let next = children.get(index + 1).map(String::as_str);
        let glyph = if index == 0 { "●" } else { "○" };
        body.push(format!("  {glyph} [ {state} ]"));
        body.extend(override_lines(diagram, lane, state, next, rules));
        body.extend(compound_forward_hop(lane, state, next));
        body.extend(compound_side_hops(lane, state, next));
    }
    body
}

fn compound_forward_hop(lane: &RegionLane, state: &str, next: Option<&str>) -> Vec<String> {
    let Some(to) = next else {
        return Vec::new();
    };
    let Some(hop) = lane.hops.iter().find(|h| h.from == state && h.to == to) else {
        return Vec::new();
    };
    vec![
        "   │".into(),
        format!("   │  {}", hop.command),
        "   ▼".into(),
    ]
}

fn compound_side_hops(lane: &RegionLane, state: &str, next: Option<&str>) -> Vec<String> {
    lane.hops
        .iter()
        .filter(|hop| hop.from == state && next != Some(hop.to.as_str()))
        .map(|hop| format!("   ↺ {} → [ {} ]", hop.command, hop.to))
        .collect()
}

fn compound_parent_exit(lane: &RegionLane, parent: &str, shift: usize) -> Vec<String> {
    let mut lines = Vec::new();
    if lane.hops.iter().any(|hop| !hop.guard.is_empty()) {
        lines.push(
            "  child override is tried first; if its condition is false the group exit applies "
                .into(),
        );
    }
    let Some(hop) = lane.parent_hops.first() else {
        return lines;
    };
    let pad = " ".repeat(6 + shift);
    lines.push(format!("{pad}│ {} (from any state in {parent})", hop.command));
    lines.push(format!("{pad}▼"));
    lines.push(outside_state_line(lane, &hop.to));
    lines
}

fn compound_outside(lane: &RegionLane, parent: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for state in &lane.states {
        if lane.children.iter().any(|child| child == state) {
            continue;
        }
        lines.extend(outside_state_block(lane, parent, state));
    }
    lines
}

fn outside_state_block(lane: &RegionLane, parent: &str, state: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let is_parent_dest = lane.parent_hops.iter().any(|hop| hop.to == *state);
    if !is_parent_dest {
        lines.push(outside_state_line(lane, state));
    }
    for hop in lane.hops.iter().filter(|hop| hop.from == *state) {
        lines.push(reentry_line(parent, lane, hop));
    }
    lines
}

fn outside_state_line(lane: &RegionLane, state: &str) -> String {
    if lane.terminals.iter().any(|name| name == state) {
        format!("  ✔ [ {state} ]  END")
    } else {
        format!("  ○ [ {state} ]")
    }
}

fn reentry_line(parent: &str, lane: &RegionLane, hop: &super::RegionHop) -> String {
    let enter = lane.children.first().cloned().unwrap_or_default();
    if hop.to == parent || lane.children.iter().any(|child| child == &hop.to) {
        format!(
            "      ↗ {} → [ {parent} ]  (enters {enter})",
            hop.command
        )
    } else {
        format!("      ↺ {} → [ {} ]", hop.command, hop.to)
    }
}

fn dashed_group(title: &str, body: &[String], spine: usize, indent: usize) -> Vec<String> {
    let pad = " ".repeat(indent);
    let title_mid = format!("┄ {title} (group) ");
    let inner = body
        .iter()
        .map(|row| display_width(row))
        .max()
        .unwrap_or(0)
        .max(display_width(&title_mid))
        .max(spine + 2)
        + 2;
    let fill = inner.saturating_sub(display_width(&title_mid));
    let mut lines = vec![format!("{pad}╭{title_mid}{}╮", "┄".repeat(fill))];
    for row in body {
        let pad_right = inner.saturating_sub(display_width(row));
        lines.push(format!("{pad}┆{row}{}┆", " ".repeat(pad_right)));
    }
    let mut bottom = pad;
    bottom.push('╰');
    for col in 0..inner {
        if col == spine {
            bottom.push('┼');
        } else {
            bottom.push('┄');
        }
    }
    bottom.push('╯');
    lines.push(bottom);
    lines
}

fn override_lines(
    diagram: &Diagram,
    lane: &RegionLane,
    state: &str,
    next: Option<&str>,
    rules: &mut Vec<(String, PrettyRule)>,
) -> Vec<String> {
    let Some(hop) = lane
        .hops
        .iter()
        .find(|hop| hop.from == state && next != Some(hop.to.as_str()) && !hop.guard.is_empty())
    else {
        return Vec::new();
    };
    let args = command_args(diagram, &hop.command);
    let extra = pretty::diagram_guard(&hop.guard, &args)
        .map(|rule| {
            if rule.inline {
                format!("(if: {})", rule.pretty)
            } else {
                let id = format!("r{}", rules.len() + 1);
                rules.push((id.clone(), rule));
                format!("(if: [{id}])")
            }
        })
        .unwrap_or_default();
    let mut lines = vec![format!(" ↗ override: {} → [ {} ]", hop.command, hop.to)];
    if extra.is_empty() {
        return lines;
    }
    let rule = format!("   {extra}");
    if display_width(&rule) <= 60 {
        lines.push(rule);
    } else {
        lines.extend(wrap_width(&rule, 60));
    }
    lines
}

fn box_wrap(title: &str, width: usize, body: &[String]) -> Vec<String> {
    let inner = body
        .iter()
        .map(|row| display_width(row))
        .max()
        .unwrap_or(0)
        .max(display_width(title) + 4);
    let width = width.max(inner + 2);
    let label = format!("┌─ {title} ");
    let fill = width.saturating_sub(display_width(&label) + 1);
    let mut lines = vec![format!("{label}{}┐", "─".repeat(fill))];
    for row in body {
        let pad = width.saturating_sub(2 + display_width(row));
        lines.push(format!("│{row}{}│", " ".repeat(pad)));
    }
    lines.push(format!("└{}┘", "─".repeat(width.saturating_sub(2))));
    lines
}

fn zip_box(left: &[String], right: &[String]) -> Vec<String> {
    let height = left.len().max(right.len());
    let mut lines = Vec::new();
    for row in 0..height {
        let a = left.get(row).cloned().unwrap_or_else(|| " ".repeat(BOX));
        let b = right.get(row).cloned().unwrap_or_else(|| " ".repeat(BOX));
        lines.push(format!("{a} {b}"));
    }
    lines
}

fn join_lines(lines: Vec<String>, ascii: bool) -> String {
    let wrapped: Vec<String> = lines
        .into_iter()
        .flat_map(|line| wrap_width(&line, MAX_COLS))
        .collect();
    let mut body = wrapped.join("\n");
    if ascii {
        body = ascii_fallback(&body);
    }
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body
}

fn wrap_width(text: &str, width: usize) -> Vec<String> {
    if display_width(text) <= width {
        return vec![text.to_string()];
    }
    for sep in [" | ", "; ", " · ", " "] {
        if text.contains(sep) {
            return wrap_sep(text, sep, width);
        }
    }
    vec![text.to_string()]
}

fn wrap_sep(text: &str, sep: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for part in text.split(sep) {
        let next = if current.is_empty() {
            part.to_string()
        } else {
            format!("{current}{sep}{part}")
        };
        if display_width(&next) > width && !current.is_empty() {
            lines.push(current);
            current = part.to_string();
        } else {
            current = next;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn ascii_fallback(text: &str) -> String {
    text.replace('─', "-")
        .replace('━', "=")
        .replace('═', "=")
        .replace('┄', ".")
        .replace('▶', ">")
        .replace('◀', "<")
        .replace('●', "*")
        .replace('○', "o")
        .replace('✔', "Y")
        .replace('↺', "@")
        .replace('✂', "X")
        .replace('⟪', "<")
        .replace('⟫', ">")
        .replace('╭', "+")
        .replace('╰', "+")
        .replace('✚', "+")
        .replace('↗', "^")
        .replace('┬', "+")
        .replace('┼', "+")
        .replace('┆', "|")
        .replace('╘', "+")
        .replace('╛', "+")
        .replace('①', "1")
        .replace('②', "2")
        .replace('③', "3")
        .replace('④', "4")
        .replace('⑤', "5")
        .replace('·', ".")
}
