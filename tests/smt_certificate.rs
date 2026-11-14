//! Negative controls for the inductive-invariant checker.

use ik::smt::{self, Target};

const MOVE: &str = r#"
schema_version = 1
machine_id = "smt.cert.move"
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

#[test]
fn wrong_invariants_are_rejected() {
    let Some(z3) = skip() else {
        return;
    };
    let cvc5 = ik::smt::discover_solvers().1;
    let def = ik::declarative::parse_definition(MOVE).expect("def");
    let target = Target::State("done".into());
    let rows = smt::reject_wrong_invariants(&def, &target, z3, cvc5.as_ref()).expect("reject");
    assert_eq!(rows.len(), 3);
    for (name, check) in rows {
        assert_eq!(
            check.accepted(),
            false,
            "{name} was accepted: {}",
            check.summary()
        );
    }
}

#[test]
fn reachable_set_invariant_is_accepted_for_lost_state() {
    let Some(z3) = skip() else {
        return;
    };
    let cvc5 = ik::smt::discover_solvers().1;
    let def = ik::declarative::parse_definition(MOVE).expect("def");
    let machine = ik::declarative::parse_and_compile(MOVE).expect("machine");
    let target = Target::State("lost".into());
    let ex = smt::exhaustive_search(&def, &machine, &target).expect("bfs");
    assert!(ex.complete, "{:?}", ex.incomplete_reason);
    assert!(ex.witness.is_none());
    let check =
        smt::check_invariant(&def, &ex.invariant_smt, &target, Some(z3), cvc5.as_ref()).expect("I");
    assert!(
        check.accepted(),
        "reachable-set I rejected: {}",
        check.summary()
    );
}

#[test]
fn large_reachable_set_invariant_is_accepted() {
    let Some(z3) = skip() else {
        return;
    };
    let cvc5 = ik::smt::discover_solvers().1;
    let src = flags_src(7);
    let def = ik::declarative::parse_definition(&src).expect("def");
    let machine = ik::declarative::parse_and_compile(&src).expect("machine");
    let target = Target::State("lost".into());
    let ex = ik::smt::exhaustive_search(&def, &machine, &target).expect("bfs");
    assert!(ex.complete, "{:?}", ex.incomplete_reason);
    assert!(ex.nodes > 64, "nodes={}", ex.nodes);
    assert!(ex.witness.is_none());
    let check =
        ik::smt::check_invariant(&def, &ex.invariant_smt, &target, Some(z3), cvc5.as_ref())
            .expect("I");
    assert!(
        check.accepted(),
        "reachable-set I rejected at {} nodes: {}",
        ex.nodes,
        check.summary()
    );
    if cvc5.is_some() {
        assert!(
            check.dual_checked(),
            "both solvers must agree: {}",
            check.summary()
        );
    }
}

#[test]
fn symbolic_int_args_are_in_the_step_query() {
    let Some(z3) = skip() else {
        return;
    };
    let cvc5 = ik::smt::discover_solvers().1;
    const SRC: &str = r#"
schema_version = 1
machine_id = "smt.cert.intarg"
machine_version = 1
[facts]
n = { type = "int", policy = "replace", initial = 0 }
flag = { type = "bool", policy = "replace", initial = false }
[states]
idle = { initial = true }
[commands.set]
from = ["idle"]
args = { x = "int" }
outcomes = [
  { to = "idle", when = "$x == n", then = { n = "$x" } },
  { to = "idle", when = "flag == true", then = { flag = "true" } },
]
"#;
    let def = ik::declarative::parse_definition(SRC).expect("def");
    let machine = ik::declarative::parse_and_compile(SRC).expect("machine");
    let target = Target::Outcome {
        command: "set".into(),
        index: 1,
    };
    let ex = ik::smt::exhaustive_search(&def, &machine, &target).expect("bfs");
    assert!(ex.witness.is_none(), "{:?}", ex.witness);
    let query = ik::smt::encode_invariant_step(&def, &ex.invariant_smt).expect("step");
    assert!(
        query.text.contains("|av:x|"),
        "step query must quantify the integer argument"
    );
    let check =
        ik::smt::check_invariant(&def, &ex.invariant_smt, &target, Some(z3), cvc5.as_ref())
            .expect("I");
    assert!(
        check.accepted(),
        "symbolic-int I rejected: {}",
        check.summary()
    );
    if cvc5.is_some() {
        assert!(check.dual_checked(), "{}", check.summary());
    }
}

#[test]
fn symbolic_int_shadowed_arm_is_checked_invariant() {
    let Some(z3) = skip() else {
        return;
    };
    let cvc5 = ik::smt::discover_solvers().1;
    const SRC: &str = r#"
schema_version = 1
machine_id = "smt.cert.shadowint"
machine_version = 1
[facts]
ready = { type = "bool", policy = "replace", initial = false }
n = { type = "int", policy = "replace", initial = 0 }
[states]
idle = { initial = true }
seen = {}
done = { terminal = true }
held = { terminal = true }
[commands.mark]
from = ["idle"]
args = { x = "int" }
when = "$x == 0 || $x == 1"
then = { n = "$x", ready = "true" }
to = "seen"
[commands.classify]
from = ["seen"]
args = { extra = "bool" }
outcomes = [
  { to = "done", when = "ready == true" },
  { to = "held", when = "$extra == true" },
  { to = "held" },
]
"#;
    let def = ik::declarative::parse_definition(SRC).expect("def");
    let machine = ik::declarative::parse_and_compile(SRC).expect("machine");
    let target = Target::Outcome {
        command: "classify".into(),
        index: 1,
    };
    let ex = ik::smt::exhaustive_search(&def, &machine, &target).expect("bfs");
    assert_eq!(ex.witness.is_some(), false, "{:?}", ex.witness);
    let query = ik::smt::encode_invariant_step(&def, &ex.invariant_smt).expect("step");
    assert!(
        query.text.contains("|av:x|") || query.text.contains("|av:extra|"),
        "step query must keep symbolic arguments: {}",
        query.text
    );
    let check =
        ik::smt::check_invariant(&def, &ex.invariant_smt, &target, Some(z3), cvc5.as_ref())
            .expect("I");
    if !check.accepted() {
        let projected =
            ik::smt::check_invariant(&def, &ex.projected_smt, &target, Some(z3), cvc5.as_ref())
                .expect("projected I");
        assert!(
            projected.accepted(),
            "sampled={} projected={} replay={:?}",
            check.summary(),
            projected.summary(),
            check.replay_notes()
        );
        if cvc5.is_some() {
            assert!(projected.dual_checked(), "{}", projected.summary());
        }
        return;
    }
    if cvc5.is_some() {
        assert!(check.dual_checked(), "{}", check.summary());
    }
}

fn flags_src(n: usize) -> String {
    let mut out = String::from(
        r#"
schema_version = 1
machine_id = "smt.cert.flags"
machine_version = 1
[facts]
"#,
    );
    for i in 0..n {
        out.push_str(&format!(
            "f{i} = {{ type = \"bool\", policy = \"replace\", initial = false }}\n"
        ));
    }
    out.push_str("[states]\nidle = { initial = true }\nlost = {}\n");
    for i in 0..n {
        out.push_str(&format!(
            "[commands.set{i}]\nfrom = [\"idle\"]\nto = \"idle\"\nthen = {{ f{i} = \"true\" }}\n"
        ));
    }
    out
}

fn skip() -> Option<&'static ik::smt::SolverBin> {
    static CELL: std::sync::OnceLock<Option<ik::smt::SolverBin>> = std::sync::OnceLock::new();
    match CELL.get_or_init(|| ik::smt::discover_solvers().0).as_ref() {
        Some(bin) => Some(bin),
        None => {
            eprintln!("smt certificate: skipping; z3 not installed");
            None
        }
    }
}
