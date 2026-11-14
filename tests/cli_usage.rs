//! `ik --help` and the argument-error usage come from one text, so a documented flag cannot exist
//! in one and be missing from the other.

use std::process::Command;

fn ik(args: &[&str]) -> (String, String, Option<i32>) {
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
fn help_and_argument_error_print_the_same_usage_including_version_and_session() {
    let (help, _, help_code) = ik(&["--help"]);
    let (_, error, error_code) = ik(&["--definitely-not-a-flag"]);
    let usage_in_error: String = error
        .lines()
        .filter(|line| !line.starts_with("ik: "))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(help_code, Some(0));
    assert_eq!(error_code, Some(2));
    assert_eq!(help, usage_in_error);
    assert!(help.contains("--session"));
    assert!(help.contains("ik --version"));
}

#[test]
fn the_version_line_the_usage_advertises_is_printed() {
    let (out, _, code) = ik(&["--version"]);
    assert_eq!(code, Some(0));
    assert!(out.starts_with("invariant-kernel "));
    assert!(out.contains(" protocol 1."));
}

fn help_subcommand_names(help: &str) -> Vec<String> {
    help.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let rest = trimmed.strip_prefix("usage: ").unwrap_or(trimmed);
            let mut words = rest.split_whitespace();
            if words.next() != Some("ik") {
                return None;
            }
            let name = words.next()?;
            if name.starts_with('-') {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect()
}

#[test]
fn help_lists_every_dispatched_subcommand_and_the_binary_accepts_each() {
    let (help, _, code) = ik(&["--help"]);
    assert_eq!(code, Some(0));
    let help_names = help_subcommand_names(&help);
    let dispatched: Vec<String> = ik::cli::dispatched_subcommand_names()
        .map(str::to_string)
        .collect();
    assert_eq!(help_names, dispatched, "help:\n{help}");
    assert!(
        dispatched.iter().any(|name| name == "scenario"),
        "{dispatched:?}"
    );
    assert!(
        dispatched.iter().any(|name| name == "handoff"),
        "{dispatched:?}"
    );
    for name in &dispatched {
        let (_, err, _) = ik(&[name, "--help"]);
        assert!(
            !err.contains("--store <path> is required"),
            "{name} fell through to store usage:\n{err}"
        );
    }
}
