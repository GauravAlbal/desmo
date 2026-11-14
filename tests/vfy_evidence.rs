//! VFY-02: existing IK-DX artifacts adapt onto verification obligations.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::def::OutcomeDef;
use ik::declarative::{
    discharge_exploration, discharge_hollow, explain, explore, parse_and_compile, parse_definition,
    transition_evidence, verification_plan, Bounds, CaseCorpus, Definition,
};
use ik::machine::Machine;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

fn va_def() -> Definition {
    parse_definition(&std::fs::read_to_string(machine_def("verification_attempt")).unwrap())
        .expect("va")
}

fn args(pairs: &[(&str, &str)]) -> BTreeMap<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), json!(*v)))
        .collect()
}

fn va_corpus() -> CaseCorpus {
    let mut corpus = CaseCorpus::new();
    corpus.insert(
        "submit",
        "happy",
        args(&[("artifact_digest", "a1"), ("baseline_digest", "b1")]),
    );
    corpus.insert(
        "verify",
        "match",
        args(&[
            ("receipt_digest", "r1"),
            ("receipt_artifact_digest", "a1"),
            ("receipt_baseline_digest", "b1"),
        ]),
    );
    corpus
}

fn bounds() -> Bounds {
    Bounds {
        max_nodes: 4096,
        max_depth: 16,
    }
}

#[test]
fn transition_evidence_is_composed_from_explain() {
    let machine = load_machine("verification_attempt");
    let explanation = explain(
        &machine,
        &machine.initial_state(),
        &json!({"name": "start"}),
    );
    let evidence = transition_evidence(&explanation);
    assert_eq!(evidence.schema_version, 1);
    assert_eq!(evidence.machine_id, "verification_attempt");
    assert_eq!(evidence.spec_hash, machine.spec_hash().0);
    assert_eq!(evidence.pre_state_digest, explanation.basis.state_digest);
    assert!(!evidence.refused);
    assert_eq!(evidence.source_state.as_deref(), Some("created"));
    assert_eq!(evidence.selected_outcome, Some(0));
    assert_eq!(explanation.verdict, "would_commit");
}

#[test]
fn exploration_discharges_executable_outcomes() {
    let def = va_def();
    let plan = verification_plan(&def);
    let machine =
        parse_and_compile(&std::fs::read_to_string(machine_def("verification_attempt")).unwrap())
            .expect("compile");
    let run = explore(&machine, &va_corpus(), bounds());
    let discharged = discharge_exploration(&plan, &run);
    for needle in ["outcome/reject/0/executable", "outcome/reject/1/executable"] {
        let hit = discharged
            .iter()
            .find(|row| row.obligation_id.contains(needle))
            .unwrap_or_else(|| panic!("missing {needle}"));
        assert_eq!(hit.status, "satisfied", "{needle}: {hit:?}");
        assert_eq!(hit.engine, "explore");
    }
}

#[test]
fn hollow_load_does_not_discharge() {
    let def = va_def();
    let plan = verification_plan(&def);
    let _machine = load_machine("verification_attempt");
    let discharged = discharge_hollow(&plan);
    assert!(discharged.iter().any(|row| {
        row.obligation_id.contains("outcome/reject/0/executable") && row.status == "uncovered"
    }));
    assert!(discharged
        .iter()
        .filter(|row| row.obligation_id.contains("/outcome/"))
        .all(|row| row.status == "uncovered"));
}

#[test]
fn stale_spec_hash_is_inapplicable() {
    let original = va_def();
    let plan = verification_plan(&original);
    let mut edited = original;
    edited
        .commands
        .get_mut("reject")
        .expect("reject")
        .outcomes
        .as_mut()
        .expect("outcomes")
        .push(OutcomeDef {
            when: Some("attempt_count < 0".into()),
            to: "repairable".into(),
            then: None,
        });
    let mutated = parse_and_compile(&toml::to_string(&edited).expect("toml")).expect("compile");
    let run = explore(&mutated, &va_corpus(), bounds());
    assert_ne!(run.spec_hash, plan.machine.spec_hash);
    let discharged = discharge_exploration(&plan, &run);
    assert!(discharged.iter().all(|row| row.status == "inapplicable"));
    let _ = Map::new();
}

fn two_source_finish_def() -> Definition {
    parse_definition(
        r#"
schema_version = 1
machine_id = "cmd_source_exact"
machine_version = 1
[facts]
[states]
a = { initial = true }
b = {}
done = { terminal = true }
[commands.go]
from = ["a"]
to = "b"
args = { token = "string" }
[commands.finish]
from = ["a", "b"]
to = "done"
"#,
    )
    .expect("def")
}

fn discharge_status(id: &str, discharged: &[ik::declarative::ObligationDischarge]) -> String {
    discharged
        .iter()
        .find(|row| row.obligation_id == id)
        .unwrap_or_else(|| panic!("missing {id}"))
        .status
        .clone()
}

#[test]
fn command_from_a_does_not_discharge_source_b() {
    let def = two_source_finish_def();
    let plan = verification_plan(&def);
    let machine = parse_and_compile(&toml::to_string(&def).expect("toml")).expect("compile");
    let run = explore(&machine, &CaseCorpus::new(), bounds());
    let discharged = discharge_exploration(&plan, &run);
    assert_eq!(
        discharge_status("command/finish/from/a/executable", &discharged),
        "satisfied"
    );
    assert_eq!(
        discharge_status("command/finish/from/b/executable", &discharged),
        "uncovered"
    );
}

#[test]
fn removing_sole_command_source_witness_uncovers_obligation() {
    let def = two_source_finish_def();
    let plan = verification_plan(&def);
    let machine = parse_and_compile(&toml::to_string(&def).expect("toml")).expect("compile");
    let mut corpus = CaseCorpus::new();
    corpus.insert("go", "token", args(&[("token", "x")]));
    let covered = explore(&machine, &corpus, bounds());
    let covered_discharges = discharge_exploration(&plan, &covered);
    assert_eq!(
        discharge_status("command/finish/from/b/executable", &covered_discharges),
        "satisfied"
    );
    let uncovered = explore(&machine, &CaseCorpus::new(), bounds());
    let uncovered_discharges = discharge_exploration(&plan, &uncovered);
    assert_eq!(
        discharge_status("command/finish/from/b/executable", &uncovered_discharges),
        "uncovered"
    );
}
