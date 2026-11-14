//! IK-01: an initial state that violates the machine's declared hard laws must
//! be refused before ANY genesis row, journal event, receipt, or capability
//! exists, on every public creation path.

use ik::declarative::parse_and_compile;
use ik::diagnostic::DiagnosticCode;
use ik::identity::AggregateId;
use ik::machine::Machine;
use ik::store::Store;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;

fn machine(machine_id: &str, facts: &str, invariant: &str) -> Box<dyn Machine> {
    let text = format!(
        r#"
schema_version = 1
machine_id = "{machine_id}"
machine_version = 1

[facts]
{facts}

[states]
s = {{ initial = true }}
t = {{ terminal = true }}

[commands.go]
from = ["s"]
to = "t"

{invariant}
"#
    );
    Box::new(parse_and_compile(&text).expect("definition is statically valid"))
}

fn false_initial() -> Box<dyn Machine> {
    machine(
        "false_genesis",
        r#"x = { type = "int", policy = "replace", initial = 9 }"#,
        "[invariants.x_limit]\nassert = \"x <= 3\"",
    )
}

fn missing_fact() -> Box<dyn Machine> {
    machine(
        "missing_genesis",
        r#"x = { type = "int", policy = "replace" }"#,
        "[invariants.x_limit]\nassert = \"x <= 3\"",
    )
}

fn erroring_gate() -> Box<dyn Machine> {
    machine(
        "gate_error_genesis",
        r#"x = { type = "int", policy = "replace" }"#,
        "[invariants.x_limit]\nwhen = \"x > 0\"\nassert = \"x <= 3\"",
    )
}

fn valid() -> Box<dyn Machine> {
    machine(
        "valid_genesis",
        r#"x = { type = "int", policy = "replace", initial = 1 }"#,
        "[invariants.x_limit]\nassert = \"x <= 3\"",
    )
}

/// Every authoritative row, rendered in stable order.
fn authoritative_rows(path: &Path) -> Value {
    let conn = Connection::open(path).expect("open store");
    let mut out = serde_json::Map::new();
    for (table, order) in [
        ("aggregates", "aggregate_id"),
        ("journal", "aggregate_id, sequence"),
        ("receipts", "aggregate_id, sequence"),
    ] {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
            .expect("prepare");
        let width = stmt.column_count();
        let rows = stmt
            .query_map([], |row| {
                let mut cells = Vec::with_capacity(width);
                for i in 0..width {
                    let value: rusqlite::types::Value = row.get(i)?;
                    cells.push(format!("{value:?}"));
                }
                Ok(Value::from(cells))
            })
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("rows");
        out.insert(table.to_string(), Value::from(rows));
    }
    Value::Object(out)
}

fn id(name: &str) -> AggregateId {
    AggregateId(name.to_string())
}

/// Observations from attempting both creation paths with an invalid genesis.
struct Attempt {
    plain: ik::diagnostic::Diagnostic,
    with_command: ik::diagnostic::Diagnostic,
    rows_before: Value,
    rows_after_plain: Value,
    rows_after_with_command: Value,
}

fn attempt_invalid_genesis(machine: &dyn Machine) -> Attempt {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut store = Store::create(&path).unwrap();
    // A pre-existing, valid aggregate under another machine proves existing
    // authoritative rows are not modified by the refused creation.
    let good = valid();
    store
        .create_aggregate(good.as_ref(), &id("good"), good.identity())
        .expect("valid genesis succeeds");
    let rows_before = authoritative_rows(&path);

    let plain = store
        .create_aggregate(machine, &id("bad"), machine.identity())
        .expect_err("invalid genesis refuses");
    let rows_after_plain = authoritative_rows(&path);

    let with_command = store
        .create_aggregate_with_command(
            machine,
            &id("bad2"),
            machine.identity(),
            &json!({"name": "go"}),
            None,
        )
        .expect_err("invalid genesis refuses create-with-command");
    let rows_after_with_command = authoritative_rows(&path);
    Attempt {
        plain,
        with_command,
        rows_before,
        rows_after_plain,
        rows_after_with_command,
    }
}

#[test]
fn false_initial_invariant_is_refused_without_rows() {
    let attempt = attempt_invalid_genesis(false_initial().as_ref());
    assert_eq!(attempt.plain.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(attempt.with_command.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(attempt.plain.rule.as_deref(), Some("x_limit"));
    assert_eq!(attempt.rows_after_plain, attempt.rows_before);
    assert_eq!(attempt.rows_after_with_command, attempt.rows_before);
}

#[test]
fn missing_initial_fact_is_refused_without_rows() {
    let attempt = attempt_invalid_genesis(missing_fact().as_ref());
    assert_eq!(attempt.plain.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(attempt.with_command.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(attempt.rows_after_plain, attempt.rows_before);
    assert_eq!(attempt.rows_after_with_command, attempt.rows_before);
}

#[test]
fn unevaluable_invariant_gate_is_refused_without_rows() {
    let attempt = attempt_invalid_genesis(erroring_gate().as_ref());
    assert_eq!(attempt.plain.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(attempt.with_command.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(attempt.rows_after_plain, attempt.rows_before);
    assert_eq!(attempt.rows_after_with_command, attempt.rows_before);
}

#[test]
fn valid_genesis_succeeds_on_both_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut store = Store::create(&path).unwrap();
    let machine = valid();
    let plain = store
        .create_aggregate(machine.as_ref(), &id("a"), machine.identity())
        .expect("plain create");
    assert!(plain.mutation);
    store
        .create_aggregate_with_command(
            machine.as_ref(),
            &id("b"),
            machine.identity(),
            &json!({"name": "go"}),
            None,
        )
        .expect("create-with-command");
}

#[test]
fn public_cli_create_refuses_false_genesis_and_leaves_no_aggregate() {
    let dir = tempfile::tempdir().unwrap();
    let machine_path = dir.path().join("bad.ik.toml");
    std::fs::write(
        &machine_path,
        r#"
schema_version = 1
machine_id = "cli_false_genesis"
machine_version = 1

[facts]
x = { type = "int", policy = "replace", initial = 9 }

[states]
s = { initial = true }
t = { terminal = true }

[commands.go]
from = ["s"]
to = "t"

[invariants.x_limit]
assert = "x <= 3"
"#,
    )
    .unwrap();
    let identity: Value = {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_ik"))
            .arg("identity")
            .arg(&machine_path)
            .output()
            .unwrap();
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let store = dir.path().join("s.db");
    let request = |op: &str, id: &str| {
        json!({
            "protocol": {"major": 1, "minor": 0},
            "runtime_version": env!("CARGO_PKG_VERSION"),
            "request_id": id,
            "op": op,
            "aggregate_id": "a1",
            "machine": identity,
        })
    };
    let created = common_cli(&store, &machine_path, &request("create", "r1"));
    assert_eq!(created["ok"], false, "{created}");
    assert_eq!(created["diagnostic"]["code"], "IK_INVARIANT_FAILED");
    assert!(created["result"].is_null());
    // A creation that never wrote leaves either no store or an empty one.
    if store.exists() {
        let rows = authoritative_rows(&store);
        assert_eq!(rows["aggregates"], json!([]));
        assert_eq!(rows["journal"], json!([]));
        assert_eq!(rows["receipts"], json!([]));
    }
}

fn common_cli(store: &Path, machine: &Path, request: &Value) -> Value {
    use std::io::Write;
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("--store")
        .arg(store)
        .arg("--machine")
        .arg(machine)
        .arg(request["op"].as_str().unwrap())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    serde_json::from_slice(&output.stdout).expect("json envelope")
}

/// IK-06: delete the genesis wall; the honest refusal assertions must then fail because the
/// invalid aggregate is admitted. `with_law_mutant` proves the seam was reached.
#[cfg(feature = "test-support")]
#[test]
fn skipping_genesis_validation_is_killed() {
    use ik::law_mutant::{with_law_mutant, LawMutant};
    let honest = attempt_invalid_genesis(false_initial().as_ref());
    assert_eq!(honest.plain.code, DiagnosticCode::IkInvariantFailed);

    let machine = false_initial();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut store = Store::create(&path).unwrap();
    let before = authoritative_rows(&path);
    let admitted = with_law_mutant(LawMutant::SkipGenesisValidation, || {
        store.create_aggregate(machine.as_ref(), &id("bad"), machine.identity())
    });
    assert!(
        admitted.is_ok(),
        "without the wall the invalid genesis is admitted"
    );
    assert_ne!(
        authoritative_rows(&path),
        before,
        "and its rows are persisted: the refusal-without-rows assertion would fail"
    );
}

/// The same wall guards `create_aggregate_with_command`; deleting it must admit an invalid genesis there too.
#[cfg(feature = "test-support")]
#[test]
fn skipping_genesis_validation_is_killed_for_create_with_command() {
    use ik::law_mutant::{with_law_mutant, LawMutant};
    // The law applies only in state `s`, so the first command `go` leaves a valid post-state: the genesis
    // wall is the only thing that can refuse this admission.
    let repairable = || {
        machine(
            "repairable_genesis",
            r#"x = { type = "int", policy = "replace", initial = 9 }"#,
            "[invariants.x_limit]\nwhen = \"state == 's'\"\nassert = \"x <= 3\"",
        )
    };
    let admit = |store: &mut Store, name: &str| {
        let machine = repairable();
        store.create_aggregate_with_command(
            machine.as_ref(),
            &id(name),
            machine.identity(),
            &json!({"name": "go"}),
            None,
        )
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.db");
    let mut store = Store::create(&path).unwrap();
    let before = authoritative_rows(&path);
    let refused = admit(&mut store, "honest").expect_err("the genesis wall refuses");
    assert_eq!(refused.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(authoritative_rows(&path), before);

    let admitted = with_law_mutant(LawMutant::SkipGenesisValidation, || {
        admit(&mut store, "mutated")
    });
    assert!(
        admitted.is_ok(),
        "without the wall the invalid genesis is admitted with its command"
    );
    assert_ne!(authoritative_rows(&path), before);
}
