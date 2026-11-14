//! Two-tier coverage: authored behavior stays mandatory; generated product
//! contexts may be discharged by a sound impossibility proof.

use ik::declarative::{
    check_machine, parse_definition, verification_plan, Bounds, CaseCorpus, CheckResult,
    Definition, PLAN_SCHEMA_BASE,
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

fn with_and_without_adapter(mut body: impl FnMut()) {
    body();
    let _guard = ik::smt::ThreadAdapter::auto();
    body();
}

fn int_args(x: i64) -> BTreeMap<String, serde_json::Value> {
    let mut args = BTreeMap::new();
    args.insert("x".into(), serde_json::json!(x));
    args
}

fn string_args(s: &str) -> BTreeMap<String, serde_json::Value> {
    let mut args = BTreeMap::new();
    args.insert("s".into(), serde_json::json!(s));
    args
}

fn uncovered(result: &CheckResult, id: &str) -> bool {
    result
        .report
        .obligations
        .uncovered_ids
        .iter()
        .any(|row| row == id)
}

fn recorded<'a>(result: &'a CheckResult, id: &str) -> Option<&'a ik::declarative::ImpossibilityRecordV1> {
    result
        .report
        .impossibilities
        .iter()
        .find(|row| row.obligation_id == id)
}

fn starter_go(x: i64) -> CaseCorpus {
    let mut corpus = CaseCorpus::new();
    corpus.insert("go", "sample", int_args(x));
    corpus
}

const ALWAYS_TRUE_FIRST: &str = r#"
schema_version = 1
machine_id = "always_true_first"
machine_version = 1
[facts]
[states]
a = { initial = true }
b = {}
c = {}
[commands.go]
from = ["a"]
args = { x = "int" }
outcomes = [
  { when = "$x >= 0 || $x < 0", to = "b" },
  { when = "$x == 3", to = "c" }
]
"#;

const MISSING_BRANCH: &str = r#"
schema_version = 1
machine_id = "missing_branch"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = {}
other = {}
[commands.go]
from = ["idle"]
to = "done"
"#;

const DEAD_COMMAND: &str = r#"
schema_version = 1
machine_id = "dead_command"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = {}
other = {}
[commands.live]
from = ["idle"]
to = "done"
[commands.ghost]
from = ["other"]
to = "done"
"#;

const STRING_GT: &str = r#"
schema_version = 1
machine_id = "string_gt"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = {}
[commands.step]
from = ["idle"]
args = { s = "string" }
when = '$s > "a"'
to = "done"
"#;

const REGION_BASE: &str = r#"
schema_version = 2
machine_id = "lane_hold"
machine_version = 1
[regions.lane.states]
start = { initial = true }
ok = {}
rare = {}
[regions.lane.commands.go]
from = ["start"]
args = { x = "int" }
outcomes = [
  { when = "$x >= 0 || $x < 0", to = "ok" },
  { when = "$x == 3", to = "rare" }
]
"#;

const REGION_WITH_FLAG: &str = r#"
schema_version = 2
machine_id = "lane_hold"
machine_version = 1
[regions.lane.states]
start = { initial = true }
ok = {}
rare = {}
[regions.lane.commands.go]
from = ["start"]
args = { x = "int" }
outcomes = [
  { when = "$x >= 0 || $x < 0", to = "ok" },
  { when = "$x == 3", to = "rare" }
]
[regions.flag.states]
off = { initial = true }
on = {}
[regions.flag.commands.flip]
from = ["off"]
to = "on"
"#;

const PARENT_CHILD: &str = r#"
schema_version = 2
machine_id = "parent_child_arms"
machine_version = 1
[regions.power.states]
running = { initial = true }
stopped = {}
[regions.power.commands.halt]
from = ["running"]
args = { force = "bool" }
when = "$force == true"
to = "stopped"
[regions.power.parent]
name = "unit"
children = ["running"]
[regions.power.parent.commands.halt]
from = ["unit"]
args = { force = "bool" }
to = "stopped"
"#;

const GENERATED_IMPOSSIBLE: &str = r#"
schema_version = 2
machine_id = "generated_dead_product"
machine_version = 1
[regions.mode.states]
idle = { initial = true }
done = {}
[regions.mode.commands.finish]
from = ["idle"]
to = "done"
[regions.flag.states]
off = { initial = true }
on = {}
[regions.flag.commands.flip]
from = ["off"]
to = "on"
[invariants.no_done_on]
assert = 'region_mode != "done" || region_flag != "on"'
"#;

#[test]
fn missing_authored_branch_is_refused() {
    with_and_without_adapter(|| {
        let result = check_def(MISSING_BRANCH, &CaseCorpus::new());
        assert_eq!(result.verdict, "REFUSED", "{}", result.text);
        assert!(uncovered(&result, "state/other/executable"), "{}", result.text);
    });
}

#[test]
fn always_true_first_arm_leaves_later_authored_arm_refused() {
    with_and_without_adapter(|| {
        let result = check_def(ALWAYS_TRUE_FIRST, &starter_go(1));
        assert_eq!(result.verdict, "REFUSED", "{}", result.text);
        assert!(
            uncovered(&result, "outcome/go/1/executable"),
            "later authored arm must stay mandatory: {}",
            result.text
        );
        let row = recorded(&result, "outcome/go/1/executable").expect("shadow record");
        assert_eq!(row.class, "ordered_outcome");
        assert!(!row.discharged, "schema-1 authored arm must not discharge");
        assert!(row.basis.contains("shadowed_by"), "{}", row.basis);
    });
}

#[test]
fn dead_authored_command_is_refused() {
    with_and_without_adapter(|| {
        let mut corpus = CaseCorpus::new();
        corpus.insert("live", "go", BTreeMap::new());
        let result = check_def(DEAD_COMMAND, &corpus);
        assert_eq!(result.verdict, "REFUSED", "{}", result.text);
        assert!(
            uncovered(&result, "command/ghost/from/other/executable")
                || uncovered(&result, "outcome/ghost/0/executable"),
            "{}",
            result.text
        );
    });
}

#[test]
fn always_true_first_arm_on_lowered_machine_stays_refused() {
    with_and_without_adapter(|| {
        let result = check_def(REGION_BASE, &starter_go(1));
        assert_eq!(result.verdict, "REFUSED", "{}", result.text);
        assert!(
            uncovered(&result, "authored/outcome/go/child/1/executable"),
            "lowered later authored arm must stay mandatory: {}",
            result.text
        );
    });
}

#[test]
fn child_witness_does_not_satisfy_parent_fallback() {
    with_and_without_adapter(|| {
        let mut corpus = CaseCorpus::new();
        let mut args = BTreeMap::new();
        args.insert("force".into(), serde_json::json!(true));
        corpus.insert("halt", "child", args);
        let result = check_def(PARENT_CHILD, &corpus);
        assert_eq!(result.verdict, "REFUSED", "{}", result.text);
        assert!(
            uncovered(&result, "authored/outcome/halt/parent/0/executable"),
            "parent fallback needs its own witness: {}",
            result.text
        );
    });
}

#[test]
fn parent_and_child_halt_are_distinct_authored_goals() {
    let plan = verification_plan(&def(PARENT_CHILD));
    let child = plan
        .authored_goals
        .iter()
        .find(|row| row.obligation_id == "authored/outcome/halt/child/0/executable");
    let parent = plan
        .authored_goals
        .iter()
        .find(|row| row.obligation_id == "authored/outcome/halt/parent/0/executable");
    assert!(child.is_some(), "{:?}", plan.authored_goals);
    assert!(parent.is_some(), "{:?}", plan.authored_goals);
    assert_ne!(
        child.expect("child").obligation_id,
        parent.expect("parent").obligation_id
    );
}

#[test]
fn adding_unrelated_region_does_not_multiply_existing_authored_goals() {
    let base = verification_plan(&def(REGION_BASE));
    let with_flag = verification_plan(&def(REGION_WITH_FLAG));
    let base_lane: Vec<_> = base
        .authored_goals
        .iter()
        .filter(|row| {
            row.obligation_id.contains("/lane/") || row.obligation_id.contains("/go/")
        })
        .map(|row| row.obligation_id.clone())
        .collect();
    let with_lane: Vec<_> = with_flag
        .authored_goals
        .iter()
        .filter(|row| {
            row.obligation_id.contains("/lane/") || row.obligation_id.contains("/go/")
        })
        .map(|row| row.obligation_id.clone())
        .collect();
    assert_eq!(base_lane, with_lane, "authored lane goals must stay stable");
    assert!(
        with_flag.authored_goals.len() > base.authored_goals.len(),
        "new region adds its own authored goals"
    );
}

#[test]
fn generated_impossible_product_is_discharged_authored_state_stays() {
    with_and_without_adapter(|| {
    let mut corpus = CaseCorpus::new();
    corpus.insert("finish", "go", BTreeMap::new());
    corpus.insert("flip", "go", BTreeMap::new());
    let result = check_def(GENERATED_IMPOSSIBLE, &corpus);
    let dead = recorded(&result, "state/done__on/executable");
    if let Some(row) = dead {
        assert!(row.discharged, "{}", result.text);
        assert!(
            row.basis.contains("invariant")
                || row.basis.contains("fact_flow")
                || row.basis.contains("checked_induction")
                || row.basis.contains("kernel_exhaustive"),
            "{}",
            row.basis
        );
    }
    let authored_done_open = uncovered(&result, "authored/state/mode/done/executable")
        && result.verdict != "REFUSED";
    assert_eq!(
        authored_done_open,
        false,
        "authored done must not be satisfied by the product proof: {}",
        result.text
    );
    let plan = verification_plan(&def(GENERATED_IMPOSSIBLE));
    assert!(
        plan.authored_goals
            .iter()
            .any(|row| row.obligation_id == "authored/state/mode/done/executable"),
        "{:?}",
        plan.authored_goals
    );
    });
}

#[test]
fn string_ordering_is_unknown_not_discharged() {
    with_and_without_adapter(|| {
        let mut corpus = CaseCorpus::new();
        corpus.insert("step", "above", string_args("b"));
        let result = check_def(STRING_GT, &corpus);
        assert!(
            recorded(&result, "outcome/step/0/executable").is_none(),
            "string ordering must not produce a discharge: {:?}",
            result.report.impossibilities
        );
        assert_eq!(result.verdict, "ACCEPT", "{}", result.text);
    });
}

#[test]
fn unknown_arithmetic_stays_retained() {
    const SRC: &str = r#"
schema_version = 1
machine_id = "indirect_arith"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = {}
[commands.step]
from = ["idle"]
args = { n = "int" }
when = "$n + 1 > 4"
to = "done"
"#;
    with_and_without_adapter(|| {
        let result = check_def(SRC, &CaseCorpus::new());
        assert_eq!(result.verdict, "REFUSED", "{}", result.text);
        assert!(uncovered(&result, "command/step/from/idle/executable"), "{}", result.text);
        assert!(
            recorded(&result, "command/step/from/idle/executable").is_none(),
            "indirect arithmetic stays unknown: {:?}",
            result.report.impossibilities
        );
    });
}

#[test]
fn schema_one_without_records_keeps_base_plan_schema() {
    const SRC: &str = r#"
schema_version = 1
machine_id = "linear_ok"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = {}
[commands.go]
from = ["idle"]
to = "done"
"#;
    let plan = verification_plan(&def(SRC));
    assert_eq!(plan.schema_version, PLAN_SCHEMA_BASE);
    assert!(plan.impossibilities.is_empty());
    assert!(plan.authored_goals.is_empty());
}

#[test]
fn explicit_profile_goal_stays_failed_when_proved_impossible() {
    with_and_without_adapter(|| {
    use ik::declarative::profile::{evaluate, parse_profile};
    let machine = def(GENERATED_IMPOSSIBLE);
    let profile = parse_profile(
        r#"
profile_version = 1
[[goals]]
id = "done-on"
kind = "reach_state"
state = "done__on"
required = true
"#,
    )
    .expect("profile");
    let envelope = evaluate(&profile, &machine, &CaseCorpus::new(), &{
        let compiled = ik::declarative::parse_and_compile(GENERATED_IMPOSSIBLE).expect("compile");
        ik::declarative::explore(&compiled, &CaseCorpus::new(), bounds())
    });
    let goal = envelope
        .goals
        .iter()
        .find(|row| row.id == "done-on")
        .expect("goal");
    assert_ne!(goal.status, "witness_found");
    if let Some(detail) = &goal.detail {
        assert!(
            detail.contains("proved impossible") || detail.contains("unreachable"),
            "{detail}"
        );
    }
    });
}
