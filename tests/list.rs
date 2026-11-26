//! Read-only aggregate query tests for `ik list`.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::identity::{AggregateId, Capability};
use ik::machine::Machine;
use ik::store::{CommandRequest, EffectOutcomeRequest, Store};
use serde_json::{json, Value};
use std::process::Command;

fn run_ik(args: &[&str]) -> (String, String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_ik"))
        .args(args)
        .output()
        .expect("spawn ik");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn cap(
    identity: &ik::identity::MachineIdentity,
    agg: &AggregateId,
    generation: u64,
) -> Capability {
    Capability {
        generation,
        nonce: ik::runtime::derive_nonce(identity, agg, generation),
    }
}

#[test]
fn list_filters_by_machine_state_effect_terminal_and_paginates() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store_path = dir.path().join(".state").join("mixed.db");

    let orders_toml = include_str!("../examples/orders/order.ik.toml");
    let orders = ik::declarative::parse_and_compile(orders_toml).expect("compile order");
    let orders_id = orders.identity().clone();

    let fx_machine = load_machine("effect_probe");
    let fx_id = fx_machine.identity().clone();

    {
        let mut store = Store::open_or_create(&store_path).expect("create store");

        // ord-1: paid (non-terminal)
        let o1 = AggregateId("ord-1".into());
        let a1 = store
            .create_aggregate(&orders, &o1, &orders_id)
            .expect("ord-1");
        store
            .command(
                &orders,
                &CommandRequest {
                    aggregate_id: o1,
                    machine: orders_id.clone(),
                    capability: a1.capability,
                    command: json!({"name": "pay", "args": {"payment_ref": "ch_1"}}),
                    observation: None,
                },
            )
            .expect("pay ord-1");

        // ord-2: cancelled (terminal)
        let o2 = AggregateId("ord-2".into());
        let a2 = store
            .create_aggregate(&orders, &o2, &orders_id)
            .expect("ord-2");
        store
            .command(
                &orders,
                &CommandRequest {
                    aggregate_id: o2,
                    machine: orders_id.clone(),
                    capability: a2.capability,
                    command: json!({"name": "cancel", "args": {}}),
                    observation: None,
                },
            )
            .expect("cancel ord-2");

        // ord-3: cart (non-terminal)
        store
            .create_aggregate(&orders, &AggregateId("ord-3".into()), &orders_id)
            .expect("ord-3");

        // fx-pending: pending effect
        let fp = AggregateId("fx-pending".into());
        let ap = store
            .create_aggregate(&fx_machine, &fp, &fx_id)
            .expect("fx-pending");
        store
            .command(
                &fx_machine,
                &CommandRequest {
                    aggregate_id: fp,
                    machine: fx_id.clone(),
                    capability: ap.capability,
                    command: json!({"name": "plan", "args": {"effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1"}}),
                    observation: None,
                },
            )
            .expect("plan fx-pending");

        // fx-recorded: recorded effect
        let fr = AggregateId("fx-recorded".into());
        let ar = store
            .create_aggregate(&fx_machine, &fr, &fx_id)
            .expect("fx-recorded");
        let planned = store
            .command(
                &fx_machine,
                &CommandRequest {
                    aggregate_id: fr.clone(),
                    machine: fx_id.clone(),
                    capability: ar.capability,
                    command: json!({"name": "plan", "args": {"effect_id": "e2", "payload_digest": "p2", "idempotency_key": "k2"}}),
                    observation: None,
                },
            )
            .expect("plan fx-recorded");
        let intent = planned.effect.pending_intent().expect("pending intent").clone();
        store
            .record_effect(&EffectOutcomeRequest {
                aggregate_id: fr.clone(),
                machine: fx_id.clone(),
                capability: cap(&fx_id, &fr, planned.generation),
                effect: ik::effect::EffectOutcomeV1 {
                    effect_id: intent.effect_id,
                    aggregate_id: intent.aggregate_id,
                    planned_generation: intent.generation,
                    payload_digest: intent.payload_digest,
                    idempotency_key: intent.idempotency_key,
                    outcome_digest: "out-2".into(),
                },
            })
            .expect("record fx-recorded");
    }

    let store_s = store_path.to_str().unwrap();

    // Filter by machine + keyset pagination.
    let (out, err, code) = run_ik(&["list", store_s, "--machine", "order", "--limit", "2", "--json"]);
    assert_eq!(code, Some(0), "stderr: {err}");
    let p1: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(p1["schema"], "ik.list.v1");
    assert_eq!(p1["has_more"], true);
    assert_eq!(p1["next_after"], "ord-2");
    assert_eq!(p1["aggregates"].as_array().unwrap().len(), 2);

    let (out, err, code) = run_ik(&[
        "list",
        store_s,
        "--machine",
        "order",
        "--after",
        "ord-2",
        "--limit",
        "2",
        "--json",
    ]);
    assert_eq!(code, Some(0), "stderr: {err}");
    let p2: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(p2["has_more"], false);
    assert_eq!(p2["aggregates"][0]["aggregate_id"], "ord-3");

    // Filter by state.
    let (out, err, code) = run_ik(&["list", store_s, "--state", "paid", "--json"]);
    assert_eq!(code, Some(0), "stderr: {err}");
    let paid: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(paid["aggregates"].as_array().unwrap().len(), 1);
    assert_eq!(paid["aggregates"][0]["aggregate_id"], "ord-1");
    assert_eq!(paid["aggregates"][0]["state"]["payment_ref"], "ch_1");

    // Filter by effect=pending and effect=recorded.
    let (out, err, code) = run_ik(&["list", store_s, "--effect", "pending", "--json"]);
    assert_eq!(code, Some(0), "stderr: {err}");
    let pend: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(pend["aggregates"].as_array().unwrap().len(), 1);
    assert_eq!(pend["aggregates"][0]["aggregate_id"], "fx-pending");
    assert_eq!(pend["aggregates"][0]["effect_status"], "pending");

    let (out, err, code) = run_ik(&["list", store_s, "--effect", "recorded", "--json"]);
    assert_eq!(code, Some(0), "stderr: {err}");
    let rec: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(rec["aggregates"].as_array().unwrap().len(), 1);
    assert_eq!(rec["aggregates"][0]["aggregate_id"], "fx-recorded");

    // Filter by terminal=true.
    let (out, err, code) = run_ik(&["list", store_s, "--terminal", "true", "--json"]);
    assert_eq!(code, Some(0), "stderr: {err}");
    let term: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(term["aggregates"].as_array().unwrap().len(), 1);
    assert_eq!(term["aggregates"][0]["aggregate_id"], "ord-2");

    // Text table format works cleanly.
    let (txt, err, code) = run_ik(&["list", store_s, "--effect", "pending"]);
    assert_eq!(code, Some(0), "stderr: {err}");
    assert!(txt.contains("fx-pending"), "txt:\n{txt}");
}

#[test]
fn list_on_missing_store_creates_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let absent = dir.path().join("missing-parent").join("store.db");
    let (_, err, code) = run_ik(&["list", absent.to_str().unwrap(), "--json"]);
    assert_eq!(code, Some(2), "stderr: {err}");
    assert!(!absent.exists());
    assert!(!absent.parent().unwrap().exists());
}

#[test]
fn store_uses_synchronous_full_by_default_and_secondary_indexes_for_list() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("indexed.db");
    let store = Store::open_or_create(&db).expect("open_or_create");
    drop(store);

    let conn = rusqlite::Connection::open(&db).expect("open sqlite");
    let plan: String = conn
        .query_row(
            "EXPLAIN QUERY PLAN SELECT aggregate_id FROM aggregates WHERE machine_id = 'order' AND terminal = 0 AND aggregate_id > '' ORDER BY aggregate_id ASC",
            [],
            |row| row.get(3),
        )
        .expect("query plan");
    assert!(
        plan.contains("idx_aggregates_machine_terminal_id"),
        "expected index scan in query plan, got: {plan}"
    );

    // Default `Store::open` / `Store::create` enforces PRAGMA synchronous = FULL (2) unless overridden.
    let (out, err, code) = run_ik(&["store", "verify", db.to_str().unwrap(), "--json"]);
    assert_eq!(code, Some(0), "verify failed: {out} {err}");
}
