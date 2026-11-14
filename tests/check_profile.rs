//! Check profiles: useful-path goals, scope preview, and unchanged baseline output.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ik"))
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/profile")
        .join(name)
}

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/publish-approval/machines")
        .join(name)
}

fn run(args: &[String]) -> Output {
    bin().args(args).output().expect("run ik")
}

fn arg(value: impl AsRef<Path>) -> String {
    value.as_ref().display().to_string()
}

fn write_profile(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write profile");
    path
}

fn json_stdout(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "json stdout ({err}) status={:?} stdout={} stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn always_refuse_required_goal_fails_while_safety_holds() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write_profile(
        dir.path(),
        "checks.toml",
        "profile_version = 1\n[[goals]]\nid = \"finish\"\nkind = \"reach_terminal\"\nterminal = \"published\"\nrequired = true\n",
    );
    let output = run(&[
        "check".into(),
        arg(example("publish_approval_all_refuse.ik.toml")),
        "--corpus".into(),
        arg(fixture("always_refuse.cases.json")),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let doc = json_stdout(&output);
    assert!(doc["invariant_witnesses"].as_array().unwrap().is_empty());
    let reasons = doc["judgment"]["reasons"].as_array().unwrap();
    assert!(reasons
        .iter()
        .all(|reason| { !reason.as_str().unwrap_or("").contains("invariant") }));
    let goal = &doc["profile"]["goals"][0];
    assert_eq!(goal["status"], "input_coverage_gap");
    assert_ne!(goal["status"], "witness_found");
    assert!(!goal["detail"]
        .as_str()
        .unwrap()
        .contains("unreachable within the supplied corpus"));
    let identity = run(&[
        "identity".into(),
        arg(example("publish_approval_all_refuse.ik.toml")),
    ]);
    let identity = json_stdout(&identity);
    assert_eq!(doc["profile"]["machine_spec_hash"], identity["spec_hash"]);
}

#[test]
fn terminal_with_no_outgoing_edge_is_not_reported_stuck() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write_profile(
        dir.path(),
        "checks.toml",
        "profile_version = 1\n[[goals]]\nid = \"open\"\nkind = \"no_stuck_states\"\n",
    );
    let output = run(&[
        "check".into(),
        arg(fixture("dead_end.ik.toml")),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let goal = &json_stdout(&output)["profile"]["goals"][0];
    assert_eq!(goal["status"], "stuck_states_found");
    assert_eq!(goal["proven_deadlock"], false);
    let stuck: Vec<&str> = goal["stuck_states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["state"].as_str().unwrap())
        .collect();
    assert_eq!(stuck, vec!["stuck"]);
    assert!(!stuck.contains(&"done"));
    assert_eq!(
        goal["stuck_states"][0]["classification"],
        "no_outgoing_command"
    );
}

#[test]
fn missing_corpus_case_is_a_coverage_gap_not_unreachability() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write_profile(
        dir.path(),
        "checks.toml",
        "profile_version = 1\n[[goals]]\nid = \"done\"\nkind = \"reach_terminal\"\nterminal = \"done\"\n",
    );
    let output = run(&[
        "check".into(),
        arg(fixture("needs_args.ik.toml")),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let goal = &json_stdout(&output)["profile"]["goals"][0];
    assert_eq!(goal["status"], "input_coverage_gap");
    assert_ne!(goal["status"], "no_witness_found_complete");
    assert!(!goal["detail"]
        .as_str()
        .unwrap()
        .contains("unreachable within the supplied corpus"));
    assert_eq!(goal["gaps"][0]["command"], "go");
    assert_eq!(goal["gaps"][0]["classification"], "no_corpus_case");
}

#[test]
fn truncated_search_stays_incomplete_and_required_goal_fails() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write_profile(
        dir.path(),
        "checks.toml",
        "profile_version = 1\n[limits]\nmax_depth = 1\n[[goals]]\nid = \"done\"\nkind = \"reach_terminal\"\nterminal = \"done\"\nrequired = true\n",
    );
    let output = run(&[
        "check".into(),
        arg(fixture("linear.ik.toml")),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let doc = json_stdout(&output);
    assert_eq!(doc["profile"]["search_complete"], false);
    assert_eq!(doc["profile"]["bounds"]["max_depth"], 1);
    let goal = &doc["profile"]["goals"][0];
    assert_eq!(goal["status"], "no_witness_found_incomplete");
    assert!(!goal["detail"]
        .as_str()
        .unwrap()
        .contains("unreachable within the supplied corpus"));
}

#[test]
fn goal_content_changes_digests_and_reformatting_does_not() {
    let dir = tempfile::tempdir().expect("temp");
    let machine = fixture("linear.ik.toml");
    let formatted = write_profile(
        dir.path(),
        "a.toml",
        "profile_version = 1\n\n[[goals]]\nid = \"where\"\nkind = \"reach_state\"\nstate = \"mid\"\n",
    );
    let reformatted = write_profile(
        dir.path(),
        "b.toml",
        "# comment\nprofile_version = 1\n\n[[goals]]\nstate = \"mid\"\nrequired = true\nkind = \"reach_state\"\nid = \"where\"\n",
    );
    let changed = write_profile(
        dir.path(),
        "c.toml",
        "profile_version = 1\n[[goals]]\nid = \"where\"\nkind = \"reach_state\"\nstate = \"start\"\n",
    );
    let first = json_stdout(&run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&formatted),
        "--plan".into(),
        "--json".into(),
    ]));
    let second = json_stdout(&run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&reformatted),
        "--plan".into(),
        "--json".into(),
    ]));
    let third = json_stdout(&run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&changed),
        "--plan".into(),
        "--json".into(),
    ]));
    assert_eq!(first["profile_digest"], second["profile_digest"]);
    assert_eq!(
        first["goals"][0]["content_digest"],
        second["goals"][0]["content_digest"]
    );
    assert_ne!(first["profile_digest"], third["profile_digest"]);
    assert_ne!(
        first["goals"][0]["content_digest"],
        third["goals"][0]["content_digest"]
    );
    assert_eq!(first["goals"][0]["id"], third["goals"][0]["id"]);
    let identity = json_stdout(&run(&["identity".into(), arg(&machine)]));
    assert_eq!(first["machine_spec_hash"], identity["spec_hash"]);
    assert_eq!(third["machine_spec_hash"], identity["spec_hash"]);
}

#[test]
fn plan_prints_scope_without_running_exploration() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write_profile(
        dir.path(),
        "checks.toml",
        "profile_version = 1\n[limits]\nmax_nodes = 1000000\nmax_depth = 1000000\n[[goals]]\nid = \"finish\"\nkind = \"reach_terminal\"\nterminal = \"published\"\nrequired = true\n",
    );
    let output = run(&[
        "check".into(),
        arg(example("publish_approval_all_refuse.ik.toml")),
        "--corpus".into(),
        arg(fixture("always_refuse.cases.json")),
        "--profile".into(),
        arg(&profile),
        "--plan".into(),
        "--json".into(),
    ]);
    assert_eq!(output.status.code(), Some(0));
    let plan = json_stdout(&output);
    assert!(plan.get("search_complete").is_none());
    assert!(plan["goals"][0].get("status").is_none());
    assert!(plan["goals"][0].get("witness").is_none());
    assert_eq!(
        plan["scope"],
        "search up to 1000000 nodes / depth 1000000 over the supplied corpus; a witness proves reachability, absence of a witness does not prove unreachability unless the search is complete"
    );
    assert_eq!(plan["goals"][0]["kind"], "reach_terminal");
    assert!(plan["corpus_digest"].as_str().unwrap().len() == 64);
    assert!(plan["profile_digest"].as_str().unwrap().len() == 64);
}

#[test]
fn check_without_new_flags_matches_recorded_output() {
    let machine = example("publish_approval_v3.ik.toml");
    let corpus = example("publish_approval_v3.cases.json");
    let text = run(&[
        "check".into(),
        arg(&machine),
        "--corpus".into(),
        arg(&corpus),
    ]);
    let json = run(&[
        "check".into(),
        arg(&machine),
        "--corpus".into(),
        arg(&corpus),
        "--json".into(),
    ]);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/profile/golden");
    let expected_text = std::fs::read(root.join("publish_approval_v3.txt")).expect("golden text");
    let expected_json = std::fs::read(root.join("publish_approval_v3.json")).expect("golden json");
    assert_eq!(text.status.code(), Some(0));
    assert_eq!(json.status.code(), Some(0));
    assert_eq!(text.stdout, expected_text);
    assert_eq!(json.stdout, expected_json);
    assert!(text.stderr.is_empty());
    assert!(json.stderr.is_empty());
}

#[test]
fn unknown_goal_kind_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write_profile(
        dir.path(),
        "checks.toml",
        "profile_version = 1\n[[goals]]\nid = \"x\"\nkind = \"not_a_goal\"\n",
    );
    let output = run(&[
        "check".into(),
        arg(fixture("linear.ik.toml")),
        "--profile".into(),
        arg(&profile),
    ]);
    assert_eq!(output.status.code(), Some(2));
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("unknown goal kind"));
    assert!(err.contains("not_a_goal"));
}

#[test]
fn duplicate_goal_id_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write_profile(
        dir.path(),
        "checks.toml",
        "profile_version = 1\n[[goals]]\nid = \"same\"\nkind = \"reach_state\"\nstate = \"start\"\n[[goals]]\nid = \"same\"\nkind = \"reach_state\"\nstate = \"mid\"\n",
    );
    let output = run(&[
        "check".into(),
        arg(fixture("linear.ik.toml")),
        "--profile".into(),
        arg(&profile),
    ]);
    assert_eq!(output.status.code(), Some(2));
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("duplicate goal id"));
    assert!(err.contains("same"));
}

#[test]
fn undeclared_state_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write_profile(
        dir.path(),
        "checks.toml",
        "profile_version = 1\n[[goals]]\nid = \"missing\"\nkind = \"reach_state\"\nstate = \"nowhere\"\n",
    );
    let output = run(&[
        "check".into(),
        arg(fixture("linear.ik.toml")),
        "--profile".into(),
        arg(&profile),
    ]);
    assert_eq!(output.status.code(), Some(2));
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("undeclared state"));
    assert!(err.contains("nowhere"));
}

#[test]
fn undeclared_command_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write_profile(
        dir.path(),
        "checks.toml",
        "profile_version = 1\n[[goals]]\nid = \"missing\"\nkind = \"command_executes\"\ncommand = \"nope\"\n",
    );
    let output = run(&[
        "check".into(),
        arg(fixture("linear.ik.toml")),
        "--profile".into(),
        arg(&profile),
    ]);
    assert_eq!(output.status.code(), Some(2));
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("undeclared command"));
    assert!(err.contains("nope"));
}

#[test]
fn unknown_goal_field_is_a_usage_error() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write_profile(
        dir.path(),
        "checks.toml",
        "profile_version = 1\n[[goals]]\nid = \"x\"\nkind = \"reach_state\"\nstate = \"start\"\nsurprise = true\n",
    );
    let output = run(&[
        "check".into(),
        arg(fixture("linear.ik.toml")),
        "--profile".into(),
        arg(&profile),
    ]);
    assert_eq!(output.status.code(), Some(2));
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("surprise"));
}

#[test]
fn reached_terminal_witness_satisfies_a_required_goal() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write_profile(
        dir.path(),
        "checks.toml",
        "profile_version = 1\n[[goals]]\nid = \"done\"\nkind = \"reach_terminal\"\nterminal = \"done\"\n[[goals]]\nid = \"finish\"\nkind = \"command_executes\"\ncommand = \"finish\"\n",
    );
    let machine = fixture("linear.ik.toml");
    let plain = run(&["check".into(), arg(&machine)]);
    let output = run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    assert_eq!(plain.status.code(), Some(0));
    assert_eq!(output.status.code(), Some(0));
    let doc = json_stdout(&output);
    assert_eq!(doc["verdict"], "ACCEPT");
    assert_eq!(doc["profile"]["search_complete"], true);
    let goal = &doc["profile"]["goals"][0];
    assert_eq!(goal["status"], "witness_found");
    let commands: Vec<&str> = goal["witness"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| step["command"].as_str().unwrap())
        .collect();
    assert_eq!(commands, vec!["step", "finish"]);
    let executed = &doc["profile"]["goals"][1];
    assert_eq!(executed["kind"], "command_executes");
    assert_eq!(executed["status"], "witness_found");
    assert_eq!(executed["witness"][1]["command"], "finish");
    let text = run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&profile),
    ]);
    let plain_text = String::from_utf8(plain.stdout).unwrap();
    let profile_text = String::from_utf8(text.stdout).unwrap();
    assert!(profile_text.starts_with(&plain_text));
    assert!(profile_text.contains("\nPROFILE\n"));
}

#[test]
fn optional_unsatisfied_goal_does_not_change_the_exit_code() {
    let dir = tempfile::tempdir().expect("temp");
    let optional = write_profile(
        dir.path(),
        "optional.toml",
        "profile_version = 1\n[[goals]]\nid = \"open\"\nkind = \"no_stuck_states\"\nrequired = false\n",
    );
    let required = write_profile(
        dir.path(),
        "required.toml",
        "profile_version = 1\n[[goals]]\nid = \"open\"\nkind = \"no_stuck_states\"\nrequired = true\n",
    );
    let machine = fixture("dead_end.ik.toml");
    let plain = run(&["check".into(), arg(&machine)]);
    let optional_out = run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&optional),
        "--json".into(),
    ]);
    let required_out = run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&required),
        "--json".into(),
    ]);
    assert_eq!(plain.status.code(), Some(0));
    assert_eq!(optional_out.status.code(), Some(0));
    assert_eq!(required_out.status.code(), Some(1));
    let optional_doc = json_stdout(&optional_out);
    let required_doc = json_stdout(&required_out);
    assert_eq!(optional_doc["verdict"], "ACCEPT");
    assert_eq!(required_doc["verdict"], "ACCEPT");
    assert_eq!(
        optional_doc["profile"]["goals"][0]["status"],
        "stuck_states_found"
    );
    assert_eq!(
        required_doc["profile"]["goals"][0]["status"],
        "stuck_states_found"
    );
}

#[test]
fn complete_search_without_a_witness_says_unreachable_within_the_corpus() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write_profile(
        dir.path(),
        "checks.toml",
        "profile_version = 1\n[[goals]]\nid = \"other\"\nkind = \"reach_state\"\nstate = \"other\"\nrequired = true\n",
    );
    let output = run(&[
        "check".into(),
        arg(fixture("orphan.ik.toml")),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let doc = json_stdout(&output);
    assert_eq!(doc["profile"]["search_complete"], true);
    assert_eq!(
        doc["profile"]["goals"][0]["status"],
        "no_witness_found_complete"
    );
    let detail = doc["profile"]["goals"][0]["detail"]
        .as_str()
        .expect("detail");
    assert!(
        detail.contains("unreachable within the supplied corpus"),
        "{detail}"
    );
    assert!(
        detail.contains("proved impossible"),
        "explicit required goal must carry the impossibility proof: {detail}"
    );
}

#[test]
fn allow_waiting_is_not_stuck_and_refused_continuation_is_not_a_deadlock() {
    let dir = tempfile::tempdir().expect("temp");
    let open = write_profile(
        dir.path(),
        "open.toml",
        "profile_version = 1\n[[goals]]\nid = \"wait\"\nkind = \"no_stuck_states\"\n",
    );
    let allowed = write_profile(
        dir.path(),
        "allowed.toml",
        "profile_version = 1\n[[goals]]\nid = \"wait\"\nkind = \"no_stuck_states\"\nallow_waiting = [\"waiting\"]\n",
    );
    let machine = fixture("refused_leave.ik.toml");
    let open_out = run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&open),
        "--json".into(),
    ]);
    let allowed_out = run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&allowed),
        "--json".into(),
    ]);
    let open_goal = &json_stdout(&open_out)["profile"]["goals"][0];
    let allowed_goal = &json_stdout(&allowed_out)["profile"]["goals"][0];
    assert_eq!(open_goal["status"], "stuck_states_found");
    assert_eq!(open_goal["proven_deadlock"], false);
    assert_eq!(open_goal["stuck_states"][0]["state"], "waiting");
    assert_eq!(
        open_goal["stuck_states"][0]["classification"],
        "all_cases_refused"
    );
    let names: Vec<&str> = open_goal["stuck_states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["state"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"done"));
    assert_eq!(allowed_goal["status"], "no_stuck_states_found");
    assert!(allowed_goal.get("stuck_states").is_none());
}
