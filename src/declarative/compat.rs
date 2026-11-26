//! Drain-and-recreate compatibility between two versions of a declarative machine.
//!
//! The kernel cannot move a running instance to another definition: an aggregate keeps
//! the identity it was created under. So the question is not "which states can swap" but
//! "if an instance is drained and recreated under the new version, does its history keep
//! working": do the version-1 exits survive, and does the new version read facts that a
//! version-1 instance in that state may never have set.
//!
//! These checks compare definitions. They find differences; they do not prove safety.
//! Stored values are not read, in-flight effects are not inspected, and hierarchical
//! machines are not analysed.

use super::def::{CommandDef, Definition};
use super::expr::{self, Expr, Operand};
use std::collections::{BTreeMap, BTreeSet};

/// What happens to one version-1 exit under the new version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitCompat {
    pub command: String,
    /// `same`, `guard changed`, `target changed` or `removed`.
    pub verdict: &'static str,
}

/// Compatibility of one version-1 state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateCompat {
    pub state: String,
    pub exits: Vec<ExitCompat>,
    /// Commands the new version offers from this state that version 1 did not.
    pub gained: Vec<String>,
    /// Why this state needs review. Empty when the checks found no difference.
    pub reasons: Vec<String>,
}

/// The result of comparing two versions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compat {
    pub states: Vec<StateCompat>,
    /// Differences that apply to every state (new or changed invariants, changed effects).
    pub global: Vec<String>,
    /// What these checks did not establish.
    pub not_established: Vec<String>,
}

const VALUES_NOT_READ: &str =
    "stored fact values were not checked against the new rules (the checks read definitions only)";
const EFFECTS_NOT_READ: &str = "unresolved effect intents of existing instances were not inspected";
const REGIONS_NOT_ANALYSED: &str =
    "compatibility across region configurations is not computed for hierarchical machines";

/// Compare `old` with `new`.
pub fn drain_compat(old: &Definition, new: &Definition) -> Compat {
    if old.chart.hierarchical || new.chart.hierarchical {
        return Compat {
            not_established: vec![REGIONS_NOT_ANALYSED.into()],
            ..Compat::default()
        };
    }
    let written = must_written(old);
    let invariant_reads = invariant_facts(new);
    let states = old
        .states
        .keys()
        .map(|state| {
            let known = written.get(state).cloned().unwrap_or_default();
            state_compat(old, new, state, &known, &invariant_reads)
        })
        .collect();
    let mut global = changed_invariants(old, new);
    global.extend(changed_effects(old, new));
    Compat {
        states,
        global,
        not_established: vec![VALUES_NOT_READ.into(), EFFECTS_NOT_READ.into()],
    }
}

fn state_compat(
    old: &Definition,
    new: &Definition,
    state: &str,
    known: &BTreeSet<String>,
    invariant_reads: &BTreeMap<String, BTreeSet<String>>,
) -> StateCompat {
    let old_exits = exits_of(old, state);
    let new_exits = exits_of(new, state);
    let exits: Vec<ExitCompat> = old_exits
        .iter()
        .map(|(command, outcomes)| ExitCompat {
            command: command.clone(),
            verdict: verdict(outcomes, new_exits.get(command)),
        })
        .collect();
    let gained: Vec<String> = new_exits
        .keys()
        .filter(|command| !old_exits.contains_key(*command))
        .cloned()
        .collect();
    let mut reasons: Vec<String> = exits
        .iter()
        .filter(|exit| exit.verdict != "same")
        .map(|exit| format!("exit `{}`: {}", exit.command, exit.verdict))
        .collect();
    reasons.extend(unset_fact_reasons(new, state, &new_exits, known, invariant_reads));
    StateCompat {
        state: state.to_string(),
        exits,
        gained,
        reasons,
    }
}

type Outcomes = Vec<(Option<String>, String)>;

/// The exits of `state` as `command -> [(guard, target)]`.
fn exits_of(def: &Definition, state: &str) -> BTreeMap<String, Outcomes> {
    def.commands
        .iter()
        .filter(|(_, command)| command.from.iter().any(|from| from == state))
        .map(|(name, command)| (name.clone(), outcomes_of(command)))
        .collect()
}

fn outcomes_of(command: &CommandDef) -> Outcomes {
    match &command.outcomes {
        Some(list) => list
            .iter()
            .map(|item| (trimmed(item.when.as_deref()), item.to.clone()))
            .collect(),
        None => vec![(
            trimmed(command.when.as_deref()),
            command.to.clone().unwrap_or_default(),
        )],
    }
}

fn trimmed(guard: Option<&str>) -> Option<String> {
    guard.map(str::trim).filter(|text| !text.is_empty()).map(str::to_string)
}

fn verdict(old: &Outcomes, new: Option<&Outcomes>) -> &'static str {
    let Some(new) = new else {
        return "removed";
    };
    if old == new {
        return "same";
    }
    let targets = |items: &Outcomes| items.iter().map(|(_, to)| to.clone()).collect::<Vec<_>>();
    if targets(old) == targets(new) {
        "guard changed"
    } else {
        "target changed"
    }
}

/// Facts that are set on every path from creation into each state.
fn must_written(def: &Definition) -> BTreeMap<String, BTreeSet<String>> {
    let all: BTreeSet<String> = def.facts.keys().cloned().collect();
    let initial_facts: BTreeSet<String> = def
        .facts
        .iter()
        .filter(|(_, fact)| fact.initial.is_some())
        .map(|(name, _)| name.clone())
        .collect();
    let mut known: BTreeMap<String, BTreeSet<String>> = def
        .states
        .iter()
        .map(|(name, state)| {
            let start = if state.initial { initial_facts.clone() } else { all.clone() };
            (name.clone(), start)
        })
        .collect();
    let edges = edges_of(def);
    loop {
        let mut changed = false;
        for (from, to, writes) in &edges {
            let reach: BTreeSet<String> = known[from].union(writes).cloned().collect();
            let next: BTreeSet<String> = known[to].intersection(&reach).cloned().collect();
            if next != known[to] {
                known.insert(to.clone(), next);
                changed = true;
            }
        }
        if !changed {
            return known;
        }
    }
}

/// Every `(source, target, facts written)` move.
fn edges_of(def: &Definition) -> Vec<(String, String, BTreeSet<String>)> {
    let mut out = Vec::new();
    for command in def.commands.values() {
        let base: BTreeSet<String> = command
            .then
            .iter()
            .flat_map(|map| map.keys().cloned())
            .collect();
        let steps: Vec<(String, BTreeSet<String>)> = match &command.outcomes {
            Some(list) => list
                .iter()
                .map(|item| (item.to.clone(), union(&base, item.then.as_ref())))
                .collect(),
            None => vec![(command.to.clone().unwrap_or_default(), base.clone())],
        };
        for from in &command.from {
            for (to, writes) in &steps {
                if def.states.contains_key(from) && def.states.contains_key(to) {
                    out.push((from.clone(), to.clone(), writes.clone()));
                }
            }
        }
    }
    out
}

fn union(base: &BTreeSet<String>, extra: Option<&BTreeMap<String, String>>) -> BTreeSet<String> {
    let mut out = base.clone();
    if let Some(map) = extra {
        out.extend(map.keys().cloned());
    }
    out
}

/// Facts a guard reads by value. `exists` tests presence and is safe on a missing fact.
fn value_reads(expr: &Expr) -> BTreeSet<String> {
    match expr {
        Expr::And(parts) | Expr::Or(parts) => parts.iter().flat_map(value_reads).collect(),
        Expr::Cmp(cmp) => operand_facts(&cmp.left)
            .into_iter()
            .chain(operand_facts(&cmp.right))
            .collect(),
        Expr::TextIn(text) => operand_facts(&text.value),
        Expr::Exists(_) | Expr::NotExists(_) => BTreeSet::new(),
    }
}

fn operand_facts(operand: &Operand) -> BTreeSet<String> {
    match operand {
        Operand::Fact(name) => BTreeSet::from([name.clone()]),
        Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => {
            operand_facts(base)
        }
        Operand::BinArith { left, right, .. } => {
            operand_facts(left).into_iter().chain(operand_facts(right)).collect()
        }
        Operand::Arg(_)
        | Operand::State
        | Operand::EffectStatus
        | Operand::EffectKind
        | Operand::EffectOutcome
        | Operand::Literal(_) => BTreeSet::new(),
    }
}

fn guard_reads(guard: Option<&str>) -> BTreeSet<String> {
    guard
        .and_then(|text| expr::parse(text).ok())
        .map(|parsed| value_reads(&parsed))
        .unwrap_or_default()
}

/// Facts each version-2 invariant reads by value.
fn invariant_facts(def: &Definition) -> BTreeMap<String, BTreeSet<String>> {
    def.invariants
        .iter()
        .map(|(name, item)| {
            let mut reads = guard_reads(Some(&item.assert));
            reads.extend(guard_reads(item.when.as_deref()));
            (name.clone(), reads)
        })
        .collect()
}

fn unset_fact_reasons(
    new: &Definition,
    state: &str,
    exits: &BTreeMap<String, Outcomes>,
    known: &BTreeSet<String>,
    invariant_reads: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<String> {
    let mut out = Vec::new();
    for (command, outcomes) in exits {
        let reads: BTreeSet<String> = outcomes
            .iter()
            .flat_map(|(guard, _)| guard_reads(guard.as_deref()))
            .collect();
        for fact in reads.difference(known).filter(|fact| new.facts.contains_key(*fact)) {
            out.push(format!(
                "`{command}` reads fact `{fact}`, which version 1 instances in `{state}` may not have set"
            ));
        }
    }
    for (name, reads) in invariant_reads {
        for fact in reads.difference(known).filter(|fact| new.facts.contains_key(*fact)) {
            out.push(format!(
                "invariant `{name}` reads fact `{fact}`, which version 1 instances in `{state}` may not have set"
            ));
        }
    }
    out
}

fn changed_invariants(old: &Definition, new: &Definition) -> Vec<String> {
    new.invariants
        .iter()
        .filter_map(|(name, item)| match old.invariants.get(name) {
            None => Some(format!(
                "invariant `{name}` is new: version 1 instances were never checked against it"
            )),
            Some(before) if before.assert != item.assert || before.when != item.when => Some(format!(
                "invariant `{name}` changed: version 1 instances were never checked against the new form"
            )),
            Some(_) => None,
        })
        .collect()
}

fn changed_effects(old: &Definition, new: &Definition) -> Vec<String> {
    let mut out = Vec::new();
    for (name, command) in &old.commands {
        let Some(effect) = &command.effect else {
            continue;
        };
        let now = new.commands.get(name).and_then(|item| item.effect.as_ref());
        let same = now.is_some_and(|item| {
            item.kind == effect.kind
                && item.id == effect.id
                && item.payload_digest == effect.payload_digest
                && item.idempotency_key == effect.idempotency_key
        });
        if !same {
            out.push(format!(
                "the effect of `{name}` changed or was removed: a version 1 effect intent still waiting for its outcome may not be accepted under version 2"
            ));
        }
    }
    out
}
