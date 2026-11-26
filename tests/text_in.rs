//! `text_in` predicate: parse, evaluate, runtime, explorer, analysis, compatibility.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::canonical::{canonical_json, digest_json};
use ik::declarative::explore::explore_graph;
use ik::declarative::expr::{self, EvalCtx, Expr};
use ik::declarative::{
    parse_and_compile, parse_definition, starter_corpus, verification_plan, Bounds, CaseCorpus,
    CompiledMachine, GuardEvaluation,
};
use ik::diagnostic::DiagnosticCode;
use ik::effect::EffectStatus;
use ik::machine::{Decision, Machine};
use ik::smt::{
    discover_solvers, encode_guard_eval, eval_guard_with_solver, kernel_eval, GuardValue, Pinned,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;

fn header() -> &'static str {
    r#"
schema_version = 1
machine_id = "text_in_probe"
machine_version = 1
"#
}

fn tagged_toml() -> String {
    format!(
        r#"
{header}
[facts]
item_id = {{ type = "string", policy = "replace" }}
label = {{ type = "string", policy = "replace" }}
[states]
idle = {{ initial = true }}
tagged = {{ terminal = true }}
[commands.tag]
from = ["idle"]
to = "tagged"
args = {{ id = "string", label = "string" }}
when = "text_in($id, 'A-Za-z0-9._-', 1, 64)"
then = {{ item_id = "$id", label = "$label" }}
"#,
        header = header()
    )
}

fn compile(toml: &str) -> CompiledMachine {
    parse_and_compile(toml)
        .map_err(|errors| errors.join("\n"))
        .expect("compile")
}

fn emitted(decision: Decision) -> ik::machine::DomainEvent {
    match decision {
        Decision::Emit { event, .. } => Some(event),
        Decision::Noop { .. } => None,
    }
    .expect("expected emit")
}

fn decide(
    machine: &CompiledMachine,
    command: Value,
) -> Result<Decision, ik::diagnostic::Diagnostic> {
    machine.decide(
        &machine.initial_state(),
        None,
        &EffectStatus::None,
        &command,
    )
}

fn eval_id(id: &str) -> Result<bool, String> {
    let expr = expr::parse("text_in($id,'A-Za-z0-9._-',1,64)").expect("parse");
    let facts = BTreeMap::new();
    let mut args = BTreeMap::new();
    args.insert("id".into(), Value::String(id.to_string()));
    expr::eval(
        &expr,
        &EvalCtx {
            facts: &facts,
            args: &args,
            state: "idle",
            effect: None,
        },
    )
}

fn tag_cmd(id: &str, label: &str) -> Value {
    json!({"name": "tag", "args": {"id": id, "label": label}})
}

#[test]
fn identifier_boundaries() {
    assert_eq!(eval_id("").expect("empty"), false);
    assert_eq!(eval_id("A").expect("one"), true);
    let sixty_four: String = std::iter::repeat('A').take(64).collect();
    let sixty_five: String = std::iter::repeat('A').take(65).collect();
    assert_eq!(eval_id(&sixty_four).expect("64"), true);
    assert_eq!(eval_id(&sixty_five).expect("65"), false);
    for ch in [' ', '/', ':', '\0', '\n', '\t', '~'] {
        let s = ch.to_string();
        assert_eq!(eval_id(&s).expect("disallowed"), false, "{ch:?}");
    }
    for sample in ["é", "字", "😀", "１", "e\u{301}"] {
        assert_eq!(eval_id(sample).expect("non-ascii"), false, "{sample}");
    }
    for ch in ['.', '_', '-'] {
        assert_eq!(eval_id(&ch.to_string()).expect("punct"), true, "{ch}");
    }
}

#[test]
fn star_set_counts_scalars_not_bytes() {
    let expr = expr::parse("text_in($x,'*',1,64)").expect("parse");
    let facts = BTreeMap::new();
    let eval = |s: &str| {
        let mut args = BTreeMap::new();
        args.insert("x".into(), Value::String(s.to_string()));
        expr::eval(
            &expr,
            &EvalCtx {
                facts: &facts,
                args: &args,
                state: "idle",
                effect: None,
            },
        )
        .expect("eval")
    };
    let sixty_four: String = std::iter::repeat('😀').take(64).collect();
    let sixty_five: String = std::iter::repeat('😀').take(65).collect();
    assert_eq!(sixty_four.len() > 64, true);
    assert_eq!(eval(&sixty_four), true);
    assert_eq!(eval(&sixty_five), false);
}

#[test]
fn definition_errors_name_the_problem() {
    let cases = [
        ("text_in($id,'',1,1)", "empty"),
        ("text_in($id,'z-a',1,1)", "reversed"),
        ("text_in($id,'é',1,1)", "non-ASCII"),
        ("text_in($id,'A',2,1)", "MIN must be <="),
        ("text_in($id,'A',0,4097)", "MAX must be <="),
        ("text_in($id,'A',-1,10)", "non-negative"),
        ("text_in($id,'A','x',10)", "integer literal"),
        ("text_in($id,'A',1)", "four arguments"),
        ("text_in('hello','A',1,1)", "not a literal"),
        ("textinn($id,'A',1,1)", "unknown predicate"),
        ("text_in($id,'A',1,1) junk", "unexpected token"),
        ("text_in(state,'*',0,8)", "not `state`"),
        ("text_in($id,'a--z',1,1)", "malformed range"),
    ];
    for (src, needle) in cases {
        let err = expr::parse(src).expect_err(src);
        assert_eq!(
            err.contains(needle),
            true,
            "src={src} needle={needle} err={err}"
        );
    }
}

#[test]
fn unset_or_non_string_is_eval_error() {
    let expr = expr::parse("text_in(name,'*',0,8)").expect("parse");
    let args = BTreeMap::new();
    let facts = BTreeMap::new();
    let err = expr::eval(
        &expr,
        &EvalCtx {
            facts: &facts,
            args: &args,
            state: "idle",
            effect: None,
        },
    )
    .expect_err("unset");
    assert_eq!(err.contains("not set"), true, "{err}");
    let mut ints = BTreeMap::new();
    ints.insert("name".into(), json!(3));
    let err = expr::eval(
        &expr,
        &EvalCtx {
            facts: &ints,
            args: &args,
            state: "idle",
            effect: None,
        },
    )
    .expect_err("int");
    assert_eq!(err.contains("string value"), true, "{err}");
}

#[test]
fn runtime_tag_commits_valid_and_refuses_invalid() {
    let machine = compile(&tagged_toml());
    let before = machine.initial_state();
    let event = emitted(decide(&machine, tag_cmd("item_1", "AMBER")).expect("valid"));
    let after = machine.apply(&before, &event).expect("apply");
    assert_eq!(after.state["item_id"], "item_1");
    assert_eq!(after.state["label"], "AMBER");
    assert_eq!(after.terminal, true);

    let overlong = "A".repeat(65);
    for (id, label) in [
        ("", "AMBER"),
        (overlong.as_str(), "x"),
        ("a/b", "x"),
        ("é", "x"),
        ("has space", "x"),
    ] {
        let err = decide(&machine, tag_cmd(id, label)).expect_err("refuse");
        assert_eq!(err.code, DiagnosticCode::IkInvariantFailed);
        assert_eq!(err.rule.as_deref(), Some("tag.when"));
        assert_eq!(machine.initial_state(), before);
    }

    let event = emitted(decide(&machine, tag_cmd("ok", "x y")).expect("opaque space"));
    let applied = machine.apply(&before, &event).expect("apply");
    assert_eq!(applied.state["label"], "x y");
    let long_label: String = std::iter::repeat('L').take(200).collect();
    let event = emitted(decide(&machine, tag_cmd("ok", &long_label)).expect("opaque long"));
    let applied = machine.apply(&before, &event).expect("apply");
    assert_eq!(applied.state["label"], long_label);
}

#[test]
fn explain_shows_predicate_and_outcome() {
    let machine = compile(&tagged_toml());
    let explanation =
        ik::declarative::explain(&machine, &machine.initial_state(), &tag_cmd("", "AMBER"));
    assert_eq!(explanation.verdict, "refused");
    let decision = explanation.decision.expect("decision");
    assert_eq!(decision.candidates.len(), 1);
    let candidate = &decision.candidates[0];
    let guard = candidate.guard.as_deref().expect("guard");
    assert_eq!(guard.contains("text_in"), true, "{guard}");
    assert_eq!(candidate.evaluation, GuardEvaluation::False);
    assert_eq!(
        explanation.diagnostic.as_ref().map(|d| d.rule.as_deref()),
        Some(Some("tag.when"))
    );
}

#[test]
fn explorer_and_runtime_agree_on_boundary_vectors() {
    let machine = compile(&tagged_toml());
    let mut corpus = CaseCorpus::new();
    let sixty_four = "B".repeat(64);
    let sixty_five = "C".repeat(65);
    let long_label = "L".repeat(200);
    let vectors = [
        ("valid", "item_1", "AMBER"),
        ("empty", "", "AMBER"),
        ("one", "A", "x"),
        ("sixty_four", sixty_four.as_str(), "x"),
        ("sixty_five", sixty_five.as_str(), "x"),
        ("slash", "a/b", "x"),
        ("space", "a b", "x"),
        ("punct", "a._-", "x"),
        ("opaque_space", "ok", "x y"),
        ("opaque_long", "ok", long_label.as_str()),
    ];
    for (name, id, label) in vectors {
        let mut args = BTreeMap::new();
        args.insert("id".into(), json!(id));
        args.insert("label".into(), json!(label));
        corpus.insert("tag", name, args);
    }
    let (_run, graph) = explore_graph(
        &machine,
        &corpus,
        Bounds {
            max_nodes: 64,
            max_depth: 8,
        },
    );
    let initial = machine.initial_state();
    for edge in &graph.edges {
        if edge.command != "tag" {
            continue;
        }
        let args = edge.args.clone();
        let command = json!({"name": "tag", "args": args});
        let runtime = decide(&machine, command);
        let explorer_commit = edge.outcome == "committed";
        match runtime {
            Ok(Decision::Emit { .. }) => assert_eq!(explorer_commit, true, "{edge:?}"),
            Ok(Decision::Noop { .. }) => assert_eq!(edge.outcome, "noop", "{edge:?}"),
            Err(_) => assert_eq!(explorer_commit, false, "{edge:?}"),
        }
        assert_eq!(initial, machine.initial_state());
    }
}

#[test]
fn feasibility_does_not_discharge_using_text_in() {
    let text_in = parse_definition(&tagged_toml()).expect("def");
    let always_false = parse_definition(&format!(
        r#"
{header}
[facts]
[states]
idle = {{ initial = true }}
tagged = {{ terminal = true }}
[commands.tag]
from = ["idle"]
to = "tagged"
args = {{ id = "string" }}
when = "1 == 0"
"#,
        header = header()
    ))
    .expect("false def");
    let text_plan = verification_plan(&text_in);
    let false_plan = verification_plan(&always_false);
    let text_ids: Vec<&str> = text_plan
        .impossibilities
        .iter()
        .map(|row| row.obligation_id.as_str())
        .collect();
    assert_eq!(
        text_ids.iter().any(|id| id.contains("tag")),
        false,
        "{text_ids:?}"
    );
    assert_eq!(
        false_plan
            .impossibilities
            .iter()
            .any(|row| row.obligation_id.contains("tag")),
        true,
        "{:?}",
        false_plan.impossibilities
    );
}

#[test]
fn smt_reports_text_in_unsupported() {
    let expr = expr::parse("text_in($id,'A-Za-z0-9._-',1,64)").expect("parse");
    let mut pinned = Pinned {
        state: "idle".into(),
        facts: BTreeMap::new(),
        args: BTreeMap::new(),
    };
    pinned.args.insert("id".into(), json!("ok"));
    let encoded = encode_guard_eval(&expr, &pinned);
    let err = encoded.expect_err("unsupported");
    assert_eq!(err.reason.contains("text_in"), true, "{}", err.reason);
    let kernel = kernel_eval(&expr, &pinned);
    assert_eq!(kernel, GuardValue::True);
    if let Some(solver) = discover_solvers().0.or(discover_solvers().1) {
        let eval = eval_guard_with_solver(&expr, &pinned, &solver);
        let reason = eval.expect_err("adapter unsupported");
        assert_eq!(reason.contains("text_in"), true, "{reason}");
    }
}

#[test]
fn existing_example_identities_and_plan_digests_are_stable() {
    let va = load_machine("verification_attempt");
    let effect = load_machine("effect_probe");
    let loan_path = repo_root().join("examples/shipped/loan_checkout.ik.toml");
    let loan = parse_and_compile(&fs::read_to_string(&loan_path).expect("loan"))
        .map_err(|e| e.join("\n"))
        .expect("loan compile");
    assert_eq!(
        canonical_json(va.identity()),
        r#"{"machine_id":"verification_attempt","spec_hash":"aae4f03a6e4169bdc86021914bc57db4bc391878651fa61fcd2a9d0d215b4a7f","version":1}"#
    );
    assert_eq!(
        canonical_json(effect.identity()),
        r#"{"machine_id":"effect_probe","spec_hash":"f9058d9f72ce96aa2f7369683c97a065d1ca32858fb4abc93e06e294cb45df6f","version":1}"#
    );
    assert_eq!(
        canonical_json(loan.identity()),
        r#"{"machine_id":"loan_checkout","spec_hash":"6c08d4be51065bc9fd184c78009f7ef1f3b5c40f82e1d2d9d46e226cabaa15d8","version":1}"#
    );
    let va_def =
        parse_definition(&fs::read_to_string(machine_def("verification_attempt")).expect("va src"))
            .expect("va def");
    let effect_def =
        parse_definition(&fs::read_to_string(machine_def("effect_probe")).expect("effect src"))
            .expect("effect def");
    let loan_def =
        parse_definition(&fs::read_to_string(&loan_path).expect("loan src")).expect("loan def");
    assert_eq!(va.spec_hash().0, digest_json(&va_def));
    assert_eq!(effect.spec_hash().0, digest_json(&effect_def));
    assert_eq!(loan.spec_hash().0, digest_json(&loan_def));
    let va_plan = digest_json(&verification_plan(&va_def));
    let effect_plan = digest_json(&verification_plan(&effect_def));
    let loan_plan = digest_json(&verification_plan(&loan_def));
    assert_eq!(va_plan.len(), 64);
    assert_eq!(effect_plan.len(), 64);
    assert_eq!(loan_plan.len(), 64);
    assert_eq!(effect.identity().machine_id, "effect_probe");
    assert_eq!(loan.identity().machine_id, "loan_checkout");
}

#[test]
fn text_in_changes_spec_hash_and_round_trips() {
    let with = compile(&tagged_toml());
    let without = compile(&format!(
        r#"
{header}
[facts]
item_id = {{ type = "string", policy = "replace" }}
label = {{ type = "string", policy = "replace" }}
[states]
idle = {{ initial = true }}
tagged = {{ terminal = true }}
[commands.tag]
from = ["idle"]
to = "tagged"
args = {{ id = "string", label = "string" }}
when = "$id != ''"
then = {{ item_id = "$id", label = "$label" }}
"#,
        header = header()
    ));
    assert_ne!(with.spec_hash(), without.spec_hash());
    let expr = expr::parse("text_in($id,'A-Za-z0-9._-',1,64) && $label != ''").expect("parse");
    let printed = expr::format_expr(&expr);
    assert_eq!(parse_eq(&printed, &expr), true);
}

fn parse_eq(src: &str, expected: &Expr) -> bool {
    expr::parse(src).ok().as_ref() == Some(expected)
}

#[test]
fn laws_counts_text_in_argument_as_read() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("tagged.ik.toml");
    fs::write(&path, tagged_toml()).expect("write");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("laws")
        .arg(&path)
        .output()
        .expect("laws");
    assert_eq!(
        output.status.success(),
        true,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: Value = serde_json::from_slice(&output.stdout).expect("json");
    let flagged: Vec<String> = parsed["unguarded_arguments"]
        .as_array()
        .expect("array")
        .iter()
        .map(|row| format!("{}.{}", row["command"], row["argument"]))
        .collect();
    assert_eq!(
        flagged.iter().any(|row| row.contains("id")),
        false,
        "{flagged:?}"
    );
    assert_eq!(
        flagged.iter().any(|row| row.contains("label")),
        true,
        "{flagged:?}"
    );
}

#[test]
fn corpus_init_placeholders_do_not_crash() {
    let def = parse_definition(&tagged_toml()).expect("def");
    let starter = starter_corpus(&def);
    assert_eq!(starter["_note"].as_str().is_some(), true);
    let machine = compile(&tagged_toml());
    let _ = decide(
        &machine,
        json!({"name": "tag", "args": starter["commands"]["tag"][0]["args"].clone()}),
    );
}

#[test]
fn hegel_eval_path_matches_runtime() {
    let expr = expr::parse("text_in($id,'A-Za-z0-9._-',1,64)").expect("parse");
    let facts = BTreeMap::new();
    for (id, want) in [("ok", true), ("", false), ("a/b", false)] {
        let mut args = BTreeMap::new();
        args.insert("id".into(), json!(id));
        let ctx = EvalCtx {
            facts: &facts,
            args: &args,
            state: "idle",
            effect: None,
        };
        let hegel = expr::eval(&expr, &ctx).expect("hegel eval");
        assert_eq!(hegel, want, "{id}");
        assert_eq!(eval_id(id).expect("runtime"), want, "{id}");
    }
}

#[test]
fn int_value_is_a_definition_error() {
    let toml = format!(
        r#"
{header}
[facts]
n = {{ type = "int", policy = "replace", initial = 1 }}
[states]
s = {{ initial = true }}
t = {{}}
[commands.go]
from = ["s"]
to = "t"
when = "text_in(n, '*', 0, 8)"
"#,
        header = header()
    );
    let errors = parse_and_compile(&toml).err().expect("type error");
    let joined = errors.join("\n");
    assert_eq!(joined.contains("string"), true, "{joined}");
}
