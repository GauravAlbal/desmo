//! VFY-07: law-linked semantic mutation adequacy.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::{
    curated_stale_capability_killed, mutation_report, parse_definition, run_guard_boundary_mutant,
    Bounds, CaseCorpus, Definition,
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
    corpus
}

fn bounds() -> Bounds {
    Bounds {
        max_nodes: 4096,
        max_depth: 16,
    }
}

#[test]
fn reject_lt_le_mutant_is_killed() {
    let result =
        run_guard_boundary_mutant(&va_def(), "reject", 0, &va_corpus(), bounds()).expect("mutant");
    assert_eq!(result.kind, "CHL-GUARD-BOUNDARY");
    assert_eq!(result.status, "killed");
    assert!(result.witness.is_some(), "{result:?}");
}

#[test]
fn adequacy_is_reported_by_challenge() {
    let result =
        run_guard_boundary_mutant(&va_def(), "reject", 0, &va_corpus(), bounds()).expect("mutant");
    let report = mutation_report(vec![result, curated_stale_capability_killed()]);
    assert_eq!(report.schema_version, 1);
    assert!(report
        .results
        .iter()
        .any(|r| r.kind == "CHL-GUARD-BOUNDARY"));
    assert!(report.results.iter().any(|r| r.kind == "OBL-CAP-001"));
    let encoded = serde_json::to_string(&report).expect("json");
    assert!(!encoded.contains("mutation_score"));
    assert!(!encoded.contains("94%"));
}

#[test]
fn stale_capability_law_is_recorded_killed() {
    let row = curated_stale_capability_killed();
    assert_eq!(row.status, "killed");
    assert_eq!(row.engine, "store.command");
    assert_eq!(
        row.witness.as_deref(),
        Some("stale_capability_sequence_refuses")
    );
}
