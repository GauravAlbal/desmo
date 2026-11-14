//! Gaps visible in a declarative definition: places where the definition itself is silent or
//! disconnected. Every item is derived from the definition alone, so the list is a closed-world
//! count over what is enumerated. It is never a claim that nothing else is missing: whether the
//! requirement says more than the definition does, and whether a state can remain occupied
//! forever, are not decided here.

use super::analyze::machine_graph;
use super::def::{CommandDef, Definition};
use super::expr::{self, Expr, Operand};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// One gap in a definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    /// Stable kind, for example `unguarded_argument`.
    pub kind: &'static str,
    /// What the gap is about: a command, argument, fact, state or `machine`.
    pub subject: String,
    /// One sentence saying what is and is not known.
    pub detail: String,
    /// A gap about what the machine accepts or does, not about documentation or evidence.
    pub material: bool,
}

/// True when `expr` reads the command argument `$name` as a whole token.
fn mentions_arg(expr: &str, name: &str) -> bool {
    let needle = format!("${name}");
    expr.match_indices(needle.as_str()).any(|(at, _)| {
        expr[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|next| !(next.is_alphanumeric() || next == '_'))
    })
}

/// Every guard text of a command: its own `when` and each outcome's.
fn command_guards(cmd: &CommandDef) -> Vec<&str> {
    cmd.when
        .iter()
        .map(String::as_str)
        .chain(cmd.outcomes.iter().flatten().filter_map(|outcome| outcome.when.as_deref()))
        .collect()
}

/// Declared command arguments that no guard (`when`, command or outcome) reads.
/// Advisory: such an argument is only checked by host code, so a direct runtime
/// command may carry any value of its declared type (for example an empty string).
pub fn unguarded_arguments(def: &Definition) -> Vec<Value> {
    let mut out = Vec::new();
    for (command, cmd) in &def.commands {
        let Some(args) = cmd.args.as_ref() else {
            continue;
        };
        let guards = command_guards(cmd);
        for (arg, ty) in args {
            if guards.iter().any(|guard| mentions_arg(guard, arg)) {
                continue;
            }
            out.push(json!({
                "command": command,
                "argument": arg,
                "type": ty,
                "advice": "no guard reads this argument; a rule on its value (non-empty, bounded, in a set, related to a fact) is enforced only by host code",
            }));
        }
    }
    out
}

fn operand_facts(operand: &Operand, out: &mut BTreeSet<String>) {
    match operand {
        Operand::Fact(name) => {
            out.insert(name.clone());
        }
        Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => operand_facts(base, out),
        Operand::BinArith { left, right, .. } => {
            operand_facts(left, out);
            operand_facts(right, out);
        }
        Operand::Arg(_) | Operand::State | Operand::Literal(_) => {}
    }
}

fn expr_facts(node: &Expr, out: &mut BTreeSet<String>) {
    match node {
        Expr::And(parts) | Expr::Or(parts) => parts.iter().for_each(|part| expr_facts(part, out)),
        Expr::Cmp(cmp) => {
            operand_facts(&cmp.left, out);
            operand_facts(&cmp.right, out);
        }
        Expr::Exists(name) | Expr::NotExists(name) => {
            out.insert(name.clone());
        }
        Expr::TextIn(text) => operand_facts(&text.value, out),
    }
}

/// Facts any guard or invariant reads. A text that does not parse reads nothing here; the
/// definition's own static check already refuses such a definition.
fn facts_read(def: &Definition) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let texts = def
        .commands
        .values()
        .flat_map(command_guards)
        .chain(def.invariants.values().flat_map(|inv| inv.when.iter().chain(std::iter::once(&inv.assert)).map(String::as_str)));
    for text in texts {
        if let Ok(parsed) = expr::parse(text) {
            expr_facts(&parsed, &mut out);
        }
    }
    out
}

/// Facts some command assigns on commit.
fn facts_written(def: &Definition) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for cmd in def.commands.values() {
        out.extend(cmd.then.iter().flat_map(|then| then.keys().cloned()));
        for outcome in cmd.outcomes.iter().flatten() {
            out.extend(outcome.then.iter().flat_map(|then| then.keys().cloned()));
        }
    }
    out
}

fn gap(kind: &'static str, subject: impl Into<String>, detail: impl Into<String>, material: bool) -> Gap {
    Gap { kind, subject: subject.into(), detail: detail.into(), material }
}

fn argument_gaps(def: &Definition) -> Vec<Gap> {
    unguarded_arguments(def)
        .iter()
        .map(|item| {
            let command = item["command"].as_str().unwrap_or("");
            let argument = item["argument"].as_str().unwrap_or("");
            gap(
                "unguarded_argument",
                format!("{command} ${argument}"),
                "no guard reads this argument, so any value of its type is accepted by the machine; a rule on it is enforced only by host code",
                true,
            )
        })
        .collect()
}

fn command_gaps(def: &Definition) -> Vec<Gap> {
    let mut out = Vec::new();
    for (name, cmd) in &def.commands {
        if command_guards(cmd).is_empty() {
            out.push(gap(
                "unguarded_command",
                name.as_str(),
                "no guard: it is accepted whenever the instance is in one of its source states",
                false,
            ));
        }
    }
    out
}

fn fact_gaps(def: &Definition) -> Vec<Gap> {
    let (read, written) = (facts_read(def), facts_written(def));
    let mut out = Vec::new();
    for (name, fact) in &def.facts {
        let set = fact.initial.is_some() || written.contains(name);
        if read.contains(name) && !set && fact.policy != "kernel_only" {
            out.push(gap(
                "fact_never_set",
                name.as_str(),
                "a guard or invariant reads it, but it has no initial value and no command assigns it, so it stays absent",
                true,
            ));
        }
        if set && !read.contains(name) {
            out.push(gap(
                "fact_never_read",
                name.as_str(),
                "it is stored, but no guard or invariant depends on it (host code may)",
                false,
            ));
        }
    }
    out
}

fn graph_gaps(def: &Definition) -> Vec<Gap> {
    let graph = machine_graph(def);
    let analysis = super::analyze::analyze(&graph);
    let mut out = Vec::new();
    for state in graph.states.iter().filter(|state| !state.terminal) {
        if !graph.edges.iter().any(|edge| edge.source == state.name) {
            out.push(gap(
                "dead_end",
                state.name.as_str(),
                "not an end state, and no command leaves it",
                true,
            ));
        }
    }
    for name in &analysis.unreachable_states {
        out.push(gap("unreachable_state", name.as_str(), "no sequence of commands reaches it (structural)", true));
    }
    out
}

/// All gaps of a flat declarative definition, material ones first, each group in name order.
pub fn definition_gaps(def: &Definition) -> Vec<Gap> {
    let mut out = argument_gaps(def);
    out.extend(command_gaps(def));
    out.extend(fact_gaps(def));
    out.extend(graph_gaps(def));
    if def.invariants.is_empty() {
        out.push(gap(
            "no_invariants",
            "machine",
            "no invariant is declared, so nothing is asserted to hold in every state",
            true,
        ));
    }
    out.sort_by_key(|item| !item.material);
    out
}
