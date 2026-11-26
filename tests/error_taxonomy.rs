//! Missing-store classification and read-only non-creation.
//!
//! Contract: a path that does not exist is `IK_STORE_NOT_FOUND` (`store.missing`),
//! not `IK_STORE_CORRUPT`. `show`, `replay`, `verify`, and `lookup_operation`
//! must not create the file or a missing parent. `create` is the only creator.
//! A present but unreadable file remains `IK_STORE_CORRUPT`.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::diagnostic::DiagnosticCode;
use ik::protocol::RUNTIME_VERSION;
use ik::store::Store;
use serde_json::{json, Value};
use std::fs;

fn envelope(op: &str) -> Value {
    json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": RUNTIME_VERSION,
        "request_id": format!("req-{op}"),
        "op": op,
    })
}

#[test]
fn open_of_a_missing_store_is_not_corruption_and_creates_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("absent-parent").join("store.db");
    assert!(
        !path.exists() && path.parent().map_or(true, |p| !p.exists()),
        "store and parent must not be created"
    );
    let err = match Store::open(&path) {
        Err(err) => err,
        Ok(_) => panic!("missing store must refuse"),
    };
    assert_eq!(err.code, DiagnosticCode::IkStoreNotFound);
    assert_eq!(err.rule.as_deref(), Some("store.missing"));
    assert_ne!(err.code, DiagnosticCode::IkStoreCorrupt);
    assert!(
        !path.exists() && path.parent().map_or(true, |p| !p.exists()),
        "store and parent must not be created"
    );
}

#[test]
fn read_only_protocol_ops_do_not_create_a_missing_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("absent-parent").join("store.db");
    let machine = machine_def("verification_attempt");
    let identity = serde_json::to_value(identity_of("verification_attempt")).unwrap();
    assert!(
        !path.exists() && path.parent().map_or(true, |p| !p.exists()),
        "store and parent must not be created"
    );

    let mut show = envelope("show");
    show["aggregate_id"] = json!("missing");
    let response = run_cli_json(&path, None, &show);
    assert_eq!(
        (
            response["ok"].clone(),
            response["diagnostic"]["code"].as_str(),
            response["diagnostic"]["rule"].as_str()
        ),
        (
            json!(false),
            Some("IK_STORE_NOT_FOUND"),
            Some("store.missing")
        ),
        "{response}"
    );
    assert!(
        !path.exists() && path.parent().map_or(true, |p| !p.exists()),
        "store and parent must not be created"
    );

    let mut replay = envelope("replay");
    replay["aggregate_id"] = json!("missing");
    let response = run_cli_json(&path, Some(&machine), &replay);
    assert_eq!(
        (
            response["ok"].clone(),
            response["diagnostic"]["code"].as_str(),
            response["diagnostic"]["rule"].as_str()
        ),
        (
            json!(false),
            Some("IK_STORE_NOT_FOUND"),
            Some("store.missing")
        ),
        "{response}"
    );
    assert!(
        !path.exists() && path.parent().map_or(true, |p| !p.exists()),
        "store and parent must not be created"
    );

    let mut verify = envelope("verify");
    verify["aggregate_id"] = json!("missing");
    let response = run_cli_json(&path, Some(&machine), &verify);
    assert_eq!(
        (
            response["ok"].clone(),
            response["diagnostic"]["code"].as_str(),
            response["diagnostic"]["rule"].as_str()
        ),
        (
            json!(false),
            Some("IK_STORE_NOT_FOUND"),
            Some("store.missing")
        ),
        "{response}"
    );
    assert!(
        !path.exists() && path.parent().map_or(true, |p| !p.exists()),
        "store and parent must not be created"
    );

    let mut lookup = envelope("lookup_operation");
    lookup["aggregate_id"] = json!("missing");
    lookup["operation_id"] = json!("op-1");
    lookup["machine"] = identity;
    lookup["capability"] = json!({"generation": 0, "nonce": "n"});
    lookup["command"] = json!({"name": "start"});
    let response = run_cli_json(&path, None, &lookup);
    assert_eq!(
        (
            response["ok"].clone(),
            response["diagnostic"]["code"].as_str(),
            response["diagnostic"]["rule"].as_str()
        ),
        (
            json!(false),
            Some("IK_STORE_NOT_FOUND"),
            Some("store.missing")
        ),
        "{response}"
    );
    assert!(
        !path.exists() && path.parent().map_or(true, |p| !p.exists()),
        "store and parent must not be created"
    );
}

#[test]
fn command_does_not_create_a_missing_store_and_create_does() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("no-parent").join("store.db");
    let machine = machine_def("verification_attempt");
    let identity = serde_json::to_value(identity_of("verification_attempt")).unwrap();
    let mut command = envelope("command");
    command["aggregate_id"] = json!("agg-1");
    command["machine"] = identity.clone();
    command["capability"] = json!({"generation": 0, "nonce": "n"});
    command["command"] = json!({"name": "start"});
    let response = run_cli_json(&missing, Some(&machine), &command);
    assert_eq!(
        (
            response["ok"].clone(),
            response["diagnostic"]["code"].as_str(),
            response["diagnostic"]["rule"].as_str()
        ),
        (
            json!(false),
            Some("IK_STORE_NOT_FOUND"),
            Some("store.missing")
        ),
        "{response}"
    );
    assert!(
        !missing.exists() && missing.parent().map_or(true, |p| !p.exists()),
        "store and parent must not be created"
    );

    let created = dir.path().join("created.db");
    assert!(!created.exists());
    let mut create = envelope("create");
    create["aggregate_id"] = json!("agg-1");
    create["machine"] = identity;
    let response = run_cli_json(&created, Some(&machine), &create);
    assert_eq!(response["ok"], json!(true), "{response}");
    assert!(
        created.exists(),
        "create must be the operation that creates the store"
    );
}

#[test]
fn a_present_garbage_file_is_corrupt_not_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("garbage.db");
    fs::write(&path, b"this is not a sqlite database").expect("write garbage");
    let err = match Store::open(&path) {
        Err(err) => err,
        Ok(_) => panic!("garbage store must refuse"),
    };
    assert_eq!(err.code, DiagnosticCode::IkStoreCorrupt);
    assert_ne!(err.code, DiagnosticCode::IkStoreNotFound);
    assert_ne!(err.rule.as_deref(), Some("store.missing"));
    assert!(path.exists());

    let mut show = envelope("show");
    show["aggregate_id"] = json!("agg-1");
    let response = run_cli_json(&path, None, &show);
    assert_eq!(response["ok"], json!(false), "{response}");
    assert_eq!(
        response["diagnostic"]["code"].as_str(),
        Some("IK_STORE_CORRUPT"),
        "{response}"
    );
    assert_ne!(
        response["diagnostic"]["rule"].as_str(),
        Some("store.missing"),
        "{response}"
    );
    let again = match Store::open(&path) {
        Err(err) => err,
        Ok(_) => panic!("garbage must stay unusable"),
    };
    assert_eq!(again.code, DiagnosticCode::IkStoreCorrupt);
}

#[test]
fn an_unknown_aggregate_is_not_a_corrupt_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("store.db");
    let machine = machine_def("verification_attempt");
    let identity = serde_json::to_value(identity_of("verification_attempt")).unwrap();
    let mut create = envelope("create");
    create["aggregate_id"] = json!("present");
    create["machine"] = identity;
    let response = run_cli_json(&path, Some(&machine), &create);
    assert_eq!(response["ok"], json!(true), "{response}");

    let mut show = envelope("show");
    show["aggregate_id"] = json!("absent");
    let response = run_cli_json(&path, None, &show);
    assert_eq!(
        response["diagnostic"]["code"].as_str(),
        Some("IK_ILLEGAL_STATE")
    );
    assert_eq!(
        response["diagnostic"]["rule"].as_str(),
        Some("aggregate.not_found")
    );
    assert_ne!(
        response["diagnostic"]["code"].as_str(),
        Some("IK_STORE_CORRUPT")
    );
}

#[test]
fn open_or_create_creates_missing_parent_dirs_while_open_remains_strict() {
    let dir = tempfile::tempdir().expect("tempdir");
    let absent = dir.path().join("still-absent").join(".state").join("orders.db");
    let err = match Store::open(&absent) {
        Err(err) => err,
        Ok(_) => panic!("Store::open must refuse missing parent without creating"),
    };
    assert_eq!(err.code, DiagnosticCode::IkStoreNotFound);
    assert_eq!(err.rule.as_deref(), Some("store.missing"));
    assert!(!absent.parent().unwrap().exists());

    let nested = dir.path().join("nested").join(".state").join("orders.db");
    assert!(!nested.parent().unwrap().exists());
    let store = Store::open_or_create(&nested).expect("open_or_create creates missing parents");
    drop(store);
    assert!(nested.is_file());
    Store::open(&nested).expect("reopen created store");
}
