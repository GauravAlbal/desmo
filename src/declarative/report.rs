//! Adequacy judgment over a compiled verification plan.
//!
//! This module consumes already-produced plans, diffs, and reports. It does
//! not execute machines, persist state, or decide transition legality.

use super::evidence::ObligationDischarge;
use super::local::LocalAdequacy;
use super::plan::{ImpossibilityRecordV1, VerificationPlanDiff, VerificationPlanV1};
use crate::canonical::digest_json;
use crate::declarative::INVALID_NONDETERMINISTIC_COUNTEREXAMPLE;
use serde::{Deserialize, Serialize};

/// Compact adequacy report. Never claims proof or equivalence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationReportV1 {
    pub schema_version: u64,
    pub machine_id: String,
    pub spec_hash: String,
    pub plan_digest: String,
    pub engines: Vec<String>,
    pub engine_versions: Vec<(String, String)>,
    pub bounds: String,
    pub obligations: ObligationCounts,
    pub challenges: ChallengeCounts,
    pub survivors: Vec<String>,
    pub counterexamples: Vec<String>,
    pub coverage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infra_failure: Option<String>,
    /// Impossible-success records copied from the plan. Discharged generated
    /// product-context rows are not uncovered debt. Authored goals stay
    /// mandatory. Schema-1 ordered-outcome rows stay recorded and retained.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub impossibilities: Vec<ImpossibilityRecordV1>,
}

/// Obligation totals plus the identities the judge needs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObligationCounts {
    pub total: usize,
    pub satisfied: usize,
    pub falsified: usize,
    pub uncovered: usize,
    pub incomplete: usize,
    pub falsified_ids: Vec<String>,
    pub uncovered_ids: Vec<String>,
}

/// Challenge totals. Survivors are listed on the parent report.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengeCounts {
    pub total: usize,
    pub killed: usize,
    pub survived: usize,
    pub incomplete: usize,
}

/// Accept or refuse a report against a plan and frozen debt.
///
/// `verdict` is `accept` or `refused`. `status` is the typed distinction
/// `accepted`, `missing_evidence`, or `witnessed_violation`. Reasons stay
/// free-text detail; they are not a second verdict vocabulary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptJudgment {
    pub verdict: String,
    #[serde(default)]
    pub status: String,
    pub reasons: Vec<String>,
}

/// Build a report bound to a plan digest. Counts are caller-supplied evidence.
pub fn verification_report(
    plan: &VerificationPlanV1,
    obligations: ObligationCounts,
    challenges: ChallengeCounts,
    survivors: Vec<String>,
    counterexamples: Vec<String>,
) -> VerificationReportV1 {
    VerificationReportV1 {
        schema_version: 1,
        machine_id: plan.machine.machine_id.clone(),
        spec_hash: plan.machine.spec_hash.clone(),
        plan_digest: digest_json(plan),
        engines: vec!["explore".into(), "mutation".into()],
        engine_versions: vec![("ik".into(), env!("CARGO_PKG_VERSION").into())],
        bounds: "bounded".into(),
        obligations,
        challenges,
        survivors,
        counterexamples,
        coverage: "bounded".into(),
        infra_failure: None,
        impossibilities: plan.impossibilities.clone(),
    }
}

/// Derive a report from evidence. Counts are never caller-supplied.
pub fn report_from_evidence(
    plan: &VerificationPlanV1,
    discharges: &[ObligationDischarge],
    local: &[LocalAdequacy],
) -> VerificationReportV1 {
    let mut counts = ObligationCounts {
        total: plan.obligations.len(),
        ..ObligationCounts::default()
    };
    for obligation in &plan.obligations {
        let status = discharges
            .iter()
            .find(|row| row.obligation_id == obligation.obligation_id)
            .map(|row| row.status.as_str())
            .unwrap_or("uncovered");
        match status {
            "satisfied" => counts.satisfied += 1,
            "falsified" => {
                counts.falsified += 1;
                counts.falsified_ids.push(obligation.obligation_id.clone());
            }
            "incomplete" | "inapplicable" => counts.incomplete += 1,
            _ => {
                counts.uncovered += 1;
                counts.uncovered_ids.push(obligation.obligation_id.clone());
            }
        }
    }
    let mut challenges = ChallengeCounts::default();
    let mut survivors = Vec::new();
    for row in local {
        for neighbor in &row.neighbors {
            challenges.total += 1;
            match neighbor.status.as_str() {
                "killed" => challenges.killed += 1,
                "survived" => {
                    challenges.survived += 1;
                    let coord = row
                        .coordinate
                        .command
                        .clone()
                        .unwrap_or_else(|| "guard".into());
                    survivors.push(format!("{coord} {}", neighbor.operator));
                }
                _ => challenges.incomplete += 1,
            }
        }
    }
    verification_report(plan, counts, challenges, survivors, vec![])
}

/// Judge a report. Existing baseline debt may remain; new mandatory debt may not.
pub fn judge_acceptance(
    plan: &VerificationPlanV1,
    report: &VerificationReportV1,
    diff: &VerificationPlanDiff,
    baseline_uncovered: &[String],
) -> AcceptJudgment {
    let mut reasons = Vec::new();
    if report.schema_version != 1
        || report.machine_id.is_empty()
        || report.spec_hash.is_empty()
        || report.plan_digest.is_empty()
        || report.obligations.total
            != report.obligations.satisfied
                + report.obligations.falsified
                + report.obligations.uncovered
                + report.obligations.incomplete
    {
        reasons.push("malformed report".into());
    }
    if report.spec_hash != plan.machine.spec_hash {
        reasons.push("stale spec-hash evidence".into());
    }
    if report.plan_digest != digest_json(plan) {
        reasons.push("stale spec-hash evidence".into());
    }
    if !report.obligations.falsified_ids.is_empty() || report.obligations.falsified > 0 {
        reasons.push("falsified mandatory obligation".into());
    }
    if report
        .survivors
        .iter()
        .any(|s| s.contains("CORE_LAWS") || s.contains("capability-currentness"))
    {
        reasons.push("surviving mandatory CORE_LAWS mutant".into());
    }
    let baseline: std::collections::BTreeSet<_> = baseline_uncovered.iter().cloned().collect();
    let uncovered: std::collections::BTreeSet<_> =
        report.obligations.uncovered_ids.iter().cloned().collect();
    let new_debt = if diff.obligations.is_empty() {
        uncovered
            .iter()
            .filter(|id| !baseline.contains(*id))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        uncovered
            .iter()
            .filter(|id| match diff.status(id) {
                Some("UNCHANGED") => !baseline.contains(*id),
                Some("MODIFIED") | Some("ADDED") => true,
                _ => true,
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    if !new_debt.is_empty() {
        reasons.push("uncovered newly introduced mandatory obligation".into());
    }
    if report
        .counterexamples
        .iter()
        .any(|c| c == INVALID_NONDETERMINISTIC_COUNTEREXAMPLE)
    {
        reasons.push("non-replayable counterexample".into());
    }
    if report.infra_failure.is_some() {
        reasons.push("verification infrastructure failure".into());
    }
    reasons.sort();
    reasons.dedup();
    judgment_from_reasons(reasons)
}

fn judgment_from_reasons(reasons: Vec<String>) -> AcceptJudgment {
    if reasons.is_empty() {
        return AcceptJudgment {
            verdict: "accept".into(),
            status: "accepted".into(),
            reasons,
        };
    }
    let witnessed = reasons.iter().any(|reason| {
        reason == "falsified mandatory obligation"
            || reason.starts_with("model transition violates invariant")
    });
    AcceptJudgment {
        verdict: "refused".into(),
        status: if witnessed {
            "witnessed_violation".into()
        } else {
            "missing_evidence".into()
        },
        reasons,
    }
}
