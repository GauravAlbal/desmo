//! Integration tests for `ik simulate`: interactive structural HTML simulator.

use serde_json::json;
use std::process::Command;

fn ik(args: &[&str]) -> (String, String, Option<i32>) {
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
fn simulate_writes_self_contained_html_to_out_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out_html = dir.path().join("nested").join("order-simulator.html");
    let (stdout, stderr, code) = ik(&[
        "simulate",
        "examples/orders/order.ik.toml",
        "--out",
        out_html.to_str().unwrap(),
    ]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.contains("wrote "), "stdout: {stdout}");
    assert!(stdout.contains("order v1"), "stdout: {stdout}");

    let html = std::fs::read_to_string(&out_html).expect("read html");
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("order v1"));
    assert!(html.contains("IK_CAPABILITY_STALE"));
    assert!(html.contains("IK_ILLEGAL_STATE"));
    assert!(html.contains("Advisory structural simulation"));
    assert!(html.contains("\"send_back\""));
}

#[test]
fn simulate_streams_html_to_stdout_when_out_omitted() {
    let (stdout, stderr, code) = ik(&["simulate", "examples/orders/order.ik.toml"]);
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.starts_with("<!DOCTYPE html>"));
    assert!(stdout.contains("order v1"));
}

#[test]
fn render_simulator_html_escapes_script_breakouts_and_html_tags() {
    let schema = json!({
        "schema": "ikaros.machine-schema.v1",
        "machine": {
            "machine_id": "</script><img src=x onerror=alert(1)>",
            "version": 1,
            "spec_hash": "abcdef0123456789",
        },
        "states": [
            {"name": "open", "initial": true, "terminal": false},
            {"name": "</script>", "initial": false, "terminal": true},
        ],
        "facts": [],
        "commands": [
            {"name": "close", "from": ["open"], "args": [], "successors": ["</script>"], "conditional_successor": false, "plans_effect": false}
        ],
        "invariants": [],
    });
    let html = ik::simulate::render_simulator_html(&schema).expect("render");
    assert!(!html.contains("<img src=x"), "raw html tag leaked");
    assert_eq!(
        html.matches("</script>").count(),
        1,
        "unexpected closing script tag inside embedded JSON"
    );
    assert!(html.contains("\\u003c/script\\u003e"));
    assert!(html.contains("&lt;/script&gt;&lt;img src=x onerror=alert(1)&gt;"));
}

#[test]
fn simulate_rejects_missing_machine_path_and_unknown_flags() {
    let (_, err1, code1) = ik(&["simulate"]);
    assert_eq!(code1, Some(2));
    assert!(err1.contains("usage: ik simulate"));

    let (_, err2, code2) = ik(&["simulate", "examples/orders/order.ik.toml", "--bogus"]);
    assert_eq!(code2, Some(2));
    assert!(err2.contains("unknown flag"));
}
