//! Tier A2 integration tests: native `ik demo`, `ik init --lang ts|py|go`,
//! `ik doctor`, and grouped `ik --help`.

use serde_json::Value;
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
fn demo_runs_orders_and_stale_worker_live_in_json_and_text() {
    let (out, err, code) = run_ik(&["demo", "--json"]);
    assert_eq!(code, Some(0), "stderr:\n{err}");
    let v: Value = serde_json::from_str(&out).expect("valid demo json");
    assert_eq!(v["orders"]["unpaid_refund_code"], "IK_ILLEGAL_STATE");
    assert_eq!(v["orders"]["unpaid_state"], "cart");
    assert_eq!(v["orders"]["cancelled_deliver_code"], "IK_ILLEGAL_STATE");
    assert_eq!(v["orders"]["happy_path_state"], "refunded");
    assert_eq!(v["orders"]["double_refund_code"], "IK_ILLEGAL_STATE");
    assert_eq!(v["stale_worker"]["stale_refusal_code"], "IK_CAPABILITY_STALE");
    assert_eq!(v["stale_worker"]["canonical_state"], "cancelled");

    let (text, err, code) = run_ik(&["demo", "stale-worker"]);
    assert_eq!(code, Some(0), "stderr:\n{err}");
    assert!(text.contains("IK_CAPABILITY_STALE"), "text:\n{text}");
}

#[test]
fn init_scaffolds_ts_py_go_and_passes_check_and_doctor() {
    for (lang, expected_file) in [
        ("ts", "order.machine.ts"),
        ("py", "order_machine.py"),
        ("go", "order_machine.go"),
    ] {
        let dir = std::env::temp_dir().join(format!("desmo-tier-a2-init-{lang}"));
        let _ = std::fs::remove_dir_all(&dir);
        let (out, err, code) = run_ik(&[
            "init",
            "order",
            "--lang",
            lang,
            "--out",
            dir.to_str().unwrap(),
        ]);
        assert_eq!(code, Some(0), "lang={lang} stdout:\n{out}\nstderr:\n{err}");
        assert!(dir.join("order.ik.toml").is_file());
        assert!(dir.join("order.ik.cases.json").is_file());
        assert!(dir.join(expected_file).is_file());
        let machine_txt = std::fs::read_to_string(dir.join("order.ik.toml")).unwrap();
        assert!(machine_txt.starts_with("#:schema "), "missing #:schema directive:\n{machine_txt}");

        // Generated project passes `ik doctor --json` cleanly before local test replace!
        let (doc_out, doc_err, doc_code) =
            run_ik(&["doctor", "--dir", dir.to_str().unwrap(), "--json"]);
        assert_eq!(doc_code, Some(0), "stderr:\n{doc_err}");
        let doc: Value = serde_json::from_str(&doc_out).expect("doctor json");
        assert_eq!(doc["healthy"], true, "doctor:\n{doc}");

        if lang == "go" {
            let gen_txt = std::fs::read_to_string(dir.join("order_machine.go")).unwrap();
            assert!(gen_txt.contains("package main\n"), "go generated wrapper must be package main:\n{gen_txt}");
            let repo_go = concat!(env!("CARGO_MANIFEST_DIR"), "/go");
            let orig_mod = std::fs::read_to_string(dir.join("go.mod")).unwrap();
            std::fs::write(
                dir.join("go.mod"),
                format!("{orig_mod}\nreplace github.com/GauravAlbal/desmo/go => {repo_go}\n"),
            )
            .unwrap();
            let build_out = Command::new("go")
                .args(["build", "."])
                .current_dir(&dir)
                .output()
                .expect("run go build on scaffold");
            assert!(
                build_out.status.success(),
                "go build failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&build_out.stdout),
                String::from_utf8_lossy(&build_out.stderr)
            );
            // `ik doctor` still ignores local `replace` lines in go.mod cleanly.
            let (_, doc2_err, doc2_code) =
                run_ik(&["doctor", "--dir", dir.to_str().unwrap(), "--json"]);
            assert_eq!(doc2_code, Some(0), "doctor with go.mod replace:\n{doc2_err}");
        } else if lang == "ts" {
            let demo_txt = std::fs::read_to_string(dir.join("demo.mjs")).unwrap();
            assert!(demo_txt.contains("import.meta.url"), "ts demo should resolve paths via import.meta.url:\n{demo_txt}");
        } else if lang == "py" {
            let demo_txt = std::fs::read_to_string(dir.join("demo.py")).unwrap();
            assert!(demo_txt.contains("from order_machine import OrderMachine"), "py demo should use generated OrderMachine:\n{demo_txt}");
        }

        // Refuses overwrite without --force.
        let (_, force_err, force_code) = run_ik(&[
            "init",
            "order",
            "--lang",
            lang,
            "--out",
            dir.to_str().unwrap(),
        ]);
        assert_eq!(force_code, Some(2));
        assert!(force_err.contains("--force"), "stderr:\n{force_err}");

        // Generated project passes `ik check` immediately!
        let (check_out, check_err, check_code) = run_ik(&[
            "check",
            dir.join("order.ik.toml").to_str().unwrap(),
            "--summary",
        ]);
        assert_eq!(
            check_code,
            Some(0),
            "lang={lang} check stdout:\n{check_out}\nstderr:\n{check_err}"
        );
        assert!(check_out.contains("ACCEPT"), "stdout:\n{check_out}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn doctor_flags_broken_machine_and_sdk_version_drift() {
    let dir = std::env::temp_dir().join("desmo-tier-a2-doctor-bad");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("broken.ik.toml"), "not valid toml [[[").unwrap();
    std::fs::write(
        dir.join("package.json"),
        r#"{"dependencies":{"@desmo/runtime":"0.1.0"}}"#,
    )
    .unwrap();

    let (out, _, code) = run_ik(&["doctor", "--dir", dir.to_str().unwrap(), "--json"]);
    assert_eq!(code, Some(1), "expected unhealthy exit 1:\n{out}");
    let v: Value = serde_json::from_str(&out).expect("doctor json");
    assert_eq!(v["healthy"], false);
    assert_eq!(v["machines"][0]["ok"], false);
    assert_eq!(v["sdk_pins"][0]["drift"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn help_is_grouped_into_quickstart_and_workbench_sections() {
    let (help, _, code) = run_ik(&["--help"]);
    assert_eq!(code, Some(0));
    assert!(help.contains("Quickstart:"), "help:\n{help}");
    assert!(help.contains("Model & Verify:"), "help:\n{help}");
    assert!(help.contains("Inspect & Store:"), "help:\n{help}");
    assert!(help.contains("--help-all"), "help:\n{help}");

    let (help_all, _, code_all) = run_ik(&["--help-all"]);
    assert_eq!(code_all, Some(0));
    assert!(help_all.contains("Advanced & Adapters:"), "help-all:\n{help_all}");
}
