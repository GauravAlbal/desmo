//! DX-03: derived structural machine graph and analysis.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::canonical::canonical_json;
use ik::declarative::{analyze, machine_graph, parse_definition};

#[test]
fn verification_attempt_graph_is_complete() {
    let def =
        parse_definition(&std::fs::read_to_string(machine_def("verification_attempt")).unwrap())
            .expect("def");
    let graph = machine_graph(&def);
    let names: Vec<_> = graph.states.iter().map(|s| s.name.as_str()).collect();
    for expected in [
        "created",
        "running",
        "submitted",
        "repairable",
        "verified",
        "exhausted",
        "cancelled",
    ] {
        assert!(names.contains(&expected), "{names:?}");
    }
    let reject: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.command == "reject")
        .collect();
    assert_eq!(reject.len(), 2, "{reject:?}");
    assert_eq!(reject[0].outcome_index, 0);
    assert_eq!(reject[0].target, "repairable");
    assert_eq!(reject[1].outcome_index, 1);
    assert_eq!(reject[1].target, "exhausted");
    assert!(graph
        .states
        .iter()
        .any(|s| s.initial && s.name == "created"));
    assert!(graph.states.iter().filter(|s| s.terminal).all(|s| [
        "verified",
        "exhausted",
        "cancelled"
    ]
    .contains(&s.name.as_str())));
}

#[test]
fn unreachable_state_is_reported() {
    let def = parse_definition(
        r#"
schema_version = 1
machine_id = "u"
machine_version = 1
[facts]
[states]
s = { initial = true }
t = {}
ghost = {}
[commands.go]
from = ["s"]
to = "t"
"#,
    )
    .expect("def");
    let analysis = analyze(&machine_graph(&def));
    assert_eq!(analysis.unreachable_states, vec!["ghost".to_string()]);
}

#[test]
fn cycle_is_reported() {
    let def = parse_definition(
        r#"
schema_version = 1
machine_id = "c"
machine_version = 1
[facts]
[states]
a = { initial = true }
b = {}
[commands.ab]
from = ["a"]
to = "b"
[commands.ba]
from = ["b"]
to = "a"
"#,
    )
    .expect("def");
    let analysis = analyze(&machine_graph(&def));
    assert!(
        analysis
            .cycles
            .iter()
            .any(|scc| scc == &["a".to_string(), "b".to_string()]),
        "{:?}",
        analysis.cycles
    );
}

#[test]
fn shortest_structural_path_is_deterministic() {
    let def =
        parse_definition(&std::fs::read_to_string(machine_def("verification_attempt")).unwrap())
            .expect("def");
    let graph = machine_graph(&def);
    let analysis = analyze(&graph);
    let to_verified = analysis
        .shortest_paths
        .get("verified")
        .expect("verified reachable");
    assert_eq!(
        to_verified.states,
        ["created", "running", "submitted", "verified"]
    );
    let again = analyze(&machine_graph(&def));
    assert_eq!(canonical_json(&analysis), canonical_json(&again));
    assert_eq!(canonical_json(&graph), canonical_json(&machine_graph(&def)));
}

#[test]
fn multi_outcome_second_branch_is_not_omitted() {
    let def = parse_definition(
        r#"
schema_version = 1
machine_id = "m"
machine_version = 1
[facts]
n = { type = "int", policy = "replace", initial = 0 }
[states]
s = { initial = true }
a = {}
b = {}
[commands.split]
from = ["s"]
outcomes = [
  { when = "n == 0", to = "a" },
  { when = "n == 1", to = "b" }
]
"#,
    )
    .expect("def");
    let graph = machine_graph(&def);
    let edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| e.command == "split")
        .map(|e| (e.outcome_index, e.target.as_str()))
        .collect();
    assert_eq!(edges, vec![(0, "a"), (1, "b")]);
}
