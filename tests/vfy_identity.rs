//! `ik identity` prints the kernel-authoritative machine identity.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::canonical::canonical_json;
use ik::machine::Machine;
use serde_json::Value;

#[test]
fn identity_prints_canonical_machine_identity() {
    let bin = env!("CARGO_BIN_EXE_ik");
    let machine = machine_def("verification_attempt");
    let output = std::process::Command::new(bin)
        .arg("identity")
        .arg(&machine)
        .output()
        .expect("run ik identity");
    assert!(
        output.status.success(),
        "ik identity failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let expected = canonical_json(load_machine("verification_attempt").identity());
    assert_eq!(stdout.trim_end(), expected);
    let parsed: Value = serde_json::from_str(&expected).expect("identity json");
    assert_eq!(parsed["machine_id"], "verification_attempt");
    assert_eq!(parsed["version"], 1);
    assert!(parsed["spec_hash"].as_str().unwrap().len() == 64);
}

#[test]
fn identity_missing_path_is_transport_failure() {
    let bin = env!("CARGO_BIN_EXE_ik");
    let output = std::process::Command::new(bin)
        .arg("identity")
        .output()
        .expect("run ik identity");
    assert_eq!(output.status.code(), Some(2));
}
