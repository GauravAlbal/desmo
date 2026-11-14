//! Coverage contract: positive success, unknown feasibility, proved impossibility,
//! and required refusal stay distinct. Fixtures are generic.

use ik::declarative::{
    check_machine, parse_definition, Bounds, CaseCorpus, CheckResult, Definition, ObligationStatus,
};
use std::collections::BTreeMap;

fn bounds() -> Bounds {
    Bounds {
        max_nodes: 64,
        max_depth: 8,
    }
}

fn def(src: &str) -> Definition {
    parse_definition(src).expect("valid fixture")
}

fn check_def(src: &str, corpus: &CaseCorpus) -> CheckResult {
    check_machine(&def(src), None, corpus, &[], bounds()).expect("check runs")
}

fn empty_args() -> BTreeMap<String, serde_json::Value> {
    BTreeMap::new()
}

fn has_obligation(result: &CheckResult, id: &str) -> bool {
    result
        .report
        .obligations
        .uncovered_ids
        .iter()
        .any(|row| row == id)
        || result
            .release
            .obligations
            .iter()
            .any(|row| row.obligation_id == id)
}

fn recorded_impossible(result: &CheckResult, id: &str) -> bool {
    result
        .report
        .impossibilities
        .iter()
        .any(|row| row.obligation_id == id)
}

#[test]
fn feasible_unwitnessed_success_stays_uncovered() {
    const SRC: &str = r#"
schema_version = 1
machine_id = "feasible_commit"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = {}
[commands.commit]
from = ["idle"]
to = "done"
args = { token = "string" }
"#;
    let result = check_def(SRC, &CaseCorpus::new());
    assert_eq!(result.verdict, "REFUSED", "{}", result.text);
    assert!(
        has_obligation(&result, "command/commit/from/idle/executable"),
        "{}",
        result.text
    );
    assert!(
        !recorded_impossible(&result, "command/commit/from/idle/executable"),
        "{}",
        result.text
    );
}

#[test]
fn unknown_feasibility_stays_retained() {
    const SRC: &str = r#"
schema_version = 1
machine_id = "unknown_flag"
machine_version = 1
[facts]
ready = { type = "bool", initial = false }
[states]
idle = { initial = true }
done = {}
[commands.commit]
from = ["idle"]
when = "ready == true"
to = "done"
"#;
    let result = check_def(SRC, &CaseCorpus::new());
    assert_eq!(result.verdict, "REFUSED", "{}", result.text);
    assert!(
        has_obligation(&result, "command/commit/from/idle/executable"),
        "unknown domain guard must stay a success obligation: {}",
        result.text
    );
    assert!(
        !recorded_impossible(&result, "command/commit/from/idle/executable"),
        "must not infer impossibility from missing corpus or initial facts: {}",
        result.text
    );
}

#[test]
fn proved_impossible_success_is_recorded_not_demanded() {
    const SRC: &str = r#"
schema_version = 1
machine_id = "seal_from_wrong_source"
machine_version = 1
[facts]
[states]
edited = { initial = true }
reviewed = {}
sealed = {}
[commands.review]
from = ["edited"]
to = "reviewed"
[commands.seal]
from = ["edited", "reviewed"]
when = 'state == "reviewed"'
to = "sealed"
"#;
    let result = check_def(SRC, &CaseCorpus::new());
    assert!(
        recorded_impossible(&result, "command/seal/from/edited/executable"),
        "edited source cannot select seal: {}",
        result.text
    );
    assert!(
        !has_obligation(&result, "command/seal/from/edited/executable"),
        "impossible success must not be uncovered debt: {}",
        result.text
    );
    assert!(
        has_obligation(&result, "command/seal/from/reviewed/executable"),
        "reviewed source remains a required success: {}",
        result.text
    );
    let edited = result
        .report
        .impossibilities
        .iter()
        .find(|row| row.obligation_id == "command/seal/from/edited/executable")
        .expect("edited impossibility");
    assert_eq!(edited.class, "command_source");
    assert!(edited.discharged);
    assert_eq!(edited.proving_source, "edited");
    assert!(edited.reason.contains("impossible"));
}

#[test]
fn required_refusal_is_not_satisfied_by_impossibility() {
    const SRC: &str = r#"
schema_version = 1
machine_id = "refusal_still_needs_witness"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = { terminal = true }
[commands.seal]
from = ["idle"]
when = 'state == "done"'
to = "done"
"#;
    let result = check_def(SRC, &CaseCorpus::new());
    assert!(
        recorded_impossible(&result, "command/seal/from/idle/executable"),
        "{}",
        result.text
    );
    let refusal = result
        .release
        .obligations
        .iter()
        .find(|row| row.obligation_id == "terminal/done/mutation_refused")
        .expect("terminal refusal obligation");
    assert_ne!(
        refusal.status,
        ObligationStatus::Satisfied,
        "impossibility must not satisfy required refusal: {:?}",
        refusal
    );
}

#[test]
fn always_refuse_runtime_fails_useful_success() {
    const SRC: &str = r#"
schema_version = 1
machine_id = "always_refuse"
machine_version = 1
[facts]
ready = { type = "bool", initial = false }
[states]
idle = { initial = true }
done = {}
[commands.commit]
from = ["idle"]
when = "ready == true"
to = "done"
"#;
    let mut corpus = CaseCorpus::new();
    corpus.insert("commit", "try", empty_args());
    let result = check_def(SRC, &corpus);
    assert_eq!(result.verdict, "REFUSED", "{}", result.text);
    assert!(
        has_obligation(&result, "command/commit/from/idle/executable"),
        "refusals must not satisfy a feasible/unknown success goal: {}",
        result.text
    );
    assert!(
        !result
            .release
            .obligations
            .iter()
            .any(
                |row| row.obligation_id == "command/commit/from/idle/executable"
                    && row.status == ObligationStatus::Satisfied
            ),
        "{}",
        result.text
    );
}

#[test]
fn first_match_shadowing_records_later_arm() {
    // Recorded, not discharged: ordered-outcome policy is the owner's.
    const SRC: &str = r#"
schema_version = 1
machine_id = "shadowed_arm"
machine_version = 1
[facts]
mode = { type = "string", initial = "extra" }
[states]
idle = { initial = true }
done = {}
extra = {}
[commands.step]
from = ["idle"]
outcomes = [
  { when = 'state == "idle"', to = "done" },
  { when = 'mode == "extra"', to = "extra" }
]
"#;
    let result = check_def(SRC, &CaseCorpus::new());
    assert_eq!(result.verdict, "REFUSED", "{}", result.text);
    assert!(
        recorded_impossible(&result, "outcome/step/1/executable"),
        "unconditional earlier arm shadows the later arm: {}",
        result.text
    );
    let shadowed = result
        .report
        .impossibilities
        .iter()
        .find(|row| row.obligation_id == "outcome/step/1/executable")
        .expect("shadowed-arm record");
    assert_eq!(shadowed.class, "ordered_outcome");
    assert!(!shadowed.discharged, "ordered-outcome proofs stay retained");
    assert!(
        result
            .report
            .obligations
            .uncovered_ids
            .iter()
            .any(|id| id == "outcome/step/1/executable"),
        "recorded ordered-outcome impossibility must not discharge the obligation: {}",
        result.text
    );
    assert!(
        has_obligation(&result, "outcome/step/0/executable"),
        "first arm remains required success: {}",
        result.text
    );
}

#[test]
fn soundness_control_keeps_guard_that_can_succeed() {
    const SRC: &str = r#"
schema_version = 1
machine_id = "can_succeed"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = {}
[commands.commit]
from = ["idle"]
when = 'state == "idle"'
to = "done"
"#;
    let result = check_def(SRC, &CaseCorpus::new());
    assert!(
        has_obligation(&result, "command/commit/from/idle/executable"),
        "guard is true at the source so success stays required: {}",
        result.text
    );
    assert!(
        !recorded_impossible(&result, "command/commit/from/idle/executable"),
        "{}",
        result.text
    );
}
