//! Release-policy judgment over `ik check` evidence.
//!
//! Obligation *status* (what the evidence shows) is kept apart from the release
//! *policy* (what a release requires). A developer check may return partial
//! results; only [`release_evidence`] can produce `release_pass`, and it requires
//! every selected evidence class explicitly. Nothing here executes a machine.

use super::check::standard_lane;
use super::evidence::ObligationDischarge;
use super::explore::{Bounds, CaseCorpus, Exploration};
use super::plan::VerificationPlanV1;
use super::report::{AcceptJudgment, VerificationReportV1};
use crate::canonical::digest_json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How one obligation stands against the evidence that actually ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationStatus {
    /// Executed and witnessed by the exploration.
    Satisfied,
    /// Executed and contradicted.
    Falsified,
    /// Not witnessed, and the exploration was truncated, so absence proves nothing.
    Incomplete,
    /// The evidence artifact does not apply to this machine version.
    NotApplicable,
    /// Outside the standard `ik check` lane; no verifier looked at it.
    NotExamined,
    /// Not witnessed, and explicitly listed as baseline debt.
    ApprovedDebt,
    /// Not witnessed, in the standard lane, and not approved debt.
    Uncovered,
}

/// One obligation's status row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusRow {
    pub obligation_id: String,
    pub kind: String,
    pub status: ObligationStatus,
}

/// What a report is bound to. Absent facts stay absent; nothing is fabricated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceProvenance {
    pub machine_id: String,
    pub spec_hash: String,
    pub plan_digest: String,
    pub corpus_digest: String,
    pub corpus_cases: usize,
    pub bounds: Bounds,
    pub exploration_complete: bool,
    pub completion_reason: Option<String>,
    pub explored_nodes: usize,
    pub engine_versions: Vec<(String, String)>,
    /// Digest of the runtime binary that produced this evidence. `None` when the
    /// build does not supply one; never synthesized.
    pub runtime_artifact_digest: Option<String>,
}

/// The release verdict and the evidence classes it required.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseJudgment {
    /// `release_pass` or `release_refused`.
    pub verdict: String,
    pub reasons: Vec<String>,
    pub required_evidence: Vec<String>,
    pub excluded_assumptions: Vec<String>,
}

/// Status, provenance, and release judgment for one check run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseEvidence {
    pub schema: String,
    pub provenance: EvidenceProvenance,
    pub status_counts: BTreeMap<String, usize>,
    pub obligations: Vec<StatusRow>,
    pub release: ReleaseJudgment,
}

/// Evidence classes a strict release requires, by name. Obligation kinds outside the
/// standard `ik check` lane have no verifier here: they stay `not_examined`, are listed as
/// excluded assumptions, and are never counted as satisfied.
pub const REQUIRED_EVIDENCE: &[&str] = &[
    "exploration_complete",
    "something_executed",
    "no_falsified_obligation",
    "no_incomplete_obligation",
    "no_unwitnessed_or_debt_obligation_in_standard_lane",
    "no_surviving_challenge",
    "verification_infrastructure_healthy",
];

impl Default for ReleaseEvidence {
    fn default() -> Self {
        Self {
            schema: "ikaros.release-evidence.v1".into(),
            provenance: EvidenceProvenance {
                machine_id: String::new(),
                spec_hash: String::new(),
                plan_digest: String::new(),
                corpus_digest: String::new(),
                corpus_cases: 0,
                bounds: Bounds {
                    max_nodes: 0,
                    max_depth: 0,
                },
                exploration_complete: false,
                completion_reason: None,
                explored_nodes: 0,
                engine_versions: Vec::new(),
                runtime_artifact_digest: None,
            },
            status_counts: BTreeMap::new(),
            obligations: Vec::new(),
            release: ReleaseJudgment {
                verdict: "release_refused".into(),
                reasons: vec!["release evidence was not evaluated".into()],
                required_evidence: REQUIRED_EVIDENCE.iter().map(|s| s.to_string()).collect(),
                excluded_assumptions: Vec::new(),
            },
        }
    }
}

/// Everything a release decision reads.
pub struct ReleaseInputs<'a> {
    pub plan: &'a VerificationPlanV1,
    pub run: &'a Exploration,
    pub discharges: &'a [ObligationDischarge],
    pub report: &'a VerificationReportV1,
    pub judgment: &'a AcceptJudgment,
    pub baseline: &'a [String],
    pub corpus: &'a CaseCorpus,
    pub bounds: Bounds,
}

fn status_of(inputs: &ReleaseInputs<'_>, obligation_id: &str, kind: &str) -> ObligationStatus {
    let discharged = inputs
        .discharges
        .iter()
        .find(|row| row.obligation_id == obligation_id)
        .map(|row| row.status.as_str())
        .unwrap_or("uncovered");
    match discharged {
        "satisfied" => ObligationStatus::Satisfied,
        "falsified" => ObligationStatus::Falsified,
        "incomplete" => ObligationStatus::Incomplete,
        "inapplicable" => ObligationStatus::NotApplicable,
        _ if !standard_lane(kind) => ObligationStatus::NotExamined,
        _ if !inputs.run.complete => ObligationStatus::Incomplete,
        _ if inputs.baseline.iter().any(|id| id == obligation_id) => ObligationStatus::ApprovedDebt,
        _ => ObligationStatus::Uncovered,
    }
}

/// Compute per-obligation status, provenance, and the strict release judgment.
pub fn release_evidence(inputs: &ReleaseInputs<'_>) -> ReleaseEvidence {
    let obligations: Vec<StatusRow> = inputs
        .plan
        .obligations
        .iter()
        .map(|o| StatusRow {
            obligation_id: o.obligation_id.clone(),
            kind: o.kind.clone(),
            status: status_of(inputs, &o.obligation_id, &o.kind),
        })
        .collect();
    let mut status_counts: BTreeMap<String, usize> = BTreeMap::new();
    for row in &obligations {
        let name = serde_json::to_value(row.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        *status_counts.entry(name).or_default() += 1;
    }
    let provenance = EvidenceProvenance {
        machine_id: inputs.plan.machine.machine_id.clone(),
        spec_hash: inputs.plan.machine.spec_hash.clone(),
        plan_digest: digest_json(inputs.plan),
        corpus_digest: inputs.run.case_corpus_digest.clone(),
        corpus_cases: inputs.corpus.commands.values().map(Vec::len).sum(),
        bounds: inputs.bounds,
        exploration_complete: inputs.run.complete,
        completion_reason: inputs.run.incomplete_reason.clone(),
        explored_nodes: inputs.run.nodes,
        engine_versions: inputs.report.engine_versions.clone(),
        runtime_artifact_digest: None,
    };
    let reasons = release_reasons(inputs, &obligations, &provenance);
    let release = ReleaseJudgment {
        verdict: if reasons.is_empty() {
            "release_pass".into()
        } else {
            "release_refused".into()
        },
        reasons,
        required_evidence: REQUIRED_EVIDENCE.iter().map(|s| s.to_string()).collect(),
        excluded_assumptions: excluded_assumptions(inputs),
    };
    ReleaseEvidence {
        schema: "ikaros.release-evidence.v1".into(),
        provenance,
        status_counts,
        obligations,
        release,
    }
}

fn count(rows: &[StatusRow], status: ObligationStatus) -> usize {
    rows.iter().filter(|row| row.status == status).count()
}

fn release_reasons(
    inputs: &ReleaseInputs<'_>,
    rows: &[StatusRow],
    provenance: &EvidenceProvenance,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if inputs.judgment.verdict != "accept" {
        reasons.push("developer acceptance is refused".to_string());
    }
    if !provenance.exploration_complete {
        reasons.push(format!(
            "exploration is incomplete ({})",
            provenance
                .completion_reason
                .as_deref()
                .unwrap_or("reason not recorded")
        ));
    }
    if inputs.run.coverage.executed_outcomes.is_empty() {
        reasons.push("nothing was executed: no command outcome was witnessed".to_string());
    }
    for (status, label) in [
        (ObligationStatus::Falsified, "falsified"),
        (ObligationStatus::Incomplete, "incomplete"),
        (
            ObligationStatus::NotApplicable,
            "not applicable (stale evidence)",
        ),
        (ObligationStatus::ApprovedDebt, "approved baseline debt"),
        (ObligationStatus::Uncovered, "unwitnessed"),
    ] {
        let n = count(rows, status);
        if n > 0 {
            reasons.push(format!("{n} obligation(s) {label}"));
        }
    }
    if !inputs.report.survivors.is_empty() || inputs.report.challenges.incomplete > 0 {
        reasons.push(format!(
            "{} surviving and {} incomplete challenge(s)",
            inputs.report.survivors.len(),
            inputs.report.challenges.incomplete
        ));
    }
    if inputs.report.infra_failure.is_some() {
        reasons.push("verification infrastructure failure".to_string());
    }
    reasons
}

fn excluded_assumptions(inputs: &ReleaseInputs<'_>) -> Vec<String> {
    let mut out = vec![
        "evidence is bounded and relative to the supplied finite corpus; it is not exhaustive proof"
            .to_string(),
        "runtime artifact digest is unavailable: this report is not bound to a specific binary"
            .to_string(),
        "external reducers, hosts, and effect executors are outside this check".to_string(),
    ];
    let not_examined: std::collections::BTreeSet<&str> = inputs
        .plan
        .obligations
        .iter()
        .filter(|o| !standard_lane(&o.kind))
        .map(|o| o.kind.as_str())
        .collect();
    if !not_examined.is_empty() {
        out.push(format!(
            "obligation kinds with no verifier in this lane stay not_examined, not satisfied: {}",
            not_examined.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    if inputs.report.challenges.total == 0 {
        out.push(
            "no change-local adequacy challenges ran (they need --prior; with --prior, none was generated for this change)"
                .to_string(),
        );
    }
    out
}

impl ReleaseEvidence {
    /// Plain-text rendering appended to the developer packet.
    pub fn render(&self) -> String {
        let mut text = String::from("\nRELEASE POLICY (strict; separate from developer ACCEPT)\n");
        text.push_str(&format!("  verdict {}\n", self.release.verdict));
        for reason in &self.release.reasons {
            text.push_str(&format!("  refused because: {reason}\n"));
        }
        let counts: Vec<String> = self
            .status_counts
            .iter()
            .map(|(status, n)| format!("{status}={n}"))
            .collect();
        text.push_str(&format!("  obligation status: {}\n", counts.join(" ")));
        text.push_str(&format!(
            "  corpus {} ({} cases), exploration {}, nodes {}\n",
            self.provenance.corpus_digest,
            self.provenance.corpus_cases,
            if self.provenance.exploration_complete {
                "complete within the supplied corpus".to_string()
            } else {
                format!(
                    "INCOMPLETE ({})",
                    self.provenance
                        .completion_reason
                        .as_deref()
                        .unwrap_or("unrecorded")
                )
            },
            self.provenance.explored_nodes
        ));
        for assumption in &self.release.excluded_assumptions {
            text.push_str(&format!("  excluded: {assumption}\n"));
        }
        text
    }
}
