//! A refusal that comes only from missing evidence names the command that supplies it.

use std::path::Path;
use std::process::Command;

fn check(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("check")
        .args(args)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
        .output()
        .expect("spawn ik");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn uncovered_refusal_names_the_corpus_commands() {
    let text = check(&["examples/publish-approval/machines/publish_approval_v3.ik.toml"]);
    assert!(text.contains("REFUSED"), "{text}");
    assert!(text.contains("NEXT"), "{text}");
    assert!(text.contains("ik corpus init"), "{text}");
    assert!(text.contains("--corpus"), "{text}");
}

#[test]
fn accepted_check_has_no_next_step() {
    let text = check(&[
        "examples/publish-approval/machines/publish_approval_defective.ik.toml",
        "--corpus",
        "examples/publish-approval/machines/publish_approval_defective.cases.json",
    ]);
    assert!(!text.contains("obligations are uncovered"), "{text}");
}
