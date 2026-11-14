//! VFY-10: MachineDiff-driven guard neighborhood adequacy.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::challenge::apply_guard_boundary;
use ik::declarative::{change_local_guards, parse_definition, Bounds, CaseCorpus, Definition};
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
    corpus
}

fn bounds() -> Bounds {
    Bounds {
        max_nodes: 4096,
        max_depth: 16,
    }
}

#[test]
fn changed_lt_guard_emits_boundary_neighborhood() {
    let old = va_def();
    let new = apply_guard_boundary(&old, "reject", 0).expect("flip < to <=");
    let report = change_local_guards(&old, &new, &va_corpus(), bounds()).expect("local");
    let reject0 = report
        .iter()
        .find(|row| {
            row.coordinate.command.as_deref() == Some("reject")
                && row.coordinate.outcome_index == Some(0)
        })
        .expect("reject.0");
    assert_eq!(reject0.declared_operator, "<=");
    let ops: Vec<_> = reject0
        .neighbors
        .iter()
        .map(|n| n.operator.as_str())
        .collect();
    assert!(ops.contains(&">"), "{ops:?}");
    assert!(ops.contains(&">="), "{ops:?}");
    assert!(ops.contains(&"<"), "{ops:?}");
}

#[test]
fn neighborhood_is_reported_per_coordinate() {
    let old = va_def();
    let new = apply_guard_boundary(&old, "reject", 0).expect("flip");
    let report = change_local_guards(&old, &new, &va_corpus(), bounds()).expect("local");
    let reject0 = report
        .iter()
        .find(|row| row.coordinate.command.as_deref() == Some("reject"))
        .expect("reject");
    assert!(reject0
        .neighbors
        .iter()
        .all(|n| n.status == "killed" || n.status == "survived" || n.status == "incomplete"));
    assert!(reject0.neighbors.iter().any(|n| n.status == "killed"));
    let encoded = serde_json::to_string(&report).expect("json");
    assert!(!encoded.contains("mutation_score"));
}

#[test]
fn equality_guard_challenges_inequality_only() {
    let old = va_def();
    let mut new = old.clone();
    new.commands
        .get_mut("reject")
        .expect("reject")
        .outcomes
        .as_mut()
        .expect("outcomes")[0]
        .when = Some("attempt_count == 0".into());
    let report = change_local_guards(&old, &new, &va_corpus(), bounds()).expect("local");
    let reject0 = report
        .iter()
        .find(|row| {
            row.coordinate.command.as_deref() == Some("reject")
                && row.coordinate.outcome_index == Some(0)
        })
        .expect("reject.0");
    assert_eq!(reject0.declared_operator, "==");
    let ops: Vec<_> = reject0
        .neighbors
        .iter()
        .map(|n| n.operator.as_str())
        .collect();
    assert_eq!(ops, vec!["!="], "{ops:?}");
}
