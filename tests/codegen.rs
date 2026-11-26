//! Integration tests for unified `ik codegen --lang ts|py|go`.

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
fn codegen_emits_typed_wrappers_for_ts_py_and_go_and_checks_staleness() {
    let dir = tempfile::tempdir().expect("tempdir");
    for (lang, ext, expected) in [
        ("ts", "ts", "export class OrderAggregate"),
        ("py", "py", "class OrderAggregate:"),
        ("go", "go", "type OrderAggregate struct"),
    ] {
        let out_file = dir.path().join(format!("order.{ext}"));
        let (stdout, stderr, code) = ik(&[
            "codegen",
            "examples/orders/order.ik.toml",
            "--lang",
            lang,
            "--out",
            out_file.to_str().unwrap(),
        ]);
        assert_eq!(code, Some(0), "{lang} stderr: {stderr}");
        assert!(stdout.contains("wrote "), "{lang} stdout: {stdout}");
        let body = std::fs::read_to_string(&out_file).expect("read generated");
        assert!(body.contains(expected), "{lang} missing `{expected}`:\n{body}");

        let (_, check_err, check_code) = ik(&[
            "codegen",
            "examples/orders/order.ik.toml",
            "--lang",
            lang,
            "--out",
            out_file.to_str().unwrap(),
            "--check",
        ]);
        assert_eq!(check_code, Some(0), "{lang} --check stderr: {check_err}");

        std::fs::write(&out_file, "// stale\n").expect("corrupt generated");
        let (_, stale_err, stale_code) = ik(&[
            "codegen",
            "examples/orders/order.ik.toml",
            "--lang",
            lang,
            "--out",
            out_file.to_str().unwrap(),
            "--check",
        ]);
        assert_eq!(stale_code, Some(1), "{lang} stale check should exit 1");
        assert!(stale_err.contains("stale"), "{lang} stale stderr: {stale_err}");
    }
}
