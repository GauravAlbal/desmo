//! IK-09: a flat (region-less) machine whose transition violates its own invariant is REFUSED by
//! `ik check`, with the bounded counterexample path, instead of being silently accepted.

use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/publish-approval/machines")
        .join(name)
}

fn check_json(machine: &str, corpus: &str) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_ik"))
        .args(["check"])
        .arg(example(machine))
        .arg("--corpus")
        .arg(example(corpus))
        .arg("--json")
        .output()
        .expect("run ik check");
    serde_json::from_slice(&out.stdout).expect("check emits json")
}

#[test]
fn defective_flat_machine_is_refused_with_the_shortest_witness_path() {
    let report = check_json(
        "publish_approval_defective.ik.toml",
        "publish_approval_defective.cases.json",
    );
    assert_eq!(report["judgment"]["verdict"], "refused");
    assert_eq!(report["judgment"]["status"], "witnessed_violation");
    let witnesses = report["invariant_witnesses"].as_array().expect("witnesses");
    assert_eq!(witnesses.len(), 1);
    assert_eq!(witnesses[0]["diagnostic"]["rule"], "pub_inv_001");
    let commands: Vec<&str> = witnesses[0]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["command"].as_str().unwrap())
        .collect();
    assert_eq!(commands, ["edit", "approve", "revise", "publish"]);
}

#[test]
fn text_report_names_the_witness_and_a_correct_machine_still_accepts() {
    let out = Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("check")
        .arg(example("publish_approval_defective.ik.toml"))
        .arg("--corpus")
        .arg(example("publish_approval_defective.cases.json"))
        .output()
        .expect("run ik check");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("INVARIANT-REFUSAL WITNESS"), "{text}");
    assert!(
        text.contains("steps: edit[doc-a] approve[doc-a] revise[doc-b] publish"),
        "{text}"
    );

    let good = check_json(
        "publish_approval_v3.ik.toml",
        "publish_approval_v3.cases.json",
    );
    assert_eq!(good["judgment"]["verdict"], "accept");
    assert_eq!(good["judgment"]["status"], "accepted");
    assert_eq!(
        good["invariant_witnesses"].as_array().map(Vec::len),
        Some(0)
    );
}
