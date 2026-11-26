//! Tier B1 integration tests: `ik events` ordered projection cursor and `ik guard` anti-bypass linter.

use ik::declarative::parse_and_compile;
use ik::identity::AggregateId;
use ik::machine::Machine;
use ik::store::{CommandRequest, Store};
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

#[test]
fn ik_events_streams_ordered_cursor_pages_and_latest_projection() {
    let dir = std::env::temp_dir().join("desmo-tier-b1-events");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("orders.sqlite");

    let order_toml = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/orders/order.ik.toml"),
    )
    .unwrap();
    let machine = parse_and_compile(&order_toml).unwrap();
    let id = machine.identity().clone();

    let mut store = Store::open_or_create(&db).unwrap();
    let o1 = store
        .create_aggregate(&machine, &AggregateId("ord-1".into()), &id)
        .unwrap();
    let _o1_paid = store
        .command(
            &machine,
            &CommandRequest {
                aggregate_id: AggregateId("ord-1".into()),
                machine: id.clone(),
                capability: o1.capability,
                command: json!({ "name": "pay", "args": { "payment_ref": "ch_1" } }),
                observation: None,
            },
        )
        .unwrap();
    let _o2 = store
        .create_aggregate(&machine, &AggregateId("ord-2".into()), &id)
        .unwrap();
    drop(store);

    let (page1, err1, code1) = run_ik(&[
        "events",
        db.to_str().unwrap(),
        "--since",
        "0",
        "--limit",
        "2",
        "--json",
    ]);
    assert_eq!(code1, Some(0), "stderr={err1}");
    let v1: Value = serde_json::from_str(&page1).unwrap();
    assert_eq!(v1["schema"], "ik.events.v1");
    assert_eq!(v1["has_more"], true);
    let events1 = v1["events"].as_array().unwrap();
    assert_eq!(events1.len(), 2);
    assert_eq!(events1[0]["event_type"], "genesis");
    assert_eq!(events1[0]["is_latest_for_aggregate"], false);
    assert_eq!(events1[1]["event_type"], "pay");
    assert_eq!(events1[1]["is_latest_for_aggregate"], true);
    assert_eq!(events1[1]["current_projection"]["state"]["state"], "paid");

    let next_cursor = v1["next_cursor"].as_u64().unwrap().to_string();
    let (page2, err2, code2) = run_ik(&[
        "events",
        db.to_str().unwrap(),
        "--since",
        &next_cursor,
        "--json",
    ]);
    assert_eq!(code2, Some(0), "stderr={err2}");
    let v2: Value = serde_json::from_str(&page2).unwrap();
    assert_eq!(v2["has_more"], false);
    let events2 = v2["events"].as_array().unwrap();
    assert_eq!(events2.len(), 1);
    assert_eq!(events2[0]["aggregate_id"], "ord-2");
    assert_eq!(events2[0]["is_latest_for_aggregate"], true);

    // Filtered aggregate query.
    let (agg_out, agg_err, agg_code) = run_ik(&[
        "events",
        db.to_str().unwrap(),
        "--aggregate",
        "ord-1",
        "--json",
    ]);
    assert_eq!(agg_code, Some(0), "stderr={agg_err}");
    let va: Value = serde_json::from_str(&agg_out).unwrap();
    assert_eq!(va["events"].as_array().unwrap().len(), 2);

    // Missing store refuses without creating anything.
    let missing = dir.join("missing.sqlite");
    let (_, _, missing_code) = run_ik(&["events", missing.to_str().unwrap()]);
    assert_eq!(missing_code, Some(2));
    assert!(!missing.exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ik_guard_catches_bypass_ignores_inline_comments_and_respects_allow_paths() {
    let dir = std::env::temp_dir().join("desmo-tier-b1-guard");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src/projections")).unwrap();
    std::fs::create_dir_all(dir.join("src/handlers")).unwrap();

    let cfg = dir.join(".ik/guard.toml");
    let (_, init_err, init_code) = run_ik(&["guard", "init", "--out", cfg.to_str().unwrap()]);
    assert_eq!(init_code, Some(0), "stderr={init_err}");
    assert!(cfg.is_file());

    // Refuses overwrite without --force.
    let (_, dup_err, dup_code) = run_ik(&["guard", "init", "--out", cfg.to_str().unwrap()]);
    assert_eq!(dup_code, Some(2), "stderr={dup_err}");

    // Authorized projection writer + test writer stay green.
    std::fs::write(
        dir.join("src/projections/orders.ts"),
        "db.exec(`UPDATE orders SET status = 'paid'`);\n",
    )
    .unwrap();
    let (ok_out, ok_err, ok_code) = run_ik(&[
        "guard",
        "check",
        "--config",
        cfg.to_str().unwrap(),
        "--dir",
        dir.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(ok_code, Some(0), "out={ok_out}\nerr={ok_err}");
    let ok_json: Value = serde_json::from_str(&ok_out).unwrap();
    assert_eq!(ok_json["ok"], true);
    assert_eq!(ok_json["violations"].as_array().unwrap().len(), 0);

    // Direct status mutation outside allow_paths fails even with a fake inline ignore comment,
    // and multi-line SQL + ORM mutations are caught while prose comments do not false-positive.
    std::fs::write(
        dir.join("src/handlers/comments_only.ts"),
        "// Legacy note: previously order.status = 'paid'; and UPDATE orders SET status = 'paid'\nconst ok = true;\n",
    )
    .unwrap();
    let (comments_out, _, comments_code) = run_ik(&[
        "guard",
        "check",
        "--config",
        cfg.to_str().unwrap(),
        "--dir",
        dir.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(comments_code, Some(0), "prose comments must not fail: {comments_out}");

    std::fs::write(
        dir.join("src/handlers/webhook.ts"),
        "// ik-guard-ignore\norder.status = 'refunded'; // ik-guard-ignore\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/handlers/multiline.sql"),
        "update orders\n  set status = 'paid'\n  where id = 1;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/handlers/orm.ts"),
        "await db.order.update({ data: {\n  status: 'paid'\n} });\n",
    )
    .unwrap();
    let (bad_out, bad_err, bad_code) = run_ik(&[
        "guard",
        "check",
        "--config",
        cfg.to_str().unwrap(),
        "--dir",
        dir.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(bad_code, Some(1), "out={bad_out}\nerr={bad_err}");
    let bad_json: Value = serde_json::from_str(&bad_out).unwrap();
    assert_eq!(bad_json["ok"], false);
    let violations = bad_json["violations"].as_array().unwrap();
    assert!(violations.iter().any(|v| v["file"] == "src/handlers/webhook.ts" && v["line"] == 2), "{violations:?}");
    assert!(violations.iter().any(|v| v["file"] == "src/handlers/multiline.sql" && v["line"] == 1), "{violations:?}");
    assert!(violations.iter().any(|v| v["file"] == "src/handlers/orm.ts" && v["line"] == 2), "{violations:?}");

    // Machine-derived `ik guard init --machine ... --table ...` works cleanly.
    let gen_cfg = dir.join(".ik/generated-guard.toml");
    let order_toml = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/orders/order.ik.toml");
    let (_, gen_err, gen_code) = run_ik(&[
        "guard",
        "init",
        "--machine",
        order_toml,
        "--table",
        "customer_orders",
        "--out",
        gen_cfg.to_str().unwrap(),
    ]);
    assert_eq!(gen_code, Some(0), "gen stderr={gen_err}");
    let gen_txt = std::fs::read_to_string(&gen_cfg).unwrap();
    assert!(gen_txt.contains("UPDATE customer_orders SET *status*"), "{gen_txt}");
    assert!(gen_txt.contains("delivered"), "{gen_txt}");

    let _ = std::fs::remove_dir_all(&dir);
}
