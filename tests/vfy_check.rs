//! DEVEX-01: ik check composes the verification loop into one command.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::def::OutcomeDef;
use ik::declarative::{
    check_machine, parse_definition, report_from_evidence, Bounds, CaseCorpus, Definition,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn va_def() -> Definition {
    parse_definition(&std::fs::read_to_string(machine_def("verification_attempt")).unwrap())
        .expect("va")
}

fn args(pairs: &[(&str, &str)]) -> BTreeMap<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), json!(*v)))
        .collect()
}

fn va_corpus() -> CaseCorpus {
    let mut corpus = CaseCorpus::new();
    corpus.insert(
        "submit",
        "happy",
        args(&[("artifact_digest", "a1"), ("baseline_digest", "b1")]),
    );
    corpus.insert(
        "verify",
        "match",
        args(&[
            ("receipt_digest", "r1"),
            ("receipt_artifact_digest", "a1"),
            ("receipt_baseline_digest", "b1"),
        ]),
    );
    corpus
}

fn bounds() -> Bounds {
    Bounds {
        max_nodes: 4096,
        max_depth: 16,
    }
}

fn baseline() -> Vec<String> {
    let text = std::fs::read_to_string(repo_root().join("validation/verification_baseline.json"))
        .expect("baseline");
    serde_json::from_str::<serde_json::Value>(&text)
        .expect("json")
        .get("uncovered_mandatory")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

fn quarantine_edit(old: &Definition) -> Definition {
    let mut new = old.clone();
    new.states.insert(
        "quarantine".into(),
        ik::declarative::def::StateDef {
            initial: false,
            terminal: true,
        },
    );
    new.commands
        .get_mut("reject")
        .expect("reject")
        .outcomes
        .as_mut()
        .expect("outcomes")
        .insert(
            0,
            OutcomeDef {
                when: Some("attempt_count == 0".into()),
                to: "quarantine".into(),
                then: None,
            },
        );
    new
}

#[test]
fn report_counts_are_derived() {
    let plan = ik::declarative::verification_plan(&va_def());
    let report = report_from_evidence(&plan, &[], &[]);
    assert_eq!(report.obligations.total, plan.obligations.len());
    assert_eq!(report.obligations.uncovered, plan.obligations.len());
    assert_eq!(report.obligations.satisfied, 0);
}

#[test]
fn quarantine_edit_refuses_with_repair_packet() {
    let old = va_def();
    let new = quarantine_edit(&old);
    let result =
        check_machine(&new, Some(&old), &va_corpus(), &baseline(), bounds()).expect("check");
    assert_eq!(result.judgment.verdict, "refused");
    assert_eq!(result.judgment.status, "missing_evidence");
    assert!(result.text.contains("NEW SEMANTIC DEBT"), "{}", result.text);
    assert!(
        result.text.contains("attempt_count == 0 → quarantine")
            || result.text.contains("outcome/reject/0/executable"),
        "{}",
        result.text
    );
    assert!(
        result.text.contains("DOWNSTREAM BEHAVIOR LOST"),
        "{}",
        result.text
    );
    assert!(result.text.contains("repairable"), "{}", result.text);
    assert!(result.text.contains("WHY"), "{}", result.text);
    assert!(result.text.contains("first-match"), "{}", result.text);
    assert!(result.text.contains("NEXT"), "{}", result.text);
    assert!(
        result.text.contains("REFUSED") || result.text.contains("refuse"),
        "{}",
        result.text
    );
}

#[test]
fn unchanged_machine_accepts_frozen_debt() {
    let def = va_def();
    let result =
        check_machine(&def, Some(&def), &va_corpus(), &baseline(), bounds()).expect("check");
    assert_eq!(result.judgment.verdict, "accept", "{}", result.text);
    assert_eq!(result.judgment.status, "accepted", "{}", result.text);
}

#[test]
fn cli_check_quarantine_exits_refused() {
    let old = machine_def("verification_attempt");
    let dir = tempfile::tempdir().expect("tmp");
    let edited = dir.path().join("verification_attempt.ik.toml");
    let src = std::fs::read_to_string(&old).expect("old");
    let patched = src
        .replace(
            "cancelled = { terminal = true }",
            "cancelled = { terminal = true }\nquarantine = { terminal = true }",
        )
        .replace(
            "outcomes = [\n  { when = \"attempt_count + 1 < max_attempts\", to = \"repairable\", then = { attempt_count = \"+1\" } },",
            "outcomes = [\n  { when = \"attempt_count == 0\", to = \"quarantine\" },\n  { when = \"attempt_count + 1 < max_attempts\", to = \"repairable\", then = { attempt_count = \"+1\" } },",
        );
    std::fs::write(&edited, patched).expect("write");
    let corpus = repo_root().join("machines/verification_attempt.cases.json");
    let baseline = repo_root().join("validation/verification_baseline.json");
    let code = ik::cli::run(&[
        "check".into(),
        edited.display().to_string(),
        "--prior".into(),
        old.display().to_string(),
        "--corpus".into(),
        corpus.display().to_string(),
        "--baseline".into(),
        baseline.display().to_string(),
    ]);
    assert_eq!(code, 1, "expected refuse");
}
