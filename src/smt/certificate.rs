//! Independent checks of an inductive invariant. `unsat` of each query is success.

use super::query::{
    dump_query, encode_invariant_exclude, encode_invariant_init, encode_invariant_step, Target,
};
use super::replay::{replay_command, state_object, CommandStep};
use super::solve::{run_smt2, ModelMap, SolverBin, SolverReply};
use crate::canonical::sha256_hex;
use crate::declarative::compile::CompiledMachine;
use crate::declarative::def::Definition;
use crate::machine::Machine;
use std::cell::RefCell;
use std::time::Duration;

thread_local! {
    static IND_CACHE: RefCell<Option<(String, QueryCheck, QueryCheck)>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryCheck {
    pub holds: bool,
    pub z3: Option<String>,
    pub cvc5: Option<String>,
    pub z3_ms: Option<u128>,
    pub cvc5_ms: Option<u128>,
    pub detail: String,
    pub replay: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvariantCheck {
    pub initial: QueryCheck,
    pub inductive: QueryCheck,
    pub excludes: QueryCheck,
}

impl InvariantCheck {
    pub fn accepted(&self) -> bool {
        self.initial.holds && self.inductive.holds && self.excludes.holds
    }

    pub fn dual_checked(&self) -> bool {
        self.accepted() && dual(&self.initial) && dual(&self.inductive) && dual(&self.excludes)
    }

    pub fn disagreed(&self) -> bool {
        disagreed(&self.initial) || disagreed(&self.inductive) || disagreed(&self.excludes)
    }

    pub fn summary(&self) -> String {
        format!(
            "init={} inductive={} excludes={}",
            flag(self.initial.holds),
            flag(self.inductive.holds),
            flag(self.excludes.holds)
        )
    }

    pub fn replay_notes(&self) -> Vec<String> {
        [&self.initial, &self.inductive, &self.excludes]
            .into_iter()
            .filter_map(|row| row.replay.clone())
            .collect()
    }
}

fn dual(row: &QueryCheck) -> bool {
    matches!(row.z3.as_deref(), Some("unsat")) && matches!(row.cvc5.as_deref(), Some("unsat"))
}

fn disagreed(row: &QueryCheck) -> bool {
    matches!(
        (row.z3.as_deref(), row.cvc5.as_deref()),
        (Some("sat"), Some("unsat")) | (Some("unsat"), Some("sat"))
    )
}

fn flag(holds: bool) -> &'static str {
    if holds {
        "holds"
    } else {
        "fails"
    }
}

pub fn check_invariant(
    def: &Definition,
    invariant: &str,
    target: &Target,
    z3: Option<&SolverBin>,
    cvc5: Option<&SolverBin>,
) -> Result<InvariantCheck, String> {
    check_invariant_replay(def, invariant, target, z3, cvc5, None)
}

pub fn check_invariant_replay(
    def: &Definition,
    invariant: &str,
    target: &Target,
    z3: Option<&SolverBin>,
    cvc5: Option<&SolverBin>,
    machine: Option<&CompiledMachine>,
) -> Result<InvariantCheck, String> {
    let Some(z3) = z3 else {
        return Err("z3 not installed".to_string());
    };
    let timeout = query_timeout(invariant);
    let (initial, inductive) = inductive_pair(def, invariant, z3, cvc5, timeout, machine)?;
    let exclude = encode_invariant_exclude(def, invariant, target).map_err(|err| err.reason)?;
    dump_query("inv-exclude", &exclude);
    Ok(InvariantCheck {
        initial,
        inductive,
        excludes: discharge(
            "I excludes target",
            def,
            &exclude,
            z3,
            cvc5,
            timeout,
            machine,
            ReplayKind::Exclude(target.clone()),
        )?,
    })
}

fn inductive_pair(
    def: &Definition,
    invariant: &str,
    z3: &SolverBin,
    cvc5: Option<&SolverBin>,
    timeout: Duration,
    machine: Option<&CompiledMachine>,
) -> Result<(QueryCheck, QueryCheck), String> {
    let key = format!(
        "{}:{}:{}",
        sha256_hex(invariant.as_bytes()),
        z3.version,
        cvc5.map(|row| row.version.as_str()).unwrap_or("-")
    );
    if let Some((init, step)) = IND_CACHE.with(|slot| {
        slot.borrow()
            .as_ref()
            .filter(|(stored, _, _)| stored == &key)
            .map(|(_, init, step)| (init.clone(), step.clone()))
    }) {
        return Ok((init, step));
    }
    let init_q = encode_invariant_init(def, invariant).map_err(|err| err.reason)?;
    let step_q = encode_invariant_step(def, invariant).map_err(|err| err.reason)?;
    dump_query("inv-init", &init_q);
    dump_query("inv-step", &step_q);
    let initial = discharge(
        "init => I",
        def,
        &init_q,
        z3,
        cvc5,
        timeout,
        machine,
        ReplayKind::Init,
    )?;
    let inductive = discharge(
        "I /\\ step => I'",
        def,
        &step_q,
        z3,
        cvc5,
        timeout,
        machine,
        ReplayKind::Step,
    )?;
    IND_CACHE.with(|slot| {
        *slot.borrow_mut() = Some((key, initial.clone(), inductive.clone()));
    });
    Ok((initial, inductive))
}

fn query_timeout(invariant: &str) -> Duration {
    if invariant.len() > 8_000 {
        Duration::from_secs(180)
    } else {
        Duration::from_secs(20)
    }
}

#[derive(Clone)]
enum ReplayKind {
    Init,
    Step,
    Exclude(Target),
}

fn discharge(
    label: &str,
    def: &Definition,
    query: &super::query::SmtQuery,
    z3: &SolverBin,
    cvc5: Option<&SolverBin>,
    timeout: Duration,
    machine: Option<&CompiledMachine>,
    kind: ReplayKind,
) -> Result<QueryCheck, String> {
    let z = run_smt2(z3, &query.text, timeout)?;
    let z_rep = reply_name(&z.reply);
    let (c_rep, c_ms) = match cvc5 {
        Some(other) => {
            let c = run_smt2(other, &query.text, timeout)?;
            (Some(reply_name(&c.reply)), Some(c.ms))
        }
        None => (None, None),
    };
    let replay = sat_replay(def, query, &z, machine, &kind);
    let holds = matches!(z.reply, SolverReply::Unsat)
        && match &c_rep {
            Some(rep) => rep == "unsat",
            None => true,
        };
    let detail = match (&z.reply, &c_rep) {
        (SolverReply::Unsat, None) => format!("{label}: z3 unsat"),
        (SolverReply::Unsat, Some(rep)) if rep == "unsat" => {
            format!("{label}: z3 and cvc5 unsat")
        }
        (SolverReply::Sat, Some(rep)) if rep == "unsat" => {
            format!("{label}: solvers disagree z3=sat cvc5=unsat")
        }
        (SolverReply::Unsat, Some(rep)) if rep == "sat" => {
            format!("{label}: solvers disagree z3=unsat cvc5=sat")
        }
        (left, right) => format!("{label}: z3={left:?} cvc5={right:?}"),
    };
    Ok(QueryCheck {
        holds,
        z3: Some(z_rep),
        cvc5: c_rep,
        z3_ms: Some(z.ms),
        cvc5_ms: c_ms,
        detail,
        replay,
    })
}

fn sat_replay(
    def: &Definition,
    query: &super::query::SmtQuery,
    solved: &super::solve::SolveResult,
    machine: Option<&CompiledMachine>,
    kind: &ReplayKind,
) -> Option<String> {
    if solved.reply != SolverReply::Sat {
        return None;
    }
    let machine = machine?;
    match replay_model(def, query, &solved.model, machine, kind) {
        Ok(note) => Some(note),
        Err(err) => Some(format!("sat model did not replay: {err}")),
    }
}

fn replay_model(
    def: &Definition,
    query: &super::query::SmtQuery,
    model: &ModelMap,
    machine: &CompiledMachine,
    kind: &ReplayKind,
) -> Result<String, String> {
    match kind {
        ReplayKind::Init => {
            let state = machine.initial_state();
            Ok(format!(
                "kernel initial state {} is a counterexample to init => I",
                state.get("state").and_then(|v| v.as_str()).unwrap_or("?")
            ))
        }
        ReplayKind::Step => replay_step(def, query, model, machine),
        ReplayKind::Exclude(target) => replay_exclude(query, model, machine, target),
    }
}

fn replay_step(
    def: &Definition,
    query: &super::query::SmtQuery,
    model: &ModelMap,
    machine: &CompiledMachine,
) -> Result<String, String> {
    let (state, step) = super::replay::transition_from_model(def, query, model)?;
    let before = state.clone();
    let after = replay_command(machine, &state, &step)?;
    Ok(format!(
        "kernel replayed {} from {} to {}; sampled I missed this transition",
        step.name,
        label(&before),
        label(&after)
    ))
}

fn replay_exclude(
    query: &super::query::SmtQuery,
    model: &ModelMap,
    machine: &CompiledMachine,
    target: &Target,
) -> Result<String, String> {
    let (name, facts) = super::replay::model_to_state(query, model, "")?;
    let state = state_object(&name, &facts);
    match target {
        Target::State(want) | Target::Terminal(want) => {
            if name == *want {
                Ok(format!("kernel state {want} lies in I; exclude failed"))
            } else {
                Err(format!("exclude model state {name} is not {want}"))
            }
        }
        Target::Outcome { command, index } => {
            let args = super::replay::model_to_args(query, model, "")?;
            let step = CommandStep {
                name: command.clone(),
                args,
            };
            let got = super::replay::selected_outcome(machine, &state, &step)?;
            if got == *index {
                Ok(format!(
                    "kernel selected {command}/{index} from {}; sampled I missed this arm",
                    label(&state)
                ))
            } else {
                Err(format!(
                    "exclude model selected {command}/{got}, not {index}"
                ))
            }
        }
        Target::CommandFrom { command, source } => {
            if name != *source {
                return Err(format!("exclude model state {name} is not {source}"));
            }
            let args = super::replay::model_to_args(query, model, "")?;
            let step = CommandStep {
                name: command.clone(),
                args,
            };
            match super::replay::selected_outcome(machine, &state, &step) {
                Ok(_) => Ok(format!(
                    "kernel ran {command} from {source}; sampled I missed this command"
                )),
                Err(err) => Err(err),
            }
        }
    }
}

fn label(state: &serde_json::Value) -> String {
    state
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string()
}

fn reply_name(reply: &SolverReply) -> String {
    match reply {
        SolverReply::Sat => "sat".to_string(),
        SolverReply::Unsat => "unsat".to_string(),
        SolverReply::Unknown(reason) => format!("unknown:{reason}"),
    }
}

/// Negative controls: each wrong invariant must be rejected.
pub fn reject_wrong_invariants(
    def: &Definition,
    target: &Target,
    z3: &SolverBin,
    cvc5: Option<&SolverBin>,
) -> Result<Vec<(String, InvariantCheck)>, String> {
    let too_weak = check_invariant(def, "true", target, Some(z3), cvc5)?;
    let not_init = check_invariant(def, "false", target, Some(z3), cvc5)?;
    let idle_only = check_invariant(def, "(= |st| 0)", target, Some(z3), cvc5)?;
    if too_weak.accepted() {
        return Err("true was accepted as an invariant that should include the target".into());
    }
    if not_init.accepted() {
        return Err("false was accepted as an invariant".into());
    }
    if idle_only.accepted() {
        return Err("(= |st| 0) was accepted; it is not inductive on a moving machine".into());
    }
    Ok(vec![
        ("too-weak:true".into(), too_weak),
        ("not-init:false".into(), not_init),
        ("not-inductive:(= |st| 0)".into(), idle_only),
    ])
}
