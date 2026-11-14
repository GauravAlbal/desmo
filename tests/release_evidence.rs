//! IK-05: obligation status is kept apart from release policy, and a developer
//! ACCEPT never implies a strict release pass.

use ik::declarative::{
    check_machine, parse_definition, standard_lane, Bounds, CaseCorpus, CheckResult, Definition,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::process::Command;

const CHAIN: &str = r#"
schema_version = 1
machine_id = "chain"
machine_version = 1
name = "Chain"

[facts]

[states]
open = { initial = true }
middle = {}
done = { terminal = true }

[commands.advance]
from = ["open"]
to = "middle"

[commands.finish]
from = ["middle"]
to = "done"
"#;

const ARGS: &str = r#"
schema_version = 1
machine_id = "needs_args"
machine_version = 1
name = "NeedsArgs"

[facts]

[states]
open = { initial = true }
done = { terminal = true }

[commands.finish]
from = ["open"]
to = "done"
args = { token = "string" }
"#;

fn def(source: &str) -> Definition {
    parse_definition(source).expect("definition parses")
}

fn bounds(max_depth: usize) -> Bounds {
    Bounds {
        max_nodes: 4096,
        max_depth,
    }
}

fn token_corpus() -> CaseCorpus {
    let mut corpus = CaseCorpus::new();
    let args: BTreeMap<String, serde_json::Value> =
        [("token".to_string(), json!("t1"))].into_iter().collect();
    corpus.insert("finish", "ok", args);
    corpus
}

fn check(source: &str, corpus: &CaseCorpus, baseline: &[String], depth: usize) -> CheckResult {
    check_machine(&def(source), None, corpus, baseline, bounds(depth)).expect("check runs")
}

fn uncovered_ids() -> Vec<String> {
    let bare = check(ARGS, &CaseCorpus::new(), &[], 16);
    rows_with_status(&bare, "uncovered")
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn rows_with_status<'a>(result: &'a CheckResult, status: &str) -> Vec<&'a str> {
    result
        .release
        .obligations
        .iter()
        .filter(|row| serde_json::to_value(row.status).unwrap() == json!(status))
        .map(|row| row.obligation_id.as_str())
        .collect()
}

#[test]
fn a_complete_fully_witnessed_standard_lane_passes_release_and_is_bound_to_its_inputs() {
    let result = check(CHAIN, &CaseCorpus::new(), &[], 16);
    assert_eq!(
        result.release.release.verdict, "release_pass",
        "{:?}",
        result.release.release.reasons
    );
    let provenance = &result.release.provenance;
    assert!(provenance.exploration_complete);
    assert_eq!(provenance.machine_id, "chain");
    assert_eq!(provenance.bounds, bounds(16));
    assert!(!provenance.corpus_digest.is_empty());
    assert!(
        provenance.runtime_artifact_digest.is_none(),
        "an unavailable artifact digest must stay absent, never fabricated"
    );
    assert!(result
        .release
        .release
        .excluded_assumptions
        .iter()
        .any(|a| a.contains("runtime artifact digest is unavailable")));
}

#[test]
fn obligations_outside_the_standard_lane_are_not_examined_never_satisfied() {
    let result = check(CHAIN, &CaseCorpus::new(), &[], 16);
    let satisfied = rows_with_status(&result, "satisfied");
    let not_examined = rows_with_status(&result, "not_examined");
    assert!(
        !not_examined.is_empty(),
        "the plan has non-standard obligations"
    );
    for row in &result.release.obligations {
        let is_satisfied = satisfied.contains(&row.obligation_id.as_str());
        assert!(
            !is_satisfied || standard_lane(&row.kind),
            "{} ({}) was counted satisfied without a verifier",
            row.obligation_id,
            row.kind
        );
    }
    assert_eq!(
        result.release.status_counts.values().sum::<usize>(),
        result.release.obligations.len(),
        "every obligation has exactly one status"
    );
}

#[test]
fn forcing_max_depth_truncation_blocks_release_and_marks_unseen_work_incomplete() {
    let truncated = check(CHAIN, &CaseCorpus::new(), &[], 1);
    let provenance = &truncated.release.provenance;
    assert!(!provenance.exploration_complete);
    assert_eq!(provenance.completion_reason.as_deref(), Some("max_depth"));
    assert_eq!(truncated.release.release.verdict, "release_refused");
    assert!(truncated
        .release
        .release
        .reasons
        .iter()
        .any(|r| r.contains("exploration is incomplete (max_depth)")));
    assert!(
        !rows_with_status(&truncated, "incomplete").is_empty(),
        "unwitnessed work under truncation is incomplete, not merely uncovered"
    );
    assert!(rows_with_status(&truncated, "uncovered").is_empty());

    let complete = check(CHAIN, &CaseCorpus::new(), &[], 16);
    assert_eq!(complete.release.release.verdict, "release_pass");
}

#[test]
fn removing_the_sole_coverage_witness_leaves_the_obligation_unwitnessed() {
    let witnessed = check(ARGS, &token_corpus(), &[], 16);
    assert_eq!(witnessed.release.release.verdict, "release_pass");

    let removed = check(ARGS, &CaseCorpus::new(), &[], 16);
    assert_eq!(removed.release.release.verdict, "release_refused");
    assert!(
        rows_with_status(&removed, "uncovered").contains(&"command/finish/from/open/executable")
    );
    assert!(removed
        .release
        .release
        .reasons
        .iter()
        .any(|r| r.contains("nothing was executed")));
}

#[test]
fn injected_baseline_debt_is_approved_debt_which_permits_developer_accept_but_not_release() {
    let debt = uncovered_ids();
    let result = check(ARGS, &CaseCorpus::new(), &debt, 16);
    assert!(
        rows_with_status(&result, "approved_debt").contains(&"command/finish/from/open/executable"),
        "baseline debt must be named as approved debt, not satisfied or uncovered"
    );
    assert!(
        !rows_with_status(&result, "satisfied").contains(&"command/finish/from/open/executable")
    );
    assert_eq!(
        result.judgment.verdict, "accept",
        "{:?}",
        result.judgment.reasons
    );
    assert_eq!(result.verdict, "ACCEPT");
    assert_eq!(result.release.release.verdict, "release_refused");
    assert!(result
        .release
        .release
        .reasons
        .iter()
        .any(|r| r.contains("approved baseline debt")));
}

#[test]
fn the_cli_exits_nonzero_for_release_when_only_developer_accept_holds() {
    let dir = std::env::temp_dir().join(format!("ik-release-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let machine = dir.join("needs_args.ik.toml");
    std::fs::write(&machine, ARGS).unwrap();
    let baseline = dir.join("baseline.json");
    std::fs::write(
        &baseline,
        json!({"machine_id": "needs_args", "uncovered_mandatory": uncovered_ids()}).to_string(),
    )
    .unwrap();

    let developer = Command::new(env!("CARGO_BIN_EXE_ik"))
        .args(["check"])
        .arg(&machine)
        .arg("--baseline")
        .arg(&baseline)
        .output()
        .unwrap();
    let strict = Command::new(env!("CARGO_BIN_EXE_ik"))
        .args(["check"])
        .arg(&machine)
        .arg("--baseline")
        .arg(&baseline)
        .arg("--release")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&strict.stdout);
    assert_eq!(
        developer.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&developer.stdout)
    );
    assert_eq!(strict.status.code(), Some(1));
    assert!(text.contains("verdict release_refused"), "{text}");
    assert!(text.contains("RELEASE POLICY"), "{text}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_law_inventory_links_each_invariant_to_its_obligation_with_an_honest_status() {
    let machine = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("machines")
        .join("verification_attempt.ik.toml");
    let laws = Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("laws")
        .arg(&machine)
        .output()
        .unwrap();
    let inventory: serde_json::Value = serde_json::from_slice(&laws.stdout).unwrap();
    let rows = inventory["laws"].as_array().unwrap();
    assert!(!rows.is_empty());
    let result = check_machine(
        &def(&std::fs::read_to_string(&machine).unwrap()),
        None,
        &CaseCorpus::new(),
        &[],
        bounds(16),
    )
    .unwrap();
    for row in rows {
        let linked = row["evidence"]["obligation_id"].as_str().unwrap();
        assert_eq!(row["evidence"]["status"], "not_examined", "{row}");
        assert!(
            result
                .release
                .obligations
                .iter()
                .any(|o| o.obligation_id == linked),
            "law {} links to {linked}, which is not a plan obligation",
            row["id"]
        );
    }
}
