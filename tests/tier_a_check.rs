//! Tier A1 integration tests: guard-guided auto-seeder, zero-corpus static lints,
//! automatic invariant falsification, and clean `--summary` / `--auto-seed` output.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn run_ik(args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_ik"))
        .args(args)
        .output()
        .expect("spawn ik");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

#[test]
fn auto_seed_accepts_verification_attempt_without_corpus() {
    let tmp = std::env::temp_dir().join("desmo-tier-a1-va");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let machine = tmp.join("va.ik.toml");
    std::fs::copy(repo("machines/verification_attempt.ik.toml"), &machine).unwrap();

    let (out, err, code) = run_ik(&[
        "check",
        machine.to_str().unwrap(),
        "--auto-seed",
        "--summary",
    ]);
    assert_eq!(code, Some(0), "stdout:\n{out}\nstderr:\n{err}");
    assert!(out.contains("ACCEPT"), "stdout:\n{out}");
    assert!(out.contains("auto_seeded_bounded"), "stdout:\n{out}");
    assert!(!out.contains("RELEASE POLICY"), "summary should omit release wall:\n{out}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn auto_seed_refuses_combination_with_release() {
    let (_, err, code) = run_ik(&[
        "check",
        repo("examples/orders/order.ik.toml").to_str().unwrap(),
        "--auto-seed",
        "--release",
    ]);
    assert_eq!(code, Some(2));
    assert!(err.contains("--auto-seed cannot be combined with --release"), "stderr:\n{err}");
}

#[test]
fn default_check_without_corpus_automatically_falsifies_defective_invariant() {
    let tmp = std::env::temp_dir().join("desmo-tier-a1-defective");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    // Copy ONLY the .ik.toml (no sibling .cases.json!).
    let machine = tmp.join("defective.ik.toml");
    std::fs::copy(
        repo("examples/publish-approval/machines/publish_approval_defective.ik.toml"),
        &machine,
    )
    .unwrap();

    let (out, err, code) = run_ik(&["check", machine.to_str().unwrap(), "--compact"]);
    assert_eq!(code, Some(1), "stdout:\n{out}\nstderr:\n{err}");
    let v: Value = serde_json::from_str(&out).expect("valid json");
    assert_eq!(v["verdict"], "REFUSED");
    assert_eq!(v["status"], "witnessed_violation");
    let witnesses = v["invariant_witnesses"].as_array().expect("witnesses array");
    assert!(!witnesses.is_empty(), "expected concrete invariant witness without corpus: {v}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn static_structural_lints_catch_unreachable_sink_shadowed_and_genesis_violation() {
    let tmp = std::env::temp_dir().join("desmo-tier-a1-lints");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let machine = tmp.join("linted.ik.toml");
    std::fs::write(
        &machine,
        r#"
schema_version = 1
machine_id = "linted"
machine_version = 1

[facts]
count = { type = "int", policy = "counter", initial = 5 }

[states]
start = { initial = true }
stuck = {}
orphan = {}
done = { terminal = true }

[commands.go]
from = ["start"]
outcomes = [
  { when = "count > 0", to = "stuck" },
  { when = "count > 0", to = "done" }
]

[invariants.bounded]
assert = "count <= 3"
"#,
    )
    .unwrap();

    let (laws_out, err, code) = run_ik(&["laws", machine.to_str().unwrap()]);
    assert_eq!(code, Some(0), "stderr:\n{err}");
    let laws: Value = serde_json::from_str(&laws_out).expect("laws json");
    let kinds: Vec<&str> = laws["structural_lints"]
        .as_array()
        .expect("structural_lints")
        .iter()
        .filter_map(|item| item["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"unreachable_state"), "kinds={kinds:?}");
    assert!(kinds.contains(&"non_terminal_sink"), "kinds={kinds:?}");
    assert!(kinds.contains(&"shadowed_outcome"), "kinds={kinds:?}");
    assert!(
        kinds.contains(&"genesis_invariant_violation"),
        "kinds={kinds:?}"
    );

    let (check_out, _, check_code) = run_ik(&["check", machine.to_str().unwrap()]);
    assert_eq!(check_code, Some(1));
    assert!(check_out.contains("STRUCTURAL LINTS"), "stdout:\n{check_out}");
    assert!(check_out.contains("non_terminal_sink"), "stdout:\n{check_out}");
    assert!(check_out.contains("shadowed_outcome"), "stdout:\n{check_out}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn corpus_init_smart_generates_guard_aware_cases() {
    let tmp = std::env::temp_dir().join("desmo-tier-a1-smart-corpus");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let out_file = tmp.join("va.cases.json");
    let (_, err, code) = run_ik(&[
        "corpus",
        "init",
        repo("machines/verification_attempt.ik.toml").to_str().unwrap(),
        "--out",
        out_file.to_str().unwrap(),
        "--smart",
    ]);
    assert_eq!(code, Some(0), "stderr:\n{err}");
    let (check_out, check_err, check_code) = run_ik(&[
        "check",
        repo("machines/verification_attempt.ik.toml").to_str().unwrap(),
        "--corpus",
        out_file.to_str().unwrap(),
        "--summary",
    ]);
    assert_eq!(check_code, Some(0), "stdout:\n{check_out}\nstderr:\n{check_err}");
    assert!(check_out.contains("ACCEPT"), "stdout:\n{check_out}");
    let _ = std::fs::remove_dir_all(&tmp);
}
