//! Optional coverage discharge from checked SMT certificates.

use super::certificate::check_invariant;
use super::config::configured_solvers;
use super::exhaustive::search as exhaustive_search;
use super::solve::{analyze_target, parse_target, ObligationReport, ProofStatus, Trust};
use crate::declarative::check::standard_lane;
use crate::declarative::compile::CompiledMachine;
use crate::declarative::def::Definition;
use crate::declarative::plan::{ImpossibilityRecordV1, VerificationObligationV1, VerificationPlanV1};
use std::collections::BTreeSet;

pub fn trust_name(trust: Trust) -> &'static str {
    match trust {
        Trust::SolverAsserted => "SolverAsserted",
        Trust::CheckedInvariant => "CheckedInvariant",
        Trust::CheckedExhaustive => "CheckedExhaustive",
    }
}

pub fn may_discharge(report: &ObligationReport) -> bool {
    report.status == ProofStatus::ProvenImpossible
        && matches!(
            report.trust,
            Trust::CheckedInvariant | Trust::CheckedExhaustive
        )
}

pub fn reverify_report(
    def: &Definition,
    machine: &CompiledMachine,
    report: &ObligationReport,
) -> bool {
    let Some(target) = parse_target(&report.id) else {
        return false;
    };
    match report.trust {
        Trust::CheckedInvariant => reverify_invariant(def, machine, report, &target),
        Trust::CheckedExhaustive => reverify_exhaustive(def, machine, report, &target),
        Trust::SolverAsserted => false,
    }
}

fn reverify_invariant(
    def: &Definition,
    machine: &CompiledMachine,
    report: &ObligationReport,
    target: &super::query::Target,
) -> bool {
    let Ok(ex) = exhaustive_search(def, machine, target) else {
        return false;
    };
    if let Some(want) = &report.reachable_digest {
        if want != &ex.reachable_digest {
            return false;
        }
    }
    let (z3, cvc5) = configured_solvers();
    let stored = report.invariant.clone().filter(|row| !row.is_empty());
    let candidates = match stored {
        Some(inv) => vec![inv],
        None => vec![ex.invariant_smt.clone(), ex.projected_smt.clone()],
    };
    for inv in candidates {
        if let Ok(check) = check_invariant(def, &inv, target, z3.as_ref(), cvc5.as_ref()) {
            if check.dual_checked() {
                return true;
            }
        }
    }
    false
}

fn reverify_exhaustive(
    def: &Definition,
    machine: &CompiledMachine,
    report: &ObligationReport,
    target: &super::query::Target,
) -> bool {
    let Ok(ex) = exhaustive_search(def, machine, target) else {
        return false;
    };
    if !ex.complete || ex.witness.is_some() {
        return false;
    }
    match &report.reachable_digest {
        Some(want) => want == &ex.reachable_digest,
        None => false,
    }
}

pub fn merge_certificates(
    def: &Definition,
    machine: &CompiledMachine,
    plan: &mut VerificationPlanV1,
) {
    let (z3, cvc5) = configured_solvers();
    if z3.is_none() {
        return;
    }
    let known: BTreeSet<String> = plan
        .impossibilities
        .iter()
        .map(|row| row.obligation_id.clone())
        .collect();
    let mut extra = Vec::new();
    for obligation in &plan.obligations {
        if let Some(row) = certificate_row(def, machine, obligation, &known, z3.as_ref(), cvc5.as_ref())
        {
            extra.push(row);
        }
    }
    let skip: BTreeSet<String> = extra
        .iter()
        .filter(|row| row.discharged)
        .map(|row| row.obligation_id.clone())
        .collect();
    plan.impossibilities.extend(extra);
    plan.obligations
        .retain(|row| !skip.contains(&row.obligation_id));
    plan.impossibilities
        .sort_by(|left, right| left.obligation_id.cmp(&right.obligation_id));
}

fn certificate_row(
    def: &Definition,
    machine: &CompiledMachine,
    obligation: &VerificationObligationV1,
    known: &BTreeSet<String>,
    z3: Option<&super::solve::SolverBin>,
    cvc5: Option<&super::solve::SolverBin>,
) -> Option<ImpossibilityRecordV1> {
    if obligation.obligation_id.starts_with("authored/") {
        return None;
    }
    if !standard_lane(&obligation.kind) || known.contains(&obligation.obligation_id) {
        return None;
    }
    let target = parse_target(&obligation.obligation_id)?;
    let report = analyze_target(
        def,
        machine,
        &obligation.obligation_id,
        target,
        z3,
        cvc5,
    );
    if report.status != ProofStatus::ProvenImpossible {
        return None;
    }
    Some(record_from_report(obligation, &report, def, machine))
}

fn record_from_report(
    obligation: &VerificationObligationV1,
    report: &ObligationReport,
    def: &Definition,
    machine: &CompiledMachine,
) -> ImpossibilityRecordV1 {
    let checked = may_discharge(report) && reverify_report(def, machine, report);
    ImpossibilityRecordV1 {
        obligation_id: obligation.obligation_id.clone(),
        kind: obligation.kind.clone(),
        class: class_of(&obligation.kind).to_string(),
        discharged: checked,
        reason: report.detail.clone(),
        proving_source: proving_source(obligation),
        coordinate: obligation.coordinate.clone(),
        basis: basis_of(report).to_string(),
        scope: report.scope.clone().unwrap_or_default(),
        trust: trust_name(report.trust).to_string(),
        certificate_digest: report.reachable_digest.clone().unwrap_or_default(),
        solvers: solver_ids(report),
    }
}

fn class_of(kind: &str) -> &'static str {
    match kind {
        "OBL-CMD-001" => "command_source",
        "OBL-OUT-001" => "ordered_outcome",
        _ => "reachability",
    }
}

fn proving_source(obligation: &VerificationObligationV1) -> String {
    obligation
        .coordinate
        .source
        .clone()
        .or_else(|| obligation.coordinate.state.clone())
        .or_else(|| obligation.coordinate.command.clone())
        .unwrap_or_else(|| obligation.obligation_id.clone())
}

fn basis_of(report: &ObligationReport) -> &'static str {
    match report.trust {
        Trust::CheckedInvariant => "checked_induction",
        Trust::CheckedExhaustive => "kernel_exhaustive",
        Trust::SolverAsserted => "solver_asserted",
    }
}

fn solver_ids(report: &ObligationReport) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(z3) = &report.z3 {
        out.push(format!("z3:{z3}"));
    }
    if let Some(cvc5) = &report.cvc5 {
        out.push(format!("cvc5:{cvc5}"));
    }
    out
}
