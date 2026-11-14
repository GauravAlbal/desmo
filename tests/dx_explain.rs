//! DX-02: shared explanation seam, parity, stability, no store writes.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::canonical::canonical_json;
use ik::declarative::{explain, parse_and_compile, GuardEvaluation};
use ik::effect::EffectStatus;
use ik::machine::{Decision, Machine};
use ik::protocol::Op;
use serde_json::json;

fn fold(
    machine: &dyn Machine,
    state: &serde_json::Value,
    command: &serde_json::Value,
) -> Result<(serde_json::Value, bool, usize), ik::diagnostic::Diagnostic> {
    let decision = machine.decide(state, None, &EffectStatus::None, command)?;
    let Decision::Emit { event, .. } = decision else {
        panic!("expected emit");
    };
    let index = event.payload["outcome"].as_u64().unwrap() as usize;
    let applied = machine.apply(state, &event)?;
    Ok((applied.state, applied.terminal, index))
}

fn submitted(machine: &dyn Machine) -> serde_json::Value {
    let running = fold(machine, &machine.initial_state(), &json!({"name": "start"}))
        .expect("start")
        .0;
    fold(
        machine,
        &running,
        &json!({"name": "submit", "args": {"artifact_digest": "a1", "baseline_digest": "b1"}}),
    )
    .expect("submit")
    .0
}

#[test]
fn reject_explanation_exposes_both_candidates() {
    let machine = load_machine("verification_attempt");
    let state = submitted(&machine);
    let explanation = explain(&machine, &state, &json!({"name": "reject"}));
    assert_eq!(explanation.verdict, "would_commit");
    let decision = explanation.decision.expect("decision");
    assert_eq!(decision.source_state, "submitted");
    assert!(decision.source_legal);
    assert_eq!(decision.candidates.len(), 2);
    assert_eq!(decision.candidates[0].evaluation, GuardEvaluation::True);
    assert!(decision.candidates[0].selected);
    assert_eq!(
        decision.candidates[1].evaluation,
        GuardEvaluation::NotConsidered
    );
    assert_eq!(decision.selected, Some(0));
    assert_eq!(explanation.post_state.unwrap()["state"], "repairable");
    assert_eq!(explanation.terminal, Some(false));
}

#[test]
fn verify_explanation_distinguishes_match_mismatch_and_eval_error() {
    let machine = load_machine("verification_attempt");
    let state = submitted(&machine);
    let match_cmd = json!({"name": "verify", "args": {
        "receipt_digest": "r1",
        "receipt_artifact_digest": "a1",
        "receipt_baseline_digest": "b1"
    }});
    let matched = explain(&machine, &state, &match_cmd);
    assert_eq!(matched.verdict, "would_commit");
    assert_eq!(matched.post_state.unwrap()["state"], "verified");

    let mismatch_cmd = json!({"name": "verify", "args": {
        "receipt_digest": "r2",
        "receipt_artifact_digest": "WRONG",
        "receipt_baseline_digest": "b1"
    }});
    let mismatched = explain(&machine, &state, &mismatch_cmd);
    assert_eq!(mismatched.verdict, "refused");
    let decision = mismatched.decision.expect("decision");
    assert_eq!(decision.candidates[0].evaluation, GuardEvaluation::False);
    assert!(mismatched.diagnostic.is_some());

    let probe = parse_and_compile(
        r#"
schema_version = 1
machine_id = "probe"
machine_version = 1
[facts]
artifact_digest = { type = "string", policy = "replace" }
[states]
s = { initial = true }
t = {}
[commands.check]
from = ["s"]
to = "t"
when = "artifact_digest == 'a1'"
"#,
    )
    .expect("probe");
    let missing = explain(&probe, &probe.initial_state(), &json!({"name": "check"}));
    assert_eq!(missing.verdict, "refused");
    assert_eq!(
        missing.decision.unwrap().candidates[0].evaluation,
        GuardEvaluation::Error
    );
}

#[test]
fn explanation_matches_authoritative_decide_apply() {
    let machine = load_machine("verification_attempt");
    let mut state = machine.initial_state();
    let corpus = [
        json!({"name": "start"}),
        json!({"name": "submit", "args": {"artifact_digest": "a1", "baseline_digest": "b1"}}),
        json!({"name": "verify", "args": {
            "receipt_digest": "r1",
            "receipt_artifact_digest": "a1",
            "receipt_baseline_digest": "b1"
        }}),
        json!({"name": "reject"}),
        json!({"name": "cancel"}),
        json!({"name": "no_such_command"}),
    ];
    for command in &corpus {
        let explanation = explain(&machine, &state, command);
        match machine.decide(&state, None, &EffectStatus::None, command) {
            Err(diagnostic) => {
                assert_eq!(explanation.verdict, "refused");
                let explained = explanation.diagnostic.expect("diagnostic");
                assert_eq!(explained.code, diagnostic.code);
                assert_eq!(explained.rule, diagnostic.rule);
            }
            Ok(Decision::Emit { event, .. }) => match machine.apply(&state, &event) {
                Ok(applied) => {
                    assert_eq!(explanation.verdict, "would_commit");
                    assert_eq!(explanation.event.as_ref(), Some(&event));
                    assert_eq!(explanation.post_state.as_ref(), Some(&applied.state));
                    assert_eq!(explanation.terminal, Some(applied.terminal));
                    assert_eq!(explanation.effect, applied.effect);
                    if !applied.terminal {
                        state = applied.state;
                    }
                }
                Err(diagnostic) => {
                    assert_eq!(explanation.verdict, "refused");
                    let explained = explanation.diagnostic.expect("diagnostic");
                    assert_eq!(explained.code, diagnostic.code);
                    assert_eq!(explained.rule, diagnostic.rule);
                }
            },
            Ok(Decision::Noop { .. }) => panic!("declarative noop"),
        }
    }
}

#[test]
fn explanation_is_byte_stable() {
    let machine = load_machine("verification_attempt");
    let state = submitted(&machine);
    let command = json!({"name": "reject"});
    let a = canonical_json(&explain(&machine, &state, &command));
    let b = canonical_json(&explain(&machine, &state, &command));
    assert_eq!(a, b);
}

#[test]
fn explanation_has_no_store_dependency() {
    let machine = load_machine("verification_attempt");
    let explanation = explain(
        &machine,
        &machine.initial_state(),
        &json!({"name": "start"}),
    );
    assert_eq!(explanation.verdict, "would_commit");
    assert_eq!(explanation.post_state.unwrap()["state"], "running");
    assert!(explanation.diagnostic.is_none());
}

#[test]
fn production_protocol_ops_are_the_declared_nine() {
    let names = [
        Op::Create,
        Op::Command,
        Op::Record,
        Op::Show,
        Op::Replay,
        Op::Verify,
        Op::Transaction,
        Op::LookupOperation,
        Op::CreateWithCommand,
    ]
    .iter()
    .map(|op| match op {
        Op::Create => "create",
        Op::Command => "command",
        Op::Record => "record",
        Op::Show => "show",
        Op::Replay => "replay",
        Op::Verify => "verify",
        Op::Transaction => "transaction",
        Op::LookupOperation => "lookup_operation",
        Op::CreateWithCommand => "create_with_command",
    })
    .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "create",
            "command",
            "record",
            "show",
            "replay",
            "verify",
            "transaction",
            "lookup_operation",
            "create_with_command"
        ]
    );
}
