//! VFY-12: standard acceptance consumes plan/report/diff and refuses new debt.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::def::OutcomeDef;
use ik::declarative::{
    judge_acceptance, parse_definition, plan_diff, verification_plan, verification_report,
    ChallengeCounts, Definition, ObligationCounts, VerificationPlanDiff, VerificationReportV1,
    INVALID_NONDETERMINISTIC_COUNTEREXAMPLE,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct BaselineFile {
    uncovered_mandatory: Vec<String>,
}

fn va_def() -> Definition {
    parse_definition(&std::fs::read_to_string(machine_def("verification_attempt")).unwrap())
        .expect("va")
}

fn baseline() -> Vec<String> {
    let text = std::fs::read_to_string(repo_root().join("validation/verification_baseline.json"))
        .expect("baseline");
    serde_json::from_str::<BaselineFile>(&text)
        .expect("json")
        .uncovered_mandatory
}

fn counts_from(uncovered: &[String], total: usize) -> ObligationCounts {
    ObligationCounts {
        total,
        satisfied: total - uncovered.len(),
        falsified: 0,
        uncovered: uncovered.len(),
        incomplete: 0,
        falsified_ids: vec![],
        uncovered_ids: uncovered.to_vec(),
    }
}

fn empty_diff() -> VerificationPlanDiff {
    VerificationPlanDiff::default()
}

#[test]
fn frozen_va_debt_is_accepted() {
    let plan = verification_plan(&va_def());
    let uncovered = baseline();
    let report = verification_report(
        &plan,
        counts_from(&uncovered, plan.obligations.len()),
        ChallengeCounts::default(),
        vec![],
        vec![],
    );
    let judgment = judge_acceptance(&plan, &report, &empty_diff(), &uncovered);
    assert_eq!(judgment.verdict, "accept");
    assert_eq!(judgment.status, "accepted");
    assert!(judgment.reasons.is_empty());
}

#[test]
fn added_uncovered_outcome_is_refused() {
    let old = va_def();
    let mut new = old.clone();
    new.commands
        .get_mut("reject")
        .expect("reject")
        .outcomes
        .as_mut()
        .expect("outcomes")
        .push(OutcomeDef {
            when: Some("attempt_count < 0".into()),
            to: "repairable".into(),
            then: None,
        });
    let old_plan = verification_plan(&old);
    let new_plan = verification_plan(&new);
    let diff = plan_diff(&old_plan, &new_plan);
    let added = diff
        .obligations_added()
        .into_iter()
        .find(|id| id == "outcome/reject/2/executable")
        .expect("new obligation");
    let report = verification_report(
        &new_plan,
        counts_from(std::slice::from_ref(&added), new_plan.obligations.len()),
        ChallengeCounts::default(),
        vec![],
        vec![],
    );
    let judgment = judge_acceptance(&new_plan, &report, &diff, &baseline());
    assert_eq!(judgment.verdict, "refused");
    assert_eq!(judgment.status, "missing_evidence");
    assert!(judgment
        .reasons
        .iter()
        .any(|r| r == "uncovered newly introduced mandatory obligation"));
}

#[test]
fn stale_spec_hash_is_refused() {
    let plan = verification_plan(&va_def());
    let mut report = verification_report(
        &plan,
        counts_from(&[], plan.obligations.len()),
        ChallengeCounts::default(),
        vec![],
        vec![],
    );
    report.spec_hash = "deadbeef".into();
    let judgment = judge_acceptance(&plan, &report, &empty_diff(), &baseline());
    assert_eq!(judgment.verdict, "refused");
    assert_eq!(judgment.status, "missing_evidence");
    assert!(judgment
        .reasons
        .iter()
        .any(|r| r == "stale spec-hash evidence"));
}

#[test]
fn malformed_report_is_refused() {
    let plan = verification_plan(&va_def());
    let report = VerificationReportV1 {
        schema_version: 0,
        machine_id: String::new(),
        spec_hash: String::new(),
        plan_digest: String::new(),
        engines: vec![],
        engine_versions: vec![],
        bounds: String::new(),
        obligations: ObligationCounts::default(),
        challenges: ChallengeCounts::default(),
        survivors: vec![],
        counterexamples: vec![],
        coverage: String::new(),
        infra_failure: None,
        impossibilities: vec![],
    };
    let judgment = judge_acceptance(&plan, &report, &empty_diff(), &[]);
    assert_eq!(judgment.verdict, "refused");
    assert_eq!(judgment.status, "missing_evidence");
    assert!(judgment.reasons.iter().any(|r| r == "malformed report"));
}

#[test]
fn surviving_core_laws_mutant_is_refused() {
    let plan = verification_plan(&va_def());
    let report = verification_report(
        &plan,
        counts_from(&baseline(), plan.obligations.len()),
        ChallengeCounts::default(),
        vec!["CORE_LAWS/capability-currentness".into()],
        vec![],
    );
    let judgment = judge_acceptance(&plan, &report, &empty_diff(), &baseline());
    assert_eq!(judgment.verdict, "refused");
    assert_eq!(judgment.status, "missing_evidence");
    assert!(judgment
        .reasons
        .iter()
        .any(|r| r == "surviving mandatory CORE_LAWS mutant"));
}

#[test]
fn non_replayable_counterexample_is_refused() {
    let plan = verification_plan(&va_def());
    let report = verification_report(
        &plan,
        counts_from(&baseline(), plan.obligations.len()),
        ChallengeCounts::default(),
        vec![],
        vec![INVALID_NONDETERMINISTIC_COUNTEREXAMPLE.into()],
    );
    let judgment = judge_acceptance(&plan, &report, &empty_diff(), &baseline());
    assert_eq!(judgment.verdict, "refused");
    assert_eq!(judgment.status, "missing_evidence");
    assert!(judgment
        .reasons
        .iter()
        .any(|r| r == "non-replayable counterexample"));
}

#[test]
fn infrastructure_failure_is_refused() {
    let plan = verification_plan(&va_def());
    let mut report = verification_report(
        &plan,
        counts_from(&baseline(), plan.obligations.len()),
        ChallengeCounts::default(),
        vec![],
        vec![],
    );
    report.infra_failure = Some("hegel unavailable".into());
    let judgment = judge_acceptance(&plan, &report, &empty_diff(), &baseline());
    assert_eq!(judgment.verdict, "refused");
    assert_eq!(judgment.status, "missing_evidence");
    assert!(judgment
        .reasons
        .iter()
        .any(|r| r == "verification infrastructure failure"));
}

#[test]
fn falsified_mandatory_obligation_is_refused() {
    let plan = verification_plan(&va_def());
    let mut counts = counts_from(&baseline(), plan.obligations.len());
    counts.falsified = 1;
    counts.satisfied = counts.satisfied.saturating_sub(1);
    counts.falsified_ids = vec!["obl/falsified".into()];
    let report = verification_report(&plan, counts, ChallengeCounts::default(), vec![], vec![]);
    let judgment = judge_acceptance(&plan, &report, &empty_diff(), &baseline());
    assert_eq!(judgment.verdict, "refused");
    assert_eq!(judgment.status, "witnessed_violation");
    assert!(judgment
        .reasons
        .iter()
        .any(|r| r == "falsified mandatory obligation"));
}

#[test]
fn unchanged_frozen_debt_survives_name_change() {
    let old = va_def();
    let mut new = old.clone();
    new.name = Some("VerificationAttemptRenamed".into());
    let diff = plan_diff(&verification_plan(&old), &verification_plan(&new));
    assert!(diff.obligations_added().is_empty());
    assert!(diff.obligations_modified().is_empty());
    assert!(!diff.obligations_unchanged().is_empty());
    let new_plan = verification_plan(&new);
    let report = verification_report(
        &new_plan,
        counts_from(&baseline(), new_plan.obligations.len()),
        ChallengeCounts::default(),
        vec![],
        vec![],
    );
    let judgment = judge_acceptance(&new_plan, &report, &diff, &baseline());
    assert_eq!(judgment.verdict, "accept", "{judgment:?}");
    assert_eq!(judgment.status, "accepted", "{judgment:?}");
}

#[test]
fn modified_frozen_invariant_requires_fresh_evidence() {
    let old = va_def();
    let mut new = old.clone();
    new.invariants
        .get_mut("va_inv_004")
        .expect("va_inv_004")
        .assert = "attempt_count < max_attempts".into();
    let new_plan = verification_plan(&new);
    let diff = plan_diff(&verification_plan(&old), &new_plan);
    assert!(diff
        .obligations_modified()
        .iter()
        .any(|id| id == "invariant/va_inv_004/evaluated"));
    let report = verification_report(
        &new_plan,
        counts_from(&baseline(), new_plan.obligations.len()),
        ChallengeCounts::default(),
        vec![],
        vec![],
    );
    let judgment = judge_acceptance(&new_plan, &report, &diff, &baseline());
    assert_eq!(judgment.verdict, "refused");
    assert_eq!(judgment.status, "missing_evidence");
    assert!(judgment
        .reasons
        .iter()
        .any(|r| r == "uncovered newly introduced mandatory obligation"));
}
