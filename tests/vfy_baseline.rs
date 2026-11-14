//! VFY-11/12: freeze uncovered mandatory obligations; new debt cannot increase.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::def::OutcomeDef;
use ik::declarative::{
    discharge_exploration, explore, parse_definition, plan_diff, verification_plan, Bounds,
    CaseCorpus, Definition,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize, Deserialize)]
struct BaselineFile {
    schema_version: u64,
    machine_id: String,
    spec_hash: String,
    uncovered_mandatory: Vec<String>,
}

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

fn uncovered_mandatory(def: &Definition) -> (String, Vec<String>) {
    let plan = verification_plan(def);
    let machine = ik::declarative::compile::compile(def.clone()).expect("compile");
    let run = explore(&machine, &va_corpus(), bounds());
    let rows = discharge_exploration(&plan, &run);
    let mut ids: Vec<String> = plan
        .obligations
        .iter()
        .filter(|o| o.mandatory)
        .filter(|o| {
            rows.iter()
                .find(|r| r.obligation_id == o.obligation_id)
                .map(|r| r.status == "uncovered")
                .unwrap_or(true)
        })
        .map(|o| o.obligation_id.clone())
        .collect();
    ids.sort();
    (plan.machine.spec_hash, ids)
}

fn baseline_path() -> std::path::PathBuf {
    repo_root().join("validation/verification_baseline.json")
}

#[test]
fn adding_outcome_is_visible_on_plan_diff() {
    let old = va_def();
    let mut new = old.clone();
    new.commands
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
    let diff = plan_diff(&verification_plan(&old), &verification_plan(&new));
    assert!(diff
        .obligations_added()
        .iter()
        .any(|id| id == "outcome/reject/2/executable"));
}

#[test]
fn frozen_baseline_debt_does_not_increase() {
    let def = va_def();
    let (spec_hash, current) = uncovered_mandatory(&def);
    let text = std::fs::read_to_string(baseline_path()).expect("baseline");
    let frozen: BaselineFile = serde_json::from_str(&text).expect("json");
    assert_eq!(frozen.machine_id, "verification_attempt");
    assert_eq!(frozen.spec_hash, spec_hash);
    let frozen_set: BTreeSet<_> = frozen.uncovered_mandatory.iter().cloned().collect();
    let current_set: BTreeSet<_> = current.iter().cloned().collect();
    let new_debt: Vec<_> = current_set.difference(&frozen_set).cloned().collect();
    assert!(
        new_debt.is_empty(),
        "new uncovered mandatory obligations: {new_debt:?}"
    );
}
