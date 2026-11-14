//! DX-04/05: generic FIFO exploration and coverage honesty.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::{explore, parse_and_compile, Bounds, CaseCorpus};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

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
    corpus.insert(
        "verify",
        "mismatch",
        args(&[
            ("receipt_digest", "r2"),
            ("receipt_artifact_digest", "WRONG"),
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
fn fifo_returns_known_shortest_witness() {
    let machine = parse_and_compile(
        r#"
schema_version = 1
machine_id = "short"
machine_version = 1
[facts]
[states]
s0 = { initial = true }
mid = {}
goal = { terminal = true }
[commands.direct]
from = ["s0"]
to = "goal"
[commands.via]
from = ["s0"]
to = "mid"
[commands.finish]
from = ["mid"]
to = "goal"
"#,
    )
    .expect("compiles");
    let run = explore(&machine, &CaseCorpus::new(), bounds());
    let witness = run.witnesses.by_terminal.get("goal").expect("goal reached");
    assert_eq!(witness.len(), 1, "{witness:?}");
    assert_eq!(witness[0].command, "direct");
}

#[test]
fn semantic_dedup_does_not_coalesce_distinct_facts() {
    let machine = parse_and_compile(
        r#"
schema_version = 1
machine_id = "count"
machine_version = 1
[facts]
n = { type = "int", policy = "counter", initial = 0 }
[states]
s = { initial = true }
done = { terminal = true }
[commands.inc]
from = ["s"]
to = "s"
then = { n = "+1" }
[commands.finish]
from = ["s"]
when = "n == 2"
to = "done"
"#,
    )
    .expect("compiles");
    let run = explore(&machine, &CaseCorpus::new(), bounds());
    assert!(
        run.coverage
            .reached_terminal_states
            .iter()
            .any(|s| s == "done"),
        "{:?}",
        run.coverage.reached_terminal_states
    );
    assert!(
        run.coverage.executably_reached_semantic_states >= 4,
        "{}",
        run.coverage.executably_reached_semantic_states
    );
    let witness = run.witnesses.by_terminal.get("done").expect("done");
    assert_eq!(witness.len(), 3, "{witness:?}");
}

#[test]
fn va_exploration_reaches_all_labels_and_both_reject_outcomes() {
    let machine = load_machine("verification_attempt");
    let run = explore(&machine, &va_corpus(), bounds());
    for label in [
        "created",
        "running",
        "submitted",
        "repairable",
        "verified",
        "exhausted",
        "cancelled",
    ] {
        assert!(
            run.coverage
                .executably_reached_states
                .iter()
                .any(|s| s == label),
            "missing {label}: {:?}",
            run.coverage.executably_reached_states
        );
    }
    assert!(run.witnesses.by_outcome.contains_key("reject.0"));
    assert!(run.witnesses.by_outcome.contains_key("reject.1"));
}

#[test]
fn removing_verify_case_lowers_executable_coverage() {
    let machine = load_machine("verification_attempt");
    let full = explore(&machine, &va_corpus(), bounds());
    let mut reduced = va_corpus();
    reduced
        .commands
        .get_mut("verify")
        .unwrap()
        .retain(|c| c.name != "match");
    let run = explore(&machine, &reduced, bounds());
    assert!(
        !run.coverage
            .reached_terminal_states
            .iter()
            .any(|s| s == "verified"),
        "{:?}",
        run.coverage.reached_terminal_states
    );
    assert!(run.coverage.unreached_outcomes.iter().any(|u| {
        u.command == "verify"
            && u.outcome_index == 0
            && u.structural == "structurally_present"
            && u.executable == "executably_unreached"
    }));
    assert!(
        run.coverage.executably_reached_states.len()
            < full.coverage.executably_reached_states.len()
            || run.coverage.executed_outcomes.len() < full.coverage.executed_outcomes.len()
    );
}

#[test]
fn bound_exceeded_is_incomplete() {
    let machine = load_machine("verification_attempt");
    let run = explore(
        &machine,
        &va_corpus(),
        Bounds {
            max_nodes: 1,
            max_depth: 16,
        },
    );
    assert!(!run.complete);
    assert_eq!(run.incomplete_reason.as_deref(), Some("max_nodes"));
}

#[test]
fn no_statechart_runtime_dependency() {
    let toml = include_str!("../Cargo.toml");
    let lowered = toml.to_ascii_lowercase();
    assert!(!lowered.contains("xstate"));
    assert!(!lowered.contains("scxml"));
}

#[test]
fn coverage_artifact_is_json_map() {
    let machine = load_machine("verification_attempt");
    let run = explore(&machine, &va_corpus(), bounds());
    let value = serde_json::to_value(&run).expect("json");
    assert!(value.as_object().is_some());
    let _ = Map::new();
}
