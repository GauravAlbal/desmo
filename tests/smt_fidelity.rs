//! Differential gate: SMT guard evaluation must match the kernel evaluator.

use ik::declarative::evaluate::{self, GuardSpec};
use ik::declarative::expr::{self, Cmp, CmpOp, EvalCtx, Expr, Operand};
use ik::smt::{encode_selectability, eval_guard_with_solver, kernel_eval, Pinned};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::OnceLock;

fn solver() -> Option<&'static ik::smt::SolverBin> {
    static Z3: OnceLock<Option<ik::smt::SolverBin>> = OnceLock::new();
    Z3.get_or_init(|| ik::smt::discover_solvers().0).as_ref()
}

fn skip_without_solver() -> Option<&'static ik::smt::SolverBin> {
    match solver() {
        Some(bin) => Some(bin),
        None => {
            eprintln!("smt fidelity: skipping solver cases; install z3 under /tmp/smtvenv2 or PATH");
            None
        }
    }
}

fn pin_int(state: &str, name: &str, n: i64) -> Pinned {
    let mut facts = BTreeMap::new();
    facts.insert(name.to_string(), Some(json!(n)));
    Pinned {
        state: state.to_string(),
        facts,
        args: BTreeMap::new(),
    }
}

fn pin_unset(state: &str, name: &str) -> Pinned {
    let mut facts = BTreeMap::new();
    facts.insert(name.to_string(), None);
    Pinned {
        state: state.to_string(),
        facts,
        args: BTreeMap::new(),
    }
}

fn check_eq(expr: &Expr, pinned: &Pinned, solver: &ik::smt::SolverBin) {
    let kernel = kernel_eval(expr, pinned);
    let smt = eval_guard_with_solver(expr, pinned, solver).expect("solver eval");
    assert_eq!(kernel, smt, "mismatch expr={expr:?} pin={pinned:?}");
}

#[test]
fn toward_zero_division_matches_kernel() {
    let Some(solver) = skip_without_solver() else {
        return;
    };
    let expr = parse("n / 2 == 0");
    check_eq(&expr, &pin_int("idle", "n", 7), solver);
    check_eq(&expr, &pin_int("idle", "n", -7), solver);
    let eq_neg3 = Expr::Cmp(Cmp {
        left: Operand::Div {
            base: Box::new(Operand::Fact("n".into())),
            amount: 2,
        },
        op: CmpOp::Eq,
        right: Operand::Literal(json!(-3)),
    });
    check_eq(&eq_neg3, &pin_int("idle", "n", -7), solver);
    let eq_neg3_div = Expr::Cmp(Cmp {
        left: Operand::Div {
            base: Box::new(Operand::Literal(json!(-7))),
            amount: 2,
        },
        op: CmpOp::Eq,
        right: Operand::Literal(json!(-3)),
    });
    check_eq(&eq_neg3_div, &pin_int("idle", "n", 0), solver);
}

#[test]
fn overflow_is_error_not_false() {
    let Some(solver) = skip_without_solver() else {
        return;
    };
    let expr = parse("n + 1 == 0");
    check_eq(&expr, &pin_int("idle", "n", i64::MAX), solver);
    let sub = parse("n - 1 == 0");
    check_eq(&sub, &pin_int("idle", "n", i64::MIN), solver);
    let mul = parse("n * 2 == 0");
    check_eq(&mul, &pin_int("idle", "n", i64::MAX), solver);
}

#[test]
fn unset_fact_is_error_except_exists() {
    let Some(solver) = skip_without_solver() else {
        return;
    };
    check_eq(&parse("n == 1"), &pin_unset("idle", "n"), solver);
    check_eq(&parse("exists(n)"), &pin_unset("idle", "n"), solver);
    check_eq(&parse("not_exists(n)"), &pin_unset("idle", "n"), solver);
    check_eq(&parse("exists(n)"), &pin_int("idle", "n", 3), solver);
}

#[test]
fn short_circuit_and_or_match_kernel() {
    let Some(solver) = skip_without_solver() else {
        return;
    };
    let mut pinned = pin_unset("idle", "n");
    pinned.facts.insert("flag".into(), Some(json!(false)));
    check_eq(&parse("flag == false && n == 1"), &pinned, solver);
    check_eq(&parse("flag == true && n == 1"), &pinned, solver);
    pinned.facts.insert("flag".into(), Some(json!(true)));
    check_eq(&parse("flag == true || n == 1"), &pinned, solver);
    check_eq(&parse("flag == false || n == 1"), &pinned, solver);
}

#[test]
fn string_ordering_is_unsupported() {
    let expr = parse(r#"label < "b""#);
    let mut pinned = Pinned {
        state: "idle".into(),
        facts: BTreeMap::new(),
        args: BTreeMap::new(),
    };
    pinned.facts.insert("label".into(), Some(json!("a")));
    let names = ik::smt::encode_guard_eval(&expr, &pinned);
    assert!(names.is_err(), "string ordering must be unsupported");
}

const HIGH_PLANE: &str = "\u{30000}";
const NONCHAR: &str = "\u{10FFFF}";
const EMOJI: &str = "😀";
const STR_POOL: &[&str] = &[
    "", "a", "b", "ready", "hello", "twin", EMOJI, HIGH_PLANE, NONCHAR,
];

fn pin_strs(facts: &[(&str, &str)], args: &[(&str, &str)]) -> Pinned {
    let mut pinned = Pinned {
        state: "idle".into(),
        facts: BTreeMap::new(),
        args: BTreeMap::new(),
    };
    for (name, value) in facts {
        pinned.facts.insert((*name).into(), Some(json!(*value)));
    }
    for (name, value) in args {
        pinned.args.insert((*name).into(), json!(*value));
    }
    pinned
}

#[test]
fn string_equality_matches_kernel() {
    let Some(solver) = skip_without_solver() else {
        return;
    };
    let cases: &[(&str, Pinned)] = &[
        ("a == b", pin_strs(&[("a", "hello"), ("b", "hello")], &[])),
        ("a == b", pin_strs(&[("a", "hello"), ("b", "world")], &[])),
        ("a != b", pin_strs(&[("a", "hello"), ("b", "hello")], &[])),
        ("a != b", pin_strs(&[("a", "hello"), ("b", "world")], &[])),
        (r#"a == "ready""#, pin_strs(&[("a", "ready")], &[])),
        (r#"a == "ready""#, pin_strs(&[("a", "other")], &[])),
        (r#"a != "ready""#, pin_strs(&[("a", "ready")], &[])),
        (r#"a == """#, pin_strs(&[("a", "")], &[])),
        (r#"a != """#, pin_strs(&[("a", "")], &[])),
        (r#"a != """#, pin_strs(&[("a", "x")], &[])),
        ("a == b", pin_strs(&[("a", EMOJI), ("b", EMOJI)], &[])),
        ("a != b", pin_strs(&[("a", EMOJI), ("b", HIGH_PLANE)], &[])),
        (
            "a == b",
            pin_strs(&[("a", HIGH_PLANE), ("b", NONCHAR)], &[]),
        ),
        ("$x == $y", pin_strs(&[], &[("x", "hello"), ("y", "hello")])),
        ("$x != $y", pin_strs(&[], &[("x", "hello"), ("y", "world")])),
        ("$x != $y", pin_strs(&[], &[("x", ""), ("y", "")])),
        ("$x == $y", pin_strs(&[], &[("x", EMOJI), ("y", NONCHAR)])),
        ("a == $x", pin_strs(&[("a", "hello")], &[("x", "hello")])),
        ("a != $x", pin_strs(&[("a", "")], &[("x", HIGH_PLANE)])),
        (
            r#"a == "ready" && b != $x"#,
            pin_strs(&[("a", "ready"), ("b", EMOJI)], &[("x", EMOJI)]),
        ),
        (
            r#"a == b || $x == """#,
            pin_strs(&[("a", "p"), ("b", "q")], &[("x", "")]),
        ),
        (
            r#"a != b && $x != $y || a == """#,
            pin_strs(
                &[("a", ""), ("b", HIGH_PLANE)],
                &[("x", EMOJI), ("y", EMOJI)],
            ),
        ),
    ];
    for (src, pinned) in cases {
        check_eq(&parse(src), pinned, solver);
    }
}

#[test]
fn random_string_equality_matches_kernel() {
    let Some(solver) = skip_without_solver() else {
        return;
    };
    let mut rng = Lcg(0xA11CE_u64);
    let mut mismatches = 0;
    let mut tested = 0;
    for _ in 0..360 {
        let (expr, pinned) = rng.string_case();
        tested += 1;
        let kernel = kernel_eval(&expr, &pinned);
        match eval_guard_with_solver(&expr, &pinned, solver) {
            Ok(smt) if smt == kernel => {}
            Ok(smt) => {
                mismatches += 1;
                eprintln!(
                    "string mismatch kernel={kernel:?} smt={smt:?} expr={expr:?} pin={pinned:?}"
                );
            }
            Err(err) => {
                mismatches += 1;
                eprintln!("string solver error {err} expr={expr:?}");
            }
        }
    }
    assert!(tested >= 300, "tested {tested} string expressions");
    assert_eq!(mismatches, 0, "{mismatches} string mismatches in {tested}");
}

#[test]
fn nonliteral_string_inequality_is_not_impossible() {
    let Some(solver) = skip_without_solver() else {
        return;
    };
    let def = ik::declarative::parse_definition(STRING_NE_MACHINE).expect("def");
    for (command, outcome) in [("fact_ne", 0), ("fact_eq", 0), ("arg_ne", 0), ("mix", 0)] {
        let q = encode_selectability(&def, command, "idle", outcome).expect(command);
        let solved =
            ik::smt::run_smt2(solver, &q.text, std::time::Duration::from_secs(10)).expect(command);
        assert_eq!(
            solved.reply,
            ik::smt::SolverReply::Sat,
            "{command} must be sat (collapsed OTHER would make != unsat): {}",
            solved.raw
        );
    }
}

const STRING_NE_MACHINE: &str = r#"
schema_version = 1
machine_id = "smt.string-ne"
machine_version = 1

[facts]
a = { type = "string", policy = "replace" }
b = { type = "string", policy = "replace" }

[states]
idle = { initial = true }
done = { terminal = true }

[commands.fact_ne]
from = ["idle"]
to = "done"
when = "a != b"

[commands.fact_eq]
from = ["idle"]
to = "done"
when = "a == b"

[commands.arg_ne]
from = ["idle"]
to = "done"
when = "$x != $y"
args = { x = "string", y = "string" }

[commands.mix]
from = ["idle"]
to = "done"
when = "a == $x"
args = { x = "string" }
"#;

#[test]
fn ordered_outcome_earlier_error_refuses() {
    let n_unset = BTreeMap::new();
    let args = BTreeMap::new();
    let ctx = EvalCtx {
        facts: &n_unset,
        args: &args,
        state: "idle",
    };
    let erroring = parse("n == 1");
    let specs = [
        GuardSpec {
            when: Some(&erroring),
            when_src: Some("n == 1"),
            when_rule: "g0",
            target: "done",
        },
        GuardSpec {
            when: None,
            when_src: None,
            when_rule: "g1",
            target: "done",
        },
    ];
    let selection = evaluate::select_outcomes("idle", true, &specs, &ctx);
    assert!(selection.error.is_some());
    assert_eq!(selection.selected, None);

    let false_and_error = parse("1 == 2 && n == 1");
    let specs_ok = [
        GuardSpec {
            when: Some(&false_and_error),
            when_src: Some("1 == 2 && n == 1"),
            when_rule: "g0",
            target: "done",
        },
        GuardSpec {
            when: None,
            when_src: None,
            when_rule: "g1",
            target: "done",
        },
    ];
    let selection = evaluate::select_outcomes("idle", true, &specs_ok, &ctx);
    assert!(selection.error.is_none());
    assert_eq!(selection.selected, Some(1));

    let Some(solver) = skip_without_solver() else {
        return;
    };
    check_eq(&erroring, &pin_unset("idle", "n"), solver);
    let def = ik::declarative::parse_definition(OUTCOME_MACHINE).expect("def");
    let q0 = encode_selectability(&def, "go", "idle", 0).expect("q0");
    let q1 = encode_selectability(&def, "go", "idle", 1).expect("q1");
    let s0 = ik::smt::run_smt2(solver, &q0.text, std::time::Duration::from_secs(10)).expect("s0");
    let s1 = ik::smt::run_smt2(solver, &q1.text, std::time::Duration::from_secs(10)).expect("s1");
    assert_eq!(s0.reply, ik::smt::SolverReply::Sat);
    assert_eq!(s1.reply, ik::smt::SolverReply::Sat);
}

#[test]
fn random_expressions_match_kernel() {
    let Some(solver) = skip_without_solver() else {
        return;
    };
    let mut rng = Lcg(0xC0FFEE_u64);
    let mut mismatches = 0;
    let mut tested = 0;
    for _ in 0..420 {
        let (expr, pinned) = rng.case();
        tested += 1;
        let kernel = kernel_eval(&expr, &pinned);
        match eval_guard_with_solver(&expr, &pinned, solver) {
            Ok(smt) if smt == kernel => {}
            Ok(smt) => {
                mismatches += 1;
                eprintln!("mismatch kernel={kernel:?} smt={smt:?} expr={expr:?} pin={pinned:?}");
            }
            Err(err) => {
                mismatches += 1;
                eprintln!("solver error {err} expr={expr:?}");
            }
        }
    }
    assert!(
        tested >= 300,
        "tested {tested} expressions, need several hundred"
    );
    assert_eq!(
        mismatches, 0,
        "{mismatches} mismatches in {tested} expressions"
    );
}

fn parse(src: &str) -> Expr {
    expr::parse(src).expect(src)
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.0
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.next() as usize % items.len()]
    }

    fn i64(&mut self) -> i64 {
        *self.pick(&[
            0,
            1,
            2,
            7,
            -7,
            42,
            i64::MAX,
            i64::MIN,
            i64::MAX - 1,
            i64::MIN + 1,
        ])
    }

    fn case(&mut self) -> (Expr, Pinned) {
        match self.next() % 9 {
            0 => self.arith_case(),
            1 => self.div_case(),
            2 => self.logic_case(),
            3 => self.exists_case(),
            4 => self.eq_case(),
            5 => self.overflow_case(),
            6 => self.arg_case(),
            7 => self.string_case(),
            _ => self.short_case(),
        }
    }

    fn arith_case(&mut self) -> (Expr, Pinned) {
        let n = self.i64();
        let k = *self.pick(&[1i64, 2, 3, 7]);
        let expr = parse(&format!("n + {k} == m"));
        let mut facts = BTreeMap::new();
        facts.insert("n".into(), Some(json!(n)));
        facts.insert("m".into(), Some(json!(n.saturating_add(k))));
        (
            expr,
            Pinned {
                state: "idle".into(),
                facts,
                args: BTreeMap::new(),
            },
        )
    }

    fn div_case(&mut self) -> (Expr, Pinned) {
        let n = *self.pick(&[-7i64, -1, 0, 1, 7, 8, i64::MIN, i64::MAX]);
        let d = *self.pick(&[1i64, 2, 3, 7]);
        let expr = Expr::Cmp(Cmp {
            left: Operand::Div {
                base: Box::new(Operand::Fact("n".into())),
                amount: d,
            },
            op: CmpOp::Eq,
            right: Operand::Literal(json!(n / d)),
        });
        (expr, pin_int("idle", "n", n))
    }

    fn logic_case(&mut self) -> (Expr, Pinned) {
        let a = self.next() % 2 == 0;
        let b = self.next() % 2 == 0;
        let expr = parse("flag == true && other == true");
        let mut facts = BTreeMap::new();
        facts.insert("flag".into(), Some(json!(a)));
        facts.insert("other".into(), Some(json!(b)));
        (
            expr,
            Pinned {
                state: "idle".into(),
                facts,
                args: BTreeMap::new(),
            },
        )
    }

    fn exists_case(&mut self) -> (Expr, Pinned) {
        let set = self.next() % 2 == 0;
        let expr = parse("exists(n) && n == 1");
        let mut facts = BTreeMap::new();
        facts.insert("n".into(), if set { Some(json!(1)) } else { None });
        (
            expr,
            Pinned {
                state: "idle".into(),
                facts,
                args: BTreeMap::new(),
            },
        )
    }

    fn eq_case(&mut self) -> (Expr, Pinned) {
        let label = *self.pick(&["a", "b", "ready"]);
        let expr = parse(r#"label == "a""#);
        let mut facts = BTreeMap::new();
        facts.insert("label".into(), Some(json!(label)));
        (
            expr,
            Pinned {
                state: "idle".into(),
                facts,
                args: BTreeMap::new(),
            },
        )
    }

    fn string_atom(&mut self) -> Expr {
        let lit = *self.pick(STR_POOL);
        let op = if self.next() % 2 == 0 {
            CmpOp::Eq
        } else {
            CmpOp::Ne
        };
        let left = match self.next() % 5 {
            0 => Operand::Fact("a".into()),
            1 => Operand::Fact("b".into()),
            2 => Operand::Arg("x".into()),
            3 => Operand::Arg("y".into()),
            _ => Operand::Literal(json!(lit)),
        };
        let right = match self.next() % 5 {
            0 => Operand::Fact("a".into()),
            1 => Operand::Fact("b".into()),
            2 => Operand::Arg("x".into()),
            3 => Operand::Arg("y".into()),
            _ => Operand::Literal(json!(*self.pick(STR_POOL))),
        };
        Expr::Cmp(Cmp { left, op, right })
    }

    fn string_case(&mut self) -> (Expr, Pinned) {
        let atom = self.string_atom();
        let expr = match self.next() % 4 {
            0 => atom,
            1 => Expr::And(vec![atom, self.string_atom()]),
            2 => Expr::Or(vec![atom, self.string_atom()]),
            _ => Expr::And(vec![
                atom,
                Expr::Or(vec![self.string_atom(), self.string_atom()]),
            ]),
        };
        let facts = [("a", *self.pick(STR_POOL)), ("b", *self.pick(STR_POOL))];
        let args = [("x", *self.pick(STR_POOL)), ("y", *self.pick(STR_POOL))];
        (expr, pin_strs(&facts, &args))
    }

    fn overflow_case(&mut self) -> (Expr, Pinned) {
        let expr = parse("n * 2 == 0");
        (
            expr,
            pin_int("idle", "n", *self.pick(&[i64::MAX, 0, 1, -1])),
        )
    }

    fn arg_case(&mut self) -> (Expr, Pinned) {
        let n = self.i64();
        let expr = parse("$x == n");
        let mut facts = BTreeMap::new();
        facts.insert("n".into(), Some(json!(n)));
        let mut args = BTreeMap::new();
        args.insert("x".into(), json!(n));
        (
            expr,
            Pinned {
                state: "idle".into(),
                facts,
                args,
            },
        )
    }

    fn short_case(&mut self) -> (Expr, Pinned) {
        let flag = self.next() % 2 == 0;
        let expr = parse("flag == false && n == 1");
        let mut facts = BTreeMap::new();
        facts.insert("flag".into(), Some(json!(flag)));
        facts.insert("n".into(), None);
        (
            expr,
            Pinned {
                state: "idle".into(),
                facts,
                args: BTreeMap::new(),
            },
        )
    }
}

const REACH_MACHINE: &str = r#"
schema_version = 1
machine_id = "smt.reach"
machine_version = 1

[facts]
n = { type = "int", policy = "replace", initial = 0 }

[states]
idle = { initial = true }
done = { terminal = true }
lost = {}

[commands.go]
from = ["idle"]
to = "done"
"#;

#[test]
fn state_equality_uses_state_index() {
    let Some(solver) = skip_without_solver() else {
        return;
    };
    let def = ik::declarative::parse_definition(STATE_EQ_MACHINE).expect("def");
    let q = encode_selectability(&def, "go", "idle", 0).expect("q");
    let solved =
        ik::smt::run_smt2(solver, &q.text, std::time::Duration::from_secs(10)).expect("solve");
    assert_eq!(solved.reply, ik::smt::SolverReply::Sat, "{}", solved.raw);
}

const STATE_EQ_MACHINE: &str = r#"
schema_version = 1
machine_id = "smt.state-eq"
machine_version = 1

[facts]
label = { type = "string", policy = "replace", initial = "zz" }

[states]
idle = { initial = true }
done = { terminal = true }
lost = {}

[commands.go]
from = ["idle"]
to = "done"
when = "state == \"idle\" && label == \"zz\""
"#;

#[test]
fn bmc_successor_replays_through_kernel() {
    let Some(z3) = skip_without_solver() else {
        return;
    };
    let def = ik::declarative::parse_definition(REACH_MACHINE).expect("def");
    let machine = ik::declarative::parse_and_compile(REACH_MACHINE).expect("machine");
    let report = ik::smt::analyze_target(
        &def,
        &machine,
        "state/done/executable",
        ik::smt::Target::State("done".into()),
        Some(z3),
        None,
    );
    assert_eq!(
        report.status,
        ik::smt::ProofStatus::Witness,
        "{}",
        report.detail
    );
}

#[test]
fn horn_lost_state_is_impossible() {
    let Some(z3) = skip_without_solver() else {
        return;
    };
    let def = ik::declarative::parse_definition(REACH_MACHINE).expect("def");
    let machine = ik::declarative::parse_and_compile(REACH_MACHINE).expect("machine");
    let report = ik::smt::analyze_target(
        &def,
        &machine,
        "state/lost/executable",
        ik::smt::Target::State("lost".into()),
        Some(z3),
        None,
    );
    assert_eq!(
        report.status,
        ik::smt::ProofStatus::ProvenImpossible,
        "{}",
        report.detail
    );
}

#[test]
fn consumer_h_and_a_obligations() {
    let dir = std::env::var("SMT_CONSUMER_DIR")
        .unwrap_or_else(|_| "/tmp/ik-consumer-h".to_string());
    let h = format!("{dir}/host-attempt-recovery.ik.toml");
    if !std::path::Path::new(&h).is_file() {
        eprintln!("smt: no consumer copy at {h}");
        return;
    }
    let z3 = skip_without_solver();
    let cvc5 = ik::smt::discover_solvers().1;
    let text = std::fs::read_to_string(&h).expect("machine");
    let def = ik::declarative::parse_definition(&text).expect("parse");
    let machine = ik::declarative::parse_and_compile(&text).expect("compile");
    dump_classify_arms(&def);
    let sample = ik::smt::parse_target(H_FIVE[0]).expect("target");
    let uni = ik::smt::exhaustive_search(&def, &machine, &sample).expect("bfs");
    for index in 15..=23 {
        let hit = uni.reached_outcomes.contains(&("classify".to_string(), index));
        eprintln!("classify/{index} reached_in_bfs={hit}");
    }
    for id in H_FIVE {
        let target = ik::smt::parse_target(id).expect("target");
        let report = ik::smt::analyze_target(&def, &machine, id, target, z3, cvc5.as_ref());
        eprintln!(
            "H\t{}\t{:?}\t{:?}\tscope={:?}\tdigest={:?}\t{}\treplay={:?}",
            report.id,
            report.status,
            report.trust,
            report.scope,
            report.reachable_digest,
            report.detail,
            report.replay
        );
        assert_ne!(
            report.status,
            ik::smt::ProofStatus::Unknown,
            "H {id} stayed unknown: {}",
            report.detail
        );
        if let (Some(z3), Some(index)) = (z3, id.split('/').nth(2).and_then(|s| s.parse().ok())) {
            if let Ok(query) = ik::smt::encode_selectability_any(&def, "classify", index) {
                let z = ik::smt::run_smt2(z3, &query.text, std::time::Duration::from_secs(10));
                let c = cvc5.as_ref().map(|bin| {
                    ik::smt::run_smt2(bin, &query.text, std::time::Duration::from_secs(10))
                });
                eprintln!(
                    "H\t{id}\tlocal_z3={:?}\tlocal_cvc5={:?}",
                    z.as_ref().map(|r| &r.reply),
                    c.as_ref().and_then(|r| r.as_ref().ok().map(|s| &s.reply))
                );
            }
        }
    }
}

fn dump_classify_arms(def: &ik::declarative::Definition) {
    let Some(cmd) = def.commands.get("classify") else {
        return;
    };
    let Some(outcomes) = &cmd.outcomes else {
        return;
    };
    eprintln!("classify arms {}", outcomes.len());
    for index in 15..=23 {
        let Some(outcome) = outcomes.get(index) else {
            continue;
        };
        eprintln!(
            "  {index} -> {} when {}",
            outcome.to,
            outcome.when.as_deref().unwrap_or("-")
        );
    }
}

const H_FIVE: [&str; 5] = [
    "outcome/classify/17/executable",
    "outcome/classify/18/executable",
    "outcome/classify/19/executable",
    "outcome/classify/20/executable",
    "outcome/classify/21/executable",
];

const OUTCOME_MACHINE: &str = r#"
schema_version = 1
machine_id = "smt.outcome"
machine_version = 1

[facts]
n = { type = "int", policy = "replace" }

[states]
idle = { initial = true }
done = { terminal = true }

[commands.go]
from = ["idle"]
outcomes = [
  { to = "done", when = "n == 1" },
  { to = "done" },
]
"#;
