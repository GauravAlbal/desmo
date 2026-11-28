//! Deterministic seeded random-walk simulation checker (modeled on Stateright's
//! `SimulationChecker`) plus shared concrete path replay for actionable traces and
//! `ik simulate` offline explorer embedding.

use super::compile::CompiledMachine;
use super::explore::{
    candidates_for, check_reach_witnesses, compile_reach, machine_effect_outcomes,
    record_invariant_refusal, state_within_boundary, step_machine, Candidate,
    InvariantRefusalWitness, Step, WitnessStep,
};
use super::expr::Expr;
use crate::effect::EffectStatus;
use crate::machine::Machine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_SIM_SEED: u64 = 0x5EED_DE50_0000_0001;
pub const DEFAULT_SIM_STEPS: usize = 64;
pub const DEFAULT_SIM_TRACES: usize = 256;

/// Seeded random-walk configuration.
#[derive(Clone, Debug)]
pub struct SimOptions {
    pub seed: u64,
    pub max_steps: usize,
    pub traces: usize,
    pub fail_fast: bool,
    pub boundary: Vec<(String, Expr)>,
}

impl Default for SimOptions {
    fn default() -> Self {
        Self {
            seed: DEFAULT_SIM_SEED,
            max_steps: DEFAULT_SIM_STEPS,
            traces: DEFAULT_SIM_TRACES,
            fail_fast: false,
            boundary: Vec::new(),
        }
    }
}

/// Summary of a seeded random-walk run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimResult {
    pub seed: u64,
    pub traces_run: usize,
    pub steps_executed: usize,
    pub max_depth_reached: usize,
    pub invariant_refusals: Vec<InvariantRefusalWitness>,
    pub reach_witnesses: BTreeMap<String, Vec<WitnessStep>>,
    pub reached_states: BTreeSet<String>,
    pub reached_terminals: BTreeSet<String>,
    pub boundary_pruned: usize,
}

/// Single step of a replayed concrete trace with state + fact diff metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceStep {
    pub index: usize,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case: Option<String>,
    pub args: Value,
    pub from_state: String,
    pub to_state: String,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub fact_diff: Vec<FactDelta>,
}

/// Single changed fact between pre-state and post-state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FactDelta {
    pub fact: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    pub after: Value,
}

/// Fast deterministic SplitMix64 PRNG requiring zero external dependencies.
#[derive(Clone, Debug)]
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn gen_index(&mut self, len: usize) -> usize {
        debug_assert!(len > 0);
        (self.next_u64() % (len as u64)) as usize
    }
}

enum SimMove<'a> {
    RecordEffect(&'a str),
    Candidate(&'a Candidate),
}

/// Run deterministic seeded random walks over `machine` using $O(\text{depth})$ trace memory.
pub fn run_simulation(
    machine: &CompiledMachine,
    corpus: &super::explore::CaseCorpus,
    options: &SimOptions,
) -> SimResult {
    let def = machine.definition();
    let compiled_reach = compile_reach(def);
    let effect_outcomes = machine_effect_outcomes(def);
    let uses_effect_guards = effect_outcomes.is_some();
    let all_candidates = candidates_for(def, corpus);

    let mut refusals = BTreeMap::new();
    let mut reach_witnesses = BTreeMap::new();
    let mut reached_states = BTreeSet::new();
    let mut reached_terminals = BTreeSet::new();
    let mut traces_run = 0usize;
    let mut steps_executed = 0usize;
    let mut max_depth_reached = 0usize;
    let mut boundary_pruned = 0usize;

    let initial = machine.initial_state();
    if let Some(label) = state_label(&initial) {
        reached_states.insert(label);
    }
    check_reach_witnesses(
        &compiled_reach,
        &initial,
        &EffectStatus::None,
        &[],
        &mut reach_witnesses,
    );

    if !state_within_boundary(&initial, &EffectStatus::None, &options.boundary) {
        return SimResult {
            seed: options.seed,
            traces_run: 0,
            steps_executed: 0,
            max_depth_reached: 0,
            invariant_refusals: Vec::new(),
            reach_witnesses,
            reached_states,
            reached_terminals,
            boundary_pruned: 1,
        };
    }

    'outer: for t in 0..options.traces {
        traces_run += 1;
        let trace_seed = options
            .seed
            .wrapping_add((t as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut rng = SplitMix64::new(trace_seed);
        let mut state = initial.clone();
        let mut effect = EffectStatus::None;
        let mut path: Vec<WitnessStep> = Vec::new();
        let mut moves: Vec<SimMove<'_>> = Vec::new();

        for step_idx in 0..options.max_steps {
            moves.clear();
            if uses_effect_guards {
                if matches!(effect, EffectStatus::Pending(_)) {
                    if let Some(outcomes) = &effect_outcomes {
                        for digest in outcomes {
                            moves.push(SimMove::RecordEffect(digest.as_str()));
                        }
                    }
                }
            }
            let cur_state = state.get("state").and_then(Value::as_str);
            for cand in &all_candidates {
                if let (Some(st), Some(cdef)) = (cur_state, def.commands.get(&cand.command_name)) {
                    if !cdef.from.iter().any(|s| s == st) {
                        continue;
                    }
                }
                moves.push(SimMove::Candidate(cand));
            }
            if moves.is_empty() {
                break;
            }
            let choice = rng.gen_index(moves.len());
            match moves[choice] {
                SimMove::RecordEffect(digest) => {
                    let EffectStatus::Pending(intent) = &effect else {
                        continue;
                    };
                    let recorded = EffectStatus::Recorded(intent.clone(), digest.to_string(), None);
                    if !state_within_boundary(&state, &recorded, &options.boundary) {
                        boundary_pruned += 1;
                        continue;
                    }
                    effect = recorded;
                    path.push(WitnessStep {
                        command: format!("record_effect:{digest}"),
                        case: None,
                        outcome_index: None,
                    });
                    steps_executed += 1;
                    max_depth_reached = max_depth_reached.max(step_idx + 1);
                    check_reach_witnesses(
                        &compiled_reach,
                        &state,
                        &effect,
                        &path,
                        &mut reach_witnesses,
                    );
                }
                SimMove::Candidate(candidate) => {
                    match step_machine(machine, &state, &effect, &candidate.command) {
                        Step::Commit {
                            next,
                            terminal,
                            outcome_index,
                            planned_effect,
                        } => {
                            if uses_effect_guards
                                && planned_effect.is_some()
                                && matches!(effect, EffectStatus::Pending(_))
                            {
                                continue;
                            }
                            let next_effect = if uses_effect_guards {
                                match planned_effect {
                                    Some(intent) => EffectStatus::Pending(intent),
                                    None => effect.clone(),
                                }
                            } else {
                                EffectStatus::None
                            };
                            if !state_within_boundary(&next, &next_effect, &options.boundary) {
                                boundary_pruned += 1;
                                continue;
                            }
                            state = next;
                            effect = next_effect;
                            path.push(WitnessStep {
                                command: candidate.command_name.clone(),
                                case: candidate.case.clone(),
                                outcome_index: Some(outcome_index),
                            });
                            steps_executed += 1;
                            max_depth_reached = max_depth_reached.max(step_idx + 1);
                            if let Some(label) = state_label(&state) {
                                reached_states.insert(label.clone());
                                if terminal {
                                    reached_terminals.insert(label);
                                }
                            }
                            check_reach_witnesses(
                                &compiled_reach,
                                &state,
                                &effect,
                                &path,
                                &mut reach_witnesses,
                            );
                            if terminal {
                                break;
                            }
                        }
                        Step::Noop => {
                            steps_executed += 1;
                        }
                        Step::Refuse {
                            diagnostic,
                            outcome_index,
                        } => {
                            steps_executed += 1;
                            let prev_len = refusals.len();
                            record_invariant_refusal(
                                def,
                                Some(&mut refusals),
                                &path,
                                candidate,
                                &diagnostic,
                                outcome_index,
                            );
                            if options.fail_fast && refusals.len() > prev_len {
                                max_depth_reached = max_depth_reached.max(step_idx + 1);
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
    }

    SimResult {
        seed: options.seed,
        traces_run,
        steps_executed,
        max_depth_reached,
        invariant_refusals: refusals.into_values().collect(),
        reach_witnesses,
        reached_states,
        reached_terminals,
        boundary_pruned,
    }
}

/// Replay a sequence of [`WitnessStep`]s against `machine` and capture concrete state transitions,
/// arguments, diagnostics, and per-step fact diffs.
pub fn replay_path_steps(
    machine: &CompiledMachine,
    corpus: &super::explore::CaseCorpus,
    steps: &[WitnessStep],
) -> Vec<TraceStep> {
    let mut out = Vec::with_capacity(steps.len());
    let mut state = machine.initial_state();
    let mut effect = EffectStatus::None;
    let def = machine.definition();
    let uses_effect_guards = machine_effect_outcomes(def).is_some();

    for (index, step) in steps.iter().enumerate() {
        let from_state = state_label(&state).unwrap_or_default();
        if let Some(digest) = step.command.strip_prefix("record_effect:") {
            if let EffectStatus::Pending(intent) = &effect {
                effect = EffectStatus::Recorded(intent.clone(), digest.to_string(), None);
            }
            out.push(TraceStep {
                index,
                command: step.command.clone(),
                case: None,
                args: json!({}),
                from_state: from_state.clone(),
                to_state: from_state,
                outcome: "record_effect".to_string(),
                code: None,
                rule: None,
                message: None,
                fact_diff: Vec::new(),
            });
            continue;
        }
        let args = step
            .case
            .as_deref()
            .and_then(|case_name| {
                corpus
                    .commands
                    .get(&step.command)
                    .and_then(|list| list.iter().find(|c| c.name == case_name))
                    .map(|c| Value::Object(c.args.iter().map(|(k, v)| (k.clone(), v.clone())).collect()))
            })
            .unwrap_or_else(|| json!({}));
        let cmd = if args.as_object().is_some_and(|m| !m.is_empty()) {
            json!({ "name": step.command, "args": args })
        } else {
            json!({ "name": step.command })
        };
        match step_machine(machine, &state, &effect, &cmd) {
            Step::Commit {
                next,
                planned_effect,
                ..
            } => {
                let to_state = state_label(&next).unwrap_or_default();
                let diff = diff_facts(&state, &next);
                if uses_effect_guards {
                    if let Some(intent) = planned_effect {
                        effect = EffectStatus::Pending(intent);
                    }
                }
                state = next;
                out.push(TraceStep {
                    index,
                    command: step.command.clone(),
                    case: step.case.clone(),
                    args,
                    from_state,
                    to_state,
                    outcome: "committed".to_string(),
                    code: None,
                    rule: None,
                    message: None,
                    fact_diff: diff,
                });
            }
            Step::Noop => {
                out.push(TraceStep {
                    index,
                    command: step.command.clone(),
                    case: step.case.clone(),
                    args,
                    from_state: from_state.clone(),
                    to_state: from_state,
                    outcome: "noop".to_string(),
                    code: None,
                    rule: None,
                    message: None,
                    fact_diff: Vec::new(),
                });
            }
            Step::Refuse { diagnostic, .. } => {
                out.push(TraceStep {
                    index,
                    command: step.command.clone(),
                    case: step.case.clone(),
                    args,
                    from_state: from_state.clone(),
                    to_state: from_state,
                    outcome: "refused".to_string(),
                    code: Some(diagnostic.code.as_str().to_string()),
                    rule: diagnostic.rule.clone(),
                    message: Some(diagnostic.message.clone()),
                    fact_diff: Vec::new(),
                });
            }
        }
    }
    out
}

fn state_label(state: &Value) -> Option<String> {
    state
        .get("state")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn diff_facts(before: &Value, after: &Value) -> Vec<FactDelta> {
    let mut out = Vec::new();
    let Some(after_map) = after.as_object() else {
        return out;
    };
    let before_map = before.as_object();
    for (k, v_after) in after_map {
        if k == "state" {
            continue;
        }
        let v_before = before_map.and_then(|m| m.get(k));
        if v_before != Some(v_after) {
            out.push(FactDelta {
                fact: k.clone(),
                before: v_before.cloned(),
                after: v_after.clone(),
            });
        }
    }
    out
}
