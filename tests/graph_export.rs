//! Finite-graph export from the existing explorer.

use std::process::Command;

use serde_json::Value;

fn bin() -> Command {
    let path = std::env::var("IK_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_ik").to_string());
    Command::new(path)
}

fn fixture(name: &str) -> String {
    format!(
        "{}/tests/fixtures/profile/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn export(machine: &str, extra: &[&str]) -> Value {
    let dir = tempfile::tempdir().expect("temp");
    let out = dir.path().join("graph.json");
    let mut cmd = bin();
    cmd.arg("graph")
        .arg("export")
        .arg(machine)
        .arg("--out")
        .arg(&out);
    for arg in extra {
        cmd.arg(arg);
    }
    let done = cmd.output().expect("export");
    assert_eq!(
        done.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    let text = std::fs::read_to_string(&out).expect("graph");
    serde_json::from_str(&text).expect("json")
}

fn edges<'a>(graph: &'a Value, outcome_prefix: &str) -> Vec<&'a Value> {
    graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| {
            edge["outcome"]
                .as_str()
                .unwrap()
                .starts_with(outcome_prefix)
        })
        .collect()
}

#[test]
fn complete_export_closes_the_walked_graph_and_records_missing_cases() {
    let graph = export(&fixture("linear.ik.toml"), &[]);
    assert_eq!(graph["schema"], "ik.graph.v1");
    assert_eq!(graph["complete"], true);
    assert!(graph["incomplete_reason"].is_null());
    assert!(graph["frontier"].as_array().unwrap().is_empty());
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(edges(&graph, "committed").len(), 2);
    let restricted = export(&fixture("needs_args.ik.toml"), &[]);
    let limits: Vec<&str> = restricted["restrictions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["limit"].as_str().unwrap())
        .collect();
    assert!(limits.contains(&"no_corpus_case"), "{limits:?}");
    assert!(restricted["edges"]
        .as_array()
        .unwrap()
        .iter()
        .all(|edge| edge["command"] != "go"));
}

#[test]
fn tiny_depth_bound_is_incomplete_and_keeps_the_frontier_edge() {
    let graph = export(&fixture("linear.ik.toml"), &["--max-depth", "1"]);
    assert_eq!(graph["complete"], false);
    assert_eq!(graph["incomplete_reason"], "max_depth");
    let frontier = graph["frontier"].as_array().unwrap();
    assert_eq!(frontier.len(), 1, "{graph}");
    let frontier_id = frontier[0].as_str().unwrap();
    let entered = edges(&graph, "committed")
        .into_iter()
        .any(|edge| edge["to"] == frontier_id && edge["from"] != frontier_id);
    assert!(entered, "depth-cut edge was dropped: {graph}");
    let expanded = edges(&graph, "committed")
        .into_iter()
        .any(|edge| edge["from"] == frontier_id);
    assert!(!expanded, "frontier node was expanded: {graph}");
}

#[test]
fn refusal_edges_are_self_loops_distinct_from_commits() {
    let graph = export(&fixture("linear.ik.toml"), &[]);
    let refused = edges(&graph, "refused:");
    assert!(!refused.is_empty(), "{graph}");
    assert!(refused.iter().all(|edge| edge["from"] == edge["to"]));
    assert!(refused.iter().any(|edge| {
        edge["outcome"] == "refused:IK_ILLEGAL_STATE" && edge["command"] == "finish"
    }));
    let committed = edges(&graph, "committed");
    assert!(committed
        .iter()
        .any(|edge| edge["from"] != edge["to"] && edge["outcome"] == "committed"));
    assert!(committed.iter().all(|edge| edge["outcome"] == "committed"));
}
