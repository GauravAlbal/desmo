//! Bounded deterministic exploration of VerificationAttempt via the generic
//! FIFO explorer. The old hand-authored `commands_from(state)` ledger is gone.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::{explore, Bounds, CaseCorpus};
use serde_json::{json, Value};
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

#[test]
fn bounded_exploration_reaches_all_states_and_preserves_invariants() {
    let machine = load_machine("verification_attempt");
    let run = explore(
        &machine,
        &va_corpus(),
        Bounds {
            max_nodes: 4096,
            max_depth: 16,
        },
    );
    for intended in [
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
                .any(|s| s == intended),
            "state `{intended}` not reachable; {:?}",
            run.coverage.executably_reached_states
        );
    }
    for terminal in ["verified", "exhausted", "cancelled"] {
        assert!(
            run.coverage
                .reached_terminal_states
                .iter()
                .any(|s| s == terminal),
            "terminal `{terminal}` never reached; {:?}",
            run.coverage.reached_terminal_states
        );
    }
}

#[test]
fn exploration_is_deterministic() {
    let machine = load_machine("verification_attempt");
    let corpus = va_corpus();
    let bounds = Bounds {
        max_nodes: 4096,
        max_depth: 16,
    };
    let a = explore(&machine, &corpus, bounds);
    let b = explore(&machine, &corpus, bounds);
    assert_eq!(a, b);
}
