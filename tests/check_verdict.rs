//! `ik check --json` judgment.verdict is accept|refused; status names the reason class.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn ik() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ik"))
}

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn check_json(args: &[&str]) -> (Option<i32>, Value) {
    let output = ik().args(args).output().expect("spawn ik check");
    let doc = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    (output.status.code(), doc)
}

#[test]
fn missing_corpus_is_refused_for_missing_evidence() {
    let dir = tempfile::tempdir().expect("temp");
    let machine = dir.path().join("solo.ik.toml");
    fs::copy(
        repo("validation/dfy/journeys/fixtures/solo/tagged_item.ik.toml"),
        &machine,
    )
    .expect("copy");
    let machine = machine.to_string_lossy().into_owned();
    let (code, doc) = check_json(&["check", &machine, "--json"]);
    assert_eq!(code, Some(1));
    assert_eq!(doc["judgment"]["verdict"], "refused");
    assert_eq!(doc["judgment"]["status"], "missing_evidence");
}

#[test]
fn invariant_witness_is_refused_as_a_witnessed_violation() {
    let machine = repo("machines/invariant_probe.ik.toml");
    let machine = machine.to_string_lossy().into_owned();
    let (code, doc) = check_json(&["check", &machine, "--json"]);
    assert_eq!(code, Some(1));
    assert_eq!(doc["judgment"]["verdict"], "refused");
    assert_eq!(doc["judgment"]["status"], "witnessed_violation");
    assert_eq!(
        doc["invariant_witnesses"][0]["diagnostic"]["code"],
        "IK_INVARIANT_FAILED"
    );
}

#[test]
fn prior_check_text_states_version_and_structural_impact() {
    let prior = repo("validation/dfy/journeys/fixtures/loan_checkout.ik.toml");
    let candidate = repo("validation/dfy/journeys/fixtures/loan_v2.ik.toml");
    let corpus = repo("validation/dfy/journeys/fixtures/loan_v2.ik.cases.json");
    let prior = prior.to_string_lossy().into_owned();
    let candidate = candidate.to_string_lossy().into_owned();
    let corpus = corpus.to_string_lossy().into_owned();
    let output = ik()
        .args(["check", &candidate, "--prior", &prior, "--corpus", &corpus])
        .output()
        .expect("spawn ik check");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("CHANGE IMPACT"), "{text}");
    assert!(
        text.contains("new machine version 2 (prior version 1)"),
        "{text}"
    );
    assert!(
        text.contains("existing instances keep the version they were created under"),
        "{text}"
    );
    assert!(text.contains("commands added: extend"), "{text}");
    assert!(text.contains("commands removed: none"), "{text}");
    assert!(text.contains("states added: none"), "{text}");
    assert!(text.contains("states removed: none"), "{text}");
    assert!(
        text.contains(
            "existing instances stay on their version; moving one to version 2 in place is not supported"
        ),
        "{text}"
    );
    assert!(
        !text.to_ascii_lowercase().contains("extend is refused"),
        "{text}"
    );
}

#[test]
fn same_version_prior_does_not_claim_a_migration() {
    let machine = repo("tests/fixtures/profile/linear.ik.toml");
    let machine = machine.to_string_lossy().into_owned();
    let output = ik()
        .args(["check", &machine, "--prior", &machine])
        .output()
        .expect("spawn ik check");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("new machine version 1 (prior version 1)"),
        "{text}"
    );
    assert!(
        !text.contains("existing instances keep the version they were created under"),
        "{text}"
    );
    assert!(!text.contains("in place is not supported"), "{text}");
    assert!(text.contains("commands added: none"), "{text}");
    assert!(text.contains("states added: none"), "{text}");
}

#[test]
fn witnessed_corpus_accepts() {
    let machine = repo("examples/publish-approval/machines/publish_approval_v3.ik.toml");
    let corpus = repo("examples/publish-approval/machines/publish_approval_v3.cases.json");
    let machine = machine.to_string_lossy().into_owned();
    let corpus = corpus.to_string_lossy().into_owned();
    let (code, doc) = check_json(&["check", &machine, "--corpus", &corpus, "--json"]);
    assert_eq!(code, Some(0));
    assert_eq!(doc["judgment"]["verdict"], "accept");
    assert_eq!(doc["judgment"]["status"], "accepted");
}
