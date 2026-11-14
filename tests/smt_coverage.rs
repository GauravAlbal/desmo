//! Coverage-contract integration for the optional SMT certificate adapter.

use ik::declarative::{
    check_machine, parse_and_compile, parse_definition, verification_plan, Bounds, CaseCorpus,
    CheckResult, Definition,
};
use ik::smt::{self, ObligationReport, ProofStatus, Target, Trust};
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

fn with_adapter<T>(f: impl FnOnce() -> T) -> T {
    let _guard = smt::ThreadAdapter::auto();
    f()
}

fn starter_go(x: i64) -> CaseCorpus {
    let mut args = BTreeMap::new();
    args.insert("x".into(), serde_json::json!(x));
    let mut corpus = CaseCorpus::new();
    corpus.insert("go", "sample", args);
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

const MOVE: &str = r#"
schema_version = 1
machine_id = "smt.cover.move"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = { terminal = true }
lost = {}
[commands.go]
from = ["idle"]
to = "done"
"#;

fn uncovered(result: &CheckResult, id: &str) -> bool {
    result
        .report
        .obligations
        .uncovered_ids
        .iter()
        .any(|row| row == id)
}

fn problems_for(toolchain: &serde_json::Value) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("toolchain.json"), toolchain.to_string()).expect("write");
    smt::set_thread_adapter(Some(dir.path().to_str().expect("utf8 path")));
    let problems = smt::adapter_problems();
    let solvers = smt::configured_solvers();
    smt::clear_thread_adapter();
    assert!(solvers.0.is_none() && solvers.1.is_none(), "unpinned toolchain must load no solver");
    problems
}

#[test]
fn placeholder_toolchain_reports_every_unusable_solver() {
    let template: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/adapters/smt/toolchain.json"))
            .expect("shipped template"),
    )
    .expect("template json");
    let problems = problems_for(&template);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems.iter().all(|p| p.contains("no executable path")));
}

#[test]
fn real_path_with_unpinned_or_wrong_digest_loads_nothing() {
    let binary = std::env::current_exe().expect("exe").display().to_string();
    let zeros = "0".repeat(64);
    let unpinned = problems_for(&serde_json::json!({"z3": binary, "z3_sha256": zeros}));
    assert!(unpinned.iter().any(|p| p == "z3: digest not pinned"), "{unpinned:?}");
    let wrong = problems_for(&serde_json::json!({"z3": binary, "z3_sha256": "1".repeat(64)}));
    assert!(wrong.iter().any(|p| p.starts_with("z3: digest mismatch")), "{wrong:?}");
}

#[test]
fn adapter_absent_plan_matches_default() {
    let src = ALWAYS_TRUE_FIRST;
    let off = verification_plan(&def(src));
    let on_env = std::env::var_os("IK_SMT_ADAPTER");
    assert!(on_env.is_none(), "default tests must not see IK_SMT_ADAPTER");
    let result = check_def(src, &starter_go(1));
    assert_eq!(result.report.plan_digest, ik::canonical::digest_json(&off));
}

#[test]
fn always_true_first_arm_refuses_with_backend_on() {
    let result = with_adapter(|| check_def(ALWAYS_TRUE_FIRST, &starter_go(1)));
    assert_eq!(result.verdict, "REFUSED", "{}", result.text);
    assert!(
        uncovered(&result, "outcome/go/1/executable"),
        "later authored arm must stay mandatory: {}",
        result.text
    );
}

#[test]
fn dead_authored_command_refuses_with_backend_on() {
    let mut corpus = CaseCorpus::new();
    corpus.insert("live", "go", BTreeMap::new());
    let result = with_adapter(|| check_def(DEAD_COMMAND, &corpus));
    assert_eq!(result.verdict, "REFUSED", "{}", result.text);
    assert!(
        uncovered(&result, "command/ghost/from/other/executable")
            || uncovered(&result, "outcome/ghost/0/executable"),
        "{}",
        result.text
    );
}

#[test]
fn solver_asserted_does_not_discharge() {
    let report = ObligationReport {
        id: "outcome/go/1/executable".into(),
        status: ProofStatus::ProvenImpossible,
        trust: Trust::SolverAsserted,
        detail: "solver only".into(),
        replay: None,
        z3_ms: None,
        cvc5_ms: None,
        z3: None,
        cvc5: None,
        scope: Some("test".into()),
        reachable_digest: Some("abc".into()),
        invariant: Some("true".into()),
    };
    assert_eq!(smt::may_discharge(&report), false);
}

#[test]
fn wrong_certificate_does_not_reverify() {
    let defn = def(MOVE);
    let machine = parse_and_compile(MOVE).expect("machine");
    let report = ObligationReport {
        id: "state/lost/executable".into(),
        status: ProofStatus::ProvenImpossible,
        trust: Trust::CheckedInvariant,
        detail: "tampered".into(),
        replay: None,
        z3_ms: None,
        cvc5_ms: None,
        z3: None,
        cvc5: None,
        scope: Some("test".into()),
        reachable_digest: Some("deadbeef".into()),
        invariant: Some("true".into()),
    };
    let ok = with_adapter(|| smt::reverify_report(&defn, &machine, &report));
    assert_eq!(ok, false, "tampered digest must not reverify");
}

#[test]
fn checked_exhaustive_certificate_reverifies_on_finite_machine() {
    if smt::discover_solvers().0.is_none() {
        eprintln!("smt coverage: skipping; z3 not installed");
        return;
    }
    let defn = def(MOVE);
    let machine = parse_and_compile(MOVE).expect("machine");
    let target = Target::State("lost".into());
    let ex = smt::exhaustive_search(&defn, &machine, &target).expect("bfs");
    assert!(ex.complete, "{:?}", ex.incomplete_reason);
    let report = ObligationReport {
        id: "state/lost/executable".into(),
        status: ProofStatus::ProvenImpossible,
        trust: Trust::CheckedExhaustive,
        detail: "finite bfs".into(),
        replay: None,
        z3_ms: None,
        cvc5_ms: None,
        z3: None,
        cvc5: None,
        scope: Some(ex.scope.sentence()),
        reachable_digest: Some(ex.reachable_digest.clone()),
        invariant: Some(ex.invariant_smt.clone()),
    };
    assert!(smt::may_discharge(&report));
    let ok = with_adapter(|| smt::reverify_report(&defn, &machine, &report));
    assert!(ok, "finite-domain exhaustive certificate must reverify");
}
