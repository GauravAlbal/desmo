//! Change risk ledger. Only rows the definitions or evidence actually support.

use super::{ChangeView, CohortRow, RiskRow};
use crate::declarative::diff::{diff_machines, MachineDiff};
use crate::declarative::expr::{self, Expr, Operand};
use crate::declarative::explore::{Bounds, CaseCorpus};
use crate::declarative::Definition;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

pub fn attach_ledger(
    view: &mut ChangeView,
    def: &Definition,
    prior: Option<&Definition>,
    findings: &[Value],
    sidecar: Option<&Path>,
) {
    view.live_instances = "Live instance count: NOT ESTABLISHED (no store read)".into();
    view.callers = "Callers outside the definition: NOT ESTABLISHED".into();
    view.observed_note = observed_loans(findings);
    if let Some(old) = prior {
        view.rule_effect = rule_effect(old, def);
        view.risk = ledger_rows(def, old, view, sidecar);
        view.no_change = fold_unchanged(def, old);
        view.divergence = divergence_block(view, def, findings);
    }
}

fn observed_loans(findings: &[Value]) -> String {
    let mut ids = BTreeSet::new();
    for finding in findings {
        if finding["evidence_class"].as_str() != Some("continuity_observation") {
            continue;
        }
        if let Some(id) = finding["id"].as_str().and_then(aggregate_id) {
            ids.insert(id.to_string());
        }
        if let Some(id) = finding["aggregate"].as_str() {
            ids.insert(id.to_string());
        }
    }
    match ids.len() {
        0 => String::new(),
        1 => match ids.iter().next() {
            Some(id) => format!("one loan observed ({id})"),
            None => String::new(),
        },
        n => format!("{n} loans observed ({})", ids.into_iter().collect::<Vec<_>>().join(", ")),
    }
}

fn aggregate_id(id: &str) -> Option<&str> {
    let rest = id.rsplit_once('#')?.1;
    let (aggregate, _) = rest.split_once('/')?;
    if aggregate.is_empty() {
        None
    } else {
        Some(aggregate)
    }
}

fn rule_effect(old: &Definition, new: &Definition) -> String {
    let Ok(diff) = diff_machines(
        old,
        new,
        &CaseCorpus::default(),
        Bounds {
            max_nodes: 64,
            max_depth: 8,
        },
    ) else {
        return "rule change effect: needs confirming".into();
    };
    describe_witness(&diff)
}

fn describe_witness(diff: &MachineDiff) -> String {
    match diff.executable.dimension.as_deref() {
        Some("guard") | Some("guards") => "rule change has a differential witness".into(),
        Some(_) => "behavioral difference witnessed".into(),
        None if diff.structural.guards_changed.is_empty() => String::new(),
        None => "rule change effect: needs confirming".into(),
    }
}

fn ledger_rows(
    def: &Definition,
    old: &Definition,
    view: &ChangeView,
    sidecar: Option<&Path>,
) -> Vec<RiskRow> {
    let mut rows = Vec::new();
    changed_states(def, view, &mut rows);
    unreachable_ends(view, &mut rows);
    sidecar_cases(sidecar, view, &mut rows);
    fact_rows(def, view, &mut rows);
    effect_rows(def, old, &mut rows);
    invariant_rows(def, view, &mut rows);
    rows.push(RiskRow {
        target: "Live instance count".into(),
        impact: "unknown".into(),
        basis: "none".into(),
        status: "NOT ESTABLISHED".into(),
    });
    rows.push(RiskRow {
        target: "Callers outside the definition".into(),
        impact: "unknown".into(),
        basis: "none".into(),
        status: "NOT ESTABLISHED".into(),
    });
    rows
}

fn changed_states(def: &Definition, view: &ChangeView, rows: &mut Vec<RiskRow>) {
    let changed: BTreeSet<&str> = view
        .added_commands
        .iter()
        .chain(view.removed_commands.iter())
        .chain(view.guards_changed.iter())
        .map(String::as_str)
        .collect();
    for (name, cmd) in &def.commands {
        if !changed.contains(name.as_str()) {
            continue;
        }
        for from in &cmd.from {
            let impact = if view.added_commands.iter().any(|c| c == name) {
                format!("added command {name}")
            } else if view.removed_commands.iter().any(|c| c == name) {
                format!("removed command {name}")
            } else {
                format!("command {name} rule changed")
            };
            rows.push(RiskRow {
                target: format!("{from} / {name}"),
                impact,
                basis: "structural".into(),
                status: "checked".into(),
            });
        }
    }
}

fn unreachable_ends(view: &ChangeView, rows: &mut Vec<RiskRow>) {
    if view.old_every_reaches_end && !view.reachability_line.contains("not every") {
        return;
    }
    if view.reachability_line.contains("dead") || view.reachability_line.contains("not every") {
        rows.push(RiskRow {
            target: "finish reachability".into(),
            impact: view.reachability_line.clone(),
            basis: "structural".into(),
            status: "checked".into(),
        });
    }
}

fn sidecar_cases(sidecar: Option<&Path>, view: &ChangeView, rows: &mut Vec<RiskRow>) {
    let Some(dir) = sidecar else {
        return;
    };
    let names: BTreeSet<&str> = view
        .removed_commands
        .iter()
        .chain(view.guards_changed.iter())
        .map(String::as_str)
        .collect();
    if names.is_empty() {
        return;
    }
    for path in [
        dir.with_extension("ik.cases.json"),
        dir.with_extension("cases.json"),
        {
            let mut p = dir.to_path_buf();
            p.set_extension("checks.toml");
            p
        },
        dir.with_extension("ik.checks.toml"),
    ] {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for name in &names {
            if text.contains(name) {
                rows.push(RiskRow {
                    target: format!(
                        "{} in {}",
                        name,
                        path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "sidecar".into())
                    ),
                    impact: "names a changed command".into(),
                    basis: "structural".into(),
                    status: "needs confirming".into(),
                });
            }
        }
    }
}

fn fact_rows(def: &Definition, view: &ChangeView, rows: &mut Vec<RiskRow>) {
    let changed: BTreeSet<&str> = view
        .added_commands
        .iter()
        .chain(view.removed_commands.iter())
        .chain(view.guards_changed.iter())
        .map(String::as_str)
        .collect();
    for name in &changed {
        let Some(cmd) = def.commands.get(*name) else {
            continue;
        };
        if let Some(then) = &cmd.then {
            for fact in then.keys() {
                rows.push(RiskRow {
                    target: fact.clone(),
                    impact: format!("written by {name}"),
                    basis: "structural".into(),
                    status: "checked".into(),
                });
            }
        }
        if let Some(when) = &cmd.when {
            if let Ok(expr) = expr::parse(when) {
                for fact in facts_in(&expr) {
                    rows.push(RiskRow {
                        target: fact,
                        impact: format!("read by {name} guard"),
                        basis: "structural".into(),
                        status: "checked".into(),
                    });
                }
            }
        }
    }
    blast_rows(def, view, rows);
}

fn blast_rows(def: &Definition, view: &ChangeView, rows: &mut Vec<RiskRow>) {
    let mut written: BTreeSet<(String, String)> = BTreeSet::new();
    for name in &view.added_commands {
        let Some(cmd) = def.commands.get(name) else {
            continue;
        };
        if let Some(then) = &cmd.then {
            for fact in then.keys() {
                written.insert((name.clone(), fact.clone()));
            }
        }
    }
    for (writer, fact) in written {
        for (other, cmd) in &def.commands {
            if other == &writer {
                continue;
            }
            let reads_guard = cmd
                .when
                .as_ref()
                .and_then(|when| expr::parse(when).ok())
                .is_some_and(|expr| facts_in(&expr).contains(&fact));
            if reads_guard {
                rows.push(RiskRow {
                    target: fact.clone(),
                    impact: format!("written by {writer}, read by {other} guard"),
                    basis: "structural".into(),
                    status: "needs confirming".into(),
                });
            }
        }
        let inv_names: Vec<&str> = def
            .invariants
            .iter()
            .filter(|(_, inv)| {
                expr::parse(&inv.assert)
                    .ok()
                    .is_some_and(|expr| facts_in(&expr).contains(&fact))
            })
            .map(|(name, _)| name.as_str())
            .collect();
        if !inv_names.is_empty() {
            rows.push(RiskRow {
                target: fact.clone(),
                impact: format!(
                    "written by {writer}, read by invariant {}",
                    inv_names.join(", ")
                ),
                basis: "structural".into(),
                status: "needs confirming".into(),
            });
        }
    }
}

fn facts_in(expr: &Expr) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    walk_expr(expr, &mut out);
    out
}

fn walk_expr(expr: &Expr, out: &mut BTreeSet<String>) {
    match expr {
        Expr::Or(parts) | Expr::And(parts) => parts.iter().for_each(|p| walk_expr(p, out)),
        Expr::Cmp(cmp) => {
            walk_op(&cmp.left, out);
            walk_op(&cmp.right, out);
        }
        Expr::Exists(name) | Expr::NotExists(name) => {
            out.insert(name.clone());
        }
        Expr::TextIn(pred) => walk_op(&pred.value, out),
    }
}

fn walk_op(op: &Operand, out: &mut BTreeSet<String>) {
    match op {
        Operand::Fact(name) => {
            out.insert(name.clone());
        }
        Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => {
            walk_op(base, out);
        }
        Operand::BinArith { left, right, .. } => {
            walk_op(left, out);
            walk_op(right, out);
        }
        Operand::Arg(_) | Operand::State | Operand::Literal(_) => {}
    }
}

fn effect_rows(def: &Definition, old: &Definition, rows: &mut Vec<RiskRow>) {
    let new_fx: BTreeSet<&str> = def
        .commands
        .iter()
        .filter(|(_, c)| c.effect.is_some())
        .map(|(n, _)| n.as_str())
        .collect();
    let old_fx: BTreeSet<&str> = old
        .commands
        .iter()
        .filter(|(_, c)| c.effect.is_some())
        .map(|(n, _)| n.as_str())
        .collect();
    for name in new_fx.difference(&old_fx) {
        rows.push(RiskRow {
            target: (*name).into(),
            impact: "effect added".into(),
            basis: "structural".into(),
            status: "checked".into(),
        });
    }
    for name in old_fx.difference(&new_fx) {
        rows.push(RiskRow {
            target: (*name).into(),
            impact: "effect removed".into(),
            basis: "structural".into(),
            status: "checked".into(),
        });
    }
}

fn invariant_rows(def: &Definition, view: &ChangeView, rows: &mut Vec<RiskRow>) {
    let written: BTreeSet<String> = view
        .added_commands
        .iter()
        .chain(view.guards_changed.iter())
        .filter_map(|name| def.commands.get(name))
        .filter_map(|cmd| cmd.then.as_ref())
        .flat_map(|then| then.keys().cloned())
        .collect();
    for (name, inv) in &def.invariants {
        let Ok(expr) = expr::parse(&inv.assert) else {
            continue;
        };
        for fact in facts_in(&expr) {
            if written.contains(&fact) {
                rows.push(RiskRow {
                    target: name.clone(),
                    impact: format!("invariant reads {fact}"),
                    basis: "structural".into(),
                    status: "needs confirming".into(),
                });
            }
        }
    }
}

fn fold_unchanged(def: &Definition, old: &Definition) -> Vec<String> {
    let mut quiet = Vec::new();
    let new_fx = def.commands.values().any(|c| c.effect.is_some());
    let old_fx = old.commands.values().any(|c| c.effect.is_some());
    if !new_fx && !old_fx {
        quiet.push("effects".into());
    }
    let same_invariants = def.invariants.len() == old.invariants.len()
        && def
            .invariants
            .iter()
            .all(|(name, inv)| old.invariants.get(name).is_some_and(|other| other.assert == inv.assert));
    if same_invariants {
        quiet.push("invariants".into());
    }
    quiet
}

fn divergence_block(view: &ChangeView, def: &Definition, findings: &[Value]) -> Vec<String> {
    let Some(row) = view.cohorts.iter().find(|row| has_split(row, view)) else {
        return Vec::new();
    };
    let state = row.states.first().cloned().unwrap_or_default();
    let mut lines = vec![format!("DIVERGENCE at {state}")];
    lines.push(format!(
        "New loans (v{}): {}",
        view.new_version,
        new_cmd_list(view, def, &row.new_instances)
    ));
    lines.push(format!(
        "Old loans (v{}): {}",
        view.prior_version,
        old_cmd_list(&row.existing)
    ));
    match refused_extend(findings) {
        Some((cmd, code)) => {
            lines.push(format!("{cmd} REFUSED ({code}), observed"));
        }
        None => {
            for cmd in &view.added_commands {
                lines.push(format!("{cmd}: refusal not observed"));
            }
        }
    }
    lines
}

fn new_cmd_list(view: &ChangeView, def: &Definition, cell: &str) -> String {
    let mut same = Vec::new();
    let mut added = Vec::new();
    for token in cell.split_whitespace() {
        if token == "⊘" {
            continue;
        }
        if view.added_commands.iter().any(|c| c == token) {
            let extra = def
                .commands
                .get(token)
                .and_then(|cmd| cmd.when.as_ref())
                .and_then(|when| super::pretty::pretty_guard(when, &args_of(def, token)))
                .map(|rule| format!(" (if: {})", rule.pretty))
                .unwrap_or_default();
            added.push(format!("+ {token}{extra}"));
        } else {
            same.push(token.to_string());
        }
    }
    same.extend(added);
    same.join(", ")
}

fn old_cmd_list(cell: &str) -> String {
    cell.split_whitespace()
        .filter(|token| !token.starts_with('✗') && *token != "⊘")
        .collect::<Vec<_>>()
        .join(", ")
}

fn args_of(def: &Definition, command: &str) -> BTreeSet<String> {
    def.commands
        .get(command)
        .and_then(|cmd| cmd.args.as_ref())
        .map(|args| args.keys().cloned().collect())
        .unwrap_or_default()
}

fn has_split(row: &CohortRow, view: &ChangeView) -> bool {
    if row.existing == row.new_instances {
        return false;
    }
    view.added_commands.iter().any(|c| row.new_instances.contains(c))
        || view.removed_commands.iter().any(|c| row.existing.contains(c))
}

fn refused_extend(findings: &[Value]) -> Option<(String, String)> {
    findings.iter().find_map(|finding| {
        if finding["status"].as_str() != Some("refused") {
            return None;
        }
        let cmd = finding["present"]["command"]
            .as_str()
            .or_else(|| finding["source_coordinate"]["command"].as_str())?;
        let code = finding["present"]["code"].as_str().unwrap_or("refusal");
        Some((cmd.to_string(), code.to_string()))
    })
}
