//! `ik handoff` renders claims only from cited, resolvable evidence.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ik"))
        .args(args)
        .output()
        .expect("spawn ik")
}

fn arg(path: &Path) -> String {
    path.display().to_string()
}

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ik-handoff-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

fn machine() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/publish-approval/machines/publish_approval_v3.ik.toml")
}

fn relative_machine(dir: &Path) -> String {
    let from = dir.canonicalize().expect("dir");
    let to = machine().canonicalize().expect("machine");
    let mut ups = PathBuf::new();
    let mut base = from.as_path();
    loop {
        if let Ok(rest) = to.strip_prefix(base) {
            return ups.join(rest).to_string_lossy().replace('\\', "/");
        }
        ups.push("..");
        base = base.parent().expect("relative machine path");
    }
}

/// Runs a scenario whose last step expects `expect_stale`, returning the bundle directory.
fn bundle(dir: &Path, expect_stale: &str) -> PathBuf {
    let scenario = dir.join("scenario.json");
    let steps = json!([
        {"kind": "command", "label": "edit", "name": "edit", "args": {"artifact_digest": "artifact-1"}, "handle": "cap-edit", "expect": "committed"},
        {"kind": "command", "label": "again", "name": "edit", "args": {"artifact_digest": "artifact-2"}, "expect": "committed"},
        {"kind": "command", "label": "stale", "name": "edit", "args": {"artifact_digest": "artifact-3"}, "use_handle": "cap-edit", "expect": expect_stale}
    ]);
    let body = json!({
        "scenario_version": 1,
        "machine": relative_machine(dir),
        "aggregate": "handoff-agg",
        "steps": steps,
    });
    fs::write(&scenario, serde_json::to_vec(&body).expect("scenario")).expect("write");
    let out = dir.join("bundle");
    run(&[
        "scenario",
        "run",
        &arg(&machine()),
        &arg(&scenario),
        "--out",
        &arg(&out),
    ]);
    out
}

fn ids(bundle: &Path) -> Vec<String> {
    let output = run(&["report", &arg(bundle), "--json"]);
    let doc: Value = serde_json::from_slice(&output.stdout).expect("report json");
    doc["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| f["id"].as_str().expect("id").to_string())
        .collect()
}

fn id_ending(all: &[String], suffix: &str) -> String {
    all.iter()
        .find(|id| id.ends_with(suffix))
        .unwrap_or_else(|| panic!("no id ending {suffix} in {all:?}"))
        .clone()
}

fn handoff(dir: &Path, behavior: &Value, bundle: &Path) -> (Option<i32>, Value, String) {
    let path = dir.join("behavior.json");
    let behavior = &prepared(behavior, "delivery", bundle);
    fs::write(&path, serde_json::to_vec(behavior).expect("behavior")).expect("write behavior");
    let json_out = run(&["handoff", &arg(&path), "--evidence", &arg(bundle), "--json"]);
    let text_out = run(&["handoff", &arg(&path), "--evidence", &arg(bundle)]);
    let doc = serde_json::from_slice(&json_out.stdout).unwrap_or(Value::Null);
    (
        json_out.status.code(),
        doc,
        String::from_utf8_lossy(&text_out.stdout).into_owned(),
    )
}

fn claim_status<'a>(doc: &'a Value, group: &str, id: &str) -> &'a str {
    doc[group]
        .as_array()
        .expect("group")
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no claim {id} in {doc}"))["status"]
        .as_str()
        .expect("status")
}

#[test]
fn cited_committed_and_refused_steps_are_exercised_with_their_scope() {
    let dir = temp_dir("exercised");
    let bundle = bundle(&dir, "refused:IK_CAPABILITY_STALE");
    let all = ids(&bundle);
    let behavior = json!({
        "behavior_version": 1,
        "title": "edit flow",
        "outcomes": [
            {"id": "B1", "text": "a second edit commits", "evidence": [id_ending(&all, "/again")]},
            {"id": "B2", "text": "a stale handle is refused", "evidence": [id_ending(&all, "/stale")]}
        ]
    });
    let (code, doc, text) = handoff(&dir, &behavior, &bundle);
    assert_eq!(code, Some(0));
    assert_eq!(claim_status(&doc, "outcomes", "B1"), "exercised");
    assert_eq!(claim_status(&doc, "outcomes", "B2"), "exercised");
    assert!(text.contains("IK_CAPABILITY_STALE"));
    assert!(text.contains("the proposal was not committed"));
}

#[test]
fn claims_without_resolvable_evidence_are_never_exercised() {
    let dir = temp_dir("uncited");
    let bundle = bundle(&dir, "refused:IK_CAPABILITY_STALE");
    let behavior = json!({
        "behavior_version": 1,
        "title": "edit flow",
        "outcomes": [
            {"id": "B1", "text": "works", "evidence": []},
            {"id": "B2", "text": "also works", "evidence": ["no-such-finding"]}
        ]
    });
    let (code, doc, text) = handoff(&dir, &behavior, &bundle);
    assert_eq!(code, Some(0));
    assert_eq!(claim_status(&doc, "outcomes", "B1"), "no_evidence_cited");
    assert_eq!(
        claim_status(&doc, "outcomes", "B2"),
        "cited_evidence_not_found"
    );
    assert!(text.contains("cited evidence not found: no-such-finding"));
    assert_eq!(text.matches("[exercised]").count(), 0);
}

#[test]
fn a_failed_expectation_contradicts_the_claim_and_blocks_the_exit_code() {
    let dir = temp_dir("contradicted");
    let bundle = bundle(&dir, "committed");
    let all = ids(&bundle);
    let behavior = json!({
        "behavior_version": 1,
        "title": "edit flow",
        "unchanged": [
            {"id": "U1", "text": "a stale handle still commits", "evidence": [id_ending(&all, "/stale")]}
        ]
    });
    let (code, doc, _) = handoff(&dir, &behavior, &bundle);
    assert_eq!(claim_status(&doc, "unchanged", "U1"), "contradicted");
    assert_eq!(code, Some(1));
}

#[test]
fn unreadable_evidence_is_listed_and_blocks() {
    let dir = temp_dir("unreadable");
    let missing = dir.join("nothing-here");
    let behavior = json!({"behavior_version": 1, "title": "t"});
    let path = dir.join("behavior.json");
    fs::write(&path, serde_json::to_vec(&behavior).expect("behavior")).expect("write");
    let output = run(&[
        "handoff",
        &arg(&path),
        "--evidence",
        &arg(&missing),
        "--json",
    ]);
    let doc: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(doc["evidence"][0]["status"], "unreadable");
}

#[test]
fn a_decision_is_rendered_without_choosing_for_the_owner() {
    let dir = temp_dir("decision");
    let bundle = bundle(&dir, "refused:IK_CAPABILITY_STALE");
    let behavior = json!({
        "behavior_version": 1,
        "title": "edit flow",
        "decision": {
            "question": "Should a stale edit be refused or merged?",
            "options": [
                {"label": "refuse", "consequence": "the editor reloads"},
                {"label": "merge", "consequence": "needs a merge rule"}
            ]
        }
    });
    let (code, doc, text) = handoff(&dir, &behavior, &bundle);
    assert_eq!(code, Some(0));
    assert!(doc["decision"]["recommended"].is_null());
    assert!(text.contains("Decision needed"));
    assert_eq!(text.matches("Recommended:").count(), 0);
}

#[test]
fn malformed_behavior_files_are_usage_errors() {
    let dir = temp_dir("malformed");
    let path = dir.join("behavior.json");
    fs::write(&path, r#"{"behavior_version": 2, "title": "t"}"#).expect("write");
    let versioned = run(&["handoff", &arg(&path)]);
    fs::write(
        &path,
        r#"{"behavior_version": 1, "title": "t", "extra": 1}"#,
    )
    .expect("write");
    let extra = run(&["handoff", &arg(&path)]);
    let kind = run(&["handoff", &arg(&path), "--kind", "nonsense"]);
    fs::write(
        &path,
        r#"{"behavior_version":1,"title":"t","outcomes":[{"id":"B1","text":"x","claim":"not-a-claim"}]}"#,
    )
    .expect("write");
    let claim = run(&["handoff", &arg(&path)]);
    assert_eq!(versioned.status.code(), Some(2));
    assert_eq!(extra.status.code(), Some(2));
    assert_eq!(kind.status.code(), Some(2));
    assert_eq!(claim.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&claim.stderr).contains("unknown claim `not-a-claim`"));
}

fn write_scenario_result(dir: &Path, outcome: &str) -> (PathBuf, String) {
    let path = dir.join("result.json");
    let body = json!({
        "schema": "ik.scenario.result.v1",
        "aggregate": "item",
        "steps": [{
            "label": "step",
            "verdict": "pass",
            "response": { "outcome": outcome }
        }]
    });
    fs::write(&path, serde_json::to_vec(&body).expect("result")).expect("write result");
    let id = format!("{}#item/step", path.display());
    (path, id)
}

#[test]
fn evidence_admissibility_is_claim_specific() {
    let cases: &[(&str, Option<&str>, &str)] = &[
        ("committed", None, "exercised"),
        ("refused", None, "exercised"),
        ("holds_in_scope", None, "exercised"),
        ("committed_exact", None, "exercised"),
        ("historical", None, "exercised"),
        ("committed_exact", Some("reconciled"), "exercised"),
        ("historical", Some("reconciled"), "exercised"),
        ("committed", Some("reconciled"), "incomplete"),
        ("refused", Some("reconciled"), "incomplete"),
        ("committed", Some("new_mutation"), "exercised"),
        ("committed_exact", Some("new_mutation"), "incomplete"),
        ("historical", Some("new_mutation"), "incomplete"),
        ("refused", Some("new_mutation"), "incomplete"),
        ("committed_exact", Some("current_handle"), "incomplete"),
        ("historical", Some("current_handle"), "incomplete"),
        ("committed", Some("current_handle"), "incomplete"),
        ("refused", Some("current_handle"), "incomplete"),
        ("witness_found", None, "incomplete"),
        ("witness_found", Some("reachable"), "incomplete"),
        ("committed", Some("reachable"), "incomplete"),
        ("committed_exact", Some("reachable"), "incomplete"),
        ("witness_found", Some("new_mutation"), "incomplete"),
        ("witness_found", Some("reconciled"), "incomplete"),
        ("witness_found", Some("current_handle"), "incomplete"),
        (
            "committed",
            Some("continuity"),
            "continuity not demonstrated",
        ),
        ("refused", Some("continuity"), "continuity not demonstrated"),
        (
            "witness_found",
            Some("continuity"),
            "continuity not demonstrated",
        ),
        (
            "committed_exact",
            Some("continuity"),
            "continuity not demonstrated",
        ),
    ];
    for (i, (outcome, claim, expected)) in cases.iter().enumerate() {
        let dir = temp_dir(&format!("claim-{i}"));
        let (bundle, id) = write_scenario_result(&dir, outcome);
        let mut item = json!({
            "id": "B1",
            "text": "claim text",
            "evidence": [id]
        });
        if let Some(claim) = claim {
            item["claim"] = json!(claim);
        }
        let behavior = json!({
            "behavior_version": 1,
            "title": "claim matrix",
            "outcomes": [item]
        });
        let (code, doc, _) = handoff(&dir, &behavior, &bundle);
        assert_eq!(code, Some(0), "outcome={outcome} claim={claim:?}");
        assert_eq!(
            claim_status(&doc, "outcomes", "B1"),
            *expected,
            "outcome={outcome} claim={claim:?} doc={doc}"
        );
        if *outcome == "committed_exact" && claim.is_none() {
            assert_eq!(doc["outcomes"][0]["effective_claim"], "reconciled");
            assert!(doc["outcomes"][0]["claim"].is_null());
        }
    }
}

fn write_profile_result(dir: &Path, status: &str) -> (PathBuf, String) {
    let path = dir.join("check.json");
    let body = json!({
        "verdict": "ACCEPT",
        "profile": {
            "schema": "ik.check.profile.v1",
            "search_complete": true,
            "goals": [{
                "id": "tagged",
                "kind": "reach_terminal",
                "status": status
            }]
        }
    });
    fs::write(&path, serde_json::to_vec(&body).expect("check")).expect("write check");
    let id = format!("{}#tagged", path.display());
    (path, id)
}

fn handoff_kind(
    dir: &Path,
    behavior: &Value,
    bundle: &Path,
    kind: &str,
) -> (Option<i32>, Value, String) {
    let path = dir.join("behavior.json");
    let behavior = &prepared(behavior, kind, bundle);
    fs::write(&path, serde_json::to_vec(behavior).expect("behavior")).expect("write behavior");
    let json_out = run(&[
        "handoff",
        &arg(&path),
        "--evidence",
        &arg(bundle),
        "--kind",
        kind,
        "--json",
    ]);
    let text_out = run(&[
        "handoff",
        &arg(&path),
        "--evidence",
        &arg(bundle),
        "--kind",
        kind,
    ]);
    let doc = serde_json::from_slice(&json_out.stdout).unwrap_or(Value::Null);
    (
        json_out.status.code(),
        doc,
        String::from_utf8_lossy(&text_out.stdout).into_owned(),
    )
}

#[test]
fn check_witness_found_is_exercised_only_for_reachable() {
    let cases: &[(Option<&str>, &str)] = &[
        (None, "exercised"),
        (Some("reachable"), "exercised"),
        (Some("new_mutation"), "incomplete"),
        (Some("reconciled"), "incomplete"),
        (Some("current_handle"), "incomplete"),
    ];
    for (i, (claim, expected)) in cases.iter().enumerate() {
        let dir = temp_dir(&format!("reachable-{i}"));
        let (bundle, id) = write_profile_result(&dir, "witness_found");
        let mut item = json!({
            "id": "B2",
            "text": "the checked scenarios are reachable",
            "evidence": [id]
        });
        if let Some(claim) = claim {
            item["claim"] = json!(claim);
        }
        let behavior = json!({
            "behavior_version": 1,
            "title": "check delivery",
            "outcomes": [item]
        });
        let (code, doc, text) = handoff(&dir, &behavior, &bundle);
        assert_eq!(code, Some(0), "claim={claim:?}");
        assert_eq!(
            claim_status(&doc, "outcomes", "B2"),
            *expected,
            "claim={claim:?} doc={doc}"
        );
        if claim.is_none() {
            assert_eq!(doc["outcomes"][0]["effective_claim"], "reachable");
            assert!(text.contains("[exercised claim reachable]"));
            assert_eq!(text.matches("[incomplete]").count(), 0);
            assert!(text.contains("Explored only"));
            assert!(text.contains("bounded search, not a runtime commit"));
            assert!(text.contains("Exact evidence"));
            assert!(text.contains("Readiness: the owner's decision; this report does not state it."));
            assert!(text.starts_with("Desmo delivery: check delivery\n"));
        }
        if *claim == Some("new_mutation") {
            assert!(text.contains("[incomplete"));
            assert_eq!(text.matches("[exercised]").count(), 0);
        }
    }
}

#[test]
fn recovery_handoff_states_established_uncertain_and_safe_next() {
    let dir = temp_dir("recovery-reading");
    let (bundle, id) = write_scenario_result(&dir, "committed_exact");
    let behavior = json!({
        "behavior_version": 1,
        "title": "lost reply is history",
        "outcomes": [{
            "id": "R1",
            "text": "lookup of the dropped commit is committed_exact",
            "evidence": [id]
        }]
    });
    let (code, doc, text) = handoff_kind(&dir, &behavior, &bundle, "recovery");
    assert_eq!(code, Some(0));
    assert_eq!(claim_status(&doc, "outcomes", "R1"), "exercised");
    assert_eq!(doc["outcomes"][0]["effective_claim"], "reconciled");
    assert_eq!(doc["subject"], "item");
    assert_eq!(doc["readiness"], "not_stated");
    assert!(text.starts_with("Desmo recovery: item\n"));
    assert!(text.contains("[exercised claim reconciled]"));
    assert_eq!(text.matches("[incomplete]").count(), 0);
    assert!(text.contains("established:"));
    assert!(text.contains("uncertain:"));
    assert!(text.contains("safe next action:"));
    assert!(text.contains("do not resend; the earlier result stands"));
    assert!(text.contains("Exact evidence"));
    assert!(text.contains("Readiness: the owner's decision; this report does not state it."));
    let established = doc["reading"]["established"]
        .as_array()
        .expect("established");
    assert_eq!(
        established
            .iter()
            .filter(|line| line.as_str().unwrap_or("").contains("did not commit again"))
            .count(),
        1,
        "{doc}"
    );
    let mutation = json!({
        "behavior_version": 1,
        "title": "lost reply is history",
        "outcomes": [{
            "id": "R1",
            "text": "a new mutation occurred",
            "claim": "new_mutation",
            "evidence": [id]
        }]
    });
    let (_, mutation_doc, mutation_text) = handoff_kind(&dir, &mutation, &bundle, "recovery");
    assert_eq!(claim_status(&mutation_doc, "outcomes", "R1"), "incomplete");
    assert!(mutation_text.contains("[incomplete claim new_mutation]"));
}

fn write_continuity_result(dir: &Path) -> (PathBuf, Vec<String>) {
    let path = dir.join("result.json");
    let body = json!({
        "schema": "ik.scenario.result.v1",
        "aggregate": "loan-1",
        "steps": [
            {
                "label": "under_prior",
                "kind": "prior_machine",
                "verdict": "pass",
                "response": { "outcome": "committed" },
                "present": { "created_version": 1, "presented": "prior", "presented_version": 1 }
            },
            {
                "label": "recall",
                "kind": "command",
                "verdict": "pass",
                "request": { "name": "recall", "args": {} },
                "response": { "outcome": "committed" },
                "present": {
                    "created_version": 1,
                    "presented": "prior",
                    "presented_version": 1,
                    "command": "recall"
                }
            },
            {
                "label": "extend",
                "kind": "command",
                "verdict": "pass",
                "request": { "name": "extend", "args": { "extra_ms": 1 } },
                "response": { "outcome": "refused", "diagnostic_code": "IK_MACHINE_MISMATCH" },
                "present": {
                    "created_version": 1,
                    "presented": "new",
                    "presented_version": 2,
                    "command": "extend",
                    "code": "IK_MACHINE_MISMATCH"
                }
            }
        ]
    });
    fs::write(&path, serde_json::to_vec(&body).expect("result")).expect("write result");
    let ids = vec![
        format!("{}#loan-1/under_prior", path.display()),
        format!("{}#loan-1/recall", path.display()),
        format!("{}#loan-1/extend", path.display()),
    ];
    (path, ids)
}

#[test]
fn continuity_claim_is_not_supported_by_unrelated_statuses() {
    let dir = temp_dir("continuity-negative");
    let (bundle, id) = write_scenario_result(&dir, "committed");
    let behavior = json!({
        "behavior_version": 1,
        "title": "loan checkout version 2",
        "outcomes": [{
            "id": "C2",
            "text": "existing instances keep their version",
            "claim": "continuity",
            "evidence": [id]
        }]
    });
    let (code, doc, text) = handoff_kind(&dir, &behavior, &bundle, "change");
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(
        claim_status(&doc, "outcomes", "C2"),
        "continuity not demonstrated"
    );
    assert!(text.contains("continuity not demonstrated"), "{text}");
}

#[test]
fn change_handoff_without_continuity_evidence_is_not_demonstrated() {
    let dir = temp_dir("continuity-absent");
    let (bundle, id) = write_scenario_result(&dir, "committed");
    let behavior = json!({
        "behavior_version": 1,
        "title": "loan checkout version 2",
        "outcomes": [{
            "id": "C1",
            "text": "the new definition still reaches returned",
            "evidence": [id]
        }]
    });
    let (code, doc, text) = handoff_kind(&dir, &behavior, &bundle, "change");
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(claim_status(&doc, "outcomes", "C1"), "exercised");
    assert_eq!(
        doc["reading"]["uncertain"][0],
        "continuity not demonstrated"
    );
    assert!(text.contains("continuity not demonstrated"), "{text}");
}

#[test]
fn change_handoff_states_existing_instance_continuity_from_present_observations() {
    let dir = temp_dir("continuity-positive");
    let (bundle, ids) = write_continuity_result(&dir);
    let behavior = json!({
        "behavior_version": 1,
        "title": "loan checkout version 2",
        "outcomes": [{
            "id": "C2",
            "text": "existing instances keep version 1",
            "claim": "continuity",
            "evidence": ids
        }]
    });
    let (code, doc, text) = handoff_kind(&dir, &behavior, &bundle, "change");
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(claim_status(&doc, "outcomes", "C2"), "exercised");
    assert!(text.contains("[exercised claim continuity]"), "{text}");
    assert!(
        text.contains("instances created under version 1 keep version 1"),
        "{text}"
    );
    assert!(text.contains("recall still works on them"), "{text}");
    assert!(
        text.contains("extend is refused on them with IK_MACHINE_MISMATCH"),
        "{text}"
    );
    assert!(
        text.contains("existing instances stay on their version; moving one to version 2 in place is not supported"),
        "{text}"
    );
    assert_eq!(
        text.matches("continuity not demonstrated").count(),
        0,
        "{text}"
    );
}

fn write_lookup_result(dir: &Path, aggregate: &str, operation_id: &str) -> (PathBuf, String) {
    let path = dir.join("result.json");
    let body = json!({
        "schema": "ik.scenario.result.v1",
        "aggregate": aggregate,
        "steps": [{
            "label": "found",
            "verdict": "pass",
            "request": { "operation_id": operation_id },
            "response": { "outcome": "committed_exact" }
        }]
    });
    fs::write(&path, serde_json::to_vec(&body).expect("result")).expect("write result");
    let id = format!("{}#{aggregate}/found", path.display());
    (path, id)
}

#[test]
fn first_line_is_desmo_kind_and_subject() {
    let dir = temp_dir("headline");
    let bundle = bundle(&dir, "refused:IK_CAPABILITY_STALE");
    let all = ids(&bundle);
    let behavior = json!({
        "behavior_version": 1,
        "title": "edit flow",
        "outcomes": [
            {"id": "B1", "text": "a second edit commits", "evidence": [id_ending(&all, "/again")]}
        ]
    });
    let (code, doc, text) = handoff(&dir, &behavior, &bundle);
    assert_eq!(code, Some(0));
    assert_eq!(doc["subject"], "edit flow");
    assert_eq!(doc["readiness"], "not_stated");
    assert!(text.starts_with("Desmo delivery: edit flow\n"), "{text}");
    assert!(
        text.contains("Demonstrated\na second edit commits\n"),
        "{text}"
    );
    assert!(
        text.contains("Readiness: the owner's decision; this report does not state it.\n"),
        "{text}"
    );
    assert!(text.contains("Status: DECISION REQUIRED\n"), "{text}");
    assert!(
        text.contains("Decision needed\nAccept or reject each rule below as the business rule.\n"),
        "{text}"
    );
    assert!(text.contains("Exact evidence"), "{text}");
    assert_eq!(text.matches("```mermaid").count(), 0);
    assert!(text.contains("```text"), "{text}");
    assert!(text.contains("| command "), "{text}");
    let first = text.split("| command ").next().unwrap_or("");
    let last = first
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    assert!(
        last.ends_with("It does not approve a release."),
        "first block should end on the decision, last={last:?} in {first}"
    );
    let after_headline = text.split_once('\n').expect("headline").1;
    assert!(
        after_headline.contains("Exact evidence"),
        "exact evidence is an appendix after the first block: {text}"
    );
}

#[test]
fn recovery_subject_and_operation_come_from_evidence() {
    let dir = temp_dir("recovery-identity");
    let (bundle, id) = write_lookup_result(&dir, "item-lost", "op-tag");
    let behavior = json!({
        "behavior_version": 1,
        "title": "lost reply is history",
        "outcomes": [{
            "id": "R1",
            "text": "lookup of the dropped commit is committed_exact",
            "evidence": [id]
        }]
    });
    let (code, doc, text) = handoff_kind(&dir, &behavior, &bundle, "recovery");
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["subject"], "item-lost");
    assert_eq!(doc["operation_id"], "op-tag");
    assert_eq!(doc["readiness"], "not_stated");
    assert!(
        text.starts_with("Desmo recovery: item-lost\noperation: op-tag\n"),
        "{text}"
    );
    assert_eq!(
        doc["scope"]["checked"],
        "whether a recorded operation exists in the store under the given id and request"
    );
    let review = doc["duties"].as_array().and_then(|rows| rows.iter().find(|row| row["duty"] == "Independent review"));
    assert_eq!(review.map(|row| row["status"].clone()), Some(json!("not recorded")));
    assert_eq!(review.map(|row| row["expected_party"].clone()), Some(json!("Reviewer")));
    assert!(
        text.contains("Safe next action\ndo not resend; the earlier result stands\n"),
        "{text}"
    );
    assert!(
        text.contains("To change the item again\nread the item's current state for a fresh handle"),
        "{text}"
    );
}

#[test]
fn recovery_without_identity_names_subject_not_identified() {
    let dir = temp_dir("recovery-unknown");
    let (bundle, id) = write_profile_result(&dir, "witness_found");
    let behavior = json!({
        "behavior_version": 1,
        "title": "lost reply is history",
        "outcomes": [{
            "id": "R1",
            "text": "lookup of the dropped commit is committed_exact",
            "evidence": [id]
        }]
    });
    let (code, doc, text) = handoff_kind(&dir, &behavior, &bundle, "recovery");
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["subject"], "subject not identified in evidence");
    assert!(doc["operation_id"].is_null(), "{doc}");
    assert!(
        text.starts_with("Desmo recovery: subject not identified in evidence\n"),
        "{text}"
    );
    assert_eq!(text.matches("operation:").count(), 0, "{text}");
}

#[test]
fn scenario_only_evidence_never_establishes_readiness() {
    let dir = temp_dir("scenario-only-readiness");
    let bundle = bundle(&dir, "refused:IK_CAPABILITY_STALE");
    let all = ids(&bundle);
    let behavior = json!({
        "behavior_version": 1,
        "title": "edit flow",
        "outcomes": [
            {"id": "B1", "text": "a second edit commits", "evidence": [id_ending(&all, "/again")]}
        ]
    });
    let (code, doc, text) = handoff(&dir, &behavior, &bundle);
    assert_eq!(code, Some(0));
    assert_eq!(doc["readiness"], "not_stated");
    assert!(text.contains("Readiness: the owner's decision; this report does not state it."), "{text}");
    assert_eq!(text.matches("readiness: established").count(), 0, "{text}");
    let (check_bundle, check_id) = write_profile_result(&dir, "witness_found");
    let mixed = json!({
        "behavior_version": 1,
        "title": "tagged item delivery",
        "outcomes": [
            {"id": "B1", "text": "a second edit commits", "evidence": [id_ending(&all, "/again")]},
            {"id": "B2", "text": "the required terminal tagged was witnessed in the supplied cases", "evidence": [check_id]}
        ]
    });
    let mixed = prepared(&mixed, "delivery", &bundle);
    let path = dir.join("mixed.json");
    fs::write(&path, serde_json::to_vec(&mixed).expect("behavior")).expect("write");
    let json_out = run(&[
        "handoff",
        &arg(&path),
        "--evidence",
        &arg(&bundle),
        "--evidence",
        &arg(&check_bundle),
        "--json",
    ]);
    let mixed_doc: Value = serde_json::from_slice(&json_out.stdout).unwrap_or(Value::Null);
    assert_eq!(mixed_doc["readiness"], "not_stated", "{mixed_doc}");
}

#[test]
fn lookup_is_not_rendered_as_refusal_or_new_commit() {
    let dir = temp_dir("lookup-not-commit");
    let (bundle, id) = write_lookup_result(&dir, "item-lost", "op-tag");
    let behavior = json!({
        "behavior_version": 1,
        "title": "lost reply is history",
        "outcomes": [{
            "id": "R1",
            "text": "lookup of the dropped commit is committed_exact",
            "evidence": [id]
        }]
    });
    let (code, doc, text) = handoff_kind(&dir, &behavior, &bundle, "recovery");
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(claim_status(&doc, "outcomes", "R1"), "exercised");
    assert_eq!(doc["outcomes"][0]["effective_claim"], "reconciled");
    let first = text.split("Exact evidence").next().unwrap_or("");
    assert!(first.contains("this lookup did not commit again"), "{text}");
    assert_eq!(first.matches("refused").count(), 0, "{text}");
    assert_eq!(first.matches("new commit").count(), 0, "{text}");
    assert_eq!(
        first.matches("a new mutation occurred").count(),
        0,
        "{text}"
    );
}

fn write_definition_check(dir: &Path, statement: &str) -> (PathBuf, String, String) {
    let path = dir.join("check.json");
    let body = json!({
        "verdict": "ACCEPT",
        "definition": {
            "schema": "ik.definition.delta.v1",
            "prior_version": 1,
            "new_version": 2,
            "commands_added": ["extend"],
            "commands_removed": [],
            "states_added": [],
            "states_removed": [],
            "statement": statement
        },
        "profile": {
            "schema": "ik.check.profile.v1",
            "search_complete": true,
            "goals": [{
                "id": "returned",
                "kind": "reach_terminal",
                "status": "witness_found"
            }]
        }
    });
    fs::write(&path, serde_json::to_vec(&body).expect("check")).expect("write check");
    let displayed = path.display();
    (
        path.clone(),
        format!("{displayed}#definition"),
        format!("{displayed}#returned"),
    )
}

#[test]
fn change_first_block_leads_with_declared_definition_delta() {
    let dir = temp_dir("declared-delta");
    let statement = "new in version 2: command extend; removed: none";
    let (bundle, _, returned_id) = write_definition_check(&dir, statement);
    let behavior = json!({
        "behavior_version": 1,
        "title": "loan checkout version 2",
        "outcomes": [{
            "id": "C1",
            "text": "the new definition still reaches returned",
            "claim": "reachable",
            "evidence": [returned_id]
        }]
    });
    let (code, doc, text) = handoff_kind(&dir, &behavior, &bundle, "change");
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["declared"], statement);
    let first = text.split("Exact evidence").next().unwrap_or("");
    let declared_at = first.find("New in this version").expect(first);
    let explored_at = first.find("For new instances").expect(first);
    assert!(declared_at < explored_at, "{first}");
    assert!(first.contains(statement), "{first}");
    assert!(first.contains("declared in the new definition"), "{first}");
    assert!(
        first.contains("explored (bounded search, not a runtime commit)"),
        "{first}"
    );
    assert_eq!(first.matches("Demonstrated").count(), 0, "{first}");
}

#[test]
fn authored_declared_meaning_renders_as_declared_and_never_demonstrated() {
    let dir = temp_dir("declared-meaning");
    let (bundle, definition_id, returned_id) =
        write_definition_check(&dir, "new in version 2: command extend; removed: none");
    let meaning = "Version 2 adds Extend for loans created under the new rules; it sets a later deadline and refuses an equal or earlier one.";
    let behavior = json!({
        "behavior_version": 1,
        "title": "loan checkout version 2",
        "outcomes": [
            {"id": "C0", "text": meaning, "claim": "declared", "evidence": [definition_id]},
            {"id": "C1", "text": "the new definition still reaches returned", "claim": "reachable", "evidence": [returned_id]}
        ]
    });
    let (code, doc, text) = handoff_kind(&dir, &behavior, &bundle, "change");
    assert_eq!(code, Some(0), "{doc}");
    let first = text.split("Exact evidence").next().unwrap_or("");
    let declared_at = first.find("New in this version").expect(first);
    let meaning_at = first.find(meaning).expect(first);
    let label_at = first.find("declared in the new definition").expect(first);
    let explored_at = first.find("For new instances").expect(first);
    assert!(
        declared_at < meaning_at && meaning_at < label_at && label_at < explored_at,
        "{first}"
    );
    assert_eq!(first.matches("Demonstrated").count(), 0, "{first}");
}

#[test]
fn authored_declared_meaning_without_definition_evidence_is_not_rendered() {
    let dir = temp_dir("declared-meaning-unbacked");
    let (bundle, _, returned_id) =
        write_definition_check(&dir, "new in version 2: command extend; removed: none");
    let meaning = "Version 2 adds Extend and it is fully tested.";
    let behavior = json!({
        "behavior_version": 1,
        "title": "loan checkout version 2",
        "outcomes": [
            {"id": "C0", "text": meaning, "claim": "declared", "evidence": [returned_id]}
        ]
    });
    let (code, doc, text) = handoff_kind(&dir, &behavior, &bundle, "change");
    assert_eq!(code, Some(0), "{doc}");
    let first = text.split("Exact evidence").next().unwrap_or("");
    let declared = first.split("Not established").next().unwrap_or("");
    assert_eq!(declared.matches(meaning).count(), 0, "{first}");
    assert!(
        first.contains(&format!("{meaning} (incomplete)")),
        "{first}"
    );
}

#[test]
fn change_without_prior_comparison_says_what_is_new_not_stated() {
    let dir = temp_dir("declared-missing");
    let bundle = bundle(&dir, "committed");
    let behavior = json!({
        "behavior_version": 1,
        "title": "loan checkout version 2",
        "outcomes": []
    });
    let (code, doc, text) = handoff_kind(&dir, &behavior, &bundle, "change");
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["declared"], "what is new: not stated in evidence");
    let first = text.split("Exact evidence").next().unwrap_or("");
    assert!(
        first.contains("what is new: not stated in evidence"),
        "{first}"
    );
    assert!(first.contains("declared in the new definition"), "{first}");
}

#[test]
fn declared_only_evidence_never_renders_as_demonstrated() {
    let dir = temp_dir("declared-not-runtime");
    let statement = "new in version 2: command extend; removed: none";
    let (bundle, definition_id, _) = write_definition_check(&dir, statement);
    let mutation = json!({
        "behavior_version": 1,
        "title": "loan checkout version 2",
        "outcomes": [{
            "id": "C1",
            "text": "extend commits on a new loan",
            "claim": "new_mutation",
            "evidence": [definition_id]
        }]
    });
    let (code, doc, text) = handoff(&dir, &mutation, &bundle);
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(claim_status(&doc, "outcomes", "C1"), "incomplete");
    let first = text.split("Exact evidence").next().unwrap_or("");
    assert_eq!(
        first.matches("Demonstrated\nextend commits").count(),
        0,
        "{first}"
    );
    let declared = json!({
        "behavior_version": 1,
        "title": "loan checkout version 2",
        "outcomes": [{
            "id": "C1",
            "text": "the new definition declares extend",
            "claim": "declared",
            "evidence": [definition_id]
        }]
    });
    let (code, doc, text) = handoff(&dir, &declared, &bundle);
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(claim_status(&doc, "outcomes", "C1"), "exercised");
    assert_eq!(doc["outcomes"][0]["claim"], "declared");
    let first = text.split("Exact evidence").next().unwrap_or("");
    assert_eq!(
        first
            .matches("Demonstrated\nthe new definition declares extend")
            .count(),
        0,
        "{first}"
    );
}

#[test]
fn handoff_embeds_legend_and_structural_scope() {
    let dir = temp_dir("legend-scope");
    let machine = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/shipped/tagged_item.ik.toml");
    fs::copy(&machine, dir.join("machine.ik.toml")).expect("machine");
    let (evidence, _) = write_scenario_result(&dir, "committed_exact");
    let behavior = json!({
        "behavior_version": 1,
        "title": "lost reply",
        "outcomes": []
    });
    let (code, _, text) = handoff_kind(&dir, &behavior, &evidence, "recovery");
    assert_eq!(code, Some(0), "{text}");
    let first = text.split("Exact evidence").next().unwrap_or("");
    assert_eq!(first.contains("⟪"), true, "{first}");
    assert_eq!(first.contains("legend:"), true, "{first}");
    assert_eq!(first.contains("⟪ ⟫ one command, unfolded"), true, "{first}");
}

#[test]
fn handoff_delivery_keeps_structural_wording() {
    let dir = temp_dir("structural-loan");
    let machine = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("validation/dfy/journeys/fixtures/loan_checkout.ik.toml");
    fs::copy(&machine, dir.join("machine.ik.toml")).expect("machine");
    let (evidence, _) = write_scenario_result(&dir, "committed");
    let behavior = json!({
        "behavior_version": 1,
        "title": "loan",
        "outcomes": []
    });
    let (code, _, text) = handoff_kind(&dir, &behavior, &evidence, "delivery");
    assert_eq!(code, Some(0), "{text}");
    let first = text.split("Exact evidence").next().unwrap_or("");
    assert_eq!(first.contains("reaches an end"), true, "{first}");
    assert_eq!(first.contains("structural"), true, "{first}");
}

#[test]
fn handoff_stacked_regions_match_diagram() {
    let dir = temp_dir("stacked-regions");
    let machine = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/diagram/region_stacked.ik.toml");
    fs::copy(&machine, dir.join("machine.ik.toml")).expect("machine");
    let (evidence, _) = write_scenario_result(&dir, "committed");
    let behavior = json!({
        "behavior_version": 1,
        "title": "regions",
        "outcomes": []
    });
    let (code, _, text) = handoff_kind(&dir, &behavior, &evidence, "delivery");
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(text.contains("3 regions"), true, "{text}");
    assert_eq!(text.contains("7 commands (2 joints)"), true, "{text}");
    assert_eq!(text.contains("╘"), true, "{text}");
    assert_eq!(text.contains("╛"), true, "{text}");
    assert_eq!(text.contains("joint: pause_and_mute"), true, "{text}");
    assert_eq!(text.contains("joint: sleep"), true, "{text}");
    let river = text.split("Exact evidence").next().unwrap_or("");
    for line in river.lines() {
        assert_eq!(
            ik::diagram::display_width(line) <= 120,
            true,
            "wide {line}"
        );
    }
    let compound_dir = temp_dir("compound-handoff");
    let compound = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("machines/region_compound.ik.toml");
    fs::copy(&compound, compound_dir.join("machine.ik.toml")).expect("compound");
    let (evidence, _) = write_scenario_result(&compound_dir, "committed");
    let (code, _, text) = handoff_kind(
        &compound_dir,
        &behavior,
        &evidence,
        "delivery",
    );
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(text.contains("applies │"), true, "{text}");
    assert_eq!(text.contains("applies│"), false, "{text}");
}

/// Adds a default `why` for a change and default examples for the machine found in
/// the evidence, unless the test supplied its own.
fn prepared(behavior: &Value, kind: &str, evidence: &Path) -> Value {
    let mut out = behavior.clone();
    if kind == "change" && out.get("why").is_none() {
        out["why"] = json!({"text": "test requirement", "source": "tests/handoff.rs"});
    }
    if kind != "recovery" && out.get("rule_witnesses").is_none() {
        out["rule_witnesses"] = default_witnesses(evidence);
    }
    out
}

fn ex(args: Value, facts: Value, accepted: bool) -> Value {
    json!({"args": args, "facts": facts, "outcome": if accepted { "accepted" } else { "rejected" }})
}

fn default_witnesses(evidence: &Path) -> Value {
    let (machine, _) = ik::diagram::machines_in_evidence(&[evidence.to_path_buf()]);
    let text = machine
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
    let id = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("machine_id = "))
        .unwrap_or("")
        .trim_matches('"')
        .to_string();
    let digest = |a: &str| json!({ "artifact_digest": a });
    let sets = match id.as_str() {
        "publish_approval" => vec![
            json!({"rule": "approve", "examples": [
                ex(digest("d1"), digest("d1"), true),
                ex(digest("d2"), digest("d1"), false),
                ex(digest("d1"), json!({}), false),
            ]}),
            json!({"rule": "authorize_publish", "examples": [
                ex(json!({"payload_digest": "d1"}), json!({"approved_digest": "d1", "artifact_digest": "d1"}), true),
                ex(json!({"payload_digest": "d2"}), json!({"approved_digest": "d1", "artifact_digest": "d1"}), false),
                ex(json!({"payload_digest": "d1"}), json!({"approved_digest": "d1", "artifact_digest": "d2"}), false),
                ex(json!({"payload_digest": "d1"}), json!({"artifact_digest": "d1"}), false),
            ]}),
            json!({"rule": "complete", "examples": [
                ex(json!({"published_digest": "d1"}), json!({"approved_digest": "d1"}), true),
                ex(json!({"published_digest": "d2"}), json!({"approved_digest": "d1"}), false),
                ex(json!({"published_digest": "d1"}), json!({}), false),
            ]}),
        ],
        "tagged_item" => vec![tag_witnesses()],
        "loan_checkout" => loan_witnesses(text.contains("extend")),
        "region_stacked" => vec![json!({"rule": "blank", "examples": [
            ex(json!({}), json!({"region_volume": "muted"}), true),
            ex(json!({}), json!({"region_volume": "unmuted"}), false),
        ]})],
        "region_compound" => vec![json!({"rule": "power_off", "examples": [
            ex(json!({"override_enabled": true}), json!({}), true),
            ex(json!({"override_enabled": false}), json!({}), false),
        ]})],
        _ => Vec::new(),
    };
    Value::Array(sets)
}

fn tag_witnesses() -> Value {
    let id = |value: &str| json!({ "id": value });
    json!({"rule": "tag", "examples": [
        ex(id("a"), json!({}), true),
        ex(id(&"x".repeat(64)), json!({}), true),
        ex(id("a.b-9_Z"), json!({}), true),
        ex(id(""), json!({}), false),
        ex(id(&"x".repeat(65)), json!({}), false),
        ex(id("item/1"), json!({}), false),
    ]})
}

fn loan_witnesses(with_extend: bool) -> Vec<Value> {
    let mut sets = vec![
        json!({"rule": "checkout", "examples": [
            ex(json!({"loan_until_ms": 1209600100u64, "checkout_ms": 100}), json!({}), true),
            ex(json!({"loan_until_ms": 1209600101u64, "checkout_ms": 100}), json!({}), false),
            ex(json!({"loan_until_ms": 1209600099u64, "checkout_ms": 100}), json!({}), false),
        ]}),
        json!({"rule": "checkin", "examples": [
            ex(json!({"overdue": 0}), json!({"recall_placed": 0}), true),
            ex(json!({"overdue": 1}), json!({"recall_placed": 1}), true),
            ex(json!({"overdue": 1}), json!({"recall_placed": 0}), false),
        ]}),
    ];
    if with_extend {
        sets.push(json!({"rule": "extend", "examples": [
            ex(json!({"new_until_ms": 200}), json!({"loan_until_ms": 100}), true),
            ex(json!({"new_until_ms": 100}), json!({"loan_until_ms": 100}), false),
            ex(json!({"new_until_ms": 50}), json!({"loan_until_ms": 100}), false),
        ]}));
    }
    sets
}

fn delivery_behavior(accepted: Value) -> Value {
    json!({
        "behavior_version": 1,
        "title": "edit flow",
        "outcomes": [],
        "accepted_rules": accepted
    })
}

#[test]
fn delivery_asks_for_acceptance_of_every_guarded_rule_with_its_hash() {
    let dir = temp_dir("rule-ledger");
    let bundle = bundle(&dir, "refused:IK_CAPABILITY_STALE");
    let (code, doc, text) = handoff(&dir, &delivery_behavior(json!([])), &bundle);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(doc["decision_status"], "decision_required");
    let rules = doc["decisions"][0]["rules"].as_array().expect("rules");
    let names: Vec<&str> = rules.iter().filter_map(|rule| rule["name"].as_str()).collect();
    assert_eq!(names, vec!["approve", "authorize_publish", "complete"]);
    for rule in rules {
        assert_eq!(rule["hash"].as_str().map(str::len), Some(64), "{rule}");
        assert_eq!(rule["acceptance"], "needs your acceptance");
    }
    assert_eq!(text.contains("needs your acceptance"), true, "{text}");
    assert_eq!(text.contains("you do not copy hashes or edit files"), true, "{text}");
    assert_eq!(text.contains("list the rule name, its guard hash"), false, "{text}");
    assert_eq!(
        doc["scope"]["checked"],
        "whether the state transitions behave as the definition says, within the cited evidence"
    );
    assert_eq!(doc["scope"]["assumed"][0], "execution is perfect and reliable");
    assert_eq!(
        text.contains("Scope\nChecked: whether the state transitions behave as the definition says, within the cited evidence.\n"),
        true,
        "{text}"
    );
    let duties = doc["duties"].as_array().expect("duties");
    let status = |name: &str| duties.iter().find(|row| row["duty"] == name).map(|row| row["status"].clone());
    assert_eq!(status("Rule acceptance"), Some(json!("3 of 3 awaiting acceptance")));
    assert_eq!(status("Independent review"), Some(json!("not recorded")));
    assert_eq!(status("Requirement source"), Some(json!("not recorded")));
    assert_eq!(text.contains("Who attested what"), true, "{text}");
}

#[test]
fn accepting_each_rule_by_hash_clears_the_decision() {
    let dir = temp_dir("rule-accepted");
    let bundle = bundle(&dir, "refused:IK_CAPABILITY_STALE");
    let (_, doc, _) = handoff(&dir, &delivery_behavior(json!([])), &bundle);
    let accepted: Vec<Value> = doc["decisions"][0]["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .map(|rule| json!({"rule": rule["name"], "guard_hash": rule["hash"], "examples_hash": rule["examples_hash"]}))
        .collect();
    let (code, doc, text) = handoff(&dir, &delivery_behavior(json!(accepted)), &bundle);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(doc["decision_status"], "none");
    assert_eq!(text.contains("Decision needed\nnone\n"), true, "{text}");
    assert_eq!(text.contains("Status: DECISION REQUIRED"), false, "{text}");
    let duties = doc["duties"].as_array().expect("duties");
    let accepted_row = duties.iter().find(|row| row["duty"] == "Rule acceptance").map(|row| row["status"].clone());
    assert_eq!(
        accepted_row,
        Some(json!("3 of 3 accepted for the displayed content; a rule acceptance, not a release approval"))
    );
}

#[test]
fn an_acceptance_for_a_different_guard_is_stale_and_asks_again() {
    let dir = temp_dir("rule-stale");
    let bundle = bundle(&dir, "refused:IK_CAPABILITY_STALE");
    let (_, doc, _) = handoff(&dir, &delivery_behavior(json!([])), &bundle);
    let mut accepted: Vec<Value> = doc["decisions"][0]["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .map(|rule| json!({"rule": rule["name"], "guard_hash": rule["hash"], "examples_hash": rule["examples_hash"]}))
        .collect();
    accepted[0]["guard_hash"] = json!("0".repeat(64));
    let (_, doc, text) = handoff(&dir, &delivery_behavior(json!(accepted)), &bundle);
    let rules = doc["decisions"][0]["rules"].as_array().expect("rules");
    assert_eq!(rules.len(), 1);
    assert_eq!(
        rules[0]["acceptance"],
        "acceptance is stale: the rule or its examples changed after it was accepted"
    );
    assert_eq!(text.contains("acceptance is stale"), true, "{text}");
}

fn refused(dir: &Path, behavior: &Value, bundle: &Path, kind: &str) -> String {
    let path = dir.join("refusal.json");
    fs::write(&path, serde_json::to_vec(behavior).expect("behavior")).expect("write");
    let output = run(&["handoff", &arg(&path), "--evidence", &arg(bundle), "--kind", kind]);
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stdout));
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_rule_without_examples_is_refused_before_a_human_is_asked() {
    let dir = temp_dir("no-examples");
    let bundle = bundle(&dir, "refused:IK_CAPABILITY_STALE");
    let behavior = json!({"behavior_version": 1, "title": "t", "outcomes": [], "rule_witnesses": []});
    let message = refused(&dir, &behavior, &bundle, "delivery");
    for rule in ["approve", "authorize_publish", "complete"] {
        assert_eq!(
            message.contains(&format!("rule `{rule}` needs examples (rule_witnesses)")),
            true,
            "{message}"
        );
    }
}

#[test]
fn examples_that_do_not_tell_the_rule_from_a_boundary_variant_are_refused() {
    let dir = temp_dir("weak-examples");
    let machine = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/shipped/tagged_item.ik.toml");
    fs::copy(&machine, dir.join("machine.ik.toml")).expect("machine");
    let (evidence, _) = write_scenario_result(&dir, "committed");
    let trivial = json!([{"rule": "tag", "examples": [
        ex(json!({"id": "abc"}), json!({}), true),
        ex(json!({"id": ""}), json!({}), false),
    ]}]);
    let behavior = json!({"behavior_version": 1, "title": "t", "outcomes": [], "rule_witnesses": trivial});
    let message = refused(&dir, &behavior, &evidence, "delivery");
    assert_eq!(message.contains("do not tell the rule from this variant"), true, "{message}");
    assert_eq!(message.contains("1,65)"), true, "{message}");
}

#[test]
fn an_example_with_the_wrong_stated_outcome_is_refused() {
    let dir = temp_dir("wrong-outcome");
    let machine = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/shipped/tagged_item.ik.toml");
    fs::copy(&machine, dir.join("machine.ik.toml")).expect("machine");
    let (evidence, _) = write_scenario_result(&dir, "committed");
    let wrong = json!([{"rule": "tag", "examples": [ex(json!({"id": "ok"}), json!({}), false)]}]);
    let behavior = json!({"behavior_version": 1, "title": "t", "outcomes": [], "rule_witnesses": wrong});
    let message = refused(&dir, &behavior, &evidence, "delivery");
    assert_eq!(
        message.contains("example 1 is stated as rejected but the guard accepts it"),
        true,
        "{message}"
    );
}

#[test]
fn witnesses_for_a_rule_that_is_not_in_the_ledger_are_refused() {
    let dir = temp_dir("unknown-rule");
    let bundle = bundle(&dir, "refused:IK_CAPABILITY_STALE");
    let mut behavior = prepared(&json!({"behavior_version": 1, "title": "t", "outcomes": []}), "delivery", &bundle);
    behavior["rule_witnesses"]
        .as_array_mut()
        .expect("witnesses")
        .push(json!({"rule": "typo", "examples": [ex(json!({}), json!({}), true)]}));
    let message = refused(&dir, &behavior, &bundle, "delivery");
    assert_eq!(message.contains("rule `typo`"), true, "{message}");
}

#[test]
fn the_human_sees_the_checked_examples_beside_each_rule() {
    let dir = temp_dir("examples-shown");
    let machine = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/shipped/tagged_item.ik.toml");
    fs::copy(&machine, dir.join("machine.ik.toml")).expect("machine");
    let (evidence, _) = write_scenario_result(&dir, "committed");
    let behavior = json!({"behavior_version": 1, "title": "t", "outcomes": []});
    let (code, doc, text) = handoff(&dir, &behavior, &evidence);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(text.contains("accepted: $id=\"a\"; "), true, "{text}");
    assert_eq!(text.contains("rejected: $id=\"\"; "), true, "{text}");
    assert_eq!(text.contains("(65 chars)"), true, "{text}");
    assert_eq!(text.contains(&"x".repeat(40)), false, "a long example is shortened: {text}");
    assert_eq!(doc["decisions"][0]["rules"][0]["examples"]["variants_checked"].as_u64() > Some(0), true);
}

#[test]
fn changing_the_examples_makes_an_earlier_acceptance_stale() {
    let dir = temp_dir("examples-stale");
    let machine = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/shipped/tagged_item.ik.toml");
    fs::copy(&machine, dir.join("machine.ik.toml")).expect("machine");
    let (evidence, _) = write_scenario_result(&dir, "committed");
    let (_, doc, _) = handoff(&dir, &delivery_behavior(json!([])), &evidence);
    let rule = &doc["decisions"][0]["rules"][0];
    let accepted = json!([{"rule": "tag", "guard_hash": rule["hash"], "examples_hash": rule["examples_hash"]}]);
    let (_, kept, _) = handoff(&dir, &delivery_behavior(accepted.clone()), &evidence);
    assert_eq!(kept["decision_status"], "none");
    let mut more = delivery_behavior(accepted);
    let mut sets = default_witnesses(&evidence);
    sets[0]["examples"].as_array_mut().expect("examples").push(ex(json!({"id": "z"}), json!({}), true));
    more["rule_witnesses"] = sets;
    let (_, changed, _) = handoff(&dir, &more, &evidence);
    assert_eq!(
        changed["decisions"][0]["rules"][0]["acceptance"],
        "acceptance is stale: the rule or its examples changed after it was accepted"
    );
}

fn change_behavior(why: Value) -> Value {
    let mut body = json!({
        "behavior_version": 1,
        "title": "loan checkout version 2",
        "outcomes": []
    });
    if !why.is_null() {
        body["why"] = why;
    }
    body
}

fn loan_change_bundle(dir: &Path) -> PathBuf {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("validation/dfy/journeys/fixtures");
    fs::copy(fixtures.join("loan_v2.ik.toml"), dir.join("machine.ik.toml")).expect("machine");
    fs::copy(
        fixtures.join("loan_checkout.ik.toml"),
        dir.join("prior_machine.ik.toml"),
    )
    .expect("prior");
    write_scenario_result(dir, "committed").0
}

#[test]
fn change_handoff_without_why_is_refused_before_any_human_sees_it() {
    let dir = temp_dir("change-no-why");
    let evidence = loan_change_bundle(&dir);
    let path = dir.join("behavior.json");
    fs::write(&path, serde_json::to_vec(&change_behavior(Value::Null)).expect("body")).expect("write");
    let output = run(&["handoff", &arg(&path), "--evidence", &arg(&evidence), "--kind", "change"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr.contains("needs `why`"), true, "{stderr}");
}

#[test]
fn change_handoff_quotes_why_and_offers_only_options_that_exist() {
    let dir = temp_dir("change-why");
    let evidence = loan_change_bundle(&dir);
    let why = json!({"text": "Borrowers need to extend a loan once", "source": "PRD loans, section 4"});
    let (code, doc, text) = handoff_kind(&dir, &change_behavior(why), &evidence, "change");
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(doc["why"]["source"], "PRD loans, section 4");
    assert_eq!(text.contains("Borrowers need to extend a loan once"), true, "{text}");
    assert_eq!(
        text.contains("stated by the author, source PRD loans, section 4; not verified by evidence"),
        true,
        "{text}"
    );
    let policy = doc["decisions"]
        .as_array()
        .expect("decisions")
        .iter()
        .find(|item| item["kind"] == "version_policy")
        .expect("version policy");
    let labels: Vec<&str> = policy["options"]
        .as_array()
        .expect("options")
        .iter()
        .filter_map(|option| option["label"].as_str())
        .collect();
    assert_eq!(
        labels,
        vec![
            "Keep existing instances on version 1 until they finish",
            "Drain, then create new version 2 instances"
        ]
    );
    assert_eq!(text.contains("cannot use: extend"), true, "{text}");
    assert_eq!(text.to_lowercase().contains("migrate"), false, "{text}");
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert_eq!(flat.contains("no store was given with --store, so no census was taken"), true, "{text}");
    let rule = doc["decisions"][0]["rules"].as_array().expect("rules");
    assert_eq!(rule.len(), 1, "only the new rule needs acceptance: {rule:?}");
    assert_eq!(rule[0]["name"], "extend");
    assert_eq!(rule[0]["status"], "new");
}

#[test]
fn recovery_handoff_names_the_caller_side_gap_and_asks_no_rule_question() {
    let dir = temp_dir("recovery-gap");
    let (bundle, _) = write_lookup_result(&dir, "item-lost", "op-tag");
    let behavior = json!({"behavior_version": 1, "title": "lost reply", "outcomes": []});
    let (code, doc, text) = handoff_kind(&dir, &behavior, &bundle, "recovery");
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(doc["decisions"].as_array().map(Vec::len), Some(0));
    assert_eq!(
        text.contains("whether the caller saved the operation id before sending"),
        true,
        "{text}"
    );
}

#[test]
fn without_a_declarative_definition_the_handoff_says_the_rule_checks_did_not_run() {
    let dir = temp_dir("no-definition");
    let (evidence, _) = write_scenario_result(&dir, "committed");
    let behavior = json!({"behavior_version": 1, "title": "a reducer machine", "outcomes": []});
    let (code, doc, text) = handoff(&dir, &behavior, &evidence);
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(doc["decisions"].as_array().map(Vec::len), Some(0));
    assert_eq!(text.contains("no declarative definition was found in the evidence"), true, "{text}");
    assert_eq!(text.contains("Decision needed\nnone computed"), true, "{text}");
    assert_eq!(text.contains("Decision needed\nnone\n"), false, "{text}");
}

const TAG_REQUIREMENT: &str = "The id may use letters, digits, dot, underscore and hyphen, and must be between 1 and 64 characters.";

fn tag_setup(name: &str) -> (PathBuf, PathBuf) {
    let dir = temp_dir(name);
    let machine = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/shipped/tagged_item.ik.toml");
    fs::copy(&machine, dir.join("machine.ik.toml")).expect("machine");
    let (evidence, _) = write_scenario_result(&dir, "committed");
    (dir, evidence)
}

fn tag_behavior(extra: Value) -> Value {
    let mut behavior = json!({"behavior_version": 1, "title": "t", "outcomes": [], "rule_witnesses": [tag_witnesses()]});
    let target = behavior.as_object_mut().expect("object");
    for (key, value) in extra.as_object().expect("extra") {
        target.insert(key.clone(), value.clone());
    }
    behavior
}

fn tag_hash(dir: &Path, evidence: &Path) -> String {
    let (code, doc, _) = handoff_kind(dir, &tag_behavior(json!({})), evidence, "delivery");
    assert_eq!(code, Some(0), "{doc}");
    doc["origins"]["rules"][0]["guard_hash"].as_str().expect("hash").to_string()
}

fn origin(hash: &str, point: &str, origin: &str, extra: Value) -> Value {
    let mut entry = json!({"rule": "tag", "point": point, "guard_hash": hash, "origin": origin});
    for (key, value) in extra.as_object().expect("extra") {
        entry[key] = value.clone();
    }
    entry
}

#[test]
fn every_decision_point_is_unrecorded_until_an_origin_is_recorded() {
    let (dir, evidence) = tag_setup("origin-default");
    let (code, doc, text) = handoff_kind(&dir, &tag_behavior(json!({})), &evidence, "delivery");
    assert_eq!(code, Some(0), "{text}");
    let counts = &doc["origins"]["counts"];
    assert_eq!((counts["total"].clone(), counts["unrecorded"].clone()), (json!(3), json!(3)));
    assert_eq!(text.contains("Where each criterion came from"), true, "{text}");
    assert_eq!(text.contains("3 decision points"), true, "{text}");
    let scope = doc["origins"]["scope"].as_str().unwrap_or("");
    assert_eq!(scope.contains("not enumerated: existence tests"), true, "{scope}");
}

#[test]
fn stated_and_chosen_points_are_counted_and_an_acknowledgment_never_makes_a_point_stated() {
    let (dir, evidence) = tag_setup("origin-mixed");
    let hash = tag_hash(&dir, &evidence);
    let requirement = json!({"text": TAG_REQUIREMENT, "source": "owner brief", "provenance": "owner-authored"});
    let entries = json!([
        origin(&hash, "max_length:64", "stated", json!({"span": "between 1 and 64 characters"})),
        origin(&hash, "charset:A-Za-z0-9._-", "stated", json!({"span": "letters, digits, dot, underscore and hyphen", "equivalent": "letters, digits, dot, underscore and hyphen"})),
        origin(&hash, "min_length:1", "chosen", json!({})),
    ]);
    let extra = json!({"requirement": requirement, "origins": entries});
    let (code, doc, text) = handoff_kind(&dir, &tag_behavior(extra), &evidence, "delivery");
    assert_eq!(code, Some(0), "{text}");
    let counts = &doc["origins"]["counts"];
    assert_eq!((counts["stated"].clone(), counts["chosen"].clone(), counts["chosen_acknowledged"].clone()), (json!(2), json!(1), json!(0)));
    assert_eq!(text.contains("author-stated, not verified: owner brief (owner-authored)"), true, "{text}");
    assert_eq!(text.contains("chosen by the author, not acknowledged"), true, "{text}");

    let acked = json!({
        "requirement": requirement,
        "origins": entries,
        "acknowledged_origins": [{"rule": "tag", "point": "min_length:1", "guard_hash": hash}]
    });
    let (_, doc, _) = handoff_kind(&dir, &tag_behavior(acked), &evidence, "delivery");
    let counts = &doc["origins"]["counts"];
    assert_eq!((counts["stated"].clone(), counts["chosen"].clone(), counts["chosen_acknowledged"].clone()), (json!(2), json!(0), json!(1)));
}

#[test]
fn an_origin_that_fails_a_necessary_condition_is_refused_before_a_human_sees_it() {
    let (dir, evidence) = tag_setup("origin-refused");
    let hash = tag_hash(&dir, &evidence);
    let requirement = json!({"text": TAG_REQUIREMENT, "source": "owner brief"});
    let refusal = |entries: Value, requirement: Value| {
        let behavior = tag_behavior(json!({"requirement": requirement, "origins": entries}));
        refused(&dir, &behavior, &evidence, "delivery")
    };
    let not_verbatim = origin(&hash, "max_length:64", "stated", json!({"span": "at most 64 characters"}));
    assert_eq!(refusal(json!([not_verbatim]), requirement.clone()).contains("not a verbatim part of the requirement text"), true);
    let no_literal = origin(&hash, "max_length:64", "stated", json!({"span": "and must be"}));
    assert_eq!(refusal(json!([no_literal]), requirement.clone()).contains("does not contain `64`"), true);
    let no_span = origin(&hash, "max_length:64", "stated", json!({}));
    assert_eq!(refusal(json!([no_span]), requirement.clone()).contains("needs a span"), true);
    let no_requirement = origin(&hash, "max_length:64", "stated", json!({"span": "64 characters"}));
    assert_eq!(refusal(json!([no_requirement]), Value::Null).contains("needs a `requirement`"), true);
    let typo = origin(&hash, "max_length:65", "chosen", json!({}));
    assert_eq!(refusal(json!([typo]), requirement.clone()).contains("not a decision point of its guard"), true);
    let default_without_identity = origin(&hash, "min_length:1", "default", json!({}));
    assert_eq!(refusal(json!([default_without_identity]), requirement.clone()).contains("needs default_id"), true);
    let stray = json!({"rule": "nope", "point": "max_length:64", "guard_hash": hash, "origin": "chosen"});
    assert_eq!(refusal(json!([stray]), requirement.clone()).contains("not under review"), true);
}

#[test]
fn only_a_chosen_point_can_be_acknowledged_and_a_stale_origin_reads_unrecorded() {
    let (dir, evidence) = tag_setup("origin-ack-stale");
    let hash = tag_hash(&dir, &evidence);
    let requirement = json!({"text": TAG_REQUIREMENT, "source": "owner brief"});
    let stated = origin(&hash, "max_length:64", "stated", json!({"span": "between 1 and 64 characters"}));
    let wrongly_acked = json!({
        "requirement": requirement,
        "origins": [stated],
        "acknowledged_origins": [{"rule": "tag", "point": "max_length:64", "guard_hash": hash}]
    });
    let message = refused(&dir, &tag_behavior(wrongly_acked), &evidence, "delivery");
    assert_eq!(message.contains("only a chosen point can be acknowledged"), true, "{message}");
    let old = origin(&"0".repeat(64), "max_length:64", "chosen", json!({}));
    let (code, doc, _) = handoff_kind(&dir, &tag_behavior(json!({"origins": [old]})), &evidence, "delivery");
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["origins"]["counts"]["unrecorded"], json!(3));
    let detail = doc["origins"]["rules"][0]["points"][2]["detail"].as_str().unwrap_or("");
    assert_eq!(detail.starts_with("stale: the guard changed"), true, "{detail}");
}

#[test]
fn the_current_origin_wins_over_a_stale_one_and_two_current_origins_are_refused() {
    let (dir, evidence) = tag_setup("origin-current");
    let hash = tag_hash(&dir, &evidence);
    let stale = origin(&"0".repeat(64), "min_length:1", "chosen", json!({}));
    let current = origin(&hash, "min_length:1", "chosen", json!({}));
    let (code, doc, _) = handoff_kind(&dir, &tag_behavior(json!({"origins": [stale, current.clone()]})), &evidence, "delivery");
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["origins"]["counts"]["chosen"], json!(1));
    let requirement = json!({"text": TAG_REQUIREMENT, "source": "owner brief"});
    let stated = origin(&hash, "min_length:1", "stated", json!({"span": "between 1 and 64"}));
    let behavior = tag_behavior(json!({"requirement": requirement, "origins": [current, stated]}));
    let message = refused(&dir, &behavior, &evidence, "delivery");
    assert_eq!(message.contains("more than one current origin is recorded"), true, "{message}");
}

#[test]
fn an_acknowledgment_with_no_current_chosen_origin_is_refused_not_dropped() {
    let (dir, evidence) = tag_setup("origin-ack-orphan");
    let hash = tag_hash(&dir, &evidence);
    let ack = json!([{"rule": "tag", "point": "min_length:1", "guard_hash": hash}]);
    let message = refused(&dir, &tag_behavior(json!({"acknowledged_origins": ack})), &evidence, "delivery");
    assert_eq!(message.contains("only a chosen point can be acknowledged"), true, "{message}");
}

#[test]
fn the_requirement_is_labelled_author_stated_and_carries_a_digest_not_a_pin() {
    let (dir, evidence) = tag_setup("origin-requirement");
    let requirement = json!({"text": TAG_REQUIREMENT, "source": "owner brief", "provenance": "owner-authored"});
    let (code, doc, text) = handoff_kind(&dir, &tag_behavior(json!({"requirement": requirement})), &evidence, "delivery");
    assert_eq!(code, Some(0), "{text}");
    assert_eq!(doc["origins"]["requirement"]["text_digest"].as_str().map(str::len), Some(64));
    assert_eq!(text.contains("pinned:"), false, "{text}");
    assert_eq!(text.contains("author-stated, not verified: owner brief (owner-authored)"), true, "{text}");
}

#[test]
fn every_rule_under_review_is_either_enumerated_or_named_as_skipped() {
    let dir = temp_dir("origin-partition");
    let bundle = bundle(&dir, "refused:IK_CAPABILITY_STALE");
    let (code, doc, text) = handoff(&dir, &delivery_behavior(json!([])), &bundle);
    assert_eq!(code, Some(0), "{text}");
    let reviewed: Vec<String> = doc["diagram"]["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .filter(|rule| rule["status"] != "unchanged" && rule["status"] != "removed")
        .filter_map(|rule| rule["name"].as_str().map(str::to_string))
        .collect();
    let mut seen: Vec<String> = doc["origins"]["rules"]
        .as_array()
        .expect("origin rules")
        .iter()
        .filter_map(|rule| rule["rule"].as_str().map(str::to_string))
        .collect();
    seen.extend(doc["origins"]["skipped"].as_array().expect("skipped").iter().filter_map(|name| name.as_str().map(str::to_string)));
    seen.sort();
    let mut expected = reviewed;
    expected.sort();
    assert_eq!(seen, expected);
}

fn atoms_requirement(extra: Value) -> Value {
    let mut requirement = json!({
        "text": "The id may use letters, digits, dot, underscore and hyphen, and must be between 1 and 64 characters. A tag also carries a label.",
        "source": "owner brief",
        "atoms": [
            {"id": "A1", "span": "between 1 and 64 characters"},
            {"id": "A2", "span": "A tag also carries a label."}
        ]
    });
    for (key, value) in extra.as_object().expect("extra") {
        requirement[key] = value.clone();
    }
    requirement
}

#[test]
fn the_handoff_lists_what_is_missing_and_says_when_it_did_not_look() {
    let (dir, evidence) = tag_setup("gaps-default");
    let (code, doc, text) = handoff_kind(&dir, &tag_behavior(json!({})), &evidence, "delivery");
    assert_eq!(code, Some(0), "{text}");
    let gaps = &doc["gaps"];
    let definition: Vec<(String, String)> = gaps["definition"]
        .as_array()
        .expect("definition")
        .iter()
        .map(|row| (row["kind"].as_str().unwrap_or("").to_string(), row["subject"].as_str().unwrap_or("").to_string()))
        .collect();
    assert_eq!(definition.contains(&("unguarded_argument".to_string(), "tag $label".to_string())), true, "{definition:?}");
    assert_eq!(definition.contains(&("no_invariants".to_string(), "machine".to_string())), true, "{definition:?}");
    assert_eq!(gaps["evidence"][0]["kind"], "no_run_evidence");
    assert_eq!(gaps["requirement"]["atomized"], false);
    assert_eq!(text.contains("What is missing"), true, "{text}");
    let scope = gaps["scope"].as_str().unwrap_or("");
    assert_eq!(scope.contains("not enumerated: requirement statements nobody wrote down"), true, "{scope}");
    assert_eq!(scope.ends_with("A short list does not mean the definition is good"), true, "{scope}");
    assert_eq!(text.contains("A short list does not mean the definition is"), true, "{text}");
    let (_, recovery, _) = handoff_kind(&dir, &tag_behavior(json!({})), &evidence, "recovery");
    assert_eq!(recovery["gaps"], Value::Null);
}

#[test]
fn an_atomized_requirement_names_the_statement_nothing_implements_and_the_sentence_left_out() {
    let (dir, evidence) = tag_setup("gaps-atoms");
    let requirement = atoms_requirement(json!({"traces": [{"atom": "A1", "element": "command:tag"}]}));
    let (code, doc, text) = handoff_kind(&dir, &tag_behavior(json!({"requirement": requirement})), &evidence, "delivery");
    assert_eq!(code, Some(0), "{text}");
    let section = &doc["gaps"]["requirement"];
    assert_eq!(section["atomized"], true);
    assert_eq!(section["unmapped"], json!([{"id": "A2", "span": "A tag also carries a label."}]));
    assert_eq!(section["untraced_commands"], json!([]));
    assert_eq!(text.contains("traced to no definition element: A2 \"A tag also carries a label.\""), true, "{text}");
    assert_eq!(text.contains("(all commands): none found."), true, "{text}");
    let left_out = atoms_requirement(json!({"atoms": [{"id": "A1", "span": "between 1 and 64 characters"}]}));
    let (_, doc, _) = handoff_kind(&dir, &tag_behavior(json!({"requirement": left_out})), &evidence, "delivery");
    let uncovered = doc["gaps"]["requirement"]["uncovered_sentences"].as_array().expect("uncovered").clone();
    assert_eq!(uncovered.len(), 2, "{uncovered:?}");
    assert_eq!(uncovered[0].as_str().unwrap_or("").starts_with("The id may use"), true, "a fragment of a sentence does not cover it: {uncovered:?}");
    assert_eq!(uncovered[1].as_str().unwrap_or("").starts_with("A tag also carries"), true, "{uncovered:?}");
}

#[test]
fn a_stated_origin_inside_an_atom_traces_that_atom_and_not_applicable_removes_it_from_the_gaps() {
    let (dir, evidence) = tag_setup("gaps-implicit");
    let hash = tag_hash(&dir, &evidence);
    let stated = origin(&hash, "max_length:64", "stated", json!({"span": "between 1 and 64 characters"}));
    let requirement = atoms_requirement(json!({"not_applicable": [{"atom": "A2", "reason": "describes a stored field, not behavior"}]}));
    let behavior = tag_behavior(json!({"requirement": requirement, "origins": [stated]}));
    let (code, doc, text) = handoff_kind(&dir, &behavior, &evidence, "delivery");
    assert_eq!(code, Some(0), "{text}");
    let section = &doc["gaps"]["requirement"];
    assert_eq!(section["unmapped"], json!([]), "{section}");
    assert_eq!(section["untraced_commands"], json!([]), "{section}");
    assert_eq!(section["not_applicable"][0]["id"], "A2");
    assert_eq!(section["traced_by_stated_origin"][0]["element"], "command:tag");
}

#[test]
fn a_trace_that_names_nothing_real_is_refused_before_a_human_sees_it() {
    let (dir, evidence) = tag_setup("gaps-refused");
    let refusal = |extra: Value| {
        let behavior = tag_behavior(json!({"requirement": atoms_requirement(extra)}));
        refused(&dir, &behavior, &evidence, "delivery")
    };
    let ghost = refusal(json!({"traces": [{"atom": "A1", "element": "command:nope"}]}));
    assert_eq!(ghost.contains("the definition has no command `nope`"), true, "{ghost}");
    let unknown_atom = refusal(json!({"traces": [{"atom": "A9", "element": "command:tag"}]}));
    assert_eq!(unknown_atom.contains("names atom `A9`, which is not listed"), true, "{unknown_atom}");
    let bad_point = refusal(json!({"traces": [{"atom": "A1", "element": "point:tag/max_length:65"}]}));
    assert_eq!(bad_point.contains("is not a decision point of rule `tag`"), true, "{bad_point}");
    let not_verbatim = refusal(json!({"atoms": [{"id": "A1", "span": "at most 64 characters"}]}));
    assert_eq!(not_verbatim.contains("not a verbatim part of the requirement text"), true, "{not_verbatim}");
    let duplicate = refusal(json!({"atoms": [{"id": "A1", "span": "between 1 and 64 characters"}, {"id": "A1", "span": "A tag also carries a label."}]}));
    assert_eq!(duplicate.contains("appears more than once"), true, "{duplicate}");
    let no_reason = refusal(json!({"not_applicable": [{"atom": "A2", "reason": " "}]}));
    assert_eq!(no_reason.contains("marked not applicable with no reason"), true, "{no_reason}");
    let both = refusal(json!({"traces": [{"atom": "A2", "element": "command:tag"}], "not_applicable": [{"atom": "A2", "reason": "x"}]}));
    assert_eq!(both.contains("both traced and marked not applicable"), true, "{both}");
}

#[test]
fn only_runs_observed_against_the_definition_clear_an_evidence_gap_and_a_wrong_reason_refusal_never_counts() {
    let dir = temp_dir("evidence-gaps");
    let bundle = bundle(&dir, "refused:IK_CAPABILITY_STALE");
    let (_, doc, text) = handoff(&dir, &delivery_behavior(json!([])), &bundle);
    let rows: Vec<(String, String)> = doc["gaps"]["evidence"]
        .as_array()
        .expect("evidence rows")
        .iter()
        .map(|row| (row["kind"].as_str().unwrap_or("").to_string(), row["subject"].as_str().unwrap_or("").to_string()))
        .collect();
    let has = |kind: &str, subject: &str| rows.contains(&(kind.to_string(), subject.to_string()));
    assert_eq!(has("no_committed_run_cited", "edit"), false, "a committed edit is cited: {rows:?}");
    assert_eq!(has("no_committed_run_cited", "approve"), true, "{rows:?}");
    assert_eq!(has("no_guard_refusal_cited", "approve"), true, "a capability refusal does not exercise a guard: {rows:?}");
    assert_eq!(text.contains("No run refused with IK_INVARIANT_FAILED (a guard refusal) is cited for"), true, "{text}");
}

#[test]
fn every_handoff_carries_one_claim_sentence_derived_from_its_own_facts() {
    let (dir, evidence) = tag_setup("claim-delivery");
    let (code, doc, text) = handoff_kind(&dir, &tag_behavior(json!({})), &evidence, "delivery");
    assert_eq!(code, Some(0), "{text}");
    let claim = doc["claim"].as_str().unwrap_or("");
    assert_eq!(claim.starts_with("Desmo checked whether the state transitions behave as the definition says"), true, "{claim}");
    assert_eq!(claim.contains("1 of 1 rule acceptances are awaited; "), true, "{claim}");
    assert_eq!(claim.contains("material definition gaps are listed over a stated scope; "), true, "{claim}");
    assert_eq!(claim.ends_with("or readiness, which is the owner's decision"), true, "{claim}");
    assert_eq!(claim.contains(". "), false, "the claim must be one sentence: {claim}");
    let line = text.lines().find(|line| line.starts_with("Claim: ")).unwrap_or("");
    assert_eq!(line.starts_with("Claim: Desmo checked"), true, "{text}");

    let (_, accepted, _) = handoff_kind(&dir, &tag_behavior(json!({"accepted_rules": [{"rule": "tag", "guard_hash": tag_hash(&dir, &evidence), "examples_hash": doc["decisions"][0]["rules"][0]["examples_hash"].clone()}]})), &evidence, "delivery");
    let accepted_claim = accepted["claim"].as_str().unwrap_or("");
    assert_eq!(accepted_claim.contains("rule acceptances are awaited"), false, "{accepted_claim}");
}

#[test]
fn a_recovery_claim_names_only_what_a_recovery_checks() {
    let dir = temp_dir("claim-recovery");
    let (bundle, _) = write_lookup_result(&dir, "item-lost", "op-tag");
    let behavior = json!({"behavior_version": 1, "title": "lost reply", "outcomes": []});
    let (code, doc, _) = handoff_kind(&dir, &behavior, &bundle, "recovery");
    assert_eq!(code, Some(0), "{doc}");
    let claim = doc["claim"].as_str().unwrap_or("");
    assert_eq!(claim.starts_with("Desmo checked whether a recorded operation exists in the store under the given id and request; it does not claim"), true, "{claim}");
    assert_eq!(claim.contains("rule acceptances"), false, "{claim}");
    assert_eq!(claim.contains("definition gaps"), false, "{claim}");
}
