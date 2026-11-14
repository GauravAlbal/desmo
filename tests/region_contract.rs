//! RA01/02/05: finite normative region relations and refusal consistency through
//! real CLI processes and SQLite. RA03: independently released schema-1 identity.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::compile::compile;
use ik::declarative::region::parse_source;
use ik::declarative::Definition;
use ik::machine::Machine;
use rusqlite::{types::Value as SqlValue, Connection, OpenFlags};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

// Preservation authority, NOT a correctness oracle: the actual v0.2.1
// darwin-arm64 release, accepted commit 568525461ed4b73cc4874e2735cc0786bc23781c,
// artifact SHA-256 81edee455b8d139210508e1f12e525540f043deba165f6bc824969b7f24c79a5.
// These identities were captured from that binary on these unchanged fixtures,
// independently of the candidate compiler. No release download is needed at test time.
const ACCEPTED_SCHEMA1: [(&str, &str, &str); 3] = [
    (
        "verification_attempt",
        include_str!("../machines/verification_attempt.ik.toml"),
        "aae4f03a6e4169bdc86021914bc57db4bc391878651fa61fcd2a9d0d215b4a7f",
    ),
    (
        "invariant_probe",
        include_str!("../machines/invariant_probe.ik.toml"),
        "b7531c646e90c2c613243cc3a8531832f8a75dfbc5dc722f2db0659f1bf2858e",
    ),
    (
        "effect_probe",
        include_str!("../machines/effect_probe.ik.toml"),
        "f9058d9f72ce96aa2f7369683c97a065d1ca32858fb4abc93e06e294cb45df6f",
    ),
];

#[test]
fn accepted_schema1_identities_survive_direct_source_file_and_cli_loading() {
    for (machine_id, text, accepted_hash) in ACCEPTED_SCHEMA1 {
        let accepted = json!({
            "machine_id": machine_id,
            "version": 1,
            "spec_hash": accepted_hash,
        });

        let definition: Definition = toml::from_str(text).expect("schema-1 definition");
        let direct = compile(definition).expect("compile schema-1 directly");
        assert_eq!(
            serde_json::to_value(direct.identity()).unwrap(),
            accepted,
            "{machine_id}: direct compilation must retain the released identity"
        );

        let source = parse_source(text).expect("schema-1 source frontend");
        assert!(
            source.regions.is_none(),
            "{machine_id}: schema 1 must not acquire region semantics"
        );
        let frontend = compile(source.definition).expect("compile parsed source");
        assert_eq!(
            serde_json::to_value(frontend.identity()).unwrap(),
            accepted,
            "{machine_id}: source frontend must retain the released identity"
        );

        let path = machine_def(machine_id);
        let loaded = ik::declarative::load_def(&path).expect("load unchanged schema-1 file");
        assert_eq!(
            serde_json::to_value(loaded.identity()).unwrap(),
            accepted,
            "{machine_id}: file loader must retain the released identity"
        );

        let output = Command::new(env!("CARGO_BIN_EXE_ik"))
            .arg("identity")
            .arg(&path)
            .output()
            .expect("run ik identity");
        assert_eq!(
            output.status.code(),
            Some(0),
            "{machine_id}: identity failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let actual: Value = serde_json::from_slice(&output.stdout).expect("CLI identity JSON");
        assert_eq!(
            actual, accepted,
            "{machine_id}: real CLI must retain the released identity"
        );
    }
}

#[test]
fn check_cli_emits_refusal_reasons_and_invariant_witness() {
    let output = Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("check")
        .arg(machine_def("region_parallel"))
        .output()
        .expect("run ik check");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "the fixture must be refused: stdout={stdout}\nstderr={stderr}"
    );
    for expected in [
        "REFUSED",
        "REASONS",
        "no_playing_muted",
        "invariant-refusal witness",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in CLI output:\n{stdout}"
        );
    }
}

#[test]
fn check_cli_groups_region_obligations_by_command_and_resolves_products() {
    let source = r#"schema_version = 2
machine_id = "region_obligation_cli"
machine_version = 1

[facts.flag]
type = "bool"
policy = "replace"
initial = false

[regions.order.states]
draft = { initial = true }
placed = {}
[regions.order.commands.place]
from = ["draft"]
to = "placed"

[regions.payment.states]
idle = { initial = true }
captured = {}

[commands.capture]
from = { order = ["placed"], payment = ["idle"] }
when = "flag == true"
to = { payment = "captured" }
"#;
    let dir = tempfile::tempdir().expect("temporary source directory");
    let definition = dir.path().join("region-obligation.ik.toml");
    std::fs::write(&definition, source).expect("write temporary region source");
    let output = Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("check")
        .arg(&definition)
        .output()
        .expect("run ik check");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("UNRESOLVED REGION OBLIGATIONS\n"),
        "region check should emit unresolved obligations: exit={:?}, stdout={stdout}\nstderr={stderr}",
        output.status.code()
    );
    let diagnostic = stdout
        .split("UNRESOLVED REGION OBLIGATIONS\n")
        .nth(1)
        .expect("unresolved obligation section");
    assert!(
        diagnostic.starts_with("  command capture\n"),
        "joint command coupling must be named as its group:\n{diagnostic}"
    );
    assert!(
        diagnostic.contains("    source products: placed__idle (order=placed, payment=idle)\n"),
        "command source flat label must resolve to its region assignments:\n{diagnostic}"
    );
    assert!(
        diagnostic.contains("    command/capture/from/placed__idle/executable uncovered\n"),
        "source-specific command obligation must be grouped with its coupling:\n{diagnostic}"
    );
    assert!(
        diagnostic.contains("    authored/outcome/capture/joint/0/executable uncovered\n"),
        "joint coupling obligation ID and unresolved status must be auditable:\n{diagnostic}"
    );
}

// Follow the existing durable_rows pattern, but retain typed cell values rather
// than a lossy textual encoding. Read-only connections cannot repair a projection.
fn durable_rows(path: &Path) -> rusqlite::Result<BTreeMap<&'static str, Vec<Vec<SqlValue>>>> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut snapshot = BTreeMap::new();
    for (table, order) in [
        ("meta", "key"),
        ("aggregates", "aggregate_id"),
        ("journal", "aggregate_id, sequence"),
        ("receipts", "aggregate_id, sequence"),
    ] {
        let mut statement = conn.prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))?;
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..columns)
                    .map(|index| row.get::<_, SqlValue>(index))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        snapshot.insert(table, rows);
    }
    Ok(snapshot)
}

fn envelope(store: &Path, definition: Option<&Path>, request: &Value) -> (Value, i32) {
    let (stdout, code) = run_cli(store, definition, request);
    let response = serde_json::from_str(&stdout).expect("CLI response envelope JSON");
    (response, code)
}

#[test]
fn cli_unsafe_region_command_preserves_durable_state_and_capability() {
    // Normative oracle: docs/region-lowering.md, parallel example and execution
    // contract. Mute while playing violates no_playing_muted; pause is safe.
    let dir = tempfile::tempdir().expect("isolated store directory");
    let store = dir.path().join("region-safety.db");
    let definition = machine_def("region_parallel");
    let output = Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("identity")
        .arg(&definition)
        .output()
        .expect("run schema-2 CLI identity");
    assert_eq!(
        output.status.code(),
        Some(0),
        "schema-2 identity failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let identity: Value = serde_json::from_slice(&output.stdout).expect("schema-2 identity JSON");
    assert_eq!(identity["machine_id"], "region_parallel");
    assert_eq!(identity["version"], 1);
    let request = |op: &str| {
        json!({
            "protocol": {"major": 1, "minor": 0},
            "runtime_version": ik::protocol::RUNTIME_VERSION,
            "request_id": op,
            "op": op,
            "aggregate_id": "region-safety",
            "machine": identity,
        })
    };
    let initial = json!({
        "state": "playing__unmuted",
        "region_playback": "playing",
        "region_volume": "unmuted",
    });

    let (created, code) = envelope(&store, Some(&definition), &request("create"));
    assert_eq!(code, 0, "{created}");
    assert_eq!(created["ok"], true, "{created}");
    assert_eq!(created["result"]["state"], initial);
    assert_eq!(created["result"]["generation"], 0);
    assert_eq!(created["result"]["capability"]["generation"], 0);
    let capability = created["result"]["capability"].clone();
    let (before_show, code) = envelope(&store, None, &request("show"));
    assert_eq!(code, 0, "{before_show}");
    assert_eq!(before_show["ok"], true, "{before_show}");
    assert_eq!(before_show["result"]["state"], initial);
    assert_eq!(before_show["result"]["capability"], capability);
    let before = durable_rows(&store).expect("snapshot committed genesis rows");

    let mut command = request("command");
    command["capability"] = capability.clone();
    command["command"] = json!({"name": "mute"});
    let (refused, code) = envelope(&store, Some(&definition), &command);
    assert_eq!(code, 1, "unsafe mute must refuse: {refused}");
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["diagnostic"]["code"], "IK_INVARIANT_FAILED");
    assert_eq!(refused["diagnostic"]["rule"], "no_playing_muted");
    assert_eq!(
        durable_rows(&store).expect("snapshot immediately after refusal"),
        before,
        "refusal must preserve every aggregate/state/generation/nonce/effect, journal, receipt and meta cell"
    );

    let (after_show, code) = envelope(&store, None, &request("show"));
    assert_eq!(code, 0, "{after_show}");
    assert_eq!(after_show["ok"], true, "{after_show}");
    assert_eq!(after_show["result"], before_show["result"]);
    assert_eq!(after_show["result"]["state"], initial);
    assert_eq!(after_show["result"]["generation"], 0);
    assert_eq!(after_show["result"]["capability"], capability);

    // Verify before replay: replay must not hide a leaked write by rebuilding.
    let (verified, code) = envelope(&store, Some(&definition), &request("verify"));
    assert_eq!(code, 0, "{verified}");
    assert_eq!(verified["ok"], true, "{verified}");
    assert_eq!(verified["result"]["aggregates"], 1);
    assert_eq!(verified["result"]["journal_ok"], true);
    assert_eq!(verified["result"]["projection_ok"], true);
    assert_eq!(verified["result"]["verified"], true);

    let (replayed, code) = envelope(&store, Some(&definition), &request("replay"));
    assert_eq!(code, 0, "{replayed}");
    assert_eq!(replayed["ok"], true, "{replayed}");
    assert_eq!(
        replayed["result"]["aggregates"],
        json!([{
            "aggregate_id": "region-safety",
            "sequence": 0,
            "generation": 0,
            "state": initial,
            "rebuilt": false,
        }]),
        "reopening and replaying must recover only the committed safe genesis"
    );
    assert_eq!(
        durable_rows(&store).expect("snapshot after verify and replay"),
        before,
        "verify/replay must not repair away evidence of an unsafe commit"
    );

    // The pre-refusal capability still authorizes the next safe transition.
    command["command"] = json!({"name": "pause"});
    let (paused, code) = envelope(&store, Some(&definition), &command);
    assert_eq!(code, 0, "original capability must remain usable: {paused}");
    assert_eq!(paused["ok"], true, "{paused}");
    assert_eq!(paused["result"]["mutation"], true);
    assert_eq!(paused["result"]["generation"], 1);
    assert_eq!(paused["result"]["receipt"]["sequence"], 1);
    assert_eq!(
        paused["result"]["state"],
        json!({
            "state": "paused__unmuted",
            "region_playback": "paused",
            "region_volume": "unmuted",
        })
    );
}

const PARALLEL: &str = include_str!("../machines/region_parallel.ik.toml");
const COMPOUND: &str = include_str!("../machines/region_compound.ik.toml");

type Rows = BTreeMap<&'static str, Vec<Vec<SqlValue>>>;

struct CellEvidence {
    response: Value,
    code: i32,
    before_show: Value,
    after_show: Value,
    before: Rows,
    after: Rows,
    verified: Value,
    verify_code: i32,
    replayed: Value,
    replay_code: i32,
    after_replay: Rows,
    journal: Vec<Value>,
    receipts: Vec<Value>,
    history: Vec<(Value, Value)>,
}

fn request(identity: &Value, op: &str) -> Value {
    json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": ik::protocol::RUNTIME_VERSION,
        "request_id": op,
        "op": op,
        "aggregate_id": "finite-model",
        "machine": identity,
    })
}

fn json_rows(path: &Path, table: &str, column: &str) -> Vec<Value> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut statement = conn
        .prepare(&format!(
            "SELECT {column} FROM {table} WHERE aggregate_id = 'finite-model' ORDER BY sequence"
        ))
        .unwrap();
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
        .collect();
    rows
}

// This helper only drives the real program and records observations. Expected
// lifecycle targets come from the finite relations below, never lowered outcomes.
fn cli_cell(
    source: &str,
    initial: &Value,
    prefix: &[(Value, Value)],
    command: &Value,
    corrupt_source: Option<&Value>,
) -> CellEvidence {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("finite.db");
    let definition = dir.path().join("finite.ik.toml");
    std::fs::write(&definition, source).unwrap();
    let machine = compile(parse_source(source).unwrap().definition).unwrap();
    let identity = serde_json::to_value(machine.identity()).unwrap();
    let (created, code) = envelope(&store, Some(&definition), &request(&identity, "create"));
    assert_eq!(code, 0, "{created}");
    assert_eq!(created["result"]["state"], *initial);
    assert_eq!(created["result"]["generation"], 0);
    let mut capability = created["result"]["capability"].clone();
    for (index, (setup, expected)) in prefix.iter().enumerate() {
        let mut req = request(&identity, "command");
        req["capability"] = capability;
        req["command"] = setup.clone();
        let (response, code) = envelope(&store, Some(&definition), &req);
        assert_eq!(code, 0, "setup {setup}: {response}");
        assert_eq!(
            response["result"]["state"], *expected,
            "independent reachability prefix"
        );
        assert_eq!(response["result"]["generation"], index + 1);
        capability = response["result"]["capability"].clone();
    }
    if let Some(corrupted) = corrupt_source {
        let conn = Connection::open(&store).unwrap();
        assert_eq!(
            conn.execute(
                "UPDATE aggregates SET state = ?1 WHERE aggregate_id = 'finite-model'",
                [serde_json::to_string(corrupted).unwrap()],
            )
            .unwrap(),
            1
        );
    }
    let (before_show, code) = envelope(&store, None, &request(&identity, "show"));
    assert_eq!(code, 0, "{before_show}");
    let before = durable_rows(&store).unwrap();
    let mut req = request(&identity, "command");
    req["capability"] = capability;
    req["command"] = command.clone();
    let (response, code) = envelope(&store, Some(&definition), &req);
    let (after_show, show_code) = envelope(&store, None, &request(&identity, "show"));
    assert_eq!(show_code, 0, "{after_show}");
    let after = durable_rows(&store).unwrap();
    let journal = json_rows(&store, "journal", "event_json");
    let receipts = json_rows(&store, "receipts", "receipt_json");
    // Capture refusal evidence first; never let replay conceal a leaked write.
    let (verified, verify_code) =
        envelope(&store, Some(&definition), &request(&identity, "verify"));
    assert_eq!(
        durable_rows(&store).unwrap(),
        after,
        "verify must be read-only even on corruption"
    );
    let (replayed, replay_code) =
        envelope(&store, Some(&definition), &request(&identity, "replay"));
    let after_replay = durable_rows(&store).unwrap();
    let mut history = vec![(Value::Null, initial.clone())];
    history.extend_from_slice(prefix);
    CellEvidence {
        response,
        code,
        before_show,
        after_show,
        before,
        after,
        verified,
        verify_code,
        replayed,
        replay_code,
        after_replay,
        journal,
        receipts,
        history,
    }
}

fn assert_durable_cell(
    cell: &CellEvidence,
    command: &Value,
    expected_state: &Value,
    terminal: bool,
    committed: bool,
) {
    let generation = (cell.history.len() - 1 + usize::from(committed)) as u64;
    assert_eq!(cell.after_show["result"]["generation"], generation);
    assert_eq!(cell.after_show["result"]["sequence"], generation);
    assert_eq!(
        cell.after_show["result"]["capability"]["generation"],
        generation
    );
    assert_eq!(cell.after_show["result"]["terminal"], terminal);
    let mut history = cell.history.clone();
    if committed {
        history.push((command.clone(), expected_state.clone()));
        assert_eq!(cell.response["result"]["mutation"], true);
        assert_eq!(cell.response["result"]["generation"], generation);
        assert_eq!(cell.response["result"]["state"], *expected_state);
        assert_eq!(cell.response["result"]["receipt"]["sequence"], generation);
        assert_eq!(
            cell.response["result"]["receipt"]["state_digest"],
            ik::canonical::digest_json(expected_state)
        );
        assert_ne!(
            cell.after_show["result"]["capability"]["nonce"],
            cell.before_show["result"]["capability"]["nonce"]
        );
    } else {
        assert_eq!(
            cell.after, cell.before,
            "refusal changes no typed durable cell"
        );
        assert_eq!(
            cell.after_show["result"], cell.before_show["result"],
            "refusal preserves capability and projection"
        );
    }
    assert_eq!(
        cell.journal.len(),
        history.len(),
        "one event per atomic transition, no partial joint"
    );
    assert_eq!(cell.receipts.len(), history.len());
    for (sequence, ((expected_command, state), (event, receipt))) in history
        .iter()
        .zip(cell.journal.iter().zip(&cell.receipts))
        .enumerate()
    {
        assert_eq!(event["sequence"], sequence);
        assert_eq!(event["generation"], sequence);
        assert_eq!(event["state_digest"], ik::canonical::digest_json(state));
        assert_eq!(receipt["sequence"], sequence);
        assert_eq!(receipt["generation"], sequence);
        assert_eq!(receipt["state_digest"], ik::canonical::digest_json(state));
        assert_eq!(receipt["event_hash"], event["hash"]);
        if sequence > 0 {
            assert_eq!(event["event_kind"], "domain_command");
            assert_eq!(event["command"], *expected_command);
        }
    }
    assert_eq!(cell.verify_code, 0, "{}", cell.verified);
    assert_eq!(cell.verified["result"]["journal_ok"], true);
    assert_eq!(cell.verified["result"]["projection_ok"], true);
    assert_eq!(cell.verified["result"]["verified"], true);
    assert_eq!(cell.replay_code, 0, "{}", cell.replayed);
    assert_eq!(
        cell.replayed["result"]["aggregates"],
        json!([{
            "aggregate_id":"finite-model", "sequence":generation, "generation":generation,
            "state":expected_state, "rebuilt":false,
        }])
    );
    assert_eq!(
        cell.after_replay, cell.after,
        "valid replay is byte-preserving"
    );
}

#[derive(Clone, Copy, Debug)]
enum Outcome {
    Commit(&'static str),
    Refuse(&'static str, &'static str),
}

fn parallel_state(label: &str) -> Value {
    match label {
        "PU" => {
            json!({"state":"playing__unmuted","region_playback":"playing","region_volume":"unmuted"})
        }
        "AU" => {
            json!({"state":"paused__unmuted","region_playback":"paused","region_volume":"unmuted"})
        }
        "AM" => json!({"state":"paused__muted","region_playback":"paused","region_volume":"muted"}),
        _ => panic!("not in the independently declared safe domain: {label}"),
    }
}

#[test]
fn every_safe_parallel_configuration_and_command_matches_the_durable_relation() {
    use Outcome::{Commit as C, Refuse as R};
    // Mathematical oracle (docs/region-lowering.md): domain S={PU,AU,AM},
    // P=playing, A=paused, U=unmuted, M=muted; PM is forbidden.
    // Rows enumerate S, columns enumerate all authored local/joint commands.
    // The relation is authored directly; it does not inspect lowered branches.
    let commands = ["pause", "play", "mute", "unmute", "pause_and_mute"];
    let relation = [
        (
            "PU",
            [
                C("AU"),
                R("IK_ILLEGAL_STATE", "play"),
                R("IK_INVARIANT_FAILED", "no_playing_muted"),
                R("IK_ILLEGAL_STATE", "unmute"),
                C("AM"),
            ],
        ),
        (
            "AU",
            [
                R("IK_ILLEGAL_STATE", "pause"),
                C("PU"),
                C("AM"),
                R("IK_ILLEGAL_STATE", "unmute"),
                R("IK_ILLEGAL_STATE", "pause_and_mute"),
            ],
        ),
        (
            "AM",
            [
                R("IK_ILLEGAL_STATE", "pause"),
                R("IK_INVARIANT_FAILED", "no_playing_muted"),
                R("IK_ILLEGAL_STATE", "mute"),
                C("AU"),
                R("IK_ILLEGAL_STATE", "pause_and_mute"),
            ],
        ),
    ];
    for (source, outcomes) in relation {
        let prefix = match source {
            "PU" => vec![],
            "AU" => vec![(json!({"name":"pause"}), parallel_state("AU"))],
            "AM" => vec![(json!({"name":"pause_and_mute"}), parallel_state("AM"))],
            _ => unreachable!(),
        };
        for (name, outcome) in commands.into_iter().zip(outcomes) {
            let command = json!({"name":name});
            let cell = cli_cell(PARALLEL, &parallel_state("PU"), &prefix, &command, None);
            let expected = match outcome {
                C(target) => {
                    assert_eq!(cell.code, 0, "{source} x {name}: {}", cell.response);
                    assert_eq!(cell.response["ok"], true);
                    parallel_state(target)
                }
                R(code, rule) => {
                    assert_eq!(cell.code, 1, "{source} x {name}: {}", cell.response);
                    assert_eq!(cell.response["ok"], false);
                    assert_eq!(cell.response["diagnostic"]["code"], code);
                    if code == "IK_INVARIANT_FAILED" {
                        assert_eq!(cell.response["diagnostic"]["rule"], rule);
                    }
                    parallel_state(source)
                }
            };
            assert_eq!(
                cell.after_show["result"]["state"], expected,
                "{source} x {name}"
            );
            assert_durable_cell(&cell, &command, &expected, false, matches!(outcome, C(_)));
        }
    }
}

fn power_state(leaf: &str) -> Value {
    json!({"state":leaf,"region_power":leaf})
}

#[test]
fn every_compound_leaf_command_and_guard_case_matches_the_durable_relation() {
    use Outcome::{Commit as C, Refuse as R};
    // S={grinding,brewing,ready,off}. Columns are brew, finish, power_off
    // false, power_off true, and power_off with a declared but unavailable
    // guard fact. Error is NOT false. The child guard is only applicable at
    // grinding; brewing/ready use parent, and off has no commands.
    let unavailable = format!(
        "{}\n[facts.flag]\ntype = 'bool'\npolicy = 'replace'\n",
        COMPOUND.replace("$override_enabled == true", "flag == true"),
    );
    let commands = [
        json!({"name":"brew"}),
        json!({"name":"finish"}),
        json!({"name":"power_off","args":{"override_enabled":false}}),
        json!({"name":"power_off","args":{"override_enabled":true}}),
        json!({"name":"power_off","args":{"override_enabled":false}}),
    ];
    let relation = [
        (
            "grinding",
            [
                C("brewing"),
                R("IK_ILLEGAL_STATE", "finish"),
                C("off"),
                C("ready"),
                R("IK_INVARIANT_FAILED", "unavailable flag"),
            ],
        ),
        (
            "brewing",
            [
                R("IK_ILLEGAL_STATE", "brew"),
                C("ready"),
                C("off"),
                C("off"),
                C("off"),
            ],
        ),
        (
            "ready",
            [
                R("IK_ILLEGAL_STATE", "brew"),
                R("IK_ILLEGAL_STATE", "finish"),
                C("off"),
                C("off"),
                C("off"),
            ],
        ),
        (
            "off",
            [
                R("IK_ILLEGAL_STATE", "terminal"),
                R("IK_ILLEGAL_STATE", "terminal"),
                R("IK_ILLEGAL_STATE", "terminal"),
                R("IK_ILLEGAL_STATE", "terminal"),
                R("IK_ILLEGAL_STATE", "terminal"),
            ],
        ),
    ];
    for (source, outcomes) in relation {
        let prefix = match source {
            "grinding" => vec![],
            "brewing" => vec![(json!({"name":"brew"}), power_state("brewing"))],
            "ready" => vec![
                (json!({"name":"brew"}), power_state("brewing")),
                (json!({"name":"finish"}), power_state("ready")),
            ],
            "off" => vec![(commands[2].clone(), power_state("off"))],
            _ => unreachable!(),
        };
        for (index, (command, outcome)) in commands.iter().zip(outcomes).enumerate() {
            let text = if index == 4 {
                unavailable.as_str()
            } else {
                COMPOUND
            };
            // Fault-source setup cannot power_off at grinding: use a fixed
            // healthy parent path to reach off without evaluating the child.
            let fault_prefix = vec![
                (json!({"name":"brew"}), power_state("brewing")),
                (commands[2].clone(), power_state("off")),
            ];
            let setup = if source == "off" && index == 4 {
                &fault_prefix
            } else {
                &prefix
            };
            let cell = cli_cell(text, &power_state("grinding"), setup, command, None);
            let expected = match outcome {
                C(target) => {
                    assert_eq!(cell.code, 0, "{source} x {index}: {}", cell.response);
                    assert_eq!(cell.response["ok"], true);
                    power_state(target)
                }
                R(code, "unavailable flag") => {
                    assert_eq!(
                        cell.code, 1,
                        "guard errors must not fall back: {}",
                        cell.response
                    );
                    assert_eq!(cell.response["ok"], false);
                    assert_eq!(cell.response["diagnostic"]["code"], code);
                    let message = cell.response["diagnostic"]["message"].as_str().unwrap();
                    assert!(
                        message.contains("could not be evaluated") && message.contains("flag"),
                        "{message}"
                    );
                    power_state(source)
                }
                R(code, _) => {
                    assert_eq!(cell.code, 1, "{source} x {index}: {}", cell.response);
                    assert_eq!(cell.response["ok"], false);
                    assert_eq!(cell.response["diagnostic"]["code"], code);
                    power_state(source)
                }
            };
            assert_eq!(
                cell.after_show["result"]["state"], expected,
                "{source} x {index}"
            );
            assert_durable_cell(
                &cell,
                command,
                &expected,
                expected["state"] == "off",
                matches!(outcome, C(_)),
            );
        }
    }
}

const TERMINAL_SIBLING: &str = r#"schema_version = 2
machine_id = "terminal_model"
machine_version = 1
[regions.left.states]
open = { initial = true }
closed = { terminal = true }
[regions.right.states]
idle = { initial = true }
busy = {}
[regions.left.commands.close]
from = ["open"]
to = "closed"
[commands.work]
from = { right = ["idle"] }
to = { right = "busy" }
[regions.right.commands.reset]
from = ["busy"]
to = "idle"
[commands.reopen]
from = { right = ["idle"] }
to = { left = "open", right = "busy" }
[commands.touch_closed]
from = { right = ["idle"] }
to = { left = "closed", right = "busy" }
"#;

fn sibling_state(label: &str) -> Value {
    let (left, right) = match label {
        "OI" => ("open", "idle"),
        "OB" => ("open", "busy"),
        "CI" => ("closed", "idle"),
        "CB" => ("closed", "busy"),
        _ => panic!("outside terminal sibling domain"),
    };
    json!({"state":format!("{left}__{right}"),"region_left":left,"region_right":right})
}

#[test]
fn every_terminal_sibling_configuration_refuses_resurrection_and_preserves_omitted_siblings() {
    use Outcome::{Commit as C, Refuse as R};
    // Independent relation over {open,closed} x {idle,busy}. A closed left
    // component stays closed forever; work/reset may still change the right.
    // Reopen and even touch_closed are illegal once their target is completed.
    let commands = ["close", "work", "reset", "reopen", "touch_closed"];
    let relation = [
        (
            "OI",
            [
                C("CI"),
                C("OB"),
                R("IK_ILLEGAL_STATE", "reset"),
                C("OB"),
                C("CB"),
            ],
        ),
        (
            "OB",
            [
                C("CB"),
                R("IK_ILLEGAL_STATE", "work"),
                C("OI"),
                R("IK_ILLEGAL_STATE", "reopen"),
                R("IK_ILLEGAL_STATE", "touch_closed"),
            ],
        ),
        (
            "CI",
            [
                R("IK_ILLEGAL_STATE", "close"),
                C("CB"),
                R("IK_ILLEGAL_STATE", "reset"),
                R("IK_ILLEGAL_STATE", "reopen"),
                R("IK_ILLEGAL_STATE", "touch_closed"),
            ],
        ),
        (
            "CB",
            [
                R("IK_ILLEGAL_STATE", "close"),
                R("IK_ILLEGAL_STATE", "work"),
                C("CI"),
                R("IK_ILLEGAL_STATE", "reopen"),
                R("IK_ILLEGAL_STATE", "touch_closed"),
            ],
        ),
    ];
    for (source, outcomes) in relation {
        let prefix = match source {
            "OI" => vec![],
            "OB" => vec![(json!({"name":"work"}), sibling_state("OB"))],
            "CI" => vec![(json!({"name":"close"}), sibling_state("CI"))],
            "CB" => vec![
                (json!({"name":"close"}), sibling_state("CI")),
                (json!({"name":"work"}), sibling_state("CB")),
            ],
            _ => unreachable!(),
        };
        for (name, outcome) in commands.into_iter().zip(outcomes) {
            let command = json!({"name":name});
            let cell = cli_cell(
                TERMINAL_SIBLING,
                &sibling_state("OI"),
                &prefix,
                &command,
                None,
            );
            let expected = match outcome {
                C(target) => {
                    assert_eq!(cell.code, 0, "{source} x {name}: {}", cell.response);
                    sibling_state(target)
                }
                R(code, _) => {
                    assert_eq!(cell.code, 1, "{source} x {name}: {}", cell.response);
                    assert_eq!(cell.response["diagnostic"]["code"], code);
                    sibling_state(source)
                }
            };
            assert_eq!(cell.response["ok"], matches!(outcome, C(_)));
            assert_eq!(
                cell.after_show["result"]["state"], expected,
                "{source} x {name}"
            );
            assert_durable_cell(&cell, &command, &expected, false, matches!(outcome, C(_)));
        }
    }
}

const THREE_REGION: &str = r#"schema_version = 2
machine_id = "three_region_model"
machine_version = 1
[regions.power.states]
grinding = { initial = true }
brewing = {}
ready = {}
off = { terminal = true }
[regions.power.commands.brew]
from = ["grinding"]
to = "brewing"
[regions.power.commands.finish]
from = ["brewing"]
to = "ready"
[regions.power.commands.power_off]
from = ["grinding"]
to = "ready"
args = { override_enabled = "bool" }
when = '$override_enabled == true && region_volume == "unmuted"'
[regions.power.parent]
name = "on"
children = ["grinding", "brewing", "ready"]
[regions.power.parent.commands.power_off]
from = ["on"]
to = "off"
args = { override_enabled = "bool" }
[regions.volume.states]
unmuted = { initial = true }
muted = {}
[regions.volume.commands.mute]
from = ["unmuted"]
to = "muted"
[regions.volume.commands.unmute]
from = ["muted"]
to = "unmuted"
[regions.audit.states]
open = { initial = true }
closed = { terminal = true }
[regions.audit.commands.seal]
from = ["open"]
to = "closed"
[commands.start_muted]
from = { power = ["grinding"], volume = ["unmuted"] }
to = { power = "brewing", volume = "muted" }
when = 'region_audit == "open"'
"#;

fn three_region_state(power: &str, volume: &str, audit: &str) -> Value {
    // Canonical component order is lexical by region name, not declaration order.
    json!({
        "state":format!("{audit}__{power}__{volume}"),
        "region_audit":audit,
        "region_power":power,
        "region_volume":volume,
    })
}

#[test]
fn every_three_region_hierarchy_joint_and_terminal_cell_matches_the_durable_relation() {
    // Independent source-semantics oracle over all 4 x 2 x 2 configurations
    // and eight commands (128 cells). The child override depends on a sibling;
    // the joint guard depends on its omitted third region. No lowered branch,
    // generated chart, or observed runtime target supplies an expectation.
    let commands = [
        json!({"name":"brew"}),
        json!({"name":"finish"}),
        json!({"name":"power_off","args":{"override_enabled":false}}),
        json!({"name":"power_off","args":{"override_enabled":true}}),
        json!({"name":"mute"}),
        json!({"name":"unmute"}),
        json!({"name":"seal"}),
        json!({"name":"start_muted"}),
    ];
    let initial = three_region_state("grinding", "unmuted", "open");
    for power in ["grinding", "brewing", "ready", "off"] {
        for volume in ["unmuted", "muted"] {
            for audit in ["open", "closed"] {
                // Reach power first while both siblings are initial, then set
                // volume and audit. Prefix states are mathematical assignments,
                // independent of the candidate's outcome-selection machinery.
                let mut prefix = match power {
                    "grinding" => vec![],
                    "brewing" => vec![(
                        json!({"name":"brew"}),
                        three_region_state("brewing", "unmuted", "open"),
                    )],
                    "ready" => vec![
                        (
                            json!({"name":"brew"}),
                            three_region_state("brewing", "unmuted", "open"),
                        ),
                        (
                            json!({"name":"finish"}),
                            three_region_state("ready", "unmuted", "open"),
                        ),
                    ],
                    "off" => vec![(
                        json!({"name":"power_off","args":{"override_enabled":false}}),
                        three_region_state("off", "unmuted", "open"),
                    )],
                    _ => unreachable!(),
                };
                if volume == "muted" {
                    prefix.push((
                        json!({"name":"mute"}),
                        three_region_state(power, "muted", "open"),
                    ));
                }
                if audit == "closed" {
                    prefix.push((
                        json!({"name":"seal"}),
                        three_region_state(power, volume, "closed"),
                    ));
                }
                let source = three_region_state(power, volume, audit);
                for command in &commands {
                    let name = command["name"].as_str().unwrap();
                    let outcome = match name {
                        "brew" if power == "grinding" => Ok(("brewing", volume, audit)),
                        "finish" if power == "brewing" => Ok(("ready", volume, audit)),
                        "power_off" if power != "off" => {
                            let target = if power == "grinding"
                                && command["args"]["override_enabled"] == true
                                && volume == "unmuted"
                            {
                                "ready"
                            } else {
                                "off"
                            };
                            Ok((target, volume, audit))
                        }
                        "mute" if volume == "unmuted" => Ok((power, "muted", audit)),
                        "unmute" if volume == "muted" => Ok((power, "unmuted", audit)),
                        "seal" if audit == "open" => Ok((power, volume, "closed")),
                        "start_muted" if power == "grinding" && volume == "unmuted" => {
                            if audit == "open" {
                                Ok(("brewing", "muted", audit))
                            } else {
                                // Applicable joint, but its guard is false:
                                // no outcome, not an illegal source state.
                                Err("IK_INVARIANT_FAILED")
                            }
                        }
                        _ => Err("IK_ILLEGAL_STATE"),
                    };
                    let committed = outcome.is_ok();
                    let cell = cli_cell(THREE_REGION, &initial, &prefix, command, None);
                    assert_eq!(cell.before_show["result"]["state"], source);
                    let expected = match outcome {
                        Ok((target_power, target_volume, target_audit)) => {
                            assert_eq!(cell.code, 0, "{source} x {command}: {}", cell.response);
                            three_region_state(target_power, target_volume, target_audit)
                        }
                        Err(code) => {
                            assert_eq!(cell.code, 1, "{source} x {command}: {}", cell.response);
                            assert_eq!(cell.response["diagnostic"]["code"], code);
                            source.clone()
                        }
                    };
                    assert_eq!(cell.response["ok"], committed);
                    assert_eq!(
                        cell.after_show["result"]["state"], expected,
                        "{source} x {command}"
                    );
                    // Volume has no terminal leaf, even when both other
                    // regions are terminal. Local sibling commands still run.
                    assert_durable_cell(&cell, command, &expected, false, committed);
                }
            }
        }
    }
}

#[test]
fn cli_initially_invalid_region_source_cannot_create_any_durable_row() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("invalid-initial.db");
    let definition = dir.path().join("invalid-initial.ik.toml");
    drop(ik::store::Store::create(&store).unwrap());
    let before = durable_rows(&store).unwrap();
    let invalid = PARALLEL.replace(
        "unmuted = { initial = true }\nmuted = {}",
        "unmuted = {}\nmuted = { initial = true }",
    );
    std::fs::write(&definition, &invalid).unwrap();
    let healthy = compile(parse_source(PARALLEL).unwrap().definition).unwrap();
    // Bind the identity the unsafe source would have if initial validation were
    // missing, so an identity mismatch cannot masquerade as safety enforcement.
    // These are the only authored changes: initial volume and initial product.
    let mut unsafe_lowered = healthy.definition().clone();
    for (label, state) in &mut unsafe_lowered.states {
        state.initial = label == "playing__muted";
    }
    unsafe_lowered
        .facts
        .get_mut("region_volume")
        .unwrap()
        .initial = Some(json!("muted"));
    let unsafe_machine = compile(unsafe_lowered).unwrap();
    assert_eq!(
        unsafe_machine.initial_state(),
        json!({
            "state":"playing__muted","region_playback":"playing","region_volume":"muted",
        })
    );
    let req = request(
        &serde_json::to_value(unsafe_machine.identity()).unwrap(),
        "create",
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("--store")
        .arg(&store)
        .arg("--machine")
        .arg(&definition)
        .arg("create")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(&req).unwrap().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("initial region configuration violates invariant")
            && stderr.contains("IK_INVARIANT_FAILED")
            && stderr.contains("no_playing_muted"),
        "creation must refuse for the initial safety condition, not identity mismatch: {stderr}"
    );
    assert_eq!(
        durable_rows(&store).unwrap(),
        before,
        "no aggregate/genesis/receipt/capability may be minted"
    );
    for table in ["aggregates", "journal", "receipts"] {
        assert_eq!(
            before[table],
            Vec::<Vec<SqlValue>>::new(),
            "test begins with no authority rows"
        );
    }
    // The same unchanged store can still create the valid source at generation
    // zero: refusal did not consume the aggregate id or poison store metadata.
    std::fs::write(&definition, PARALLEL).unwrap();
    let req = request(&serde_json::to_value(healthy.identity()).unwrap(), "create");
    let (created, code) = envelope(&store, Some(&definition), &req);
    assert_eq!(code, 0, "{created}");
    assert_eq!(created["result"]["generation"], 0);
    assert_eq!(created["result"]["receipt"]["sequence"], 0);
    assert_eq!(created["result"]["state"], parallel_state("PU"));
}

#[test]
fn cli_corrupted_durable_source_facts_and_labels_refuse_without_healing_or_writing() {
    let initial = parallel_state("PU");
    let cases = [
        (
            json!({"state":"playing__unmuted","region_playback":"playing","region_volume":"muted"}),
            "pause",
        ),
        (
            json!({"state":"playing__unmuted","region_playback":"paused","region_volume":"unmuted"}),
            "pause",
        ),
        (
            json!({"state":"paused__unmuted","region_playback":"playing","region_volume":"unmuted"}),
            "play",
        ),
    ];
    for (corrupted, name) in cases {
        let command = json!({"name":name});
        let cell = cli_cell(PARALLEL, &initial, &[], &command, Some(&corrupted));
        assert_eq!(cell.code, 1, "{corrupted} x {name}: {}", cell.response);
        assert_eq!(cell.response["ok"], false);
        assert_eq!(cell.response["diagnostic"]["code"], "IK_INVARIANT_FAILED");
        assert_eq!(cell.response["diagnostic"]["rule"], format!("{name}.when"));
        assert!(cell.response["diagnostic"]["message"]
            .as_str()
            .unwrap()
            .contains("no outcome"));
        assert_eq!(cell.before_show["result"]["state"], corrupted);
        assert_eq!(
            cell.after_show["result"], cell.before_show["result"],
            "command cannot heal corrupted source facts"
        );
        assert_eq!(cell.after_show["result"]["generation"], 0);
        assert_eq!(
            cell.after, cell.before,
            "refusal preserves corrupted projection as evidence"
        );
        assert_eq!(cell.journal.len(), 1);
        assert_eq!(
            cell.journal[0]["state_digest"],
            ik::canonical::digest_json(&initial)
        );
        assert_eq!(cell.receipts.len(), 1);
        assert_eq!(
            cell.verify_code, 1,
            "verification must detect corruption before repair"
        );
        assert_eq!(cell.verified["diagnostic"]["code"], "IK_PROJECTION_CORRUPT");
        assert_eq!(cell.verified["diagnostic"]["rule"], "projection.replay");
        assert_eq!(cell.replay_code, 0, "{}", cell.replayed);
        assert_eq!(
            cell.replayed["result"]["aggregates"],
            json!([{
                "aggregate_id":"finite-model","sequence":0,"generation":0,
                "state":initial,"rebuilt":true,
            }])
        );
        for table in ["meta", "journal", "receipts"] {
            assert_eq!(
                cell.after_replay[table], cell.before[table],
                "replay repairs projection only, preserving {table}"
            );
        }
    }
}

#[test]
fn cli_corrupted_lowered_poststate_facts_and_labels_refuse_before_durable_commit() {
    // Fault-inject a serialized schema-1 definition through the existing CLI
    // loader. Source guards remain correct; only the proposed poststate is
    // inconsistent. This distinguishes source validation from apply validation.
    for (field, replacement, rule) in [
        (
            "region_playback",
            "\"playing\"",
            "region_projection_paused__unmuted",
        ),
        (
            "region_volume",
            "\"muted\"",
            "region_projection_paused__unmuted",
        ),
        (
            "state",
            "playing__unmuted",
            "region_projection_playing__unmuted",
        ),
    ] {
        let mut definition = parse_source(PARALLEL).unwrap().definition;
        let outcomes = definition
            .commands
            .get_mut("pause")
            .unwrap()
            .outcomes
            .as_mut()
            .unwrap();
        for outcome in outcomes {
            if field == "state" {
                outcome.to = replacement.to_string();
            } else {
                outcome
                    .then
                    .as_mut()
                    .unwrap()
                    .insert(field.into(), replacement.into());
            }
        }
        let source = toml::to_string(&definition).unwrap();
        let command = json!({"name":"pause"});
        let initial = parallel_state("PU");
        let cell = cli_cell(&source, &initial, &[], &command, None);
        assert_eq!(cell.code, 1, "{field}: {}", cell.response);
        assert_eq!(cell.response["ok"], false);
        assert_eq!(cell.response["diagnostic"]["code"], "IK_INVARIANT_FAILED");
        assert_eq!(cell.response["diagnostic"]["rule"], rule);
        assert_eq!(
            cell.after_show["result"]["state"], initial,
            "poisoned poststate is never installed"
        );
        assert_durable_cell(&cell, &command, &initial, false, false);
    }
}
