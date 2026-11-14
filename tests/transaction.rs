//! `transaction` — normative, hermetic tests for IK-PROP-012..016. Every test uses a fresh
//! temporary store and the repo's own declarative machines (no Python, no network, no sleeps, no
//! wall-clock dependence). Row-level standard: refusal and crash cases compare the canonical
//! serialization of EVERY durable row, not just projection fields.
//!
//! ```text
//! IK-PROP-012 a transaction is observationally equal to the same commands run one by one
//!             (journal, receipts, projection, capability chain, replay/live)  -> ik_prop_012_*
//! IK-PROP-013 a failing step rolls back every step: zero durable-row mutation;
//!             mutation control: deleting the law (commit the prefix) is detected -> ik_prop_013_*
//! IK-PROP-014 a transaction cannot forge or replay authority: stale starting
//!             capability, foreign machine and per-step capabilities all refuse     -> ik_prop_014_*
//! IK-PROP-015 only the last step may create an effect intent                       -> ik_prop_015_*
//! IK-PROP-016 a REAL process death at any persistence boundary inside a transaction
//!             leaves exactly the previous committed state; the retry commits once  -> ik_prop_016_*
//! ```

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::diagnostic::DiagnosticCode;
use ik::effect::EffectStatus;
use ik::identity::{AggregateId, Capability, MachineIdentity};
use ik::machine::Machine;
use ik::protocol::TransactionStep;
use ik::store::{CommandRequest, Store, TransactionRequest};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn va() -> Box<dyn Machine> {
    Box::new(load_machine("verification_attempt"))
}

fn probe() -> Box<dyn Machine> {
    Box::new(load_machine("effect_probe"))
}

fn cap(identity: &MachineIdentity, agg: &AggregateId, generation: u64) -> Capability {
    Capability {
        generation,
        nonce: ik::runtime::derive_nonce(identity, agg, generation),
    }
}

fn fresh(dir: &tempfile::TempDir, machine: &dyn Machine) -> (PathBuf, Store, AggregateId) {
    let path = dir.path().join("s.db");
    let mut store = Store::create(&path).expect("create store");
    let agg = AggregateId("agg-1".into());
    store
        .create_aggregate(machine, &agg, machine.identity())
        .expect("create aggregate");
    (path, store, agg)
}

fn start() -> Value {
    json!({"name": "start"})
}
fn submit() -> Value {
    json!({"name": "submit", "args": {"artifact_digest": "a1", "baseline_digest": "b1"}})
}
fn verify() -> Value {
    json!({"name": "verify", "args": {
        "receipt_digest": "r1", "receipt_artifact_digest": "a1", "receipt_baseline_digest": "b1"}})
}
fn steps(commands: &[Value]) -> Vec<TransactionStep> {
    commands
        .iter()
        .map(|command| TransactionStep {
            command: command.clone(),
            observation: None,
        })
        .collect()
}

fn transact(
    store: &mut Store,
    machine: &dyn Machine,
    agg: &AggregateId,
    capability: Capability,
    commands: &[Value],
) -> Result<Vec<ik::store::CommandResult>, ik::diagnostic::Diagnostic> {
    store.transaction(
        machine,
        &TransactionRequest {
            aggregate_id: agg.clone(),
            machine: machine.identity().clone(),
            capability,
            steps: steps(commands),
        },
    )
}

/// Canonical serialization of every durable row. `with_meta=false` drops the per-store random
/// `store_id` so two independent stores can be compared for observational equality.
fn durable_rows(path: &Path, with_meta: bool) -> String {
    let conn = rusqlite::Connection::open(path).expect("open store for row snapshot");
    let mut parts: Vec<String> = Vec::new();
    let tables: &[(&str, &str)] = &[
        ("meta", "key"),
        ("journal", "aggregate_id, sequence"),
        ("receipts", "aggregate_id, sequence"),
        ("aggregates", "aggregate_id"),
    ];
    for (table, order) in tables {
        if *table == "meta" && !with_meta {
            continue;
        }
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
            .expect("prepare");
        let columns = stmt.column_count();
        let rows = stmt
            .query_map([], |row| {
                Ok((0..columns)
                    .map(|index| {
                        format!("{:?}", row.get::<_, rusqlite::types::Value>(index).unwrap())
                    })
                    .collect::<Vec<_>>()
                    .join(","))
            })
            .expect("query");
        let mut encoded: Vec<String> = rows.map(|row| row.expect("row")).collect();
        encoded.sort();
        parts.push(format!("{table}:{}", encoded.join("|")));
    }
    parts.join("\n")
}

#[test]
fn ik_prop_012_transaction_equals_the_same_commands_run_one_by_one() {
    let commands = [start(), submit(), verify()];

    let dir_a = tempfile::tempdir().unwrap();
    let machine = va();
    let (path_a, mut store_a, agg) = fresh(&dir_a, machine.as_ref());
    let mut capability = cap(machine.identity(), &agg, 0);
    let mut last = None;
    for command in &commands {
        let result = store_a
            .command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: machine.identity().clone(),
                    capability: capability.clone(),
                    command: command.clone(),
                    observation: None,
                },
            )
            .expect("sequential command");
        capability = result.capability.clone();
        last = Some(result);
    }
    let last = last.unwrap();

    let dir_b = tempfile::tempdir().unwrap();
    let (path_b, mut store_b, agg_b) = fresh(&dir_b, machine.as_ref());
    let results = transact(
        &mut store_b,
        machine.as_ref(),
        &agg_b,
        cap(machine.identity(), &agg_b, 0),
        &commands,
    )
    .expect("transaction commits");

    assert_eq!(results.len(), commands.len());
    for (index, result) in results.iter().enumerate() {
        assert_eq!(
            result.generation,
            index as u64 + 1,
            "one generation per committed step"
        );
        assert!(result.mutation && result.receipt.is_some());
    }
    let final_result = results.last().unwrap();
    assert_eq!(final_result.state, last.state);
    assert_eq!(
        final_result.capability, last.capability,
        "capability chain differs"
    );
    assert!(final_result.terminal);
    assert_eq!(
        durable_rows(&path_a, false),
        durable_rows(&path_b, false),
        "transaction rows differ from sequential rows"
    );
    // Verification passes and replay reproduces the live projection WITHOUT rebuilding it.
    let summary = store_b
        .verify(machine.as_ref(), Some(&agg_b))
        .expect("verify");
    assert!(summary.verified && summary.journal_ok && summary.projection_ok);
    let replayed = store_b
        .replay(machine.as_ref(), Some(&agg_b))
        .expect("replay");
    assert_eq!(replayed.aggregates.len(), 1);
    assert!(
        !replayed.aggregates[0].rebuilt,
        "replay had to rebuild the projection"
    );
    assert_eq!(replayed.aggregates[0].state, final_result.state);
    assert_eq!(replayed.aggregates[0].generation, 3);
}

#[test]
fn ik_prop_013_failed_step_rolls_back_every_step_with_zero_row_mutation() {
    let machine = va();
    for (label, commands, failing_index) in [
        ("illegal second step", vec![start(), verify()], 1usize),
        (
            "unknown first step",
            vec![json!({"name": "teleport"}), start()],
            0,
        ),
        ("illegal third step", vec![start(), submit(), submit()], 2),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (path, mut store, agg) = fresh(&dir, machine.as_ref());
        let before = durable_rows(&path, true);
        let error = transact(
            &mut store,
            machine.as_ref(),
            &agg,
            cap(machine.identity(), &agg, 0),
            &commands,
        )
        .expect_err(label);
        assert!(
            error
                .message
                .starts_with(&format!("transaction step {failing_index}:")),
            "{label}: message must name the failing step: {}",
            error.message
        );
        assert_eq!(
            durable_rows(&path, true),
            before,
            "{label}: refusal mutated durable rows"
        );
        // The aggregate is still at generation 0 with the ORIGINAL capability: the very same
        // authority can still be used, proving nothing rotated.
        let ok = transact(
            &mut store,
            machine.as_ref(),
            &agg,
            cap(machine.identity(), &agg, 0),
            &[start()],
        )
        .expect("original capability still valid after the refused transaction");
        assert_eq!(ok[0].generation, 1);
    }
}

/// Mutation control: with the atomicity law deleted (commit the already-applied prefix when a
/// later step fails) the SAME scenario must observably persist a partial transaction, proving
/// `ik_prop_013_*` would fail if the wall were removed.
#[cfg(feature = "test-support")]
#[test]
fn ik_prop_013_mutation_control_commit_partial_transaction_is_detected() {
    let machine = va();
    let dir = tempfile::tempdir().unwrap();
    let (path, mut store, agg) = fresh(&dir, machine.as_ref());
    let before = durable_rows(&path, true);
    let outcome = ik::law_mutant::with_law_mutant(
        ik::law_mutant::LawMutant::CommitPartialTransaction,
        || {
            transact(
                &mut store,
                machine.as_ref(),
                &agg,
                cap(machine.identity(), &agg, 0),
                &[start(), verify()],
            )
        },
    );
    assert!(
        outcome.is_err(),
        "the failing step still reports its refusal"
    );
    assert_ne!(
        durable_rows(&path, true),
        before,
        "the mutant must persist the prefix; if rows are unchanged the atomicity test is blind"
    );
}

#[test]
fn ik_prop_014_transaction_cannot_forge_or_replay_authority() {
    let machine = va();
    let dir = tempfile::tempdir().unwrap();
    let (path, mut store, agg) = fresh(&dir, machine.as_ref());
    let first = transact(
        &mut store,
        machine.as_ref(),
        &agg,
        cap(machine.identity(), &agg, 0),
        &[start()],
    )
    .expect("first transaction");
    assert_eq!(first[0].generation, 1);
    let before = durable_rows(&path, true);

    // Replaying the pre-commit (generation 0) capability is stale: refused, nothing written.
    let stale = transact(
        &mut store,
        machine.as_ref(),
        &agg,
        cap(machine.identity(), &agg, 0),
        &[submit()],
    )
    .expect_err("stale capability");
    assert_eq!(stale.code, DiagnosticCode::IkCapabilityStale);
    assert_eq!(durable_rows(&path, true), before);

    // A foreign machine identity is refused before any step runs.
    let other = probe();
    let foreign = store
        .transaction(
            other.as_ref(),
            &TransactionRequest {
                aggregate_id: agg.clone(),
                machine: other.identity().clone(),
                capability: first[0].capability.clone(),
                steps: steps(&[submit()]),
            },
        )
        .expect_err("foreign machine");
    assert_eq!(foreign.code, DiagnosticCode::IkMachineMismatch);
    assert_eq!(durable_rows(&path, true), before);
}

#[test]
fn ik_prop_014_wire_refuses_per_step_capabilities_and_malformed_batches() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("s.db");
    let machine = machine_def("verification_attempt");
    let identity = serde_json::to_value(identity_of("verification_attempt")).unwrap();
    let envelope = |op: &str, request_id: &str, extra: Value| {
        let mut request = json!({
            "protocol": {"major": 1, "minor": 0},
            "runtime_version": ik::protocol::RUNTIME_VERSION,
            "request_id": request_id,
            "op": op,
        });
        for (key, value) in extra.as_object().unwrap() {
            request[key] = value.clone();
        }
        request
    };
    let created = run_cli_json(
        &store,
        Some(&machine),
        &envelope(
            "create",
            "c0",
            json!({"aggregate_id": "agg-1", "machine": identity}),
        ),
    );
    let capability = created["result"]["capability"].clone();
    let attempt = |request_id: &str, extra: Value| {
        let mut base =
            json!({"aggregate_id": "agg-1", "machine": identity, "capability": capability});
        for (key, value) in extra.as_object().unwrap() {
            base[key] = value.clone();
        }
        run_cli_json(
            &store,
            Some(&machine),
            &envelope("transaction", request_id, base),
        )
    };

    for (label, extra) in [
        (
            "per-step capability",
            json!({"commands": [{"command": start(), "capability": capability}]}),
        ),
        ("missing commands", json!({})),
        ("empty commands", json!({"commands": []})),
        (
            "top-level command alongside commands",
            json!({"command": start(), "commands": [{"command": start()}]}),
        ),
        (
            "more than 64 steps",
            json!({"commands": (0..65).map(|_| json!({"command": start()})).collect::<Vec<_>>()}),
        ),
    ] {
        let response = attempt("t", extra);
        assert_eq!(response["ok"], false, "{label}: {response}");
        assert_eq!(
            response["diagnostic"]["code"], "IK_PROTOCOL_INVALID",
            "{label}: {response}"
        );
    }
    let stray = run_cli_json(
        &store,
        Some(&machine),
        &envelope(
            "command",
            "t2",
            json!({"aggregate_id": "agg-1", "machine": identity, "capability": capability,
                   "command": start(), "commands": [{"command": start()}]}),
        ),
    );
    assert_eq!(stray["diagnostic"]["code"], "IK_PROTOCOL_INVALID");
    // None of the refusals touched the aggregate.
    let shown = run_cli_json(
        &store,
        Some(&machine),
        &envelope("show", "s", json!({"aggregate_id": "agg-1"})),
    );
    assert_eq!(shown["result"]["generation"], 0, "{shown}");

    // And the well-formed transaction over the same wire commits atomically (3 steps).
    let ok = attempt(
        "t3",
        json!({"commands": [{"command": start()}, {"command": submit()}, {"command": verify()}]}),
    );
    assert_eq!(ok["ok"], true, "{ok}");
    assert_eq!(ok["result"]["generation"], 3);
    assert_eq!(ok["result"]["steps"].as_array().unwrap().len(), 3);
    assert_eq!(ok["result"]["terminal"], true);
}

#[test]
fn ik_prop_015_only_the_last_step_may_create_an_effect_intent() {
    let machine = probe();
    let plan = json!({"name": "plan", "args": {
        "effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1"}});
    let finish = json!({"name": "finish", "args": {"result_digest": "r1"}});

    let dir = tempfile::tempdir().unwrap();
    let (path, mut store, agg) = fresh(&dir, machine.as_ref());
    let before = durable_rows(&path, true);
    let error = transact(
        &mut store,
        machine.as_ref(),
        &agg,
        cap(machine.identity(), &agg, 0),
        &[plan.clone(), finish],
    )
    .expect_err("effect-creating step before the last step");
    assert_eq!(error.code, DiagnosticCode::IkTransactionBoundary);
    assert!(
        error.message.starts_with("transaction step 0:"),
        "{}",
        error.message
    );
    assert_eq!(
        durable_rows(&path, true),
        before,
        "boundary refusal mutated rows"
    );

    // The same effect-creating command as the LAST step commits and returns the pending intent.
    let ok = transact(
        &mut store,
        machine.as_ref(),
        &agg,
        cap(machine.identity(), &agg, 0),
        &[plan],
    )
    .expect("effect step last");
    assert!(matches!(ok[0].effect, EffectStatus::Pending(_)));
}

// --------------------------------------------------------------------------------------------
// IK-PROP-016: real process death at every persistence boundary INSIDE a transaction.
// The test binary re-enters itself as a child (as in tests/crash_matrix.rs); the child opens the
// store with an explicitly injected single-use crash hook and dies at the Nth occurrence of one
// named boundary. The parent blocks on process exit (sleep-free) and then asserts the store.
// --------------------------------------------------------------------------------------------

#[cfg(feature = "test-support")]
#[test]
fn transaction_crash_child_entry() {
    if std::env::var("IK_TX_CRASH_CHILD").is_err() {
        return; // ordinary parent-side invocation of the test binary
    }
    let path = PathBuf::from(std::env::var("IK_TX_CRASH_STORE").unwrap());
    let point = std::env::var("IK_TX_CRASH_POINT").unwrap();
    let occurrence: u32 = std::env::var("IK_TX_CRASH_OCCURRENCE")
        .unwrap()
        .parse()
        .unwrap();
    let mut seen = 0u32;
    let hook: ik::store::CrashHook = Box::new(move |name: &str| {
        if name == point {
            seen += 1;
            if seen == occurrence {
                std::process::abort();
            }
        }
        false
    });
    let mut store = Store::open_with_crash_hook(&path, hook).expect("child opens store");
    let machine = va();
    let agg = AggregateId("agg-1".into());
    let _ = transact(
        &mut store,
        machine.as_ref(),
        &agg,
        cap(machine.identity(), &agg, 0),
        &[start(), submit(), verify()],
    );
    // Reaching here means the armed boundary never fired: the parent treats that as a failure.
    std::fs::write(path.with_extension("done"), "completed").unwrap();
}

#[cfg(feature = "test-support")]
#[test]
fn ik_prop_016_process_death_inside_a_transaction_leaves_exactly_the_previous_state() {
    use std::os::unix::process::ExitStatusExt;
    let machine = va();
    for point in [
        "crash.transaction.after_journal",
        "crash.transaction.after_receipts",
        "crash.transaction.after_projection",
    ] {
        // occurrence 1 = inside the first step's persist; 3 = inside the LAST step's persist,
        // after two earlier steps were already written inside the same open transaction.
        for occurrence in [1u32, 2, 3] {
            let dir = tempfile::tempdir().unwrap();
            let (path, store, agg) = fresh(&dir, machine.as_ref());
            drop(store);
            let before = durable_rows(&path, true);

            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "transaction_crash_child_entry", "--nocapture"])
                .env("IK_TX_CRASH_CHILD", "1")
                .env("IK_TX_CRASH_STORE", &path)
                .env("IK_TX_CRASH_POINT", point)
                .env("IK_TX_CRASH_OCCURRENCE", occurrence.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("spawn crash child");
            assert_eq!(
                status.signal(),
                Some(6),
                "{point}#{occurrence}: child must die by SIGABRT"
            );
            assert!(
                !path.with_extension("done").exists(),
                "{point}#{occurrence}: armed boundary never fired"
            );

            assert_eq!(
                durable_rows(&path, true),
                before,
                "{point}#{occurrence}: a torn transaction survived the crash"
            );
            let conn = rusqlite::Connection::open(&path).unwrap();
            let integrity: String = conn
                .query_row("PRAGMA integrity_check", [], |row| row.get(0))
                .unwrap();
            assert_eq!(integrity, "ok");
            drop(conn);

            // Recovery: verify passes, and the identical transaction commits EXACTLY ONCE.
            let mut reopened = Store::open(&path).expect("reopen");
            reopened
                .verify(machine.as_ref(), Some(&agg))
                .expect("verify after crash");
            let results = transact(
                &mut reopened,
                machine.as_ref(),
                &agg,
                cap(machine.identity(), &agg, 0),
                &[start(), submit(), verify()],
            )
            .expect("retry commits");
            assert_eq!(results.last().unwrap().generation, 3);
            let duplicate = transact(
                &mut reopened,
                machine.as_ref(),
                &agg,
                cap(machine.identity(), &agg, 0),
                &[start(), submit(), verify()],
            )
            .expect_err("a second submission of the same transaction must not commit");
            assert_eq!(duplicate.code, DiagnosticCode::IkCapabilityStale);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Bounded resource: a transaction is capped so one SQLite write transaction stays short. The cap
// (64) is wire contract; the test uses a self-looping machine so that ONLY batch size can refuse.
// ---------------------------------------------------------------------------------------------

const TICKER: &str = r#"
schema_version = 1
machine_id = "ticker"
machine_version = 1
name = "Ticker"

[facts]
ticks = { type = "int", policy = "counter", initial = 0 }

[states]
on = { initial = true }
off = { terminal = true }

[commands.tick]
from = ["on"]
to = "on"
then = { ticks = "+1" }

[commands.stop]
from = ["on"]
to = "off"
"#;

fn wire_transaction(
    store: &Path,
    machine_path: &Path,
    machine: &dyn Machine,
    agg: &AggregateId,
    commands: &[Value],
) -> Value {
    let identity = machine.identity();
    let request = json!({
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": ik::protocol::RUNTIME_VERSION,
        "request_id": "t",
        "op": "transaction",
        "aggregate_id": agg.0,
        "machine": identity,
        "capability": cap(identity, agg, 0),
        "commands": commands.iter().map(|c| json!({"command": c})).collect::<Vec<_>>(),
    });
    run_cli_json(store, Some(machine_path), &request)
}

#[test]
fn transaction_step_cap_is_exactly_sixty_four_and_refusal_mutates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let machine_path = dir.path().join("ticker.ik.toml");
    std::fs::write(&machine_path, TICKER).unwrap();
    let machine = ik::declarative::load_def(&machine_path).expect("ticker compiles");
    let tick = json!({"name": "tick"});

    for (len, accepted) in [(0usize, false), (1, true), (64, true), (65, false)] {
        let store_dir = tempfile::tempdir().unwrap();
        let (path, store, agg) = fresh(&store_dir, &machine);
        drop(store);
        let before = durable_rows(&path, true);
        let commands = vec![tick.clone(); len];
        let response = wire_transaction(&path, &machine_path, &machine, &agg, &commands);
        assert_eq!(response["ok"], json!(accepted), "len {len}: {response}");
        if accepted {
            assert_eq!(response["result"]["generation"], json!(len), "len {len}");
        } else {
            assert_eq!(
                durable_rows(&path, true),
                before,
                "len {len}: refusal mutated rows"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// STATE_MACHINE / BROAD_INPUT_SPACE: for ARBITRARY command sequences (legal, illegal, unknown
// names, mismatched args) a transaction is observationally the sequential run when every step
// succeeds, and otherwise the all-or-nothing no-op that names the same failing step. The oracle is
// the independently-authoritative one-command-at-a-time path, not a re-derivation of the
// transaction algorithm.
// ---------------------------------------------------------------------------------------------

use proptest::prelude::*;

fn any_command() -> impl Strategy<Value = Value> {
    prop_oneof![
        3 => Just(start()),
        3 => Just(submit()),
        3 => Just(verify()),
        2 => Just(json!({"name": "reject"})),
        2 => Just(json!({"name": "retry"})),
        1 => Just(json!({"name": "cancel"})),
        1 => Just(json!({"name": "teleport"})),
        1 => Just(json!({"name": "verify", "args": {
            "receipt_digest": "r1", "receipt_artifact_digest": "WRONG", "receipt_baseline_digest": "b1"}})),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, max_shrink_iters: 512, .. ProptestConfig::default() })]

    #[test]
    fn ik_prop_012_arbitrary_sequences_transaction_equals_sequential_or_is_a_named_noop(
        commands in proptest::collection::vec(any_command(), 1..10)
    ) {
        let machine = va();

        let dir_a = tempfile::tempdir().unwrap();
        let (path_a, mut store_a, agg) = fresh(&dir_a, machine.as_ref());
        let mut capability = cap(machine.identity(), &agg, 0);
        let mut failing: Option<(usize, DiagnosticCode)> = None;
        let mut last_state = None;
        for (index, command) in commands.iter().enumerate() {
            match store_a.command(machine.as_ref(), &CommandRequest {
                aggregate_id: agg.clone(), machine: machine.identity().clone(),
                capability: capability.clone(), command: command.clone(), observation: None,
            }) {
                Ok(result) => { capability = result.capability.clone(); last_state = Some(result.state); }
                Err(diagnostic) => { failing = Some((index, diagnostic.code)); break; }
            }
        }

        let dir_b = tempfile::tempdir().unwrap();
        let (path_b, mut store_b, agg_b) = fresh(&dir_b, machine.as_ref());
        let before = durable_rows(&path_b, true);
        let outcome = transact(&mut store_b, machine.as_ref(), &agg_b, cap(machine.identity(), &agg_b, 0), &commands);

        match (failing, outcome) {
            (None, Ok(results)) => {
                prop_assert_eq!(results.len(), commands.len());
                prop_assert_eq!(durable_rows(&path_a, false), durable_rows(&path_b, false));
                prop_assert_eq!(&results.last().unwrap().state, last_state.as_ref().unwrap());
            }
            (Some((index, code)), Err(diagnostic)) => {
                prop_assert_eq!(diagnostic.code, code, "different refusal code than the sequential path");
                let marker = format!("transaction step {index}:");
                prop_assert!(diagnostic.message.contains(&marker), "wrong failing step: {}", diagnostic.message);
                prop_assert_eq!(durable_rows(&path_b, true), before, "failed transaction mutated durable rows");
            }
            (sequential, transaction) => {
                prop_assert!(false, "divergence: sequential failing={sequential:?} transaction ok={}", transaction.is_ok());
            }
        }
    }
}
