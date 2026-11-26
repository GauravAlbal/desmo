//! Tier A3 integration tests: declarative effect-outcome guards (`effect_status`,
//! `effect_kind`, `effect_outcome`) enforced at runtime, explored by `ik check`,
//! rejected in `[invariants]`, and rendered in `ik simulate`.

use ik::declarative::{parse_and_compile, parse_definition};
use ik::effect::EffectOutcomeV1;
use ik::identity::AggregateId;
use ik::machine::Machine;
use ik::store::{CommandRequest, EffectOutcomeRequest, Store};
use serde_json::{json, Value};
use std::process::Command;

const PAYMENT_MACHINE: &str = r#"
schema_version = 1
machine_id = "payment_effect"
machine_version = 1

[facts]
amount = { type = "int", policy = "replace" }

[states]
created = { initial = true }
charging = {}
paid = { terminal = true }
failed = { terminal = true }

[commands.charge]
from = ["created"]
to = "charging"
args = { amount = "int", charge_id = "string" }
when = "$amount > 0"
then = { amount = "$amount" }
effect = { id = "$charge_id", kind = "stripe.charge", payload_digest = "$charge_id", idempotency_key = "$charge_id" }

[commands.complete]
from = ["charging"]
to = "paid"
when = "effect_status == 'recorded' && effect_kind == 'stripe.charge' && effect_outcome == 'succeeded'"

[commands.mark_failed]
from = ["charging"]
to = "failed"
when = "effect_status == 'recorded' && effect_outcome == 'failed'"
"#;

#[test]
fn runtime_enforces_effect_outcome_guards_end_to_end() {
    let machine = parse_and_compile(PAYMENT_MACHINE).expect("compile payment machine");
    let id = machine.identity().clone();
    let db = std::env::temp_dir().join(format!(
        "desmo-tier-a3-effect-guards-{}.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&db);
    let mut store = Store::open_or_create(&db).expect("open store");

    let created = store
        .create_aggregate(&machine, &AggregateId("pay-1".into()), &id)
        .expect("create");
    let charging = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: AggregateId("pay-1".into()),
                machine: id.clone(),
                capability: created.capability,
                command: json!({ "name": "charge", "args": { "amount": 500, "charge_id": "ch_1" } }),
                observation: None,
            },
        )
        .expect("charge plans effect");
    let intent = charging.effect.pending_intent().expect("pending intent").clone();

    // Calling `complete` while effect is still pending must refuse via `effect_status == 'recorded'` guard.
    let pending_err = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: AggregateId("pay-1".into()),
                machine: id.clone(),
                capability: charging.capability.clone(),
                command: json!({ "name": "complete", "args": {} }),
                observation: None,
            },
        )
        .expect_err("must refuse while effect pending");
    assert_eq!(pending_err.code.as_str(), "IK_INVARIANT_FAILED");

    // Record `failed` outcome -> `complete` MUST refuse (wrong effect_outcome), `mark_failed` succeeds!
    let recorded_fail = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: AggregateId("pay-1".into()),
            machine: id.clone(),
            capability: charging.capability,
            effect: EffectOutcomeV1 {
                effect_id: intent.effect_id,
                aggregate_id: intent.aggregate_id,
                planned_generation: intent.generation,
                payload_digest: intent.payload_digest,
                idempotency_key: intent.idempotency_key,
                outcome_digest: "failed".into(),
            },
        })
        .expect("record failed");

    let wrong_outcome_err = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: AggregateId("pay-1".into()),
                machine: id.clone(),
                capability: recorded_fail.capability.clone(),
                command: json!({ "name": "complete", "args": {} }),
                observation: None,
            },
        )
        .expect_err("complete must refuse when effect_outcome == 'failed'");
    assert_eq!(wrong_outcome_err.code.as_str(), "IK_INVARIANT_FAILED");

    let failed = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: AggregateId("pay-1".into()),
                machine: id.clone(),
                capability: recorded_fail.capability,
                command: json!({ "name": "mark_failed", "args": {} }),
                observation: None,
            },
        )
        .expect("mark_failed commits on effect_outcome == 'failed'");
    assert_eq!(failed.state["state"], "failed");

    // Second aggregate records `succeeded` -> `complete` commits to `paid`.
    let c2 = store
        .create_aggregate(&machine, &AggregateId("pay-2".into()), &id)
        .expect("create pay-2");
    let ch2 = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: AggregateId("pay-2".into()),
                machine: id.clone(),
                capability: c2.capability,
                command: json!({ "name": "charge", "args": { "amount": 500, "charge_id": "ch_2" } }),
                observation: None,
            },
        )
        .expect("charge 2");
    let i2 = ch2.effect.pending_intent().expect("pending intent 2").clone();
    let rec2 = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: AggregateId("pay-2".into()),
            machine: id.clone(),
            capability: ch2.capability,
            effect: EffectOutcomeV1 {
                effect_id: i2.effect_id,
                aggregate_id: i2.aggregate_id,
                planned_generation: i2.generation,
                payload_digest: i2.payload_digest,
                idempotency_key: i2.idempotency_key,
                outcome_digest: "succeeded".into(),
            },
        })
        .expect("record succeeded");
    let paid = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: AggregateId("pay-2".into()),
                machine: id,
                capability: rec2.capability,
                command: json!({ "name": "complete", "args": {} }),
                observation: None,
            },
        )
        .expect("complete commits on effect_outcome == 'succeeded'");
    assert_eq!(paid.state["state"], "paid");
    let _ = std::fs::remove_file(&db);
}

#[test]
fn effect_operands_are_rejected_in_invariants_and_as_fact_names() {
    let bad_inv = r#"
schema_version = 1
machine_id = "bad_inv"
machine_version = 1
[facts]
[states]
a = { initial = true }
b = { terminal = true }
[commands.go]
from = ["a"]
to = "b"
[invariants.no_fx]
assert = "effect_status == 'none'"
"#;
    let errs = parse_definition(bad_inv).expect_err("must reject effect operand in invariant");
    assert!(
        errs.iter().any(|e| e.contains("effect_status")),
        "errors={errs:?}"
    );

    let bad_fact = r#"
schema_version = 1
machine_id = "bad_fact"
machine_version = 1
[facts]
effect_outcome = { type = "string", policy = "replace" }
[states]
a = { initial = true }
b = { terminal = true }
[commands.go]
from = ["a"]
to = "b"
"#;
    let errs2 = parse_definition(bad_fact).expect_err("must reject reserved fact name");
    assert!(
        errs2.iter().any(|e| e.contains("reserved word")),
        "errors={errs2:?}"
    );
}

#[test]
fn ik_check_auto_seed_explores_through_effect_outcomes() {
    let tmp = std::env::temp_dir().join("desmo-tier-a3-check-fx");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let path = tmp.join("payment.ik.toml");
    std::fs::write(&path, PAYMENT_MACHINE).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ik"))
        .args(["check", path.to_str().unwrap(), "--auto-seed", "--compact"])
        .output()
        .expect("spawn ik");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let v: Value = serde_json::from_str(&stdout).expect("valid compact json");
    assert_eq!(v["verdict"], "ACCEPT", "compact={v}");
    assert_eq!(v["status"], "accepted", "compact={v}");
    assert_eq!(v["assurance_mode"], "auto_seeded_bounded", "compact={v}");
    let _ = std::fs::remove_dir_all(&tmp);
}
