//! Bounded behavioral comparison of two declarative definitions.
//!
//! A run that observes no difference reports `observed_no_difference_within_bound`.
//! It never claims proof of equivalence.

use super::analyze::{analyze, machine_graph};
use super::compile::{compile, CompiledMachine};
use super::def::Definition;
use super::explore::{explore, Bounds, CaseCorpus, Coverage, Exploration, WitnessStep};
use crate::canonical::digest_json;
use crate::diagnostic::Diagnostic;
use crate::effect::{EffectIntentV1, EffectStatus};
use crate::machine::{Decision, Machine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Versioned comparison of two machines that share a `machine_id`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MachineDiff {
    pub schema_version: u64,
    pub old_machine: DiffIdentity,
    pub new_machine: DiffIdentity,
    pub bound: Bounds,
    pub case_corpus_digest: String,
    pub observation: String,
    pub old_complete: bool,
    pub new_complete: bool,
    pub paired_comparison_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete_reason: Option<String>,
    pub old_coverage: Coverage,
    pub new_coverage: Coverage,
    pub structural: StructuralDelta,
    pub executable: ExecutableDelta,
}

/// Identity slice used in a diff.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffIdentity {
    pub machine_id: String,
    pub version: u64,
    pub spec_hash: String,
}

/// Deterministic structural changes. Empty means no declared-shape change.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralDelta {
    pub states_added: Vec<String>,
    pub states_removed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_changed: Option<(String, String)>,
    pub terminality_changed: Vec<String>,
    pub commands_added: Vec<String>,
    pub commands_removed: Vec<String>,
    pub sources_changed: Vec<String>,
    pub targets_changed: Vec<String>,
    pub guards_changed: Vec<String>,
    pub outcome_order_changed: Vec<String>,
    pub facts_changed: Vec<String>,
    pub assignments_changed: Vec<String>,
    pub invariants_changed: Vec<String>,
    pub effects_changed: Vec<String>,
}

/// Observed executable difference, if any, within the bound and corpus.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecutableDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub witness: Option<Vec<WitnessStep>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_result: Option<StepResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_result: Option<StepResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension: Option<String>,
    pub old_nodes: usize,
    pub new_nodes: usize,
}

/// One side of an executable comparison.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepResult {
    pub verdict: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_state: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<EffectIntentV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_rule: Option<String>,
}

/// Compare two checked definitions of the same `machine_id`.
pub fn diff_machines(
    old: &Definition,
    new: &Definition,
    corpus: &CaseCorpus,
    bounds: Bounds,
) -> Result<MachineDiff, String> {
    if old.machine_id != new.machine_id {
        return Err(format!(
            "machine_id mismatch: `{}` vs `{}`",
            old.machine_id, new.machine_id
        ));
    }
    let old_machine = compile(old.clone()).map_err(|e| e.join("; "))?;
    let new_machine = compile(new.clone()).map_err(|e| e.join("; "))?;
    let old_run = explore(&old_machine, corpus, bounds);
    let new_run = explore(&new_machine, corpus, bounds);
    diff_machines_with_runs(
        old,
        new,
        &old_machine,
        &new_machine,
        &old_run,
        &new_run,
        corpus,
        bounds,
    )
}

/// Compare two already-compiled and already-explored machines without redundant work.
#[allow(clippy::too_many_arguments)]
pub fn diff_machines_with_runs(
    old: &Definition,
    new: &Definition,
    old_machine: &CompiledMachine,
    new_machine: &CompiledMachine,
    old_run: &Exploration,
    new_run: &Exploration,
    corpus: &CaseCorpus,
    bounds: Bounds,
) -> Result<MachineDiff, String> {
    if old.machine_id != new.machine_id {
        return Err(format!(
            "machine_id mismatch: `{}` vs `{}`",
            old.machine_id, new.machine_id
        ));
    }
    let structural = structural_delta(old, new);
    let walk = first_divergence(old_machine, new_machine, corpus, bounds);
    let executable = executable_delta(old_run, new_run, walk.found);
    let observation = if executable.dimension.is_none() {
        "observed_no_difference_within_bound".to_string()
    } else {
        "behavioral_divergence".to_string()
    };
    Ok(MachineDiff {
        schema_version: 2,
        old_machine: identity_of(old_machine),
        new_machine: identity_of(new_machine),
        bound: bounds,
        case_corpus_digest: corpus.digest(),
        observation,
        old_complete: old_run.complete,
        new_complete: new_run.complete,
        paired_comparison_complete: walk.complete,
        incomplete_reason: walk.incomplete_reason,
        old_coverage: old_run.coverage.clone(),
        new_coverage: new_run.coverage.clone(),
        structural,
        executable,
    })
}

fn identity_of(machine: &CompiledMachine) -> DiffIdentity {
    let identity = machine.identity();
    DiffIdentity {
        machine_id: identity.machine_id.clone(),
        version: identity.version.0,
        spec_hash: identity.spec_hash.0.clone(),
    }
}

fn structural_delta(old: &Definition, new: &Definition) -> StructuralDelta {
    let old_graph = machine_graph(old);
    let new_graph = machine_graph(new);
    let old_analysis = analyze(&old_graph);
    let new_analysis = analyze(&new_graph);
    let mut delta = StructuralDelta::default();
    set_diff(
        old.states.keys().cloned().collect(),
        new.states.keys().cloned().collect(),
        &mut delta.states_removed,
        &mut delta.states_added,
    );
    if old_analysis.initial != new_analysis.initial {
        delta.initial_changed = Some((old_analysis.initial, new_analysis.initial));
    }
    for name in old.states.keys().filter(|n| new.states.contains_key(*n)) {
        let old_term = old.states[name].terminal;
        let new_term = new.states[name].terminal;
        if old_term != new_term {
            delta.terminality_changed.push(name.clone());
        }
    }
    set_diff(
        old.commands.keys().cloned().collect(),
        new.commands.keys().cloned().collect(),
        &mut delta.commands_removed,
        &mut delta.commands_added,
    );
    for name in old
        .commands
        .keys()
        .filter(|n| new.commands.contains_key(*n))
    {
        let old_cmd = &old.commands[name];
        let new_cmd = &new.commands[name];
        if old_cmd.from != new_cmd.from {
            delta.sources_changed.push(name.clone());
        }
        if outcome_targets(old_cmd) != outcome_targets(new_cmd) {
            delta.targets_changed.push(name.clone());
        }
        if outcome_guards(old_cmd) != outcome_guards(new_cmd) {
            delta.guards_changed.push(name.clone());
        }
        if outcome_count(old_cmd) != outcome_count(new_cmd) {
            delta.outcome_order_changed.push(name.clone());
        }
        if old_cmd.then != new_cmd.then || outcome_thens(old_cmd) != outcome_thens(new_cmd) {
            delta.assignments_changed.push(name.clone());
        }
        if digest_json(&old_cmd.effect) != digest_json(&new_cmd.effect) {
            delta.effects_changed.push(name.clone());
        }
    }
    if digest_json(&old.facts) != digest_json(&new.facts) {
        let old_keys: BTreeSet<String> = old.facts.keys().cloned().collect();
        let new_keys: BTreeSet<String> = new.facts.keys().cloned().collect();
        for name in old_keys.difference(&new_keys) {
            delta.facts_changed.push(name.clone());
        }
        for name in new_keys.difference(&old_keys) {
            delta.facts_changed.push(name.clone());
        }
        if delta.facts_changed.is_empty() {
            delta.facts_changed.push("facts".to_string());
        }
    }
    if digest_json(&old.invariants) != digest_json(&new.invariants) {
        let old_keys: BTreeSet<String> = old.invariants.keys().cloned().collect();
        let new_keys: BTreeSet<String> = new.invariants.keys().cloned().collect();
        for name in old_keys.difference(&new_keys) {
            delta.invariants_changed.push(name.clone());
        }
        for name in new_keys.difference(&old_keys) {
            delta.invariants_changed.push(name.clone());
        }
        if delta.invariants_changed.is_empty() {
            delta.invariants_changed.push("invariants".to_string());
        }
    }
    delta
}

fn set_diff(
    old: BTreeSet<String>,
    new: BTreeSet<String>,
    removed: &mut Vec<String>,
    added: &mut Vec<String>,
) {
    for name in old.difference(&new) {
        removed.push(name.clone());
    }
    for name in new.difference(&old) {
        added.push(name.clone());
    }
}

fn outcome_count(cmd: &super::def::CommandDef) -> usize {
    cmd.outcomes.as_ref().map(|o| o.len()).unwrap_or(1)
}

fn outcome_targets(cmd: &super::def::CommandDef) -> Vec<String> {
    if let Some(outcomes) = &cmd.outcomes {
        return outcomes.iter().map(|o| o.to.clone()).collect();
    }
    vec![cmd.to.clone().unwrap_or_default()]
}

fn outcome_guards(cmd: &super::def::CommandDef) -> Vec<Option<String>> {
    if let Some(outcomes) = &cmd.outcomes {
        return outcomes.iter().map(|o| o.when.clone()).collect();
    }
    vec![cmd.when.clone()]
}

fn outcome_thens(cmd: &super::def::CommandDef) -> Vec<Option<BTreeMap<String, String>>> {
    if let Some(outcomes) = &cmd.outcomes {
        return outcomes.iter().map(|o| o.then.clone()).collect();
    }
    vec![cmd.then.clone()]
}

fn executable_delta(
    old_run: &Exploration,
    new_run: &Exploration,
    found: Option<Found>,
) -> ExecutableDelta {
    if let Some(found) = found {
        return ExecutableDelta {
            witness: Some(found.witness),
            old_result: Some(found.old),
            new_result: Some(found.new),
            dimension: Some(found.dimension),
            old_nodes: old_run.nodes,
            new_nodes: new_run.nodes,
        };
    }
    ExecutableDelta {
        witness: None,
        old_result: None,
        new_result: None,
        dimension: None,
        old_nodes: old_run.nodes,
        new_nodes: new_run.nodes,
    }
}

struct Found {
    witness: Vec<WitnessStep>,
    old: StepResult,
    new: StepResult,
    dimension: String,
}

struct PairedWalk {
    found: Option<Found>,
    complete: bool,
    incomplete_reason: Option<String>,
}

fn first_divergence(
    old: &CompiledMachine,
    new: &CompiledMachine,
    corpus: &CaseCorpus,
    bounds: Bounds,
) -> PairedWalk {
    let mut queue = VecDeque::new();
    let mut seen = BTreeSet::new();
    queue.push_back(Pair {
        old_state: old.initial_state(),
        new_state: new.initial_state(),
        path: Vec::new(),
        depth: 0,
    });
    seen.insert((
        digest_json(&old.initial_state()),
        digest_json(&new.initial_state()),
    ));
    let mut nodes = 0usize;
    let mut hit_max_nodes = false;
    let mut hit_max_depth = false;
    while let Some(pair) = queue.pop_front() {
        nodes += 1;
        if nodes > bounds.max_nodes {
            hit_max_nodes = true;
            break;
        }
        for candidate in super_candidates(old.definition(), new.definition(), corpus) {
            let old_step = step(old, &pair.old_state, &candidate.command);
            let new_step = step(new, &pair.new_state, &candidate.command);
            if let Some(dimension) = differ(&old_step, &new_step) {
                let mut witness = pair.path.clone();
                witness.push(WitnessStep {
                    command: candidate.command_name.clone(),
                    case: candidate.case.clone(),
                    outcome_index: old_step.outcome_index.or(new_step.outcome_index),
                });
                return PairedWalk {
                    found: Some(Found {
                        witness,
                        old: old_step,
                        new: new_step,
                        dimension,
                    }),
                    complete: true,
                    incomplete_reason: None,
                };
            }
            if old_step.verdict != "commit" || new_step.verdict != "commit" {
                continue;
            }
            if pair.depth + 1 >= bounds.max_depth {
                hit_max_depth = true;
                continue;
            }
            let next_old = old_step.post_state.clone().unwrap();
            let next_new = new_step.post_state.clone().unwrap();
            let key = (digest_json(&next_old), digest_json(&next_new));
            if !seen.insert(key) {
                continue;
            }
            let mut path = pair.path.clone();
            path.push(WitnessStep {
                command: candidate.command_name,
                case: candidate.case,
                outcome_index: old_step.outcome_index,
            });
            queue.push_back(Pair {
                old_state: next_old,
                new_state: next_new,
                path,
                depth: pair.depth + 1,
            });
        }
    }
    if !queue.is_empty() {
        hit_max_nodes = true;
    }
    let incomplete_reason = if hit_max_nodes {
        Some("max_nodes".to_string())
    } else if hit_max_depth {
        Some("max_depth".to_string())
    } else {
        None
    };
    PairedWalk {
        found: None,
        complete: incomplete_reason.is_none(),
        incomplete_reason,
    }
}

struct Pair {
    old_state: Value,
    new_state: Value,
    path: Vec<WitnessStep>,
    depth: usize,
}

struct Cand {
    command_name: String,
    case: Option<String>,
    command: Value,
}

fn needs_args(cmd: &super::def::CommandDef) -> bool {
    cmd.args
        .as_ref()
        .map(|args| !args.is_empty())
        .unwrap_or(false)
}

fn super_candidates(old: &Definition, new: &Definition, corpus: &CaseCorpus) -> Vec<Cand> {
    let mut names = BTreeSet::new();
    names.extend(old.commands.keys().cloned());
    names.extend(new.commands.keys().cloned());
    let mut out = Vec::new();
    for name in names {
        let needs = old.commands.get(&name).map(needs_args).unwrap_or(false)
            || new.commands.get(&name).map(needs_args).unwrap_or(false);
        if needs {
            if let Some(cases) = corpus.commands.get(&name) {
                for case in cases {
                    out.push(Cand {
                        command_name: name.clone(),
                        case: Some(case.name.clone()),
                        command: serde_json::json!({ "name": name, "args": case.args }),
                    });
                }
            }
        } else {
            out.push(Cand {
                command_name: name.clone(),
                case: None,
                command: serde_json::json!({ "name": name }),
            });
        }
    }
    out
}

fn step(machine: &CompiledMachine, state: &Value, command: &Value) -> StepResult {
    match machine.decide(state, None, &EffectStatus::None, command) {
        Err(diagnostic) => refuse(diagnostic),
        Ok(Decision::Noop { .. }) => StepResult {
            verdict: "noop".to_string(),
            outcome_index: None,
            post_state: Some(state.clone()),
            terminal: Some(false),
            effect: None,
            diagnostic_code: None,
            diagnostic_rule: None,
        },
        Ok(Decision::Emit { event, .. }) => {
            let outcome_index = event
                .payload
                .get("outcome")
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            match machine.apply(state, &event) {
                Err(diagnostic) => refuse(diagnostic),
                Ok(applied) => StepResult {
                    verdict: "commit".to_string(),
                    outcome_index,
                    post_state: Some(applied.state),
                    terminal: Some(applied.terminal),
                    effect: applied.effect,
                    diagnostic_code: None,
                    diagnostic_rule: None,
                },
            }
        }
    }
}

fn refuse(diagnostic: Diagnostic) -> StepResult {
    StepResult {
        verdict: "refused".to_string(),
        outcome_index: None,
        post_state: None,
        terminal: None,
        effect: None,
        diagnostic_code: Some(diagnostic.code.as_str()),
        diagnostic_rule: diagnostic.rule,
    }
}

fn differ(old: &StepResult, new: &StepResult) -> Option<String> {
    if old.verdict != new.verdict {
        return Some("verdict".to_string());
    }
    if old.outcome_index != new.outcome_index {
        return Some("selected_outcome".to_string());
    }
    if old.post_state != new.post_state {
        return Some("post_state".to_string());
    }
    if old.terminal != new.terminal {
        return Some("terminality".to_string());
    }
    if old.effect != new.effect {
        return Some("effect".to_string());
    }
    if old.diagnostic_code != new.diagnostic_code {
        return Some("diagnostic_code".to_string());
    }
    if old.diagnostic_rule != new.diagnostic_rule {
        return Some("diagnostic_rule".to_string());
    }
    None
}
