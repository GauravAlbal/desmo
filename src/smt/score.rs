//! CLI backend used by `scripts/score-known-feasibility.py`.

use super::config::{adapter_configured, configured_solvers};
use super::discharge::trust_name;
use super::solve::{analyze_target, parse_target, ProofStatus, Trust};
use crate::declarative::{parse_and_compile, parse_definition, verification_plan};
use serde_json::{json, Value};
use std::path::Path;

pub fn run_feasibility_backend(args: &[String]) -> i32 {
    match run(args) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn run(args: &[String]) -> Result<String, String> {
    let flags = flags(args);
    let machine = flags
        .get("machine")
        .ok_or_else(|| "missing --machine".to_string())?;
    let engine = flags
        .get("engine")
        .map(|row| row.as_str())
        .unwrap_or("in-house");
    let text = std::fs::read_to_string(Path::new(machine))
        .map_err(|err| format!("read {machine}: {err}"))?;
    let def = parse_definition(&text).map_err(|err| err.join("; "))?;
    let compiled = parse_and_compile(&text).map_err(|err| err.join("; "))?;
    let plan = verification_plan(&def);
    let rows = if engine == "smt" {
        smt_rows(&def, &compiled, &plan)
    } else {
        in_house_rows(&plan)
    };
    Ok(format!("{}\n", serde_json::to_string(&json!({ "obligations": rows })).map_err(|err| err.to_string())?))
}

fn smt_rows(
    def: &crate::declarative::def::Definition,
    machine: &crate::declarative::compile::CompiledMachine,
    plan: &crate::declarative::plan::VerificationPlanV1,
) -> Vec<Value> {
    let (z3, cvc5) = if adapter_configured() {
        configured_solvers()
    } else {
        super::solve::discover_solvers()
    };
    let mut rows = Vec::new();
    for obligation in &plan.obligations {
        let Some(target) = parse_target(&obligation.obligation_id) else {
            continue;
        };
        let report = analyze_target(
            def,
            machine,
            &obligation.obligation_id,
            target,
            z3.as_ref(),
            cvc5.as_ref(),
        );
        rows.push(score_row(def, machine, &report));
    }
    for row in &plan.impossibilities {
        if rows.iter().any(|item| item["id"] == row.obligation_id) {
            continue;
        }
        rows.push(json!({
            "id": row.obligation_id,
            "status": "impossible",
            "trust": if row.discharged { "checked" } else { "solver" },
        }));
    }
    rows
}

fn in_house_rows(plan: &crate::declarative::plan::VerificationPlanV1) -> Vec<Value> {
    plan.impossibilities
        .iter()
        .map(|row| {
            json!({
                "id": row.obligation_id,
                "status": "impossible",
                "trust": if row.discharged { "checked" } else { "solver" },
            })
        })
        .collect()
}

fn score_row(
    def: &crate::declarative::def::Definition,
    machine: &crate::declarative::compile::CompiledMachine,
    report: &super::solve::ObligationReport,
) -> Value {
    let mut status = status_name(report.status);
    let mut trace = witness_json(def, machine, &report.id);
    if status == "witness" && trace.is_none() {
        status = "unknown";
        trace = None;
    }
    let mut row = json!({
        "id": report.id,
        "status": status,
        "trust": trust_score(report.trust),
    });
    if let Some(steps) = trace {
        row["trace"] = steps;
    }
    row
}

fn witness_json(
    def: &crate::declarative::def::Definition,
    machine: &crate::declarative::compile::CompiledMachine,
    id: &str,
) -> Option<Value> {
    let target = parse_target(id)?;
    let ex = super::exhaustive::search(def, machine, &target).ok()?;
    let steps = ex.witness?;
    if steps.is_empty() {
        return None;
    }
    Some(json!(steps
        .iter()
        .map(|step| json!({ "command": step.name, "args": step.args }))
        .collect::<Vec<_>>()))
}

fn status_name(status: ProofStatus) -> &'static str {
    match status {
        ProofStatus::Witness => "witness",
        ProofStatus::ProvenImpossible => "impossible",
        ProofStatus::Unknown => "unknown",
    }
}

fn trust_score(trust: Trust) -> &'static str {
    match trust {
        Trust::SolverAsserted => "solver",
        Trust::CheckedInvariant | Trust::CheckedExhaustive => "checked",
    }
}

fn flags(args: &[String]) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        let key = args[i].trim_start_matches('-');
        if i + 1 < args.len() && args[i].starts_with("--") {
            out.insert(key.to_string(), args[i + 1].clone());
            i += 2;
            continue;
        }
        i += 1;
    }
    out
}

pub fn run_smt_cli(args: &[String]) -> i32 {
    match smt_cli(args) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(err) => {
            eprintln!("{err}");
            2
        }
    }
}

fn smt_cli(args: &[String]) -> Result<String, String> {
    let flags = flags(args);
    let machine = flags
        .get("machine")
        .ok_or_else(|| "missing --machine".to_string())?;
    let text = std::fs::read_to_string(Path::new(machine))
        .map_err(|err| format!("read {machine}: {err}"))?;
    let def = parse_definition(&text).map_err(|err| err.join("; "))?;
    let compiled = parse_and_compile(&text).map_err(|err| err.join("; "))?;
    let (z3, cvc5) = if adapter_configured() {
        configured_solvers()
    } else {
        super::solve::discover_solvers()
    };
    let ids = match flags.get("target") {
        Some(id) => vec![id.clone()],
        None => verification_plan(&def)
            .obligations
            .iter()
            .map(|row| row.obligation_id.clone())
            .collect(),
    };
    let mut rows = Vec::new();
    for id in ids {
        let Some(target) = parse_target(&id) else {
            continue;
        };
        let report = analyze_target(&def, &compiled, &id, target, z3.as_ref(), cvc5.as_ref());
        rows.push(json!({
            "id": report.id,
            "status": status_name(report.status),
            "trust": trust_name(report.trust),
            "detail": report.detail,
            "scope": report.scope,
            "digest": report.reachable_digest,
            "z3_ms": report.z3_ms,
            "cvc5_ms": report.cvc5_ms,
            "z3": report.z3,
            "cvc5": report.cvc5,
            "replay": report.replay,
        }));
    }
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&json!({ "obligations": rows }))
            .map_err(|err| err.to_string())?
    ))
}
