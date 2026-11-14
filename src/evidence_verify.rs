//! Recompute recorded evidence identities and refuse a stale or foreign machine version.
//!
//! A goal id is not enough. A different spec hash is a different machine: nothing here
//! rebinds stored state or migrates an old aggregate onto a new definition.

use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

pub const SCHEMA: &str = "ik.evidence.verify.v1";
pub const NO_REBIND: &str = "no rebind or migration occurs: evidence for a new definition can never satisfy a persisted old-identity claim, and a persisted old identity cannot satisfy a new definition";

const SCENARIO_SCHEMA: &str = "ik.scenario.result.v1";
const ANALYSIS_PROTOCOL: &str = "ik.analysis.result.v1";
const HOST_REPORT: &str = "ik.analysis.host-report.v1";

#[derive(Clone, Debug)]
pub struct CurrentView {
    pub machine_id: String,
    pub machine_version: u64,
    pub spec_hash: String,
    pub profile_digest: Option<String>,
    pub goal_digests: BTreeMap<String, String>,
    pub corpus_digest: Option<String>,
    pub scenario_digest: Option<String>,
    pub runtime_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct VerifyReport {
    pub schema: &'static str,
    pub overall: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub axes: BTreeMap<String, AxisStatus>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AxisStatus {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mismatches: Vec<FieldMismatch>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FieldMismatch {
    pub field: String,
    pub recorded: String,
    pub current: String,
}

struct Recorded {
    spec_hashes: Vec<FieldValue>,
    machine_ids: Vec<FieldValue>,
    machine_versions: Vec<FieldValue>,
    profile_digest: Option<String>,
    goal_digests: BTreeMap<String, String>,
    goal_ids_without_digest: Vec<String>,
    corpus_digests: Vec<FieldValue>,
    scenario_digest: Option<String>,
    runtime_digests: Vec<FieldValue>,
}

struct FieldValue {
    field: String,
    value: String,
}

pub fn verify(evidence: &Value, current: &CurrentView) -> Result<VerifyReport, String> {
    let recorded = extract(evidence)?;
    let machine = compare_machine(&recorded, current);
    let profile = compare_profile(&recorded, current)?;
    let corpus = compare_corpus(&recorded, current);
    let scenario = compare_scenario(&recorded, current);
    let runtime = compare_runtime(&recorded, current);
    Ok(assemble(machine, profile, corpus, scenario, runtime))
}

pub fn render_text(report: &VerifyReport) -> String {
    let mut text = String::from("EVIDENCE\n");
    text.push_str(&format!("  overall {}\n", report.overall));
    if let Some(note) = &report.note {
        text.push_str(&format!("  {note}\n"));
    }
    for (name, axis) in &report.axes {
        append_axis(&mut text, name, axis);
    }
    text
}

fn extract(evidence: &Value) -> Result<Recorded, String> {
    if text_at(evidence, &["schema"]) == Some(SCENARIO_SCHEMA) {
        return Ok(scenario_recorded(evidence));
    }
    match text_at(evidence, &["protocol"]) {
        Some(ANALYSIS_PROTOCOL) => Ok(analysis_recorded(evidence)),
        Some(HOST_REPORT) => Err(
            "a host report does not carry subject identity; pass the ik.analysis.result.v1 document"
                .into(),
        ),
        _ if evidence.get("report").and_then(|r| r.get("spec_hash")).and_then(Value::as_str).is_some() => {
            Ok(check_recorded(evidence))
        }
        _ => Err(
            "unrecognized evidence; expected ik check --json, ik.scenario.result.v1, or ik.analysis.result.v1"
                .into(),
        ),
    }
}

fn scenario_recorded(evidence: &Value) -> Recorded {
    let mut recorded = Recorded::empty();
    push_text(
        &mut recorded.spec_hashes,
        "spec_hash",
        evidence.get("spec_hash"),
    );
    push_text(
        &mut recorded.machine_ids,
        "machine_id",
        evidence.get("machine_id"),
    );
    recorded.scenario_digest = evidence
        .get("scenario_digest")
        .and_then(Value::as_str)
        .map(str::to_string);
    push_text(
        &mut recorded.runtime_digests,
        "binary_sha256",
        evidence.pointer("/runtime/binary_sha256"),
    );
    recorded
}

fn analysis_recorded(evidence: &Value) -> Recorded {
    let mut recorded = Recorded::empty();
    push_text(
        &mut recorded.spec_hashes,
        "spec_hash",
        evidence.pointer("/subject/identity/spec_hash"),
    );
    push_text(
        &mut recorded.machine_ids,
        "machine_id",
        evidence.pointer("/subject/identity/machine_id"),
    );
    push_version(
        &mut recorded.machine_versions,
        "machine_version",
        evidence.pointer("/subject/identity/machine_version"),
    );
    recorded.profile_digest = evidence
        .get("profile_digest")
        .and_then(Value::as_str)
        .map(str::to_string);
    take_goals(&mut recorded, evidence.get("goals"));
    recorded
}

fn check_recorded(evidence: &Value) -> Recorded {
    let mut recorded = Recorded::empty();
    push_text(
        &mut recorded.spec_hashes,
        "spec_hash",
        evidence.pointer("/report/spec_hash"),
    );
    push_text(
        &mut recorded.spec_hashes,
        "spec_hash",
        evidence.pointer("/release/provenance/spec_hash"),
    );
    push_text(
        &mut recorded.spec_hashes,
        "machine_spec_hash",
        evidence.pointer("/profile/machine_spec_hash"),
    );
    push_text(
        &mut recorded.machine_ids,
        "machine_id",
        evidence.pointer("/report/machine_id"),
    );
    push_text(
        &mut recorded.machine_ids,
        "machine_id",
        evidence.pointer("/release/provenance/machine_id"),
    );
    push_text(
        &mut recorded.corpus_digests,
        "corpus_digest",
        evidence.pointer("/profile/corpus_digest"),
    );
    push_text(
        &mut recorded.corpus_digests,
        "corpus_digest",
        evidence.pointer("/release/provenance/corpus_digest"),
    );
    push_text(
        &mut recorded.runtime_digests,
        "runtime_artifact_digest",
        evidence.pointer("/release/provenance/runtime_artifact_digest"),
    );
    if let Some(profile) = evidence.get("profile").filter(|value| value.is_object()) {
        recorded.profile_digest = profile
            .get("profile_digest")
            .and_then(Value::as_str)
            .map(str::to_string);
        take_goals(&mut recorded, profile.get("goals"));
    }
    recorded
}

fn take_goals(recorded: &mut Recorded, goals: Option<&Value>) {
    let Some(goals) = goals.and_then(Value::as_array) else {
        return;
    };
    for goal in goals {
        let Some(id) = goal.get("id").and_then(Value::as_str) else {
            continue;
        };
        match goal.get("content_digest").and_then(Value::as_str) {
            Some(digest) => {
                recorded
                    .goal_digests
                    .insert(id.to_string(), digest.to_string());
            }
            None => recorded.goal_ids_without_digest.push(id.to_string()),
        }
    }
}

fn compare_machine(recorded: &Recorded, current: &CurrentView) -> AxisStatus {
    if recorded.spec_hashes.is_empty()
        && recorded.machine_ids.is_empty()
        && recorded.machine_versions.is_empty()
    {
        return not_recorded();
    }
    let mut mismatches = Vec::new();
    push_unequal(&mut mismatches, &recorded.spec_hashes, &current.spec_hash);
    push_unequal(
        &mut mismatches,
        &recorded.machine_versions,
        &current.machine_version.to_string(),
    );
    push_unequal(&mut mismatches, &recorded.machine_ids, &current.machine_id);
    axis_from(mismatches, &["spec_hash", "machine_version", "machine_id"])
}

fn compare_profile(recorded: &Recorded, current: &CurrentView) -> Result<AxisStatus, String> {
    if !profile_recorded(recorded) {
        return Ok(not_recorded());
    }
    if current.profile_digest.is_none() {
        return Err(
            "evidence records a profile digest or goal content digest; pass --profile".into(),
        );
    }
    let mut mismatches = Vec::new();
    if let Some(digest) = &recorded.profile_digest {
        if current.profile_digest.as_deref() != Some(digest.as_str()) {
            mismatches.push(mismatch(
                "profile_digest",
                digest,
                current.profile_digest.as_deref().unwrap_or(""),
            ));
        }
    }
    for (id, digest) in &recorded.goal_digests {
        let field = format!("goals.{id}.content_digest");
        match current.goal_digests.get(id) {
            Some(current_digest) if current_digest == digest => {}
            Some(current_digest) => mismatches.push(mismatch(&field, digest, current_digest)),
            None => mismatches.push(mismatch(&field, digest, "(absent)")),
        }
    }
    for id in &recorded.goal_ids_without_digest {
        mismatches.push(mismatch(
            &format!("goals.{id}.content_digest"),
            "(not recorded)",
            current
                .goal_digests
                .get(id)
                .map(String::as_str)
                .unwrap_or("(absent)"),
        ));
    }
    Ok(axis_from(mismatches, &["content_digest", "profile_digest"]))
}

fn compare_corpus(recorded: &Recorded, current: &CurrentView) -> AxisStatus {
    if recorded.corpus_digests.is_empty() {
        return not_recorded();
    }
    let Some(digest) = current.corpus_digest.as_deref() else {
        return stale_missing("corpus_digest", &recorded.corpus_digests[0].value);
    };
    let mut mismatches = Vec::new();
    push_unequal(&mut mismatches, &recorded.corpus_digests, digest);
    axis_from(mismatches, &["corpus_digest"])
}

fn compare_scenario(recorded: &Recorded, current: &CurrentView) -> AxisStatus {
    let Some(digest) = &recorded.scenario_digest else {
        return not_recorded();
    };
    match current.scenario_digest.as_deref() {
        Some(current) if current == digest => current_axis(),
        Some(current) => stale_one("scenario_digest", digest, current),
        None => stale_missing("scenario_digest", digest),
    }
}

fn compare_runtime(recorded: &Recorded, current: &CurrentView) -> AxisStatus {
    if recorded.runtime_digests.is_empty() {
        return not_recorded();
    }
    let Some(digest) = current.runtime_sha256.as_deref() else {
        return not_recorded();
    };
    let mut mismatches = Vec::new();
    push_unequal(&mut mismatches, &recorded.runtime_digests, digest);
    axis_from(mismatches, &["binary_sha256", "runtime_artifact_digest"])
}

fn assemble(
    machine: AxisStatus,
    profile: AxisStatus,
    corpus: AxisStatus,
    scenario: AxisStatus,
    runtime: AxisStatus,
) -> VerifyReport {
    let overall = overall_of(&machine, &profile, &corpus, &scenario, &runtime);
    let note = (overall == "different_machine_version").then(|| NO_REBIND.to_string());
    let mut axes = BTreeMap::new();
    axes.insert("corpus".into(), corpus);
    axes.insert("machine".into(), machine);
    axes.insert("profile".into(), profile);
    axes.insert("runtime".into(), runtime);
    axes.insert("scenario".into(), scenario);
    VerifyReport {
        schema: SCHEMA,
        overall,
        note,
        axes,
    }
}

fn overall_of(
    machine: &AxisStatus,
    profile: &AxisStatus,
    corpus: &AxisStatus,
    scenario: &AxisStatus,
    runtime: &AxisStatus,
) -> String {
    if version_mismatch(machine) {
        return "different_machine_version".into();
    }
    if machine.status == "stale" {
        return "stale_machine".into();
    }
    if profile.status == "stale" {
        return "stale_profile".into();
    }
    if corpus.status == "stale" {
        return "stale_corpus".into();
    }
    if scenario.status == "stale" {
        return "stale_scenario".into();
    }
    if runtime.status == "stale" {
        return "stale_runtime".into();
    }
    "current".into()
}

fn version_mismatch(machine: &AxisStatus) -> bool {
    machine.mismatches.iter().any(|item| {
        item.field == "spec_hash"
            || item.field == "machine_spec_hash"
            || item.field == "machine_version"
    })
}

fn profile_recorded(recorded: &Recorded) -> bool {
    recorded.profile_digest.is_some()
        || !recorded.goal_digests.is_empty()
        || !recorded.goal_ids_without_digest.is_empty()
}

fn push_unequal(out: &mut Vec<FieldMismatch>, recorded: &[FieldValue], current: &str) {
    for item in recorded {
        if item.value != current {
            out.push(mismatch(&item.field, &item.value, current));
        }
    }
}

fn axis_from(mismatches: Vec<FieldMismatch>, preferred: &[&str]) -> AxisStatus {
    if mismatches.is_empty() {
        return current_axis();
    }
    let field = preferred_field(&mismatches, preferred);
    AxisStatus {
        status: "stale".into(),
        field,
        mismatches,
    }
}

fn preferred_field(mismatches: &[FieldMismatch], preferred: &[&str]) -> Option<String> {
    for name in preferred {
        if let Some(item) = mismatches.iter().find(|item| item.field.contains(name)) {
            return Some(item.field.clone());
        }
    }
    mismatches.first().map(|item| item.field.clone())
}

fn stale_one(field: &str, recorded: &str, current: &str) -> AxisStatus {
    AxisStatus {
        status: "stale".into(),
        field: Some(field.into()),
        mismatches: vec![mismatch(field, recorded, current)],
    }
}

fn stale_missing(field: &str, recorded: &str) -> AxisStatus {
    stale_one(field, recorded, "(not supplied)")
}

fn current_axis() -> AxisStatus {
    AxisStatus {
        status: "current".into(),
        field: None,
        mismatches: Vec::new(),
    }
}

fn not_recorded() -> AxisStatus {
    AxisStatus {
        status: "not_recorded".into(),
        field: None,
        mismatches: Vec::new(),
    }
}

fn mismatch(field: &str, recorded: &str, current: &str) -> FieldMismatch {
    FieldMismatch {
        field: field.into(),
        recorded: recorded.into(),
        current: current.into(),
    }
}

fn append_axis(text: &mut String, name: &str, axis: &AxisStatus) {
    text.push_str(&format!("  {name} {}", axis.status));
    if let Some(field) = &axis.field {
        text.push_str(&format!(" field={field}"));
    }
    text.push('\n');
    for item in &axis.mismatches {
        text.push_str(&format!(
            "    field={} recorded={} current={}\n",
            item.field, item.recorded, item.current
        ));
    }
}

fn text_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor.as_str()
}

fn push_text(out: &mut Vec<FieldValue>, field: &str, value: Option<&Value>) {
    let Some(text) = value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    else {
        return;
    };
    if out
        .iter()
        .any(|item| item.field == field && item.value == text)
    {
        return;
    }
    out.push(FieldValue {
        field: field.into(),
        value: text.into(),
    });
}

fn push_version(out: &mut Vec<FieldValue>, field: &str, value: Option<&Value>) {
    let Some(number) = value.and_then(Value::as_u64) else {
        return;
    };
    out.push(FieldValue {
        field: field.into(),
        value: number.to_string(),
    });
}

impl Recorded {
    fn empty() -> Self {
        Self {
            spec_hashes: Vec::new(),
            machine_ids: Vec::new(),
            machine_versions: Vec::new(),
            profile_digest: None,
            goal_digests: BTreeMap::new(),
            goal_ids_without_digest: Vec::new(),
            corpus_digests: Vec::new(),
            scenario_digest: None,
            runtime_digests: Vec::new(),
        }
    }
}
