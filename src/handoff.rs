//! `ik handoff`: a delivery, change or recovery summary grounded in cited evidence.
//!
//! The summary is a projection. It never creates a success fact: a behavior outcome is
//! shown as exercised only when it cites evidence ids that resolve to findings in the
//! supplied evidence, and the findings' own scope sentences are printed beside it.
//! Text leads with `Desmo <kind>: <subject>` and a plain-language first block.
//! Cited detail follows under `Exact evidence`. Recovery subject and operation
//! identity come from evidence, or the subject is `subject not identified in evidence`.
//! Scenario-only evidence never establishes readiness.

use crate::handoff_witness::RuleWitnesses;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;

const USAGE: &str = "usage: ik handoff <behavior.json> --evidence <path>... [--kind delivery|change|recovery] [--store <store.sqlite>] [--json]";
const SCHEMA: &str = "ik.handoff.v1";
const KINDS: [&str; 3] = ["delivery", "change", "recovery"];

/// Statuses that contradict a claim that the behavior works.
const CONTRADICTING: [&str; 3] = ["expectation_failed", "violation", "failed"];
/// Typed claims. Evidence status is admissible only for the named claim.
const CLAIMS: [&str; 6] = [
    "reconciled",
    "new_mutation",
    "current_handle",
    "reachable",
    "continuity",
    "declared",
];
const CONTINUITY: &str = "continuity_observation";
const PROFILE: &str = "ik.check.profile.v1";
const SCENARIO_CLASSES: [&str; 2] = ["observed_trace", "continuity_observation"];
const SUBJECT_UNKNOWN: &str = "subject not identified in evidence";
const WHAT_IS_NEW_MISSING: &str = "what is new: not stated in evidence";
const LABEL_DECLARED: &str = "declared in the new definition";
const LABEL_EXPLORED: &str = "explored (bounded search, not a runtime commit)";
const LABEL_OBSERVED: &str = "observed (runtime)";
const OPERATION_ID_SAVE_UNKNOWN: &str = "whether the caller saved the operation id before sending (the kernel records it; the caller must keep its copy)";
const WHY_REQUIRED: &str = "a change handoff needs `why` {text, source}: the requirement that says why this change is made and whether existing instances stay or move";
const RULES_NOT_ESTABLISHED: &str = "rule fitness and version policy: not established (no declarative definition was found in the evidence; these checks apply to declarative machines only)";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Behavior {
    behavior_version: u64,
    title: String,
    #[serde(default)]
    outcomes: Vec<Item>,
    #[serde(default)]
    unchanged: Vec<Item>,
    #[serde(default)]
    assumptions: Vec<String>,
    #[serde(default)]
    decision: Option<Decision>,
    /// The requirement behind this change and where it is stated. Required for
    /// `--kind change`. The product quotes it; it never writes it.
    #[serde(default)]
    why: Option<Why>,
    /// Rules the human accepted, each bound to the guard hash the ledger printed.
    #[serde(default)]
    accepted_rules: Vec<AcceptedRule>,
    /// Examples per rule that needs acceptance. Required for each such rule; the
    /// product checks them and refuses when they are missing or inadequate.
    #[serde(default)]
    rule_witnesses: Vec<RuleWitnesses>,
    /// The requirement text that `origins` spans are quoted from, pinned to its source.
    #[serde(default)]
    requirement: Option<crate::handoff_origin::Requirement>,
    /// Where each decision point of a rule's guard came from, recorded by the author.
    #[serde(default)]
    origins: Vec<crate::handoff_origin::OriginEntry>,
    /// Chosen points the owner has seen and acknowledged, recorded by the agent.
    #[serde(default)]
    acknowledged_origins: Vec<crate::handoff_origin::OriginAck>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Why {
    text: String,
    source: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptedRule {
    rule: String,
    guard_hash: String,
    #[serde(default)]
    examples_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    id: String,
    text: String,
    /// What the item asserts. Omitted: `committed`, `refused`, and
    /// `holds_in_scope` support a live-behavior claim. A lookup/reconciliation
    /// finding source defaults to `reconciled`. A check `witness_found`
    /// finding source defaults to `reachable`.
    #[serde(default)]
    claim: Option<String>,
    #[serde(default)]
    evidence: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    question: String,
    #[serde(default)]
    options: Vec<DecisionOption>,
    #[serde(default)]
    recommended: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionOption {
    label: String,
    consequence: String,
}

struct Invocation {
    behavior: PathBuf,
    evidence: Vec<PathBuf>,
    kind: String,
    json: bool,
    store: Option<PathBuf>,
}

/// CLI entry for `ik handoff`.
pub fn run(args: &[String]) -> i32 {
    let invocation = match parse_args(args) {
        Ok(Some(invocation)) => invocation,
        Ok(None) => {
            println!("{USAGE}");
            return 0;
        }
        Err(message) => return usage_error(&message),
    };
    match build(&invocation) {
        Ok((document, rendered)) => emit(&invocation, &document, &rendered),
        Err(message) => usage_error(&message),
    }
}

fn usage_error(message: &str) -> i32 {
    eprintln!("ik handoff: {message}");
    eprintln!("{USAGE}");
    2
}

fn parse_args(args: &[String]) -> Result<Option<Invocation>, String> {
    let mut behavior: Option<PathBuf> = None;
    let mut evidence = Vec::new();
    let mut kind = "delivery".to_string();
    let mut json = false;
    let mut store: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => return Ok(None),
            "--json" => json = true,
            "--store" => {
                i += 1;
                store = Some(PathBuf::from(args.get(i).ok_or("--store requires a path")?));
            }
            "--kind" => {
                i += 1;
                kind = args.get(i).ok_or("--kind requires a value")?.clone();
            }
            "--evidence" => {
                i += 1;
                evidence.push(PathBuf::from(
                    args.get(i).ok_or("--evidence requires a path")?,
                ));
            }
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            path if behavior.is_none() => behavior = Some(PathBuf::from(path)),
            other => return Err(format!("unexpected argument `{other}`")),
        }
        i += 1;
    }
    if !KINDS.contains(&kind.as_str()) {
        return Err(format!("--kind must be one of {}", KINDS.join(", ")));
    }
    Ok(Some(Invocation {
        behavior: behavior.ok_or("a behavior file is required")?,
        evidence,
        kind,
        json,
        store,
    }))
}

fn load_behavior(path: &PathBuf) -> Result<Behavior, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let behavior: Behavior =
        serde_json::from_str(&text).map_err(|e| format!("behavior {}: {e}", path.display()))?;
    if behavior.behavior_version != 1 {
        return Err("behavior_version must be 1".into());
    }
    for item in behavior.outcomes.iter().chain(behavior.unchanged.iter()) {
        check_claim(item.claim.as_deref())?;
    }
    if let Some(why) = &behavior.why {
        if why.text.trim().is_empty() || why.source.trim().is_empty() {
            return Err("why needs non-empty text and source".into());
        }
    }
    Ok(behavior)
}

fn check_claim(claim: Option<&str>) -> Result<(), String> {
    match claim {
        None => Ok(()),
        Some(name) if CLAIMS.contains(&name) => Ok(()),
        Some(name) => Err(format!(
            "unknown claim `{name}`; claim must be one of {}",
            CLAIMS.join(", ")
        )),
    }
}

fn finding_supports(claim: Option<&str>, finding: &Value) -> bool {
    let status = finding["status"].as_str().unwrap_or("");
    match claim {
        Some("reconciled") => matches!(status, "committed_exact" | "historical"),
        Some("new_mutation") => status == "committed",
        Some("current_handle") => false,
        Some("reachable") => {
            finding["source_kind"].as_str() == Some(PROFILE)
                && status == "witness_found"
                && finding["evidence_class"].as_str() != Some("definition")
        }
        Some("declared") => {
            finding["evidence_class"].as_str() == Some("definition") && status == "declared"
        }
        Some("continuity") => is_continuity_finding(finding),
        None => matches!(status, "committed" | "refused" | "holds_in_scope"),
        Some(_) => false,
    }
}

fn is_reconciliation_finding(finding: &Value) -> bool {
    matches!(
        finding["status"].as_str(),
        Some("committed_exact") | Some("historical")
    )
}

fn is_reachability_finding(finding: &Value) -> bool {
    finding["source_kind"].as_str() == Some(PROFILE)
        && finding["status"].as_str() == Some("witness_found")
        && finding["evidence_class"].as_str() != Some("definition")
}

fn is_continuity_finding(finding: &Value) -> bool {
    finding["evidence_class"].as_str() == Some(CONTINUITY)
        && matches!(
            finding["status"].as_str(),
            Some("committed") | Some("refused")
        )
}

fn effective_claim<'a>(authored: Option<&'a str>, cited: &[Value]) -> Option<&'a str> {
    if authored.is_some() {
        return authored;
    }
    if !cited.is_empty() && cited.iter().all(is_reconciliation_finding) {
        return Some("reconciled");
    }
    if !cited.is_empty() && cited.iter().all(is_reachability_finding) {
        return Some("reachable");
    }
    if !cited.is_empty() && cited.iter().all(is_definition_finding) {
        return Some("declared");
    }
    None
}

fn is_definition_finding(finding: &Value) -> bool {
    finding["evidence_class"].as_str() == Some("definition")
        && finding["status"].as_str() == Some("declared")
}

fn build(invocation: &Invocation) -> Result<(Value, String), String> {
    let behavior = load_behavior(&invocation.behavior)?;
    if invocation.kind == "change" && behavior.why.is_none() {
        return Err(WHY_REQUIRED.into());
    }
    let findings = findings_from_report(&invocation.evidence);
    let outcomes: Vec<Value> = behavior
        .outcomes
        .iter()
        .map(|item| claim(item, &findings))
        .collect();
    let unchanged: Vec<Value> = behavior
        .unchanged
        .iter()
        .map(|item| claim(item, &findings))
        .collect();
    let mut document = base_document(invocation, &behavior, &outcomes, &unchanged, &findings);
    maybe_insert_reading(
        &mut document,
        &invocation.kind,
        &findings,
        &outcomes,
        &unchanged,
    );
    attach_headline(&mut document, &invocation.kind, &behavior.title, &findings);
    attach_diagram(
        &mut document,
        &invocation.kind,
        &invocation.evidence,
        &findings,
    );
    let checked = crate::handoff_witness::check_rules(
        &document["diagram"]["rules"],
        &behavior.rule_witnesses,
    )?;
    let origins = crate::handoff_origin::check(
        &document["diagram"]["rules"],
        behavior.requirement.as_ref(),
        &behavior.origins,
        &behavior.acknowledged_origins,
    )?;
    let census = if invocation.kind == "change" {
        match &invocation.store {
            Some(path) => Some(crate::census::read(path)?),
            None => None,
        }
    } else if invocation.store.is_some() {
        return Err("--store applies only to --kind change".into());
    } else {
        None
    };
    attach_decisions(&mut document, &invocation.kind, &behavior, &checked, census.as_deref());
    let gaps = crate::handoff_gaps::check(
        &invocation.kind,
        &invocation.evidence,
        &findings,
        &document["diagram"]["rules"],
        behavior.requirement.as_ref(),
        &behavior.origins,
        &origins,
    )?;
    if let Some(object) = document.as_object_mut() {
        object.insert("origins".into(), crate::handoff_origin::to_value(&origins, behavior.requirement.as_ref()));
        object.insert("gaps".into(), gaps);
    }
    crate::handoff_frame::attach(&mut document);
    let rendered = render(&document);
    Ok((document, rendered))
}

fn findings_from_report(evidence: &[PathBuf]) -> Vec<Value> {
    let report = crate::report::report_value(evidence);
    report["findings"].as_array().cloned().unwrap_or_default()
}

fn base_document(
    invocation: &Invocation,
    behavior: &Behavior,
    outcomes: &[Value],
    unchanged: &[Value],
    findings: &[Value],
) -> Value {
    json!({
        "schema": SCHEMA,
        "kind": invocation.kind,
        "title": behavior.title,
        "outcomes": outcomes,
        "unchanged": unchanged,
        "assumptions": behavior.assumptions,
        "decision": decision_value(behavior.decision.as_ref()),
        "evidence": evidence_index(findings),
    })
}

fn maybe_insert_reading(
    document: &mut Value,
    kind: &str,
    findings: &[Value],
    outcomes: &[Value],
    unchanged: &[Value],
) {
    let reading = match kind {
        "recovery" => recovery_reading(&cited_findings(outcomes, unchanged)),
        "change" => change_reading(findings),
        _ => return,
    };
    insert_reading(document, reading);
}

fn insert_reading(document: &mut Value, reading: Value) {
    if !reading_has_lines(&reading) {
        return;
    }
    if let Some(object) = document.as_object_mut() {
        object.insert("reading".into(), reading);
    }
}

fn attach_diagram(document: &mut Value, kind: &str, evidence: &[PathBuf], findings: &[Value]) {
    let Some(diagram) = crate::diagram::from_handoff(kind, evidence, findings) else {
        return;
    };
    if let Some(object) = document.as_object_mut() {
        object.insert("diagram".into(), diagram.json_field());
    }
}

fn decision_value(decision: Option<&Decision>) -> Value {
    match decision {
        None => Value::Null,
        Some(d) => json!({
            "question": d.question,
            "options": d.options.iter().map(|o| json!({"label": o.label, "consequence": o.consequence})).collect::<Vec<_>>(),
            "recommended": d.recommended,
        }),
    }
}

fn find<'a>(findings: &'a [Value], id: &str) -> Option<&'a Value> {
    findings.iter().find(|f| f["id"].as_str() == Some(id))
}

fn status_of(
    cited: &[Value],
    missing: usize,
    none_cited: bool,
    claim: Option<&str>,
) -> &'static str {
    if claim == Some("continuity") {
        return continuity_status(cited, missing, none_cited);
    }
    if none_cited {
        return "no_evidence_cited";
    }
    if cited.is_empty() {
        return "cited_evidence_not_found";
    }
    if cited
        .iter()
        .any(|finding| CONTRADICTING.contains(&finding["status"].as_str().unwrap_or("")))
    {
        return "contradicted";
    }
    if missing > 0 || !cited.iter().all(|finding| finding_supports(claim, finding)) {
        return "incomplete";
    }
    "exercised"
}

fn continuity_status(cited: &[Value], missing: usize, none_cited: bool) -> &'static str {
    if cited
        .iter()
        .any(|finding| CONTRADICTING.contains(&finding["status"].as_str().unwrap_or("")))
    {
        return "contradicted";
    }
    if none_cited
        || cited.is_empty()
        || missing > 0
        || !cited
            .iter()
            .all(|finding| finding_supports(Some("continuity"), finding))
    {
        return "continuity not demonstrated";
    }
    "exercised"
}

fn claim(item: &Item, findings: &[Value]) -> Value {
    let cited: Vec<Value> = item
        .evidence
        .iter()
        .filter_map(|id| find(findings, id).cloned())
        .collect();
    let missing: Vec<&String> = item
        .evidence
        .iter()
        .filter(|id| find(findings, id).is_none())
        .collect();
    let claim = effective_claim(item.claim.as_deref(), &cited);
    let status = status_of(&cited, missing.len(), item.evidence.is_empty(), claim);
    let mut body = json!({
        "id": item.id,
        "text": item.text,
        "claim": item.claim,
        "status": status,
        "cited": item.evidence,
        "missing_evidence": missing,
        "findings": cited.iter().map(finding_line).collect::<Vec<_>>(),
    });
    if let Some(name) = claim {
        if item.claim.as_deref() != Some(name) {
            if let Some(object) = body.as_object_mut() {
                object.insert("effective_claim".into(), json!(name));
            }
        }
    }
    body
}

fn finding_line(finding: &Value) -> Value {
    json!({
        "id": finding["id"],
        "status": finding["status"],
        "scope": finding["scope"],
        "committed": finding["committed"],
        "evidence_class": finding["evidence_class"],
        "completeness": finding["completeness"],
    })
}

struct StatusReading {
    status: &'static str,
    established: &'static str,
    uncertain: &'static str,
    safe_next: &'static str,
    priority: u8,
}

const READINGS: &[StatusReading] = &[
    StatusReading {
        status: "committed_exact",
        established: "the earlier operation committed; this lookup did not commit again",
        uncertain: "whether any handle from this lookup is current",
        safe_next: "do not resend; the earlier result stands",
        priority: 40,
    },
    StatusReading {
        status: "historical",
        established: "the earlier operation already committed; this attempt did not commit again",
        uncertain: "whether any returned handle is current",
        safe_next: "do not treat this as a new commit or a current handle",
        priority: 35,
    },
    StatusReading {
        status: "reply_dropped",
        established: "the reply to the earlier attempt was not delivered",
        uncertain: "this dropped-reply record does not by itself confirm the commit",
        safe_next: "look up the earlier operation; do not resend from the dropped reply",
        priority: 30,
    },
    StatusReading {
        status: "conflict",
        established: "a recorded operation exists but does not match this lookup",
        uncertain: "whether this is the intended operation",
        safe_next: "do not treat this as the same operation; inspect the conflicting record",
        priority: 25,
    },
    StatusReading {
        status: "not_recorded",
        established: "no matching recorded operation was found",
        uncertain: "whether the attempt never reached the store or used a different operation id",
        safe_next: "do not treat the earlier attempt as committed from this lookup",
        priority: 20,
    },
    StatusReading {
        status: "not_found",
        established: "no matching recorded operation was found",
        uncertain: "whether the attempt never reached the store or used a different operation id",
        safe_next: "do not treat the earlier attempt as committed from this lookup",
        priority: 20,
    },
    StatusReading {
        status: "not_committed",
        established: "the earlier attempt was not committed",
        uncertain: "nothing further about store history from this dropped reply",
        safe_next: "do not treat a dropped reply as a commit",
        priority: 15,
    },
];

fn reading_for(status: &str) -> Option<&'static StatusReading> {
    READINGS.iter().find(|row| row.status == status)
}

fn push_unique(lines: &mut Vec<String>, line: &str) {
    if !lines.iter().any(|existing| existing == line) {
        lines.push(line.to_string());
    }
}

fn cited_findings(outcomes: &[Value], unchanged: &[Value]) -> Vec<Value> {
    outcomes
        .iter()
        .chain(unchanged.iter())
        .flat_map(|item| item["findings"].as_array().cloned().unwrap_or_default())
        .collect()
}

fn change_reading(findings: &[Value]) -> Value {
    let rows: Vec<&Value> = findings
        .iter()
        .filter(|row| is_continuity_finding(row))
        .collect();
    if rows.is_empty() {
        return json!({
            "established": [],
            "uncertain": ["continuity not demonstrated"],
            "safe_next_action": "existing instances stay on their version; moving one to the new version in place is not supported"
        });
    }
    continuity_reading(&rows)
}

fn continuity_reading(rows: &[&Value]) -> Value {
    let mut established = Vec::new();
    if let Some(version) = first_u64(rows, "created_version") {
        established.push(format!(
            "instances created under version {version} keep version {version}"
        ));
    }
    for name in working_commands(rows) {
        established.push(format!("{name} still works on them"));
    }
    for (name, code) in refused_commands(rows) {
        established.push(format!("{name} is refused on them with {code}"));
    }
    let next = match first_new_version(rows) {
        Some(version) => format!(
            "existing instances stay on their version; moving one to version {version} in place is not supported"
        ),
        None => {
            "existing instances stay on their version; moving one to the new version in place is not supported"
                .into()
        }
    };
    json!({
        "established": established,
        "uncertain": [],
        "safe_next_action": next
    })
}

fn first_u64(rows: &[&Value], key: &str) -> Option<u64> {
    rows.iter()
        .filter_map(|row| row["present"].get(key).and_then(Value::as_u64))
        .next()
}

fn first_new_version(rows: &[&Value]) -> Option<u64> {
    rows.iter().find_map(|row| {
        if row["present"]["presented"].as_str() == Some("new") {
            row["present"]["presented_version"].as_u64()
        } else {
            None
        }
    })
}

fn working_commands(rows: &[&Value]) -> BTreeSet<String> {
    rows.iter()
        .filter(|row| row["status"].as_str() == Some("committed"))
        .filter_map(|row| row["present"]["command"].as_str().map(str::to_string))
        .collect()
}

fn refused_commands(rows: &[&Value]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for row in rows {
        if row["status"].as_str() != Some("refused") {
            continue;
        }
        let Some(name) = row["present"]["command"].as_str() else {
            continue;
        };
        if !seen.insert(name.to_string()) {
            continue;
        }
        let code = row["present"]["code"].as_str().unwrap_or("refusal");
        out.push((name.to_string(), code.to_string()));
    }
    out
}

fn recovery_reading(findings: &[Value]) -> Value {
    let mut established = Vec::new();
    let mut uncertain = Vec::new();
    let mut next = None;
    let mut priority = 0u8;
    for finding in findings {
        let Some(status) = finding["status"].as_str() else {
            continue;
        };
        let Some(row) = reading_for(status) else {
            continue;
        };
        push_unique(&mut established, row.established);
        push_unique(&mut uncertain, row.uncertain);
        if row.priority > priority {
            priority = row.priority;
            next = Some(row.safe_next);
        }
    }
    json!({
        "established": established,
        "uncertain": uncertain,
        "safe_next_action": next.unwrap_or(""),
    })
}

fn reading_has_lines(reading: &Value) -> bool {
    reading["established"]
        .as_array()
        .is_some_and(|rows| !rows.is_empty())
        || reading["uncertain"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
        || reading["safe_next_action"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
}

fn evidence_index(findings: &[Value]) -> Vec<Value> {
    findings
        .iter()
        .map(|f| {
            json!({
                "id": f["id"],
                "source_kind": f["source_kind"],
                "status": f["status"],
                "evidence_class": f["evidence_class"],
                "completeness": f["completeness"],
                "artifacts": f["artifacts"],
            })
        })
        .collect()
}

fn bad_status(status: &str) -> bool {
    matches!(status, "contradicted")
}

fn blocked(document: &Value) -> bool {
    let claims = ["outcomes", "unchanged"]
        .iter()
        .flat_map(|key| document[key].as_array().cloned().unwrap_or_default())
        .any(|c| bad_status(c["status"].as_str().unwrap_or("")));
    let unreadable = document["evidence"]
        .as_array()
        .map(|rows| rows.iter().any(|r| r["status"] == "unreadable"))
        .unwrap_or(false);
    claims || unreadable
}

fn emit(invocation: &Invocation, document: &Value, rendered: &str) -> i32 {
    if invocation.json {
        println!("{}", crate::canonical::canonical_json(document));
    } else {
        print!("{rendered}");
    }
    if blocked(document) {
        1
    } else {
        0
    }
}

fn scenario_only(findings: &[Value]) -> bool {
    if findings.is_empty() {
        return true;
    }
    findings
        .iter()
        .all(|finding| SCENARIO_CLASSES.contains(&finding["evidence_class"].as_str().unwrap_or("")))
}

fn aggregate_from_id(id: &str) -> Option<&str> {
    let rest = id.rsplit_once('#')?.1;
    let (aggregate, step) = rest.split_once('/')?;
    if aggregate.is_empty() || step.is_empty() {
        None
    } else {
        Some(aggregate)
    }
}

fn operation_id_from_finding(finding: &Value) -> Option<String> {
    let attempted = finding["attempted"].as_str()?;
    let value: Value = serde_json::from_str(attempted).ok()?;
    value
        .get("operation_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn first_aggregate(findings: &[Value]) -> Option<String> {
    findings.iter().find_map(|finding| {
        finding["id"]
            .as_str()
            .and_then(aggregate_from_id)
            .map(str::to_string)
    })
}

fn recovery_identity(findings: &[Value]) -> (Option<String>, Option<String>) {
    (
        first_aggregate(findings),
        findings.iter().find_map(operation_id_from_finding),
    )
}

fn attach_headline(document: &mut Value, kind: &str, title: &str, findings: &[Value]) {
    let Some(object) = document.as_object_mut() else {
        return;
    };
    object.insert("scenario_only".into(), json!(scenario_only(findings)));
    // Desmo never states readiness; it is the owner's decision. The key stays so JSON consumers
    // see an explicit non-verdict rather than a missing field.
    object.insert("readiness".into(), json!("not_stated"));
    if kind == "recovery" {
        let (subject, operation) = recovery_identity(findings);
        object.insert(
            "subject".into(),
            json!(subject.unwrap_or_else(|| SUBJECT_UNKNOWN.to_string())),
        );
        if let Some(operation_id) = operation {
            object.insert("operation_id".into(), json!(operation_id));
        }
    } else {
        object.insert("subject".into(), json!(title));
    }
    if kind == "change" {
        object.insert(
            "declared".into(),
            json!(declared_statement(findings).unwrap_or_else(|| WHAT_IS_NEW_MISSING.to_string())),
        );
    }
}

fn declared_statement(findings: &[Value]) -> Option<String> {
    findings.iter().find_map(|finding| {
        if !is_definition_finding(finding) {
            return None;
        }
        finding["scope"]
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

fn claim_class(claim: &Value) -> Option<&str> {
    claim["claim"]
        .as_str()
        .or_else(|| claim["effective_claim"].as_str())
}

fn each_claim(document: &Value) -> Vec<Value> {
    ["outcomes", "unchanged"]
        .iter()
        .flat_map(|key| document[*key].as_array().cloned().unwrap_or_default())
        .collect()
}

fn push_strings(lines: &mut Vec<String>, value: &Value) {
    let Some(rows) = value.as_array() else {
        return;
    };
    for row in rows {
        if let Some(text) = row.as_str() {
            push_unique(lines, text);
        }
    }
}

fn append_block(out: &mut String, title: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    out.push_str(title);
    out.push('\n');
    for line in lines {
        if line.is_empty() {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
}

fn append_labeled(out: &mut String, title: &str, lines: &[String], label: &str) {
    if lines.is_empty() {
        return;
    }
    let mut rows = lines.to_vec();
    push_unique(&mut rows, label);
    append_block(out, title, &rows);
}

struct FirstBlock {
    declared: Vec<String>,
    demonstrated: Vec<String>,
    explored: Vec<String>,
    uncertain: Vec<String>,
    not_established: Vec<String>,
    contradicted: Vec<String>,
    safe_next: Vec<String>,
    unresolved: Vec<String>,
}

fn record_exercised(block: &mut FirstBlock, class: Option<&str>, text: &str) {
    if class == Some("declared") {
        return;
    }
    if class == Some("reachable") {
        push_unique(&mut block.explored, text);
        return;
    }
    if matches!(class, Some("continuity") | Some("reconciled")) {
        if block.demonstrated.is_empty() {
            block.demonstrated.push(text.to_string());
        }
        return;
    }
    push_unique(&mut block.demonstrated, text);
}

fn record_unproven(block: &mut FirstBlock, status: &str, text: &str) {
    match status {
        "contradicted" => block.contradicted.push(text.to_string()),
        "continuity not demonstrated" => {
            push_unique(&mut block.not_established, "continuity not demonstrated");
        }
        "incomplete" | "no_evidence_cited" | "cited_evidence_not_found" => {
            block.not_established.push(format!("{text} ({status})"));
        }
        "" => {}
        other => block.not_established.push(format!("{text} ({other})")),
    }
}

fn record_claim(block: &mut FirstBlock, claim: &Value) {
    let status = claim["status"].as_str().unwrap_or("");
    let text = claim["text"].as_str().unwrap_or("");
    if status == "exercised" {
        record_exercised(block, claim_class(claim), text);
    } else {
        record_unproven(block, status, text);
    }
}

fn empty_first_block() -> FirstBlock {
    FirstBlock {
        declared: Vec::new(),
        demonstrated: Vec::new(),
        explored: Vec::new(),
        uncertain: Vec::new(),
        not_established: Vec::new(),
        contradicted: Vec::new(),
        safe_next: Vec::new(),
        unresolved: Vec::new(),
    }
}

fn absorb_reading(block: &mut FirstBlock, kind: &str, reading: &Value) {
    push_strings(&mut block.demonstrated, &reading["established"]);
    push_strings(&mut block.uncertain, &reading["uncertain"]);
    let Some(next) = reading["safe_next_action"].as_str() else {
        return;
    };
    if next.is_empty() {
        return;
    }
    if kind == "recovery" {
        block.safe_next.push(next.to_string());
    } else {
        push_unique(&mut block.not_established, next);
    }
}

/// Agent-authored statements of what the new definition declares, shown only when each cites
/// a definition finding (status `exercised` under the `declared` claim): declared, never demonstrated.
fn authored_declared_meaning(document: &Value) -> Vec<String> {
    each_claim(document)
        .iter()
        .filter(|claim| {
            claim_class(claim) == Some("declared") && claim["status"].as_str() == Some("exercised")
        })
        .filter_map(|claim| claim["text"].as_str().map(str::to_string))
        .collect()
}

fn evidence_names_saved(document: &Value) -> bool {
    document["evidence"]
        .as_array()
        .is_some_and(|rows| rows.iter().any(|row| row["status"].as_str() == Some("saved")))
}

fn collect_first_block(document: &Value) -> FirstBlock {
    let kind = document["kind"].as_str().unwrap_or("delivery");
    let mut block = empty_first_block();
    absorb_reading(&mut block, kind, &document["reading"]);
    if kind == "change" {
        let declared = document["declared"].as_str().unwrap_or(WHAT_IS_NEW_MISSING);
        push_unique(&mut block.declared, declared);
        for text in authored_declared_meaning(document) {
            push_unique(&mut block.declared, &text);
        }
        push_unique(&mut block.declared, LABEL_DECLARED);
    }
    for claim in each_claim(document) {
        record_claim(&mut block, &claim);
    }
    if kind != "recovery" {
        for line in std::mem::take(&mut block.uncertain) {
            push_unique(&mut block.not_established, &line);
        }
    } else if !evidence_names_saved(document) {
        push_unique(&mut block.not_established, OPERATION_ID_SAVE_UNKNOWN);
    }
    if kind != "recovery" && document["diagram"].is_null() {
        push_unique(&mut block.not_established, RULES_NOT_ESTABLISHED);
    }
    block.unresolved = document["assumptions"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row.as_str().map(str::to_string))
        .collect();
    block
}

fn attach_decisions(
    document: &mut Value,
    kind: &str,
    behavior: &Behavior,
    checked: &[crate::handoff_witness::Checked],
    census: Option<&[crate::census::CensusRow]>,
) {
    let accepted: Vec<crate::handoff_decision::Acceptance> = behavior
        .accepted_rules
        .iter()
        .map(|item| crate::handoff_decision::Acceptance {
            rule: item.rule.clone(),
            guard_hash: item.guard_hash.clone(),
            examples_hash: item.examples_hash.clone(),
        })
        .collect();
    let computed =
        crate::handoff_decision::required(kind, &document["diagram"], &accepted, checked, census);
    let open = !computed.is_empty() || !document["decision"].is_null();
    let Some(object) = document.as_object_mut() else {
        return;
    };
    object.insert("decisions".into(), Value::Array(computed));
    object.insert(
        "decision_status".into(),
        json!(if open { "decision_required" } else { "none" }),
    );
    if let Some(why) = &behavior.why {
        object.insert("why".into(), json!({"text": why.text, "source": why.source}));
    }
}

fn decision_block(document: &Value) -> String {
    let mut out = String::from("Decision needed\n");
    let computed = document["decisions"].as_array().cloned().unwrap_or_default();
    if document["decision"].is_null() && computed.is_empty() {
        let none = if document["diagram"].is_null() && document["kind"] != "recovery" {
            "none computed: no declarative definition was found in the evidence\n\n"
        } else {
            "none\n\n"
        };
        out.push_str(none);
        return out;
    }
    if !document["decision"].is_null() {
        out.push_str(&authored_decision(&document["decision"]));
    }
    for decision in &computed {
        out.push_str(&computed_decision(decision));
    }
    out
}

fn authored_decision(decision: &Value) -> String {
    let mut out = String::new();
    out.push_str(decision["question"].as_str().unwrap_or(""));
    out.push('\n');
    for option in decision["options"].as_array().cloned().unwrap_or_default() {
        out.push_str(option["label"].as_str().unwrap_or(""));
        out.push_str(": ");
        out.push_str(option["consequence"].as_str().unwrap_or(""));
        out.push('\n');
    }
    if let Some(label) = decision["recommended"].as_str() {
        out.push_str("Recommended: ");
        out.push_str(label);
        out.push('\n');
    }
    out.push('\n');
    out
}

const TEXT_WIDTH: usize = 100;

fn push_wrapped(out: &mut String, indent: &str, text: &str) {
    for line in crate::diagram::wrap_text(text, TEXT_WIDTH) {
        out.push_str(indent);
        out.push_str(&line);
        out.push('\n');
    }
}

fn computed_decision(decision: &Value) -> String {
    let mut out = String::new();
    push_wrapped(&mut out, "", decision["question"].as_str().unwrap_or(""));
    for line in decision["basis"].as_array().cloned().unwrap_or_default() {
        push_wrapped(&mut out, "", line.as_str().unwrap_or(""));
    }
    out.push('\n');
    if let Some(rules) = decision["rules"].as_array() {
        out.push_str(&rule_table(rules));
        out.push('\n');
        out.push_str(&rule_examples(rules));
        push_wrapped(
            &mut out,
            "",
            "To decide: tell the agent accept or reject for each rule above. The agent records your decision against exactly this displayed content; you do not copy hashes or edit files. Accepting says the behavior is what the business wants. It does not approve a release.",
        );
    }
    for option in decision["options"].as_array().cloned().unwrap_or_default() {
        push_wrapped(&mut out, "- ", option["label"].as_str().unwrap_or(""));
        push_wrapped(&mut out, "    ", option["consequence"].as_str().unwrap_or(""));
    }
    out.push_str(&version_details(decision));
    for line in decision["not_established"].as_array().cloned().unwrap_or_default() {
        push_wrapped(
            &mut out,
            "",
            &format!("Not established: {}", line.as_str().unwrap_or("")),
        );
    }
    out.push('\n');
    out
}

/// Census, compatibility table and shared differences of a version-policy decision.
fn version_details(decision: &Value) -> String {
    let mut out = String::new();
    for line in decision["found"].as_array().cloned().unwrap_or_default() {
        push_labeled(&mut out, "Found: ", line.as_str().unwrap_or(""));
    }
    let rows: Vec<Vec<String>> = decision["compat_rows"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|row| {
                    row.as_array()
                        .map(|cells| cells.iter().map(|cell| cell.as_str().unwrap_or("").to_string()).collect())
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        return out;
    }
    out.push_str("\nCompatibility of new lifecycles after old work finishes (definition-level only). The checks compare the two definitions; they find differences and do not prove safety.\n\n");
    out.push_str(&crate::diagram::md_table(
        &["state", "version 1 exits", "in the new version", "review"],
        &rows,
    ));
    for line in decision["compat_global"].as_array().cloned().unwrap_or_default() {
        push_wrapped(&mut out, "Applies to every state: ", line.as_str().unwrap_or(""));
    }
    out.push('\n');
    out
}

fn push_labeled(out: &mut String, label: &str, text: &str) {
    let pad = " ".repeat(label.chars().count());
    let width = TEXT_WIDTH.saturating_sub(label.chars().count()).max(20);
    for (index, line) in crate::diagram::wrap_text(text, width).iter().enumerate() {
        out.push_str(if index == 0 { label } else { &pad });
        out.push_str(line);
        out.push('\n');
    }
}

fn rule_examples(rules: &[Value]) -> String {
    let mut out = String::new();
    for rule in rules {
        let name = rule["name"].as_str().unwrap_or("");
        let examples = &rule["examples"];
        if examples.is_null() {
            let note = format!("{name}: examples not checked (the command has several guarded outcomes)");
            push_wrapped(&mut out, "", &note);
            continue;
        }
        out.push_str(name);
        out.push('\n');
        push_labeled(&mut out, "  accepted: ", &list_of(&examples["accepted"]));
        push_labeled(&mut out, "  rejected: ", &list_of(&examples["rejected"]));
        let tried = examples["variants_checked"].as_u64().unwrap_or(0);
        let note = format!(
            "  checked by ik: every stated outcome, every clause toggled, {tried} variants of the rule told apart{}",
            equivalent_note(&examples["declared_equivalent"])
        );
        push_wrapped(&mut out, "", &note);
    }
    out.push('\n');
    out
}

fn list_of(value: &Value) -> String {
    value
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "))
        .unwrap_or_default()
}

fn equivalent_note(value: &Value) -> String {
    let items = list_of(value);
    if items.is_empty() {
        String::new()
    } else {
        format!(" (the author declares these equivalent: {items})")
    }
}

fn rule_table(rules: &[Value]) -> String {
    let rows: Vec<Vec<String>> = rules
        .iter()
        .map(|rule| {
            ["name", "moves", "rule", "acceptance"]
                .iter()
                .map(|key| rule[*key].as_str().unwrap_or("").to_string())
                .collect()
        })
        .collect();
    crate::diagram::md_table(&["rule", "moves", "meaning", "status"], &rows)
}

fn why_lines(document: &Value) -> Vec<String> {
    let why = &document["why"];
    let (Some(text), Some(source)) = (why["text"].as_str(), why["source"].as_str()) else {
        return Vec::new();
    };
    vec![
        text.to_string(),
        format!("stated by the author, source {source}; not verified by evidence"),
    ]
}

fn first_block(document: &Value) -> String {
    let kind = document["kind"].as_str().unwrap_or("delivery");
    let block = collect_first_block(document);
    let mut out = String::new();
    if kind == "change" {
        append_block(&mut out, "New in this version", &block.declared);
        append_block(&mut out, "Why", &why_lines(document));
        append_labeled(
            &mut out,
            "For new instances",
            &block.explored,
            LABEL_EXPLORED,
        );
        append_labeled(
            &mut out,
            "Existing instances",
            &block.demonstrated,
            LABEL_OBSERVED,
        );
    } else {
        append_block(&mut out, "Demonstrated", &block.demonstrated);
        append_labeled(&mut out, "Explored only", &block.explored, LABEL_EXPLORED);
    }
    if kind == "recovery" {
        append_block(&mut out, "Uncertain", &block.uncertain);
    }
    append_block(&mut out, "Contradicted", &block.contradicted);
    append_block(&mut out, "Not established", &block.not_established);
    append_block(&mut out, "Unresolved", &block.unresolved);
    out.push_str(&crate::handoff_frame::text(document));
    if let Some(view) = document["diagram"]["compressed"]
        .as_str()
        .filter(|text| !text.is_empty())
    {
        out.push_str(view);
        if !view.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    } else if let Some(view) = document["diagram"]["route_map"]
        .as_str()
        .filter(|text| !text.is_empty())
    {
        out.push_str("```text\n");
        out.push_str(view);
        if !view.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("```\n\n");
    }
    if kind == "recovery" {
        append_block(&mut out, "Safe next action", &block.safe_next);
        if block.safe_next.iter().any(|next| next.contains("the earlier result stands")) {
            let how = "read the item's current state for a fresh handle; this lookup's result is history, not a handle".to_string();
            append_block(&mut out, "To change the item again", &[how]);
        }
    } else {
        out.push_str(&decision_block(document));
    }
    out
}

fn render(document: &Value) -> String {
    let kind = document["kind"].as_str().unwrap_or("delivery");
    let fallback = if kind == "recovery" {
        SUBJECT_UNKNOWN
    } else {
        document["title"].as_str().unwrap_or("")
    };
    let subject = document["subject"]
        .as_str()
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback);
    let mut out = format!("Desmo {kind}: {subject}\n");
    if document["decision_status"].as_str() == Some("decision_required") {
        out.push_str("Status: DECISION REQUIRED\n");
    }
    if let Some(operation_id) = document["operation_id"].as_str() {
        if !operation_id.is_empty() {
            out.push_str("operation: ");
            out.push_str(operation_id);
            out.push('\n');
        }
    }
    out.push('\n');
    out.push_str(&first_block(document));
    if let Some(table) = document["diagram"]["table_markdown"].as_str() {
        if !table.is_empty() {
            out.push_str(table);
            if !table.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
    }
    out.push_str("Exact evidence\n\n");
    out.push_str(&section(
        "Behavior and what the evidence shows",
        &document["outcomes"],
    ));
    out.push_str(&section(
        "Requirements that were not to change",
        &document["unchanged"],
    ));
    out.push_str(&reading_text(&document["reading"]));
    out.push_str(&assumptions(&document["assumptions"]));
    out.push_str(&decision(&document["decision"]));
    out.push_str(&index(&document["evidence"]));
    out
}

fn section(title: &str, claims: &Value) -> String {
    let rows = claims.as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        return String::new();
    }
    let mut out = format!("## {title}\n\n");
    for claim in &rows {
        out.push_str(&claim_text(claim));
    }
    out.push('\n');
    out
}

fn claim_text(claim: &Value) -> String {
    let kind = match claim["claim"]
        .as_str()
        .or_else(|| claim["effective_claim"].as_str())
    {
        Some(name) => format!(" claim {name}"),
        None => String::new(),
    };
    let mut out = format!(
        "- {} [{}{}]: {}\n",
        claim["id"].as_str().unwrap_or(""),
        claim["status"].as_str().unwrap_or(""),
        kind,
        claim["text"].as_str().unwrap_or("")
    );
    for finding in claim["findings"].as_array().cloned().unwrap_or_default() {
        out.push_str(&format!(
            "    evidence {}: {} ({}); {}\n",
            finding["id"].as_str().unwrap_or(""),
            finding["status"].as_str().unwrap_or(""),
            finding["evidence_class"].as_str().unwrap_or(""),
            finding["scope"].as_str().unwrap_or("")
        ));
    }
    for id in claim["missing_evidence"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        out.push_str(&format!(
            "    cited evidence not found: {}\n",
            id.as_str().unwrap_or("")
        ));
    }
    out
}

fn reading_text(value: &Value) -> String {
    if value.is_null() {
        return String::new();
    }
    let mut out = String::from("## Reading\n\n");
    append_reading_lines(&mut out, "established", &value["established"]);
    append_reading_lines(&mut out, "uncertain", &value["uncertain"]);
    if let Some(next) = value["safe_next_action"].as_str() {
        if !next.is_empty() {
            out.push_str("safe next action: ");
            out.push_str(next);
            out.push('\n');
        }
    }
    out.push('\n');
    out
}

fn append_reading_lines(out: &mut String, label: &str, rows: &Value) {
    let Some(rows) = rows.as_array() else {
        return;
    };
    for row in rows {
        let Some(text) = row.as_str() else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        out.push_str(label);
        out.push_str(": ");
        out.push_str(text);
        out.push('\n');
    }
}

fn assumptions(items: &Value) -> String {
    let rows = items.as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        return String::new();
    }
    let mut out = String::from("## Assumptions\n\n");
    for row in &rows {
        out.push_str(&format!("- {}\n", row.as_str().unwrap_or("")));
    }
    out.push('\n');
    out
}

fn decision(value: &Value) -> String {
    if value.is_null() {
        return String::new();
    }
    let mut out = format!(
        "## Decision needed\n\n{}\n\n",
        value["question"].as_str().unwrap_or("")
    );
    for option in value["options"].as_array().cloned().unwrap_or_default() {
        out.push_str(&format!(
            "- {}: {}\n",
            option["label"].as_str().unwrap_or(""),
            option["consequence"].as_str().unwrap_or("")
        ));
    }
    if let Some(label) = value["recommended"].as_str() {
        out.push_str(&format!("Recommended: {label}\n"));
    }
    out.push('\n');
    out
}

fn index(rows: &Value) -> String {
    let rows = rows.as_array().cloned().unwrap_or_default();
    let mut out = String::from("## Evidence index\n\n");
    if rows.is_empty() {
        out.push_str("No evidence was supplied.\n");
    }
    for row in &rows {
        out.push_str(&format!(
            "- {} | {} | {} | {} | {}\n",
            row["id"].as_str().unwrap_or(""),
            row["source_kind"].as_str().unwrap_or(""),
            row["status"].as_str().unwrap_or(""),
            row["evidence_class"].as_str().unwrap_or(""),
            row["completeness"].as_str().unwrap_or("")
        ));
    }
    out
}
