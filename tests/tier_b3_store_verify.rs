use ik::declarative::compile::compile;
use ik::declarative::parse_and_compile;
use ik::declarative::region::{parse_source_with_limits, RegionLimits};
use ik::identity::AggregateId;
use ik::machine::Machine;
use ik::operation::OperationId;
use ik::store::{CommandRequest, Store};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::Command;

fn ik_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ik")
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "desmo-tier-b3-{name}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("mkdir");
    dir
}

fn write_region_machine(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("deploy.ik.toml");
    fs::write(
        &path,
        r#"schema_version = 2
machine_id = "deploy"
machine_version = 1

[regions.build.states]
queued = { initial = true }
ready = { terminal = true }

[regions.approval.states]
pending = { initial = true }
approved = { terminal = true }

[regions.build.commands.finish_build]
from = ["queued"]
to = "ready"

[regions.approval.commands.approve]
from = ["pending"]
to = "approved"
"#,
    )
    .expect("write region machine");
    path
}

fn run_ik(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(ik_bin()).args(args).output().expect("run ik");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn populate_multi_machine_store(dir: &Path, region_machine: &Path) -> std::path::PathBuf {
    let db = dir.join("multi.sqlite");
    let order_toml = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/orders/order.ik.toml");
    let order_src = fs::read_to_string(order_toml).expect("read order toml");
    let m_order = parse_and_compile(&order_src).expect("compile order");
    let m_order_id = m_order.identity().clone();

    let region_src = fs::read_to_string(region_machine).expect("read region toml");
    let parsed_region =
        parse_source_with_limits(&region_src, RegionLimits::default()).expect("parse region");
    let m_region = compile(parsed_region.definition).expect("compile region");
    let m_region_id = m_region.identity().clone();

    let mut store = Store::open_or_create(&db).expect("open store");
    let ord1 = store
        .create_aggregate(&m_order, &AggregateId("ord-1".into()), &m_order_id)
        .expect("create ord-1");
    store
        .command_op(
            &m_order,
            &CommandRequest {
                aggregate_id: AggregateId("ord-1".into()),
                machine: m_order_id,
                capability: ord1.capability,
                command: json!({ "name": "pay", "args": { "payment_ref": "ch_1" } }),
                observation: None,
            },
            &OperationId::parse("op-ord-pay-1").expect("op id"),
        )
        .expect("pay ord-1");

    let dep1 = store
        .create_aggregate(&m_region, &AggregateId("dep-1".into()), &m_region_id)
        .expect("create dep-1");
    store
        .command_op(
            &m_region,
            &CommandRequest {
                aggregate_id: AggregateId("dep-1".into()),
                machine: m_region_id,
                capability: dep1.capability,
                command: json!({ "name": "finish_build", "args": {} }),
                observation: None,
            },
            &OperationId::parse("op-dep-build-1").expect("op id"),
        )
        .expect("finish_build dep-1");
    drop(store);
    db
}

#[test]
fn ik_store_verify_validates_multi_machine_store_read_only() {
    let dir = temp_dir("valid");
    let region_toml = write_region_machine(&dir);
    let db = populate_multi_machine_store(&dir, &region_toml);
    let order_toml = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/orders/order.ik.toml");

    let before_bytes = fs::read(&db).expect("read before");
    let before_mtime = fs::metadata(&db).expect("meta before").modified().expect("mtime");

    let (code, stdout, stderr) =
        run_ik(&["store", "verify", db.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "cryptographic store verify failed: {stdout} {stderr}");
    let v: Value = serde_json::from_str(&stdout).expect("crypto verify json");
    assert_eq!(v["schema"], "ik.store_verify.v1");
    assert_eq!(v["verified"], true);
    assert_eq!(v["mode"], "cryptographic");
    assert_eq!(v["aggregates"], 2);
    assert_eq!(v["events"], 4);
    assert_eq!(v["operations"], 2);

    let (code, stdout, stderr) = run_ik(&[
        "store",
        "verify",
        db.to_str().unwrap(),
        "--machine",
        order_toml,
        "--machine",
        region_toml.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(code, 0, "semantic store verify failed: {stdout} {stderr}");
    let v2: Value = serde_json::from_str(&stdout).expect("semantic verify json");
    assert_eq!(v2["verified"], true);
    assert_eq!(v2["mode"], "semantic");
    assert_eq!(v2["aggregates"], 2);

    let after_bytes = fs::read(&db).expect("read after");
    let after_mtime = fs::metadata(&db).expect("meta after").modified().expect("mtime");
    assert_eq!(before_bytes, after_bytes, "store verify must not modify store bytes");
    assert_eq!(before_mtime, after_mtime, "store verify must not touch mtime");

    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn ik_store_verify_refuses_tampering_orphans_and_unmatched_machines() {
    let dir = temp_dir("tamper");
    let region_toml = write_region_machine(&dir);
    let clean_db = populate_multi_machine_store(&dir, &region_toml);
    let order_toml = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/orders/order.ik.toml");

    // Partial --machine coverage must refuse instead of silently skipping the second machine.
    let (code, stdout, _) = run_ik(&[
        "store",
        "verify",
        clean_db.to_str().unwrap(),
        "--machine",
        order_toml,
        "--json",
    ]);
    assert_eq!(code, 1, "partial --machine set must fail: {stdout}");
    let v: Value = serde_json::from_str(&stdout).expect("partial json");
    assert_eq!(v["verified"], false);
    assert_eq!(v["error"]["code"], "IK_MACHINE_MISMATCH");

    // 1. Tampered projection state
    let proj_db = dir.join("tamper-proj.sqlite");
    fs::copy(&clean_db, &proj_db).expect("copy proj");
    {
        let conn = Connection::open(&proj_db).expect("open proj");
        conn.execute(
            "UPDATE aggregates SET state = '{\"state\":\"refunded\"}' WHERE aggregate_id = 'ord-1'",
            [],
        )
        .expect("tamper projection");
    }
    let (code, stdout, _) = run_ik(&["store", "verify", proj_db.to_str().unwrap(), "--json"]);
    assert_eq!(code, 1, "tampered projection must fail: {stdout}");
    let vp: Value = serde_json::from_str(&stdout).expect("proj json");
    assert_eq!(vp["verified"], false);
    assert_eq!(vp["error"]["code"], "IK_PROJECTION_CORRUPT");

    // 2. Tampered journal event payload
    let jrn_db = dir.join("tamper-journal.sqlite");
    fs::copy(&clean_db, &jrn_db).expect("copy journal");
    {
        let conn = Connection::open(&jrn_db).expect("open journal");
        let payload: String = conn
            .query_row(
                "SELECT event_json FROM journal WHERE aggregate_id = 'ord-1' AND sequence = 1",
                [],
                |row| row.get(0),
            )
            .expect("read event");
        let mutated = payload.replacen("ch_1", "ch_forged", 1);
        conn.execute(
            "UPDATE journal SET event_json = ?1 WHERE aggregate_id = 'ord-1' AND sequence = 1",
            [mutated],
        )
        .expect("tamper event");
    }
    let (code, stdout, _) = run_ik(&["store", "verify", jrn_db.to_str().unwrap(), "--json"]);
    assert_eq!(code, 1, "tampered journal must fail: {stdout}");
    let vj: Value = serde_json::from_str(&stdout).expect("journal json");
    assert_eq!(vj["verified"], false);
    assert_eq!(vj["error"]["code"], "IK_JOURNAL_CORRUPT");

    // 3. Orphan receipt row
    let orp_db = dir.join("tamper-orphan.sqlite");
    fs::copy(&clean_db, &orp_db).expect("copy orphan");
    {
        let conn = Connection::open(&orp_db).expect("open orphan");
        conn.execute(
            "INSERT INTO receipts (aggregate_id, sequence, receipt_id, receipt_json) VALUES ('ghost', 0, 'bad', '{}')",
            [],
        )
        .expect("insert orphan receipt");
    }
    let (code, stdout, _) = run_ik(&["store", "verify", orp_db.to_str().unwrap(), "--json"]);
    assert_eq!(code, 1, "orphan receipt must fail: {stdout}");
    let vo: Value = serde_json::from_str(&stdout).expect("orphan json");
    assert_eq!(vo["verified"], false);

    fs::remove_dir_all(&dir).expect("cleanup");
}
