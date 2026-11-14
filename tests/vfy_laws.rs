//! `ik laws` prints an advisory inventory bound to kernel identity.

#[path = "common/mod.rs"]
mod common;

use common::*;
use serde_json::Value;

#[test]
fn laws_inventory_includes_runtime_invariants() {
    let bin = env!("CARGO_BIN_EXE_ik");
    let machine = machine_def("publish_approval");
    let output = std::process::Command::new(bin)
        .arg("laws")
        .arg(&machine)
        .output()
        .expect("run ik laws");
    assert!(
        output.status.success(),
        "ik laws failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: Value = serde_json::from_str(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(parsed["advisory"], true);
    assert_eq!(parsed["machine_id"], "publish_approval");
    assert_eq!(parsed["version"], 1);
    assert!(parsed["spec_hash"].as_str().unwrap().len() == 64);
    let ids: Vec<&str> = parsed["laws"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"pub_inv_001"));
}

#[test]
fn laws_missing_path_is_transport_failure() {
    let bin = env!("CARGO_BIN_EXE_ik");
    let output = std::process::Command::new(bin)
        .arg("laws")
        .output()
        .expect("run ik laws");
    assert_eq!(output.status.code(), Some(2));
}

const UNGUARDED: &str = r#"
schema_version = 1
machine_id = "note_desk"
machine_version = 1
name = "NoteDesk"

[facts]
note = { type = "string", policy = "replace" }
note_id = { type = "string", policy = "replace" }
tag = { type = "string", policy = "replace" }

[states]
idle = { initial = true }
done = { terminal = true }

[commands.finish]
from = ["idle"]
to = "done"
args = { note = "string", note_id = "string", tag = "string" }
when = "$note_id != ''"
then = { note = "$note", note_id = "$note_id", tag = "$tag" }
"#;

#[test]
fn laws_flags_arguments_no_guard_reads_and_matches_whole_tokens() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("note_desk.ik.toml");
    std::fs::write(&path, UNGUARDED).expect("write machine");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("laws")
        .arg(&path)
        .output()
        .expect("run ik laws");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let parsed: Value = serde_json::from_slice(&output.stdout).expect("json");
    let mut flagged: Vec<String> = parsed["unguarded_arguments"]
        .as_array()
        .expect("array")
        .iter()
        .map(|row| format!("{}.{}", row["command"], row["argument"]))
        .collect();
    flagged.sort();
    // `$note_id` is read; `note` shares a prefix with it but is a different token.
    assert_eq!(flagged, vec!["\"finish\".\"note\"", "\"finish\".\"tag\""]);
}
