//! Known-solution machines: kernel BFS, Z3, and cvc5 must agree.

use ik::smt::{self, ProofStatus, Trust};
use serde_json::json;

#[derive(Clone, Copy)]
enum Expect {
    Feasible,
    Impossible,
}

struct Case {
    id: &'static str,
    src: &'static str,
    target: &'static str,
    expect: Expect,
}

fn cases() -> Vec<Case> {
    vec![
        Case { id: "k01-empty-initial", src: M_DONE_IS_INIT, target: "state/idle/executable", expect: Expect::Feasible },
        Case { id: "k02-one-step", src: M_MOVE, target: "state/done/executable", expect: Expect::Feasible },
        Case { id: "k03-lost-state", src: M_MOVE, target: "state/lost/executable", expect: Expect::Impossible },
        Case { id: "k04-shadow-second", src: M_SHADOW, target: "outcome/go/1/executable", expect: Expect::Impossible },
        Case { id: "k05-first-arm", src: M_SHADOW, target: "outcome/go/0/executable", expect: Expect::Feasible },
        Case { id: "k06-catch-all", src: M_ORDER, target: "outcome/go/1/executable", expect: Expect::Feasible },
        Case { id: "k07-n-equals-one", src: M_ORDER, target: "outcome/go/0/executable", expect: Expect::Feasible },
        Case { id: "k08-string-hit", src: M_STR, target: "outcome/go/0/executable", expect: Expect::Feasible },
        Case { id: "k09-string-other", src: M_STR, target: "state/lost/executable", expect: Expect::Impossible },
        Case { id: "k10-counter-neg", src: M_COUNTER, target: "state/neg/executable", expect: Expect::Impossible },
        Case { id: "k11-counter-inc", src: M_COUNTER, target: "state/pos/executable", expect: Expect::Feasible },
        Case { id: "k12-overflow-dest", src: M_OVERFLOW, target: "state/wrapped/executable", expect: Expect::Impossible },
        Case { id: "k13-overflow-hot", src: M_OVERFLOW, target: "state/hot/executable", expect: Expect::Feasible },
        Case { id: "k14-terminal", src: M_MOVE, target: "terminal/done/reachable", expect: Expect::Feasible },
        Case { id: "k15-dead-terminal", src: M_MOVE, target: "terminal/lost/reachable", expect: Expect::Impossible },
        Case { id: "k16-bool-true", src: M_BOOL, target: "outcome/go/0/executable", expect: Expect::Feasible },
        Case { id: "k17-bool-shadow", src: M_BOOL, target: "outcome/go/1/executable", expect: Expect::Feasible },
        Case { id: "k18-two-command", src: M_PATH, target: "state/done/executable", expect: Expect::Feasible },
        Case { id: "k19-blocked-mid", src: M_PATH, target: "state/lost/executable", expect: Expect::Impossible },
        Case { id: "k20-cap", src: M_CAP, target: "outcome/tick/1/executable", expect: Expect::Feasible },
        Case { id: "k21-cap-shadowed", src: M_CAP, target: "outcome/tick/0/executable", expect: Expect::Feasible },
        Case { id: "k22-unsat-guard", src: M_FALSE, target: "outcome/go/0/executable", expect: Expect::Impossible },
        Case { id: "k23-cycle-lost", src: M_CYCLE, target: "state/lost/executable", expect: Expect::Impossible },
        Case { id: "k24-cycle-here", src: M_CYCLE, target: "state/here/executable", expect: Expect::Feasible },
        Case { id: "k25-string-neq", src: M_STR_NE, target: "outcome/go/0/executable", expect: Expect::Feasible },
        Case { id: "k26-int-boundary", src: M_BOUND, target: "outcome/go/0/executable", expect: Expect::Feasible },
    ]
}

#[test]
fn known_solutions_agree() {
    let z3 = ik::smt::discover_solvers().0;
    let cvc5 = ik::smt::discover_solvers().1;
    let mut feasible = 0usize;
    let mut impossible = 0usize;
    for case in cases() {
        let def = ik::declarative::parse_definition(case.src)
            .unwrap_or_else(|e| panic!("{} parse: {e:?}", case.id));
        let machine = ik::declarative::parse_and_compile(case.src)
            .unwrap_or_else(|e| panic!("{} compile: {e:?}", case.id));
        let target = ik::smt::parse_target(case.target)
            .unwrap_or_else(|| panic!("{} bad target", case.id));
        let ex = smt::exhaustive_search(&def, &machine, &target)
            .unwrap_or_else(|e| panic!("{} bfs: {e}", case.id));
        match case.expect {
            Expect::Feasible => {
                assert!(
                    ex.witness.is_some(),
                    "{} expected witness; complete={} reason={:?} digest={}",
                    case.id,
                    ex.complete,
                    ex.incomplete_reason,
                    ex.reachable_digest
                );
                smt::replay_trace(&machine, ex.witness.as_ref().expect("w"))
                    .unwrap_or_else(|e| panic!("{} replay: {e}", case.id));
                feasible += 1;
            }
            Expect::Impossible => {
                assert!(
                    ex.witness.is_none(),
                    "{} expected no witness; complete={} reason={:?}",
                    case.id,
                    ex.complete,
                    ex.incomplete_reason
                );
                impossible += 1;
            }
        }
        if let Some(z3) = z3.as_ref() {
            let report = smt::analyze_target(
                &def,
                &machine,
                case.target,
                target,
                Some(z3),
                cvc5.as_ref(),
            );
            match case.expect {
                Expect::Feasible => {
                    assert_eq!(
                        report.status,
                        ProofStatus::Witness,
                        "{} solver {} {}",
                        case.id,
                        report.detail,
                        report.replay.unwrap_or_default()
                    );
                    assert_ne!(report.trust, Trust::SolverAsserted, "{}", case.id);
                }
                Expect::Impossible => {
                    assert_eq!(
                        report.status,
                        ProofStatus::ProvenImpossible,
                        "{} solver {}",
                        case.id,
                        report.detail
                    );
                    assert_ne!(
                        report.trust,
                        Trust::SolverAsserted,
                        "{} stayed solver-asserted: {}",
                        case.id,
                        report.detail
                    );
                }
            }
        }
    }
    eprintln!(
        "known solutions: {} feasible, {} impossible, solvers z3={} cvc5={}",
        feasible,
        impossible,
        z3.as_ref().map(|s| s.version.as_str()).unwrap_or("absent"),
        cvc5.as_ref().map(|s| s.version.as_str()).unwrap_or("absent")
    );
    assert!(feasible >= 12, "feasible={feasible}");
    assert!(impossible >= 8, "impossible={impossible}");
}

const M_DONE_IS_INIT: &str = r#"
schema_version = 1
machine_id = "smt.known.init"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = { terminal = true }
[commands.noop]
from = ["idle"]
to = "idle"
"#;

const M_MOVE: &str = r#"
schema_version = 1
machine_id = "smt.known.move"
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

const M_SHADOW: &str = r#"
schema_version = 1
machine_id = "smt.known.shadow"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = { terminal = true }
[commands.go]
from = ["idle"]
outcomes = [
  { to = "done", when = "0 == 0" },
  { to = "done" },
]
"#;

const M_ORDER: &str = r#"
schema_version = 1
machine_id = "smt.known.order"
machine_version = 1
[facts]
n = { type = "int", policy = "replace", initial = 0 }
[states]
idle = { initial = true }
done = { terminal = true }
[commands.go]
from = ["idle"]
args = { n = "int" }
outcomes = [
  { to = "done", when = "$n == 1" },
  { to = "done" },
]
"#;

const M_STR: &str = r#"
schema_version = 1
machine_id = "smt.known.str"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = { terminal = true }
lost = {}
[commands.go]
from = ["idle"]
args = { label = "string" }
outcomes = [
  { to = "done", when = "$label == \"aa\"" },
]
"#;

const M_COUNTER: &str = r#"
schema_version = 1
machine_id = "smt.known.counter"
machine_version = 1
[facts]
n = { type = "int", policy = "counter", initial = 0 }
[states]
zero = { initial = true }
pos = {}
neg = {}
[commands.inc]
from = ["zero"]
to = "pos"
then = { n = "+1" }
[commands.dec]
from = ["zero"]
to = "neg"
when = "n < 0"
"#;

const M_OVERFLOW: &str = r#"
schema_version = 1
machine_id = "smt.known.overflow"
machine_version = 1
[facts]
n = { type = "int", policy = "counter", initial = 9223372036854775806 }
[states]
idle = { initial = true }
hot = {}
wrapped = {}
[commands.bump]
from = ["idle"]
to = "hot"
then = { n = "+1" }
[commands.again]
from = ["hot"]
to = "wrapped"
then = { n = "+1" }
"#;

const M_BOOL: &str = r#"
schema_version = 1
machine_id = "smt.known.bool"
machine_version = 1
[facts]
[states]
idle = { initial = true }
yes = {}
no = {}
[commands.go]
from = ["idle"]
args = { flag = "bool" }
outcomes = [
  { to = "yes", when = "$flag == true" },
  { to = "no" },
]
"#;

const M_PATH: &str = r#"
schema_version = 1
machine_id = "smt.known.path"
machine_version = 1
[facts]
[states]
idle = { initial = true }
mid = {}
done = { terminal = true }
lost = {}
[commands.step]
from = ["idle"]
to = "mid"
[commands.finish]
from = ["mid"]
to = "done"
"#;

const M_CAP: &str = r#"
schema_version = 1
machine_id = "smt.known.cap"
machine_version = 1
[facts]
attempts = { type = "int", policy = "replace", initial = 0 }
[states]
idle = { initial = true }
run = {}
held = { terminal = true }
[commands.tick]
from = ["idle"]
args = { n = "int" }
outcomes = [
  { to = "held", when = "$n >= 2" },
  { to = "run", then = { attempts = "$n" } },
]
"#;

const M_FALSE: &str = r#"
schema_version = 1
machine_id = "smt.known.false"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = { terminal = true }
[commands.go]
from = ["idle"]
outcomes = [
  { to = "done", when = "0 == 1" },
]
"#;

const M_CYCLE: &str = r#"
schema_version = 1
machine_id = "smt.known.cycle"
machine_version = 1
[facts]
[states]
here = { initial = true }
lost = {}
[commands.spin]
from = ["here"]
to = "here"
"#;

const M_STR_NE: &str = r#"
schema_version = 1
machine_id = "smt.known.strne"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = { terminal = true }
[commands.go]
from = ["idle"]
args = { a = "string", b = "string" }
outcomes = [
  { to = "done", when = "$a != $b" },
]
"#;

const M_BOUND: &str = r#"
schema_version = 1
machine_id = "smt.known.bound"
machine_version = 1
[facts]
[states]
idle = { initial = true }
done = { terminal = true }
[commands.go]
from = ["idle"]
args = { n = "int" }
outcomes = [
  { to = "done", when = "$n == 9223372036854775807" },
]
"#;

#[test]
fn suggested_case_json_is_object() {
    let step = ik::smt::CommandStep {
        name: "go".into(),
        args: [("n".into(), json!(1))].into_iter().collect(),
    };
    let value = smt::suggested_cases(&[step]);
    assert!(value["commands"]["go"].as_array().is_some());
}
