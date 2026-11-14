//! DX-06: bounded behavioral machine diff controls.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::{diff_machines, parse_definition, Bounds, CaseCorpus};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn args(pairs: &[(&str, &str)]) -> BTreeMap<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), json!(*v)))
        .collect()
}

fn va_text() -> String {
    std::fs::read_to_string(machine_def("verification_attempt")).expect("va toml")
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

#[test]
fn identical_machines_observe_no_difference() {
    let def = parse_definition(&va_text()).expect("def");
    let diff = diff_machines(&def, &def, &va_corpus(), bounds()).expect("diff");
    assert!(diff.structural.states_added.is_empty());
    assert!(diff.structural.commands_added.is_empty());
    assert!(diff.structural.guards_changed.is_empty());
    assert_eq!(diff.observation, "observed_no_difference_within_bound");
    assert!(diff.executable.dimension.is_none());
    assert!(diff.old_complete);
    assert!(diff.new_complete);
    assert!(diff.paired_comparison_complete);
    assert!(diff.incomplete_reason.is_none());
    assert!(!diff.observation.contains("equivalent"));
}

#[test]
fn comment_only_difference_is_semantically_identical() {
    let old = parse_definition(&va_text()).expect("old");
    let mut commented = va_text();
    commented.push_str("\n# formatting only\n");
    let new = parse_definition(&commented).expect("new");
    let diff = diff_machines(&old, &new, &va_corpus(), bounds()).expect("diff");
    assert_eq!(diff.observation, "observed_no_difference_within_bound");
    assert_eq!(diff.old_machine.spec_hash, diff.new_machine.spec_hash);
}

#[test]
fn guard_boundary_mutation_returns_shortest_witness() {
    let old = parse_definition(&va_text()).expect("old");
    let mutated = va_text().replace(
        r#"{ when = "attempt_count + 1 < max_attempts", to = "repairable", then = { attempt_count = "+1" } },
  { when = "attempt_count + 1 >= max_attempts", to = "exhausted", then = { attempt_count = "+1" } }"#,
        r#"{ when = "attempt_count + 1 >= max_attempts", to = "repairable", then = { attempt_count = "+1" } },
  { when = "attempt_count + 1 < max_attempts", to = "exhausted", then = { attempt_count = "+1" } }"#,
    );
    let new = parse_definition(&mutated).expect("new");
    let diff = diff_machines(&old, &new, &va_corpus(), bounds()).expect("diff");
    assert_eq!(diff.observation, "behavioral_divergence");
    assert!(diff.structural.guards_changed.iter().any(|c| c == "reject"));
    let witness = diff.executable.witness.expect("witness");
    assert!(witness.iter().any(|s| s.command == "reject"), "{witness:?}");
    assert_eq!(
        diff.executable.dimension.as_deref(),
        Some("selected_outcome")
    );
}

#[test]
fn terminality_mutation_is_detected() {
    let old = parse_definition(&va_text()).expect("old");
    let mutated = va_text().replace("verified = { terminal = true }", "verified = {}");
    let new = parse_definition(&mutated).expect("new");
    let diff = diff_machines(&old, &new, &va_corpus(), bounds()).expect("diff");
    assert!(diff
        .structural
        .terminality_changed
        .iter()
        .any(|s| s == "verified"));
    assert_eq!(diff.observation, "behavioral_divergence");
    assert_eq!(diff.executable.dimension.as_deref(), Some("terminality"));
}

#[test]
fn missed_corpus_difference_does_not_claim_equivalence() {
    let old = parse_definition(&va_text()).expect("old");
    let mutated = va_text().replace(
        "$receipt_artifact_digest == artifact_digest && $receipt_baseline_digest == baseline_digest",
        "1 == 0",
    );
    let new = parse_definition(&mutated).expect("new");
    let mut corpus = CaseCorpus::new();
    corpus.insert(
        "submit",
        "happy",
        args(&[("artifact_digest", "a1"), ("baseline_digest", "b1")]),
    );
    let diff = diff_machines(&old, &new, &corpus, bounds()).expect("diff");
    assert!(diff.structural.guards_changed.iter().any(|c| c == "verify"));
    assert_eq!(diff.observation, "observed_no_difference_within_bound");
    assert!(!diff.observation.contains("equivalent"));
    assert!(!diff.observation.contains("proof"));
}

#[test]
fn truncated_paired_search_is_marked_incomplete() {
    let def = parse_definition(&va_text()).expect("def");
    let diff = diff_machines(
        &def,
        &def,
        &va_corpus(),
        Bounds {
            max_nodes: 1,
            max_depth: 16,
        },
    )
    .expect("diff");
    assert_eq!(diff.observation, "observed_no_difference_within_bound");
    assert!(!diff.observation.contains("equivalent"));
    assert!(!diff.paired_comparison_complete);
    assert_eq!(diff.incomplete_reason.as_deref(), Some("max_nodes"));
}

#[test]
fn new_only_command_yields_executable_witness() {
    let old = parse_definition(&va_text()).expect("old");
    let mut added = va_text();
    added.push_str(
        r#"
[commands.ping]
from = ["created"]
to = "created"
"#,
    );
    let new = parse_definition(&added).expect("new");
    let diff = diff_machines(&old, &new, &va_corpus(), bounds()).expect("diff");
    assert!(diff.structural.commands_added.iter().any(|c| c == "ping"));
    assert_eq!(diff.observation, "behavioral_divergence");
    let witness = diff.executable.witness.expect("witness");
    assert!(witness.iter().any(|s| s.command == "ping"), "{witness:?}");
}

#[test]
fn effect_identity_change_is_detected() {
    let old_text = std::fs::read_to_string(machine_def("effect_probe")).expect("probe");
    let new_text = old_text.replace(
        r#"payload_digest = "$payload_digest", idempotency_key = "$idempotency_key""#,
        r#"payload_digest = "other", idempotency_key = "$idempotency_key""#,
    );
    let old = parse_definition(&old_text).expect("old");
    let new = parse_definition(&new_text).expect("new");
    let mut corpus = CaseCorpus::new();
    corpus.insert(
        "plan",
        "happy",
        args(&[
            ("effect_id", "e1"),
            ("payload_digest", "A"),
            ("idempotency_key", "k1"),
        ]),
    );
    let diff = diff_machines(&old, &new, &corpus, bounds()).expect("diff");
    assert_eq!(diff.observation, "behavioral_divergence");
    assert_eq!(diff.executable.dimension.as_deref(), Some("effect"));
}
