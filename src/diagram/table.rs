//! Adjacent plain-text table. The diagram compresses this table.

use super::mdtable::md_table;
use super::{Diagram, DiagramKind};
use std::collections::BTreeSet;

const RULE_GLOSS: &str = "Rules: given = supplied by the caller, stored = kept by the machine.";

pub(super) fn render(diagram: &Diagram) -> String {
    if diagram.kind == DiagramKind::Recovery {
        return recovery_table(diagram);
    }
    if diagram.rows.iter().any(|row| row.class == "region" || row.class == "joint") {
        return region_table(diagram);
    }
    let mut out = command_table(diagram);
    out.push_str(rules_table(diagram).as_str());
    out
}

fn command_table(diagram: &Diagram) -> String {
    let mut rows: Vec<Vec<String>> = diagram
        .rows
        .iter()
        .map(|row| {
            vec![
                row.command.clone(),
                row.from.clone(),
                row.to.clone(),
                row.guard.clone(),
                row.effect.clone(),
                row.class.clone(),
            ]
        })
        .collect();
    for station in diagram.stations.iter().filter(|station| station.collapsed) {
        rows.push(vec![
            "(collapsed)".into(),
            station.collapsed_names.join(", "),
        ]);
    }
    md_table(&["command", "from", "to", "guard", "effect", "class"], &rows)
}

fn rules_table(diagram: &Diagram) -> String {
    let mut rows = Vec::new();
    for row in &diagram.rows {
        if row.guard.trim().is_empty() {
            continue;
        }
        let args = super::route::command_args(diagram, &row.command);
        if let Some(rule) = super::pretty::pretty_guard(&row.guard, &args) {
            rows.push(vec![row.command.clone(), rule.pretty, rule.verbatim]);
        }
    }
    if rows.is_empty() {
        return String::new();
    }
    let mut out = format!("\n{RULE_GLOSS}\n\n");
    out.push_str(&md_table(&["name", "rule", "verbatim"], &rows));
    out
}

fn region_table(diagram: &Diagram) -> String {
    let regions: Vec<Vec<String>> = diagram
        .rows
        .iter()
        .filter(|row| row.class == "region")
        .map(|row| {
            vec![
                row.effect.clone(),
                row.command.clone(),
                row.from.clone(),
                row.to.clone(),
                plain_rule(&row.guard),
            ]
        })
        .collect();
    let joints: Vec<Vec<String>> = diagram
        .rows
        .iter()
        .filter(|row| row.class == "joint")
        .map(|row| {
            vec![
                "joint".into(),
                row.command.clone(),
                row.from.clone(),
                row.guard.clone(),
            ]
        })
        .collect();
    let mut out = md_table(&["region", "command", "from", "to", "rule"], &regions);
    out.push('\n');
    out.push_str(&md_table(&["joint", "command", "changes", "rule"], &joints));
    out
}

fn recovery_table(diagram: &Diagram) -> String {
    let mut rows = Vec::new();
    if let Some(view) = &diagram.recovery {
        for step in &view.timeline {
            rows.push(vec![
                step.label.clone(),
                step.how.to_string(),
                step.note.clone(),
            ]);
        }
    }
    md_table(&["event", "how known", "note"], &rows)
}

fn plain_rule(guard: &str) -> String {
    if guard.trim().is_empty() {
        return String::new();
    }
    super::pretty::diagram_guard(guard, &BTreeSet::new())
        .map(|rule| rule.pretty)
        .unwrap_or_else(|| guard.to_string())
}
