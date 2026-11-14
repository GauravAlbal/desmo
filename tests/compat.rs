//! Drain-and-recreate compatibility must flag each kind of difference, not only the
//! additive loan change, and the census must report what the store holds.

use ik::declarative::compat::{drain_compat, Compat};
use ik::declarative::{parse_definition, Definition};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const V1: &str = r#"
schema_version = 1
machine_id = "job"
machine_version = 1
[facts]
owner = { type = "string", policy = "replace" }
due = { type = "int", policy = "replace" }
score = { type = "int", policy = "replace" }
[states]
open = { initial = true }
claimed = {}
done = { terminal = true }
[commands.claim]
from = ["open"]
to = "claimed"
args = { owner = "string", due = "int" }
when = "$due > 0"
then = { owner = "$owner", due = "$due" }
[commands.finish]
from = ["claimed"]
to = "done"
when = "due > 0"
[commands.release]
from = ["claimed"]
to = "open"
"#;

fn def(text: &str) -> Definition {
    parse_definition(text).expect("definition")
}

fn compat_of(edit: impl Fn(String) -> String) -> Compat {
    drain_compat(&def(V1), &def(&edit(V1.replace("machine_version = 1", "machine_version = 2"))))
}

fn state<'a>(compat: &'a Compat, name: &str) -> &'a ik::declarative::compat::StateCompat {
    compat
        .states
        .iter()
        .find(|item| item.state == name)
        .unwrap_or_else(|| panic!("no state {name}"))
}

#[test]
fn an_unchanged_machine_has_no_difference_in_any_state() {
    let compat = compat_of(|text| text);
    assert_eq!(compat.states.iter().all(|item| item.reasons.is_empty()), true, "{compat:?}");
    assert_eq!(compat.global.is_empty(), true);
}

#[test]
fn a_removed_exit_is_flagged_on_its_state_only() {
    let compat = compat_of(|text| {
        text.replace("[commands.release]\nfrom = [\"claimed\"]\nto = \"open\"\n", "")
    });
    assert_eq!(state(&compat, "claimed").reasons, vec!["exit `release`: removed".to_string()]);
    assert_eq!(state(&compat, "open").reasons.is_empty(), true);
}

#[test]
fn a_changed_guard_and_a_changed_target_are_told_apart() {
    let guard = compat_of(|text| text.replace("when = \"due > 0\"", "when = \"due > 5\""));
    assert_eq!(state(&guard, "claimed").reasons, vec!["exit `finish`: guard changed".to_string()]);
    let target = compat_of(|text| {
        text.replace("[commands.release]\nfrom = [\"claimed\"]\nto = \"open\"", "[commands.release]\nfrom = [\"claimed\"]\nto = \"done\"")
    });
    assert_eq!(state(&target, "claimed").reasons, vec!["exit `release`: target changed".to_string()]);
}

#[test]
fn a_new_guard_that_reads_a_fact_version_one_never_set_is_flagged() {
    let compat = compat_of(|text| {
        text.replace(
            "[commands.release]\nfrom = [\"claimed\"]\nto = \"open\"",
            "[commands.release]\nfrom = [\"claimed\"]\nto = \"open\"\nwhen = \"score >= 0\"",
        )
    });
    let reasons = &state(&compat, "claimed").reasons;
    assert_eq!(
        reasons.iter().any(|reason| reason.contains("`score`") && reason.contains("may not have set")),
        true,
        "{reasons:?}"
    );
}

#[test]
fn a_fact_set_on_every_path_in_is_not_flagged_and_one_set_on_some_paths_is() {
    let reads_owner = compat_of(|text| {
        text.replace(
            "[commands.release]\nfrom = [\"claimed\"]\nto = \"open\"",
            "[commands.release]\nfrom = [\"claimed\"]\nto = \"open\"\nwhen = \"owner != \\\"\\\"\"",
        )
    });
    assert_eq!(
        state(&reads_owner, "claimed").reasons.iter().any(|reason| reason.contains("may not have set")),
        false,
        "{reads_owner:?}"
    );
    let two_paths = V1.replace(
        "[commands.release]",
        "[commands.skip]\nfrom = [\"open\"]\nto = \"claimed\"\n\n[commands.release]",
    );
    let before = def(&two_paths);
    let after = def(
        &two_paths
            .replace("machine_version = 1", "machine_version = 2")
            .replace(
                "[commands.release]\nfrom = [\"claimed\"]\nto = \"open\"",
                "[commands.release]\nfrom = [\"claimed\"]\nto = \"open\"\nwhen = \"owner != \\\"\\\"\"",
            ),
    );
    let compat = drain_compat(&before, &after);
    assert_eq!(
        state(&compat, "claimed").reasons.iter().any(|reason| reason.contains("`owner`")),
        true,
        "{compat:?}"
    );
}

#[test]
fn a_new_invariant_is_reported_for_every_state() {
    let compat = compat_of(|text| format!("{text}\n[invariants.positive]\nassert = \"score >= 0\"\n"));
    assert_eq!(
        compat.global.iter().any(|line| line.contains("invariant `positive` is new")),
        true,
        "{compat:?}"
    );
    assert_eq!(
        state(&compat, "open").reasons.iter().any(|reason| reason.contains("invariant `positive`")),
        true,
        "{compat:?}"
    );
}

#[test]
fn a_dropped_effect_is_reported_as_an_unresolved_intent_risk() {
    let with_effect = V1.replace(
        "when = \"due > 0\"\n[commands.release]",
        "args = { ref = \"string\" }\nwhen = \"due > 0\"\n\n[commands.finish.effect]\nkind = \"notify\"\nid = \"$ref\"\npayload_digest = \"$ref\"\nidempotency_key = \"$ref\"\n\n[commands.release]",
    );
    let before = def(&with_effect);
    let after = def(&V1.replace("machine_version = 1", "machine_version = 2"));
    let compat = drain_compat(&before, &after);
    assert_eq!(
        compat.global.iter().any(|line| line.contains("the effect of `finish` changed or was removed")),
        true,
        "{compat:?}"
    );
}

#[test]
fn a_renamed_state_shows_up_as_missing_in_the_new_version() {
    let compat = compat_of(|text| text.replace("claimed", "taken"));
    assert_eq!(state(&compat, "claimed").exits.iter().all(|exit| exit.verdict == "removed"), true, "{compat:?}");
}

#[test]
fn a_hierarchical_machine_is_not_analysed_and_says_so() {
    let region = repo("tests/fixtures/diagram/region_stacked.ik.toml");
    let text = fs::read_to_string(region).expect("region");
    let compat = drain_compat(&def(&text), &def(&text));
    assert_eq!(compat.states.is_empty(), true);
    assert_eq!(compat.not_established.iter().any(|line| line.contains("hierarchical")), true);
}

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn ik(args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_ik")).args(args).output().expect("spawn ik");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn loan_store(name: &str) -> PathBuf {
    let out = std::env::temp_dir().join(format!("ik-census-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&out);
    let (_, err, code) = ik(&[
        "scenario",
        "run",
        &repo("validation/dfy/journeys/fixtures/loan_v2.ik.toml").display().to_string(),
        &repo("validation/dfy/journeys/fixtures/scenarios/existing-loan.json").display().to_string(),
        "--out",
        &out.display().to_string(),
    ]);
    assert_eq!(code, Some(0), "{err}");
    out.join("scratch/store.sqlite")
}

#[test]
fn census_counts_a_version_one_instance_that_is_still_unfinished() {
    let store = loan_store("count");
    let (out, _, code) = ik(&["census", &store.display().to_string(), "--json"]);
    assert_eq!(code, Some(0));
    let doc: serde_json::Value = serde_json::from_str(&out).expect("json");
    let rows = doc["instances"].as_array().expect("rows");
    assert_eq!(rows.len(), 1, "{doc}");
    assert_eq!(rows[0]["machine_id"], "loan_checkout");
    assert_eq!(rows[0]["machine_version"], 1);
    assert_eq!(rows[0]["state"], "on_loan");
    assert_eq!(rows[0]["terminal"], false);
    assert_eq!(rows[0]["count"], 1);
}

#[test]
fn census_does_not_change_the_store_and_refuses_what_is_not_a_store() {
    let store = loan_store("readonly");
    let before = fs::read(&store).expect("bytes");
    let (_, _, code) = ik(&["census", &store.display().to_string()]);
    assert_eq!(code, Some(0));
    assert_eq!(fs::read(&store).expect("bytes"), before);
    let junk = store.with_file_name("junk.sqlite");
    fs::write(&junk, b"not a database").expect("junk");
    let (_, err, code) = ik(&["census", &junk.display().to_string()]);
    assert_eq!(code, Some(2), "{err}");
    let (_, err, code) = ik(&["census", "/no/such/store.sqlite"]);
    assert_eq!(code, Some(2), "{err}");
}

#[test]
fn the_change_handoff_reports_the_census_and_the_compatibility_table() {
    let store = loan_store("handoff");
    let evidence = store.parent().and_then(|dir| dir.parent()).expect("bundle").to_path_buf();
    let behavior = evidence.join("behavior.json");
    fs::write(
        &behavior,
        serde_json::json!({
            "behavior_version": 1,
            "title": "loan v2",
            "outcomes": [],
            "why": {"text": "extend a loan", "source": "test"},
            "rule_witnesses": [{"rule": "extend", "examples": [
                {"args": {"new_until_ms": 200}, "facts": {"loan_until_ms": 100}, "outcome": "accepted"},
                {"args": {"new_until_ms": 100}, "facts": {"loan_until_ms": 100}, "outcome": "rejected"},
                {"args": {"new_until_ms": 50}, "facts": {"loan_until_ms": 100}, "outcome": "rejected"}
            ]}]
        })
        .to_string(),
    )
    .expect("behavior");
    let (text, err, code) = ik(&[
        "handoff",
        &behavior.display().to_string(),
        "--evidence",
        &evidence.display().to_string(),
        "--kind",
        "change",
        "--store",
        &store.display().to_string(),
    ]);
    assert_eq!(code, Some(0), "{err}{text}");
    assert_eq!(text.contains("Found: version 1 instances in the store: 1 unfinished, 0 finished"), true, "{text}");
    assert_eq!(text.contains("Found: unfinished by state: on_loan 1"), true, "{text}");
    assert_eq!(text.contains("no census was taken"), false, "{text}");
    assert_eq!(text.contains("Compatibility of new lifecycles after old work finishes"), true, "{text}");
    assert_eq!(text.contains("adds extend"), true, "{text}");
    let (_, err, code) = ik(&[
        "handoff",
        &behavior.display().to_string(),
        "--evidence",
        &evidence.display().to_string(),
        "--kind",
        "delivery",
        "--store",
        &store.display().to_string(),
    ]);
    assert_eq!(code, Some(2), "{err}");
    assert_eq!(err.contains("--store applies only to --kind change"), true, "{err}");
}

#[test]
fn a_change_handoff_expects_the_requirement_to_speak_only_to_new_or_changed_commands() {
    let store = loan_store("gaps-change");
    let evidence = store.parent().and_then(|dir| dir.parent()).expect("bundle").to_path_buf();
    let behavior = evidence.join("behavior.json");
    let requirement = |traces: serde_json::Value| {
        serde_json::json!({
            "text": "Extend can set a later deadline. Returns are unaffected.",
            "source": "test",
            "atoms": [
                {"id": "R1", "span": "Extend can set a later deadline."},
                {"id": "R2", "span": "Returns are unaffected."}
            ],
            "traces": traces,
            "not_applicable": [{"atom": "R2", "reason": "a statement about what does not change"}]
        })
    };
    let run = |traces: serde_json::Value| {
        fs::write(
            &behavior,
            serde_json::json!({
                "behavior_version": 1,
                "title": "loan v2",
                "outcomes": [],
                "why": {"text": "extend a loan", "source": "test"},
                "requirement": requirement(traces),
                "rule_witnesses": [{"rule": "extend", "examples": [
                    {"args": {"new_until_ms": 200}, "facts": {"loan_until_ms": 100}, "outcome": "accepted"},
                    {"args": {"new_until_ms": 100}, "facts": {"loan_until_ms": 100}, "outcome": "rejected"},
                    {"args": {"new_until_ms": 50}, "facts": {"loan_until_ms": 100}, "outcome": "rejected"}
                ]}]
            })
            .to_string(),
        )
        .expect("behavior");
        let (out, err, code) = ik(&[
            "handoff", &behavior.display().to_string(), "--evidence", &evidence.display().to_string(),
            "--kind", "change", "--store", &store.display().to_string(), "--json",
        ]);
        assert_eq!(code, Some(0), "{err}{out}");
        serde_json::from_str::<serde_json::Value>(&out).expect("json")
    };
    let traced = run(serde_json::json!([{"atom": "R1", "element": "command:extend"}]));
    let section = &traced["gaps"]["requirement"];
    assert_eq!(section["untraced_commands"], serde_json::json!([]), "{section}");
    assert_eq!(section["unmapped"], serde_json::json!([]), "{section}");
    assert_eq!(section["not_applicable"][0]["reason"], "a statement about what does not change", "{section}");
    assert_eq!(section["commands_checked"], "new_or_changed");
    let untraced = run(serde_json::json!([{"atom": "R1", "element": "state:on_loan"}]));
    assert_eq!(untraced["gaps"]["requirement"]["untraced_commands"], serde_json::json!(["extend"]));
}
