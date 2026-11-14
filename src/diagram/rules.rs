//! Rule ledger: one entry per command that carries an authored guard. It states what the
//! definition declares and binds each rule to a hash of its guards and the moves they
//! apply to; whether a rule matches the business intent is never decided here.
//!
//! A hierarchical machine is read from its authored regions and joints, not from the
//! flat lowering, whose guards are state-encoding conjuncts the author never wrote.

use super::{Diagram, RuleEntry};
use crate::canonical::digest_json;
use crate::declarative::Definition;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

/// The guarded moves of one command.
#[derive(Clone, Default, PartialEq, Eq)]
struct Rule {
    guards: Vec<String>,
    moves: Vec<String>,
}

type Ledger = BTreeMap<String, Rule>;

pub(super) fn entries(
    diagram: &Diagram,
    def: &Definition,
    prior: Option<&Definition>,
) -> Vec<RuleEntry> {
    let now = ledger_of(def);
    let before = prior.map(ledger_of);
    let mut out: Vec<RuleEntry> = now
        .iter()
        .map(|(name, rule)| {
            let status = status_of(name, rule, before.as_ref());
            entry(diagram, name, rule, status)
        })
        .collect();
    if let Some(old) = &before {
        out.extend(dropped_guards(diagram, def, old, &now));
    }
    out
}

fn entry(diagram: &Diagram, name: &str, rule: &Rule, status: &'static str) -> RuleEntry {
    let args = super::route::command_args(diagram, name);
    let (pretty, verbatim) = describe(&rule.guards, &args);
    RuleEntry {
        name: name.to_string(),
        moves: rule.moves.join(", "),
        rule: pretty,
        verbatim,
        guards: rule.guards.clone(),
        hash: digest_json(&json!({
            "command": name,
            "guards": rule.guards,
            "moves": rule.moves,
        })),
        status,
    }
}

/// A command that still exists but lost its guard is a loosened rule.
fn dropped_guards(
    diagram: &Diagram,
    def: &Definition,
    old: &Ledger,
    now: &Ledger,
) -> Vec<RuleEntry> {
    old.iter()
        .filter(|(name, _)| def.commands.contains_key(*name) && !now.contains_key(*name))
        .map(|(name, was)| {
            let mut item = entry(diagram, name, &Rule::default(), "removed");
            item.rule = format!("no rule now (was: {})", describe(&was.guards, &BTreeSet::new()).0);
            item.moves = was.moves.join(", ");
            item
        })
        .collect()
}

fn status_of(name: &str, rule: &Rule, before: Option<&Ledger>) -> &'static str {
    let Some(map) = before else {
        return "authored";
    };
    match map.get(name) {
        None => "new",
        Some(old) if old == rule => "unchanged",
        Some(_) => "changed",
    }
}

fn describe(guards: &[String], args: &BTreeSet<String>) -> (String, String) {
    let mut pretty = Vec::with_capacity(guards.len());
    let mut verbatim = Vec::with_capacity(guards.len());
    for guard in guards {
        match super::pretty::pretty_guard(guard, args) {
            Some(rule) => {
                pretty.push(rule.pretty);
                verbatim.push(rule.verbatim);
            }
            None => {
                pretty.push(guard.clone());
                verbatim.push(guard.clone());
            }
        }
    }
    (pretty.join(" ; "), verbatim.join(" ; "))
}

/// Every authored guarded move as `(command, guard, move)`.
fn guarded_moves(def: &Definition) -> Vec<(String, String, String)> {
    match super::regions::from_def(def) {
        Some(view) => region_moves(&view),
        None => flat_moves(def),
    }
}

fn flat_moves(def: &Definition) -> Vec<(String, String, String)> {
    let graph = super::analysis::graph_of(def);
    graph
        .edges
        .iter()
        .filter_map(|edge| {
            let guard = edge.guard.as_deref().map(str::trim).unwrap_or("");
            (!guard.is_empty()).then(|| {
                (
                    edge.command.clone(),
                    guard.to_string(),
                    format!("{} → {}", edge.source, edge.target),
                )
            })
        })
        .collect()
}

fn region_moves(view: &super::RegionView) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for lane in &view.lanes {
        for hop in lane.hops.iter().chain(lane.parent_hops.iter()) {
            if !hop.guard.trim().is_empty() {
                out.push((
                    hop.command.clone(),
                    hop.guard.trim().to_string(),
                    format!("{}: {} → {}", lane.name, hop.from, hop.to),
                ));
            }
        }
    }
    for joint in &view.joints {
        if !joint.guard.trim().is_empty() {
            out.push((
                joint.command.clone(),
                joint.guard.trim().to_string(),
                format!("joint over {}", joint.changes.join(", ")),
            ));
        }
    }
    out
}

fn ledger_of(def: &Definition) -> Ledger {
    let mut guards: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    for (command, guard, step) in guarded_moves(def) {
        let slot = guards.entry(command).or_default();
        slot.0.insert(guard);
        slot.1.insert(step);
    }
    guards
        .into_iter()
        .map(|(name, (gs, ms))| {
            (
                name,
                Rule {
                    guards: gs.into_iter().collect(),
                    moves: ms.into_iter().collect(),
                },
            )
        })
        .collect()
}
