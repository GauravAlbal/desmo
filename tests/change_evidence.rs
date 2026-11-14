//! Change-aware evidence: profile diffs, identity verification, and prior baseline.

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

fn arg(value: impl AsRef<Path>) -> String {
    value.as_ref().display().to_string()
}

fn run(args: &[String]) -> Output {
    bin().args(args).output().expect("run ik")
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.path_join(name);
    std::fs::write(&path, body).expect("write");
    path
}

trait PathJoin {
    fn path_join(&self, name: &str) -> PathBuf;
}

impl PathJoin for Path {
    fn path_join(&self, name: &str) -> PathBuf {
        self.join(name)
    }
}

fn json_stdout(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "json ({err}) status={:?} stdout={} stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

const FINISH: &str = "\
profile_version = 1
[[goals]]
id = \"finish\"
kind = \"reach_terminal\"
terminal = \"done\"
required = true
";

const FINISH_MOVED: &str = "\
profile_version = 1
[[goals]]
id = \"finish\"
kind = \"reach_state\"
state = \"mid\"
required = true
";

const OPTIONAL_ONLY: &str = "\
profile_version = 1
[[goals]]
id = \"open\"
kind = \"no_stuck_states\"
required = false
";

const EMPTY_GOALS: &str = "profile_version = 1\n";

const CORPUS_A: &str = "\
{
  \"commands\": {
    \"step\": [{\"name\": \"a\", \"args\": {}}],
    \"finish\": [{\"name\": \"a\", \"args\": {}}]
  }
}
";

const CORPUS_B: &str = "\
{
  \"commands\": {
    \"step\": [{\"name\": \"b\", \"args\": {}}],
    \"finish\": [{\"name\": \"a\", \"args\": {}}]
  }
}
";

fn blocking_machine() -> &'static str {
    "\
schema_version = 1
machine_id = \"profile_linear\"
machine_version = 1

[facts]

[states]
start = { initial = true }
mid = {}
done = { terminal = true }
noted = {}

[commands.step]
from = [\"start\"]
to = \"mid\"
"
}

fn benign_machine() -> &'static str {
    "\
schema_version = 1
machine_id = \"profile_linear\"
machine_version = 1

[facts]

[states]
start = { initial = true }
mid = {}
done = { terminal = true }

[commands.step]
from = [\"start\"]
to = \"mid\"

[commands.finish]
from = [\"mid\"]
to = \"done\"

[commands.note]
from = [\"start\"]
to = \"start\"
"
}

#[test]
fn same_goal_id_with_changed_content_is_stale_profile() {
    let dir = tempfile::tempdir().expect("temp");
    let original = write(dir.path(), "original.toml", FINISH);
    let changed = write(dir.path(), "changed.toml", FINISH_MOVED);
    let machine = fixture("linear.ik.toml");
    let evidence = dir.path().join("evidence.json");
    let produced = run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&original),
        "--json".into(),
    ]);
    assert_eq!(produced.status.code(), Some(0));
    std::fs::write(&evidence, &produced.stdout).expect("save evidence");
    let stale = run(&[
        "evidence".into(),
        "verify".into(),
        arg(&evidence),
        "--machine".into(),
        arg(&machine),
        "--profile".into(),
        arg(&changed),
        "--json".into(),
    ]);
    assert_eq!(stale.status.code(), Some(1));
    let doc = json_stdout(&stale);
    assert_eq!(doc["overall"], "stale_profile");
    assert_eq!(doc["axes"]["profile"]["status"], "stale");
    let field = doc["axes"]["profile"]["field"].as_str().unwrap();
    assert!(field.contains("content_digest"), "{field}");
    assert!(field.contains("finish"), "{field}");
    let fresh = run(&[
        "evidence".into(),
        "verify".into(),
        arg(&evidence),
        "--machine".into(),
        arg(&machine),
        "--profile".into(),
        arg(&original),
        "--json".into(),
    ]);
    assert_eq!(fresh.status.code(), Some(0));
    let fresh_doc = json_stdout(&fresh);
    assert_eq!(fresh_doc["overall"], "current");
    assert_ne!(fresh_doc["overall"], "stale_profile");
    assert!(fresh_doc.get("note").is_none());
}

#[test]
fn deleting_a_required_goal_is_weakened_and_an_optional_deletion_is_not() {
    let dir = tempfile::tempdir().expect("temp");
    let required = write(dir.path(), "required.toml", FINISH);
    let optional = write(dir.path(), "optional.toml", OPTIONAL_ONLY);
    let empty = write(dir.path(), "empty.toml", EMPTY_GOALS);
    let required_gone = run(&["profile".into(), "diff".into(), arg(&required), arg(&empty)]);
    assert_eq!(required_gone.status.code(), Some(0));
    let required_text = String::from_utf8(required_gone.stdout).unwrap();
    assert!(required_text.contains("weakened"), "{required_text}");
    assert!(required_text.contains("goal_removed"), "{required_text}");
    let optional_gone = run(&["profile".into(), "diff".into(), arg(&optional), arg(&empty)]);
    assert_eq!(optional_gone.status.code(), Some(0));
    let optional_text = String::from_utf8(optional_gone.stdout).unwrap();
    assert!(!optional_text.contains("weakened"), "{optional_text}");
    assert!(optional_text.contains("goal_removed"), "{optional_text}");
    let json = run(&[
        "profile".into(),
        "diff".into(),
        arg(&required),
        arg(&empty),
        "--json".into(),
    ]);
    assert_eq!(json_stdout(&json)["weakened"], true);
}

#[test]
fn relaxing_required_is_weakened_and_tightening_it_is_not() {
    let dir = tempfile::tempdir().expect("temp");
    let required = write(dir.path(), "required.toml", FINISH);
    let optional = write(
        dir.path(),
        "optional.toml",
        &FINISH.replace("required = true", "required = false"),
    );
    let relaxed = run(&[
        "profile".into(),
        "diff".into(),
        arg(&required),
        arg(&optional),
    ]);
    let relaxed_text = String::from_utf8(relaxed.stdout).unwrap();
    assert_eq!(
        relaxed.status.code(),
        Some(0),
        "stdout={relaxed_text} stderr={}",
        String::from_utf8_lossy(&relaxed.stderr)
    );
    assert!(relaxed_text.contains("weakened"), "{relaxed_text}");
    assert!(relaxed_text.contains("required_toggled"), "{relaxed_text}");
    let tightened = run(&[
        "profile".into(),
        "diff".into(),
        arg(&optional),
        arg(&required),
    ]);
    let tightened_text = String::from_utf8(tightened.stdout).unwrap();
    assert!(!tightened_text.contains("weakened"), "{tightened_text}");
    assert!(
        tightened_text.contains("required_toggled"),
        "{tightened_text}"
    );
}

#[test]
fn changing_the_corpus_is_stale_corpus() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write(dir.path(), "checks.toml", FINISH);
    let corpus_a = write(dir.path(), "a.json", CORPUS_A);
    let corpus_b = write(dir.path(), "b.json", CORPUS_B);
    let machine = fixture("linear.ik.toml");
    let evidence = dir.path().join("evidence.json");
    let produced = run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&profile),
        "--corpus".into(),
        arg(&corpus_a),
        "--json".into(),
    ]);
    assert_eq!(produced.status.code(), Some(0));
    std::fs::write(&evidence, &produced.stdout).expect("save evidence");
    let stale = run(&[
        "evidence".into(),
        "verify".into(),
        arg(&evidence),
        "--machine".into(),
        arg(&machine),
        "--profile".into(),
        arg(&profile),
        "--corpus".into(),
        arg(&corpus_b),
        "--json".into(),
    ]);
    assert_eq!(stale.status.code(), Some(1));
    let doc = json_stdout(&stale);
    assert_eq!(doc["overall"], "stale_corpus");
    assert_eq!(doc["axes"]["corpus"]["field"], "corpus_digest");
    let same = run(&[
        "evidence".into(),
        "verify".into(),
        arg(&evidence),
        "--machine".into(),
        arg(&machine),
        "--profile".into(),
        arg(&profile),
        "--corpus".into(),
        arg(&corpus_a),
        "--json".into(),
    ]);
    assert_eq!(same.status.code(), Some(0));
    assert_eq!(json_stdout(&same)["overall"], "current");
    assert_ne!(json_stdout(&same)["axes"]["corpus"]["status"], "stale");
}

#[test]
fn a_different_spec_hash_is_a_different_machine_version_and_is_not_rebound() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write(dir.path(), "checks.toml", FINISH);
    let machine = fixture("linear.ik.toml");
    let edited = write(dir.path(), "edited.ik.toml", blocking_machine());
    let evidence = dir.path().join("evidence.json");
    let produced = run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    assert_eq!(produced.status.code(), Some(0));
    std::fs::write(&evidence, &produced.stdout).expect("save evidence");
    let foreign = run(&[
        "evidence".into(),
        "verify".into(),
        arg(&evidence),
        "--machine".into(),
        arg(&edited),
        "--profile".into(),
        arg(&profile),
    ]);
    assert_eq!(foreign.status.code(), Some(1));
    let text = String::from_utf8(foreign.stdout).unwrap();
    assert!(text.contains("different_machine_version"), "{text}");
    assert!(text.contains("no rebind or migration occurs"), "{text}");
    assert!(text.contains("spec_hash"), "{text}");
    let same = run(&[
        "evidence".into(),
        "verify".into(),
        arg(&evidence),
        "--machine".into(),
        arg(&machine),
        "--profile".into(),
        arg(&profile),
    ]);
    assert_eq!(same.status.code(), Some(0));
    let same_text = String::from_utf8(same.stdout).unwrap();
    assert!(same_text.contains("overall current"), "{same_text}");
    assert!(
        !same_text.contains("different_machine_version"),
        "{same_text}"
    );
    assert!(!same_text.contains("no rebind"), "{same_text}");
}

#[test]
fn untouched_check_evidence_verifies_current() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write(dir.path(), "checks.toml", FINISH);
    let machine = fixture("linear.ik.toml");
    let evidence = dir.path().join("evidence.json");
    let produced = run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    assert_eq!(produced.status.code(), Some(0));
    std::fs::write(&evidence, &produced.stdout).expect("save evidence");
    let verified = run(&[
        "evidence".into(),
        "verify".into(),
        arg(&evidence),
        "--machine".into(),
        arg(&machine),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    assert_eq!(verified.status.code(), Some(0));
    let doc = json_stdout(&verified);
    assert_eq!(doc["overall"], "current");
    assert_eq!(doc["axes"]["machine"]["status"], "current");
    assert_eq!(doc["axes"]["profile"]["status"], "current");
    assert_eq!(doc["axes"]["corpus"]["status"], "current");
    assert_eq!(doc["axes"]["runtime"]["status"], "not_recorded");
}

#[test]
fn blocking_a_reached_terminal_is_a_regression_and_a_benign_command_is_not() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write(dir.path(), "checks.toml", FINISH);
    let corpus = write(dir.path(), "cases.json", CORPUS_A);
    let prior = fixture("linear.ik.toml");
    let blocking = write(dir.path(), "blocking.ik.toml", blocking_machine());
    let benign = write(dir.path(), "benign.ik.toml", benign_machine());
    let blocked = run(&[
        "check".into(),
        arg(&blocking),
        "--prior".into(),
        arg(&prior),
        "--profile".into(),
        arg(&profile),
        "--corpus".into(),
        arg(&corpus),
        "--json".into(),
    ]);
    let blocked_doc = json_stdout(&blocked);
    let blocked_goal = &blocked_doc["profile"]["baseline"]["goals"][0];
    assert_eq!(blocked_goal["id"], "finish");
    assert_eq!(blocked_goal["prior"], "witness_found");
    assert_ne!(blocked_goal["candidate"], "witness_found");
    assert_eq!(blocked_goal["regression"], true);
    let blocked_text = run(&[
        "check".into(),
        arg(&blocking),
        "--prior".into(),
        arg(&prior),
        "--profile".into(),
        arg(&profile),
        "--corpus".into(),
        arg(&corpus),
    ]);
    let blocked_out = String::from_utf8(blocked_text.stdout).unwrap();
    assert!(blocked_out.contains("regression"), "{blocked_out}");
    let added = run(&[
        "check".into(),
        arg(&benign),
        "--prior".into(),
        arg(&prior),
        "--profile".into(),
        arg(&profile),
        "--corpus".into(),
        arg(&corpus),
        "--json".into(),
    ]);
    assert_eq!(added.status.code(), Some(0));
    let added_doc = json_stdout(&added);
    let added_goal = &added_doc["profile"]["baseline"]["goals"][0];
    assert_eq!(added_goal["candidate"], "witness_found");
    assert_eq!(added_goal["prior"], "witness_found");
    assert!(added_goal.get("regression").is_none());
    let added_text = String::from_utf8(
        run(&[
            "check".into(),
            arg(&benign),
            "--prior".into(),
            arg(&prior),
            "--profile".into(),
            arg(&profile),
            "--corpus".into(),
            arg(&corpus),
        ])
        .stdout,
    )
    .unwrap();
    assert!(!added_text.contains("regression"), "{added_text}");
}

#[test]
fn a_goal_whose_state_exists_only_on_the_candidate_is_not_comparable() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write(
        dir.path(),
        "checks.toml",
        "profile_version = 1\n[[goals]]\nid = \"noted\"\nkind = \"reach_state\"\nstate = \"noted\"\nrequired = false\n",
    );
    let prior = fixture("linear.ik.toml");
    let candidate = write(dir.path(), "candidate.ik.toml", blocking_machine());
    let output = run(&[
        "check".into(),
        arg(&candidate),
        "--prior".into(),
        arg(&prior),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    let goal = &json_stdout(&output)["profile"]["baseline"]["goals"][0];
    assert_eq!(goal["prior"], "not_comparable");
    assert!(goal["reason"].as_str().unwrap().contains("noted"));
    assert!(goal.get("regression").is_none());
    let comparable = run(&[
        "check".into(),
        arg(&prior),
        "--prior".into(),
        arg(&prior),
        "--profile".into(),
        arg(&write(dir.path(), "finish.toml", FINISH)),
        "--json".into(),
    ]);
    let same = &json_stdout(&comparable)["profile"]["baseline"]["goals"][0];
    assert_ne!(same["prior"], "not_comparable");
    assert_eq!(same["prior"], "witness_found");
}

#[test]
fn check_prior_without_a_profile_stays_byte_identical() {
    let machine = fixture("linear.ik.toml");
    let text = run(&[
        "check".into(),
        arg(&machine),
        "--prior".into(),
        arg(&machine),
    ]);
    let json = run(&[
        "check".into(),
        arg(&machine),
        "--prior".into(),
        arg(&machine),
        "--json".into(),
    ]);
    assert_eq!(text.status.code(), Some(0));
    assert_eq!(json.status.code(), Some(0));
    let expected_text = std::fs::read(fixture("golden/prior_linear.txt")).unwrap();
    let expected_json = std::fs::read(fixture("golden/prior_linear.json")).unwrap();
    assert_eq!(text.stdout, expected_text);
    assert_eq!(json.stdout, expected_json);
    assert!(!text.stdout.windows(8).any(|w| w == b"baseline"));
    assert!(!json.stdout.windows(9).any(|w| w == b"\"profile\""));
}

#[test]
fn scenario_evidence_tracks_the_bundle_digest_and_the_runtime_binary() {
    let dir = tempfile::tempdir().expect("temp");
    let machine = dir.path().join("linear.ik.toml");
    std::fs::copy(fixture("linear.ik.toml"), &machine).expect("copy machine");
    let scenario = write(
        dir.path(),
        "scenario.json",
        "{\"scenario_version\":1,\"machine\":\"linear.ik.toml\",\"aggregate\":\"a\",\"steps\":[{\"kind\":\"command\",\"name\":\"step\",\"expect\":\"committed\"}]}\n",
    );
    let out = dir.path().join("bundle");
    let ran = run(&[
        "scenario".into(),
        "run".into(),
        arg(&machine),
        arg(&scenario),
        "--out".into(),
        arg(&out),
    ]);
    assert_eq!(
        ran.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let result = out.join("result.json");
    let current = run(&[
        "evidence".into(),
        "verify".into(),
        arg(&result),
        "--machine".into(),
        arg(&machine),
        "--runtime-binary".into(),
        env!("CARGO_BIN_EXE_ik").into(),
        "--json".into(),
    ]);
    assert_eq!(
        current.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&current.stderr)
    );
    let doc = json_stdout(&current);
    assert_eq!(doc["overall"], "current");
    assert_eq!(doc["axes"]["scenario"]["status"], "current");
    assert_eq!(doc["axes"]["runtime"]["status"], "current");
    let mut bytes = std::fs::read(out.join("scenario.json")).unwrap();
    bytes.push(b' ');
    std::fs::write(out.join("scenario.json"), bytes).unwrap();
    let stale = run(&[
        "evidence".into(),
        "verify".into(),
        arg(&result),
        "--machine".into(),
        arg(&machine),
        "--runtime-binary".into(),
        env!("CARGO_BIN_EXE_ik").into(),
        "--json".into(),
    ]);
    assert_eq!(stale.status.code(), Some(1));
    let stale_doc = json_stdout(&stale);
    assert_eq!(stale_doc["overall"], "stale_scenario");
    assert_eq!(stale_doc["axes"]["scenario"]["field"], "scenario_digest");
    std::fs::write(dir.path().join("other-bin"), b"not-the-runtime").unwrap();
    let fresh = dir.path().join("fresh");
    let reran = run(&[
        "scenario".into(),
        "run".into(),
        arg(&machine),
        arg(&scenario),
        "--out".into(),
        arg(&fresh),
    ]);
    assert_eq!(reran.status.code(), Some(0));
    let wrong_bin = run(&[
        "evidence".into(),
        "verify".into(),
        arg(fresh.join("result.json")),
        "--machine".into(),
        arg(&machine),
        "--runtime-binary".into(),
        arg(dir.path().join("other-bin")),
        "--json".into(),
    ]);
    assert_eq!(wrong_bin.status.code(), Some(1));
    let wrong = json_stdout(&wrong_bin);
    assert_eq!(wrong["overall"], "stale_runtime");
    assert_eq!(wrong["axes"]["runtime"]["field"], "binary_sha256");
    assert_ne!(wrong["axes"]["scenario"]["status"], "stale");
}

#[test]
fn analysis_result_with_the_same_goal_id_and_a_new_content_digest_is_stale() {
    let dir = tempfile::tempdir().expect("temp");
    let profile = write(dir.path(), "checks.toml", FINISH);
    let machine = fixture("linear.ik.toml");
    let plan = json_stdout(&run(&[
        "check".into(),
        arg(&machine),
        "--profile".into(),
        arg(&profile),
        "--plan".into(),
        "--json".into(),
    ]));
    let identity = json_stdout(&run(&["identity".into(), arg(&machine)]));
    let evidence = dir.path().join("result.json");
    let body = serde_json::json!({
        "protocol": "ik.analysis.result.v1",
        "subject": {
            "kind": "finite_runtime_graph",
            "identity": {
                "machine_id": identity["machine_id"],
                "spec_hash": identity["spec_hash"]
            }
        },
        "profile_digest": plan["profile_digest"],
        "goals": [{
            "id": "finish",
            "content_digest": plan["goals"][0]["content_digest"]
        }]
    });
    std::fs::write(&evidence, serde_json::to_vec(&body).unwrap()).unwrap();
    let current = run(&[
        "evidence".into(),
        "verify".into(),
        arg(&evidence),
        "--machine".into(),
        arg(&machine),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    assert_eq!(current.status.code(), Some(0));
    assert_eq!(json_stdout(&current)["overall"], "current");
    let mut stale_body = body;
    stale_body["goals"][0]["content_digest"] = serde_json::json!("deadbeef");
    std::fs::write(&evidence, serde_json::to_vec(&stale_body).unwrap()).unwrap();
    let stale = run(&[
        "evidence".into(),
        "verify".into(),
        arg(&evidence),
        "--machine".into(),
        arg(&machine),
        "--profile".into(),
        arg(&profile),
        "--json".into(),
    ]);
    assert_eq!(stale.status.code(), Some(1));
    let doc = json_stdout(&stale);
    assert_eq!(doc["overall"], "stale_profile");
    assert!(doc["axes"]["profile"]["field"]
        .as_str()
        .unwrap()
        .contains("content_digest"));
    assert_eq!(doc["axes"]["machine"]["status"], "current");
}
