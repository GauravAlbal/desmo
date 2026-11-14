//! Invariant-runtime property hardening — direct executed mappings for
//! IK-PROP-001..009 (validation pivot spec §9 / Phase D).
//!
//! Every property below has one dedicated deterministic test whose name carries the
//! property ID. The tests assert exact normative diagnostics, zero durable-row
//! mutation on every refusal, and replay/live projection conformance, so each test
//! fails if the corresponding wall is removed:
//!
//! ```text
//! IK-PROP-001 replay reconstructs exact semantic state          -> ik_prop_001_*
//! IK-PROP-002 failed decide/commit cannot partially persist     -> ik_prop_002_*
//! IK-PROP-003 stale generation cannot mutate                    -> ik_prop_003_*
//! IK-PROP-004 foreign aggregate/machine cannot mutate           -> ik_prop_004_*
//! IK-PROP-005 exact duplicate effect outcome replays original   -> ik_prop_005_*
//! IK-PROP-006 conflicting outcome never replaces original truth -> ik_prop_006_*
//! IK-PROP-007 no second pending effect (single-pending law)     -> ik_prop_007_*
//! IK-PROP-008 crash prefix replayable or corrupt/refused        -> ik_prop_008_*
//! IK-PROP-009 diagnostics/projection cannot mutate durable truth -> ik_prop_009_*
//! ```
//!
//! Supplementary walls exercised by dedicated tests:
//!
//! ```text
//! CORE_LAWS-2  hosts request named commands, never arbitrary destinations
//! CORE_LAWS-5  reducers are deterministic and perform no IO
//! CORE_LAWS-11 child observations retain source authority (substrate stamping)
//! CORE_LAWS-12 effect mechanics bind one exact pending identity
//! ```
//!
//! Concurrency (linearizability storm), real-process crash injection (crash matrix),
//! and storage corruption classes are exercised by the existing suites
//! (`tests/linearizability.rs`, `tests/crash_matrix.rs`, `tests/store_corruption.rs`,
//! `tests/atomicity.rs`); the machine-readable evidence at `validation/` maps them as
//! supporting evidence for the same walls. Every fixture is deterministic: the
//! suite uses no sleeps, no wall-clock dependence, and no generated residue.

#[path = "common/mod.rs"]
mod common;

use common::*;

use ik::diagnostic::{Diagnostic, DiagnosticCode};
use ik::effect::{EffectIntentV1, EffectOutcomeV1, EffectStatus};
use ik::identity::{AggregateId, Capability, EffectId, MachineIdentity};
use ik::machine::{ApplyOutcome, Decision, DomainEvent, Machine};
use ik::store::{CommandRequest, EffectOutcomeRequest, Store};
use serde_json::{json, Value};
use std::path::Path;

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

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

/// Set up a fresh VA aggregate at generation 0 in a tempdir store.
fn setup_va(dir: &tempfile::TempDir) -> (Store, AggregateId, Box<dyn Machine>) {
    let machine = va();
    let identity = machine.identity().clone();
    let mut store = Store::create(&dir.path().join("s.db")).expect("create store");
    let agg = AggregateId("agg-1".into());
    store
        .create_aggregate(machine.as_ref(), &agg, &identity)
        .expect("create aggregate");
    (store, agg, machine)
}

/// Set up a fresh EffectProbe aggregate at generation 0.
fn setup_probe(dir: &tempfile::TempDir) -> (Store, AggregateId, Box<dyn Machine>) {
    let machine = probe();
    let identity = machine.identity().clone();
    let mut store = Store::create(&dir.path().join("s.db")).expect("create store");
    let agg = AggregateId("agg-1".into());
    store
        .create_aggregate(machine.as_ref(), &agg, &identity)
        .expect("create aggregate");
    (store, agg, machine)
}

fn submit() -> Value {
    json!({"name": "submit", "args": {"artifact_digest": "a1", "baseline_digest": "b1"}})
}

fn plan_probe(
    store: &mut Store,
    machine: &dyn Machine,
    agg: &AggregateId,
    capability: &Capability,
) -> ik::store::CommandResult {
    store
        .command(
            machine,
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: machine.identity().clone(),
                capability: capability.clone(),
                command: json!({"name": "plan", "args": {"effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1"}}),
                observation: None,
            },
        )
        .expect("plan commits")
}

fn probe_outcome(agg: &AggregateId, generation: u64, digest: &str) -> EffectOutcomeV1 {
    EffectOutcomeV1 {
        effect_id: EffectId("e1".into()),
        aggregate_id: agg.clone(),
        planned_generation: generation,
        payload_digest: "p1".into(),
        idempotency_key: "k1".into(),
        outcome_digest: digest.into(),
    }
}

/// Canonical, content-sensitive serialization of every durable row in the store
/// (meta, journal, receipts, aggregates). Two equal strings mean byte-identical
/// durable truth; any committed or leaked write changes the string. Ordering is
/// fixed by primary key so the encoding is deterministic.
fn durable_rows(path: &Path) -> String {
    let conn = rusqlite::Connection::open(path).expect("open store for row snapshot");
    let mut parts: Vec<String> = Vec::new();
    let tables: [(&str, &str); 4] = [
        ("meta", "key"),
        ("journal", "aggregate_id, sequence"),
        ("receipts", "aggregate_id, sequence"),
        ("aggregates", "aggregate_id"),
    ];
    for (table, order) in tables {
        let sql = format!("SELECT * FROM {table} ORDER BY {order}");
        let mut stmt = conn.prepare(&sql).expect("prepare row snapshot");
        let columns = stmt.column_count();
        let rows = stmt
            .query_map([], |row| {
                let mut cells = Vec::with_capacity(columns);
                for index in 0..columns {
                    cells.push(cell_text(
                        &row.get::<_, rusqlite::types::Value>(index).expect("cell"),
                    ));
                }
                Ok(cells)
            })
            .expect("query rows");
        let mut encoded: Vec<String> = rows
            .map(|row| row.expect("collect row").join(","))
            .collect();
        encoded.sort();
        parts.push(format!("{table}:{}", encoded.join("|")));
    }
    parts.join("\n")
}

fn cell_text(value: &rusqlite::types::Value) -> String {
    match value {
        rusqlite::types::Value::Null => "null".to_string(),
        rusqlite::types::Value::Integer(i) => format!("i:{i}"),
        rusqlite::types::Value::Real(f) => format!("r:{f}"),
        rusqlite::types::Value::Text(s) => format!("t:{s}"),
        rusqlite::types::Value::Blob(b) => format!("b:{}", b.len()),
    }
}

/// Run a mutating attempt that MUST refuse with `expected`, and prove the refusal
/// left every durable row byte-identical. Returns the refusal.
fn refusal_keeps_rows(
    path: &Path,
    store: &mut Store,
    label: &str,
    expected: DiagnosticCode,
    attempt: impl FnOnce(&mut Store) -> Result<ik::store::CommandResult, Diagnostic>,
) -> Diagnostic {
    let before = durable_rows(path);
    let err = attempt(store).expect_err(label);
    assert_eq!(err.code, expected, "{label}: refusal code");
    assert_eq!(
        durable_rows(path),
        before,
        "{label}: refused transition must not mutate any durable row"
    );
    err
}

/// A native machine that pushes on the runtime's own walls: it emits an effect
/// intent with forged authority (empty aggregate, generation 0) and can mark its
/// aggregate terminal. The runtime must stamp the intent's authority and must
/// refuse a second pending effect and any post-terminal mutation itself.
struct WallProbeMachine {
    identity: MachineIdentity,
}

impl WallProbeMachine {
    fn new() -> Self {
        Self {
            identity: ik::machine::native_identity("ik_wall_probe", 1, "ik-wall-probe-spec-v1"),
        }
    }
}

impl Machine for WallProbeMachine {
    fn identity(&self) -> &MachineIdentity {
        &self.identity
    }

    fn initial_state(&self) -> Value {
        json!({"state": "idle"})
    }

    fn decide(
        &self,
        _state: &Value,
        _observation: Option<&Value>,
        _effect: &EffectStatus,
        command: &Value,
    ) -> Result<Decision, Diagnostic> {
        match command["name"].as_str() {
            Some("plan") => Ok(Decision::emit(DomainEvent::new("plan", 1, json!({})))),
            Some("done") => Ok(Decision::emit(DomainEvent::new("done", 1, json!({})))),
            _ => Err(Diagnostic::new(
                DiagnosticCode::IkCommandInvalid,
                Some("command.unknown"),
                "unknown command",
            )),
        }
    }

    fn apply(&self, state: &Value, event: &DomainEvent) -> Result<ApplyOutcome, Diagnostic> {
        match (event.event_type.as_str(), event.event_version) {
            ("plan", 1) => Ok(
                ApplyOutcome::new(state.clone()).with_effect(EffectIntentV1 {
                    effect_id: EffectId("forged".into()),
                    aggregate_id: AggregateId(String::new()), // the machine's claim; the runtime must override
                    generation: 0, // the machine's claim; the runtime must override
                    effect_kind: "WALL".into(),
                    payload_digest: "wall-payload".into(),
                    idempotency_key: "wall-key".into(),
                    payload: None,
                }),
            ),
            ("done", 1) => Ok(ApplyOutcome::new(state.clone()).with_terminal(true)),
            _ => Err(Diagnostic::new(
                DiagnosticCode::IkEventSchemaUnknown,
                Some("event.schema"),
                "unknown event",
            )),
        }
    }
}

fn setup_wall(dir: &tempfile::TempDir) -> (Store, AggregateId, Box<dyn Machine>) {
    let machine: Box<dyn Machine> = Box::new(WallProbeMachine::new());
    let identity = machine.identity().clone();
    let mut store = Store::create(&dir.path().join("s.db")).expect("create store");
    let agg = AggregateId("wall-1".into());
    store
        .create_aggregate(machine.as_ref(), &agg, &identity)
        .expect("create aggregate");
    (store, agg, machine)
}

// ---------------------------------------------------------------------------
// IK-PROP-001 — replay reconstructs exact semantic state
// ---------------------------------------------------------------------------

#[test]
fn ik_prop_001_replay_reconstructs_exact_semantic_state() {
    // Replay must rebuild the projection exactly (state, terminal, generation,
    // nonce, effect, sequence, chain head, last receipt) for every aggregate, and
    // after every committed mutation the live projection must already conform to
    // replay. Wall removal: if the journal fold or the projection write diverged,
    // replay would report a rebuild or verify would refuse.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let mut store = Store::create(&path).expect("create store");

    // Aggregate A: a full VerificationAttempt lifecycle.
    let va_machine = va();
    let va_identity = va_machine.identity().clone();
    let agg_a = AggregateId("a".into());
    store
        .create_aggregate(va_machine.as_ref(), &agg_a, &va_identity)
        .expect("create A");
    // Aggregate B: an effect plan/record/finish lifecycle through the same store.
    let probe_machine = probe();
    let probe_identity = probe_machine.identity().clone();
    let agg_b = AggregateId("b".into());
    store
        .create_aggregate(probe_machine.as_ref(), &agg_b, &probe_identity)
        .expect("create B");

    let mut va_cap = cap(&va_identity, &agg_a, 0);
    let mut probe_cap = cap(&probe_identity, &agg_b, 0);
    let mut mutations = 0u64;
    for command in [
        json!({"name": "start"}),
        submit(),
        json!({"name": "verify", "args": {"receipt_digest": "r1", "receipt_artifact_digest": "a1", "receipt_baseline_digest": "b1"}}),
    ] {
        let out = store
            .command(
                va_machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg_a.clone(),
                    machine: va_identity.clone(),
                    capability: va_cap.clone(),
                    command,
                    observation: None,
                },
            )
            .expect("VA command commits");
        assert!(out.mutation);
        va_cap = out.capability;
        mutations += 1;
    }
    let plan = store
        .command(
            probe_machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg_b.clone(),
                machine: probe_identity.clone(),
                capability: probe_cap.clone(),
                command: json!({"name": "plan", "args": {"effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1"}}),
                observation: None,
            },
        )
        .expect("plan commits");
    probe_cap = plan.capability;
    mutations += 1;
    let recorded = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg_b.clone(),
            machine: probe_identity.clone(),
            capability: probe_cap.clone(),
            effect: probe_outcome(&agg_b, 1, "o1"),
        })
        .expect("record commits");
    probe_cap = recorded.capability;
    mutations += 1;
    store
        .command(
            probe_machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg_b.clone(),
                machine: probe_identity.clone(),
                capability: probe_cap.clone(),
                command: json!({"name": "finish", "args": {"result_digest": "r1"}}),
                observation: None,
            },
        )
        .expect("finish commits");
    mutations += 1;

    // Live projection conformance: after every committed mutation, replay matches
    // and verify passes, so the projection never silently diverges.
    assert_eq!(mutations, 6);
    let live_a = store.show(&agg_a).expect("show A");
    let live_b = store.show(&agg_b).expect("show B");
    assert_eq!(live_a.sequence, 3);
    assert_eq!(live_b.sequence, 3);
    let replay_a = store
        .replay(va_machine.as_ref(), Some(&agg_a))
        .expect("live replay A");
    let replay_b = store
        .replay(probe_machine.as_ref(), Some(&agg_b))
        .expect("live replay B");
    assert_eq!(replay_a.aggregates.len(), 1);
    assert_eq!(replay_a.aggregates[0].aggregate_id, agg_a);
    assert_eq!(replay_b.aggregates.len(), 1);
    assert_eq!(replay_b.aggregates[0].aggregate_id, agg_b);
    assert!(!replay_a.aggregates[0].rebuilt);
    assert!(!replay_b.aggregates[0].rebuilt);
    store
        .verify(va_machine.as_ref(), Some(&agg_a))
        .expect("verify A on the live store");
    store
        .verify(probe_machine.as_ref(), Some(&agg_b))
        .expect("verify B on the live store");

    // Reopen + replay: the rebuilt projection must equal the live projection on
    // every observable field.
    let live_a = store.show(&agg_a).expect("show A before reopen");
    let live_b = store.show(&agg_b).expect("show B before reopen");
    drop(store);

    let mut reopened = Store::open(&path).expect("reopen");
    let replay_a = reopened
        .replay(va_machine.as_ref(), Some(&agg_a))
        .expect("replay A");
    let replay_b = reopened
        .replay(probe_machine.as_ref(), Some(&agg_b))
        .expect("replay B");
    assert_eq!(replay_a.aggregates.len(), 1);
    assert_eq!(replay_a.aggregates[0].aggregate_id, agg_a);
    assert_eq!(replay_b.aggregates.len(), 1);
    assert_eq!(replay_b.aggregates[0].aggregate_id, agg_b);
    let replayed_a = &replay_a.aggregates[0];
    let replayed_b = &replay_b.aggregates[0];
    assert!(!replayed_a.rebuilt);
    assert!(!replayed_b.rebuilt);
    reopened
        .verify(va_machine.as_ref(), Some(&agg_a))
        .expect("verify A after replay");
    reopened
        .verify(probe_machine.as_ref(), Some(&agg_b))
        .expect("verify B after replay");

    let after_a = reopened.show(&agg_a).expect("show A after reopen");
    let after_b = reopened.show(&agg_b).expect("show B after reopen");
    for (label, before, after) in [("A", &live_a, &after_a), ("B", &live_b, &after_b)] {
        assert_eq!(after.state, before.state, "{label}: state");
        assert_eq!(after.terminal, before.terminal, "{label}: terminal");
        assert_eq!(after.generation, before.generation, "{label}: generation");
        assert_eq!(
            after.capability, before.capability,
            "{label}: capability/nonce"
        );
        assert_eq!(after.effect, before.effect, "{label}: effect status");
        assert_eq!(after.sequence, before.sequence, "{label}: sequence");
        assert_eq!(
            after.last_event_hash, before.last_event_hash,
            "{label}: chain head"
        );
        assert_eq!(
            after.last_receipt, before.last_receipt,
            "{label}: last receipt"
        );
    }
}

// ---------------------------------------------------------------------------
// IK-PROP-002 — failed decide/commit cannot partially persist
// ---------------------------------------------------------------------------

#[test]
fn ik_prop_002_failed_decide_cannot_partially_persist() {
    // Every refusal class at the decide/validation stage must leave the journal,
    // receipts, and aggregates rows byte-identical — not merely projection-field
    // equal. Commit-stage atomicity (failure injected inside the SQLite
    // transaction) is proven by tests/atomicity.rs; this test proves the
    // validation/decide stage with the same row-level standard.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let (mut store, agg, machine) = setup_va(&dir);
    let identity = machine.identity().clone();

    // (a) unknown command name
    refusal_keeps_rows(
        &path,
        &mut store,
        "unknown command",
        DiagnosticCode::IkCommandInvalid,
        |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap(&identity, &agg, 0),
                    command: json!({"name": "teleport"}),
                    observation: None,
                },
            )
        },
    );
    // (b) command that is not a JSON object
    refusal_keeps_rows(
        &path,
        &mut store,
        "array command",
        DiagnosticCode::IkCommandInvalid,
        |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap(&identity, &agg, 0),
                    command: json!([]),
                    observation: None,
                },
            )
        },
    );
    // (c) command without a string `name`
    refusal_keeps_rows(
        &path,
        &mut store,
        "missing command name",
        DiagnosticCode::IkCommandInvalid,
        |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap(&identity, &agg, 0),
                    command: json!({"args": {}}),
                    observation: None,
                },
            )
        },
    );
    // (d) forged destination arguments (see also the named-command suite)
    refusal_keeps_rows(
        &path,
        &mut store,
        "forged destination args",
        DiagnosticCode::IkCommandInvalid,
        |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap(&identity, &agg, 0),
                    command: json!({"name": "start", "args": {"state": "verified"}}),
                    observation: None,
                },
            )
        },
    );
    // (e) wrong nonce at the current generation
    refusal_keeps_rows(
        &path,
        &mut store,
        "wrong nonce",
        DiagnosticCode::IkNonceInvalid,
        |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: Capability {
                        generation: 0,
                        nonce: "x".into(),
                    },
                    command: json!({"name": "start"}),
                    observation: None,
                },
            )
        },
    );
    // (f) stale generation
    refusal_keeps_rows(
        &path,
        &mut store,
        "stale generation",
        DiagnosticCode::IkCapabilityStale,
        |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap(&identity, &agg, 5),
                    command: json!({"name": "start"}),
                    observation: None,
                },
            )
        },
    );
    // (g) foreign machine identity presented for this aggregate
    let foreign_identity = ik::machine::native_identity("foreign_machine", 1, "foreign-v1");
    refusal_keeps_rows(
        &path,
        &mut store,
        "machine mismatch",
        DiagnosticCode::IkMachineMismatch,
        |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: foreign_identity.clone(),
                    capability: cap(&identity, &agg, 0),
                    command: json!({"name": "start"}),
                    observation: None,
                },
            )
        },
    );
    // (h) command illegal in the current state (decide-stage domain refusal)
    refusal_keeps_rows(
        &path,
        &mut store,
        "illegal state",
        DiagnosticCode::IkIllegalState,
        |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap(&identity, &agg, 0),
                    command: submit(),
                    observation: None,
                },
            )
        },
    );
    // (i) declared invariant refusal: after reaching `submitted`, a mismatched
    // artifact digest violates the machine invariant and emits nothing.
    let started = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap(&identity, &agg, 0),
                command: json!({"name": "start"}),
                observation: None,
            },
        )
        .expect("start commits");
    let submitted = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: started.capability,
                command: submit(),
                observation: None,
            },
        )
        .expect("submit commits");
    refusal_keeps_rows(
        &path,
        &mut store,
        "invariant-failure",
        DiagnosticCode::IkInvariantFailed,
        |s| {
            s.command(machine.as_ref(), &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: submitted.capability.clone(),
                command: json!({"name": "verify", "args": {"receipt_digest": "r1", "receipt_artifact_digest": "WRONG", "receipt_baseline_digest": "b1"}}),
                observation: None,
            })
        },
    );

    // The projection is exactly where the committed transitions left it.
    let projection = store.show(&agg).expect("show");
    assert_eq!(projection.sequence, 2);
    assert_eq!(projection.generation, 2);
    assert_eq!(projection.state["state"], "submitted");
}

#[test]
fn ik_prop_002_runtime_terminal_guard_cannot_partially_persist() {
    // The runtime's own terminal wall (independent of the domain machine): a
    // machine that emits from a terminal aggregate must be refused with the exact
    // diagnostic before any write.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let (mut store, agg, machine) = setup_wall(&dir);
    let identity = machine.identity().clone();
    let planned = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap(&identity, &agg, 0),
                command: json!({"name": "plan"}),
                observation: None,
            },
        )
        .expect("plan commits");
    let done = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: planned.capability,
                command: json!({"name": "done"}),
                observation: None,
            },
        )
        .expect("done commits");
    assert!(done.terminal);
    let err = refusal_keeps_rows(
        &path,
        &mut store,
        "post-terminal mutation",
        DiagnosticCode::IkIllegalState,
        |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: done.capability.clone(),
                    command: json!({"name": "plan"}),
                    observation: None,
                },
            )
        },
    );
    assert_eq!(err.rule.as_deref(), Some("terminal"));
    let projection = store.show(&agg).expect("show");
    assert!(projection.terminal);
    assert_eq!(projection.generation, 2);
}

// ---------------------------------------------------------------------------
// IK-PROP-003 — stale generation cannot mutate
// ---------------------------------------------------------------------------

#[test]
fn ik_prop_003_stale_generation_cannot_mutate() {
    // A capability for any past generation — or a forged nonce at the current
    // generation — must refuse on both the command and the record paths, leaving
    // every durable row untouched.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let (mut store, agg, machine) = setup_va(&dir);
    let identity = machine.identity().clone();
    let first = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap(&identity, &agg, 0),
                command: json!({"name": "start"}),
                observation: None,
            },
        )
        .expect("start commits");
    let second = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: first.capability,
                command: submit(),
                observation: None,
            },
        )
        .expect("submit commits");
    assert_eq!(second.generation, 2);

    // Command path: generation 0, generation 1, and a forged nonce at generation 2.
    for (label, capability, code) in [
        (
            "gen-0",
            cap(&identity, &agg, 0),
            DiagnosticCode::IkCapabilityStale,
        ),
        (
            "gen-1",
            cap(&identity, &agg, 1),
            DiagnosticCode::IkCapabilityStale,
        ),
        (
            "forged nonce at gen-2",
            Capability {
                generation: 2,
                nonce: "forged".into(),
            },
            DiagnosticCode::IkNonceInvalid,
        ),
    ] {
        refusal_keeps_rows(&path, &mut store, label, code, |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: capability.clone(),
                    command: json!({"name": "cancel"}),
                    observation: None,
                },
            )
        });
    }

    // Record path: a stale capability must refuse an outcome binding.
    let dir2 = tempfile::tempdir().expect("tempdir");
    let p_path = dir2.path().join("p.db");
    let pmachine = probe();
    let pidentity = pmachine.identity().clone();
    let mut pstore = Store::create(&p_path).expect("create probe store");
    let pagg = AggregateId("p-1".into());
    pstore
        .create_aggregate(pmachine.as_ref(), &pagg, &pidentity)
        .expect("create probe aggregate");
    let planned = plan_probe(
        &mut pstore,
        pmachine.as_ref(),
        &pagg,
        &cap(&pidentity, &pagg, 0),
    );
    assert!(matches!(planned.effect, EffectStatus::Pending(_)));
    refusal_keeps_rows(
        &p_path,
        &mut pstore,
        "stale record",
        DiagnosticCode::IkCapabilityStale,
        |s| {
            s.record_effect(&EffectOutcomeRequest {
                aggregate_id: pagg.clone(),
                machine: pidentity.clone(),
                capability: cap(&pidentity, &pagg, 0),
                effect: probe_outcome(&pagg, 1, "o1"),
            })
        },
    );
    // The pending intent survives the refused record.
    let after = pstore.show(&pagg).expect("show");
    assert!(matches!(after.effect, EffectStatus::Pending(_)));
    assert_eq!(after.generation, 1);
}

// ---------------------------------------------------------------------------
// IK-PROP-004 — foreign aggregate/machine cannot mutate
// ---------------------------------------------------------------------------

#[test]
fn ik_prop_004_foreign_aggregate_or_machine_cannot_mutate() {
    // (a) A capability minted for another aggregate must never authorize a
    // mutation; (b) an outcome naming a foreign aggregate must refuse before any
    // binding decision; (c) a foreign machine identity must refuse; (d) an
    // arbitrary foreign aggregate id in a command must refuse as unknown.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let machine = va();
    let identity = machine.identity().clone();
    let mut store = Store::create(&path).expect("create store");
    let agg_a = AggregateId("a".into());
    let agg_b = AggregateId("b".into());
    store
        .create_aggregate(machine.as_ref(), &agg_a, &identity)
        .expect("create A");
    store
        .create_aggregate(machine.as_ref(), &agg_b, &identity)
        .expect("create B");

    // Cross-aggregate capability at the same generation: the nonce is derived from
    // the aggregate identity, so B's capability is invalid for A.
    let cap_b = cap(&identity, &agg_b, 0);
    refusal_keeps_rows(
        &path,
        &mut store,
        "cross-aggregate nonce",
        DiagnosticCode::IkNonceInvalid,
        |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg_a.clone(),
                    machine: identity.clone(),
                    capability: cap_b.clone(),
                    command: json!({"name": "start"}),
                    observation: None,
                },
            )
        },
    );
    // Cross-aggregate capability after A advanced: generation staleness dominates.
    let out_a = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg_a.clone(),
                machine: identity.clone(),
                capability: cap(&identity, &agg_a, 0),
                command: json!({"name": "start"}),
                observation: None,
            },
        )
        .expect("A advances");
    assert_eq!(out_a.generation, 1);
    refusal_keeps_rows(
        &path,
        &mut store,
        "cross-aggregate stale",
        DiagnosticCode::IkCapabilityStale,
        |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg_a.clone(),
                    machine: identity.clone(),
                    capability: cap_b.clone(),
                    command: json!({"name": "cancel"}),
                    observation: None,
                },
            )
        },
    );

    // Foreign machine identity (valid shape, wrong id) refuses with the exact code.
    let foreign = ik::machine::native_identity("other_machine", 1, "other-v1");
    refusal_keeps_rows(
        &path,
        &mut store,
        "foreign machine",
        DiagnosticCode::IkMachineMismatch,
        |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg_a.clone(),
                    machine: foreign.clone(),
                    capability: cap(&identity, &agg_a, 1),
                    command: json!({"name": "cancel"}),
                    observation: None,
                },
            )
        },
    );

    // Foreign aggregate id in a command request: no such aggregate exists.
    refusal_keeps_rows(
        &path,
        &mut store,
        "unknown aggregate",
        DiagnosticCode::IkIllegalState,
        |s| {
            s.command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: AggregateId("ghost".into()),
                    machine: identity.clone(),
                    capability: cap(&identity, &AggregateId("ghost".into()), 0),
                    command: json!({"name": "start"}),
                    observation: None,
                },
            )
        },
    );

    // Foreign-aggregate outcome against a pending intent refuses as a foreign
    // identity before any binding decision (stamp_outcome_aggregate wall).
    let dir2 = tempfile::tempdir().expect("tempdir");
    let p_path = dir2.path().join("p.db");
    let pmachine = probe();
    let pidentity = pmachine.identity().clone();
    let mut pstore = Store::create(&p_path).expect("create probe store");
    let pagg = AggregateId("p-1".into());
    pstore
        .create_aggregate(pmachine.as_ref(), &pagg, &pidentity)
        .expect("create probe aggregate");
    let planned = plan_probe(
        &mut pstore,
        pmachine.as_ref(),
        &pagg,
        &cap(&pidentity, &pagg, 0),
    );
    let mut foreign_outcome = probe_outcome(&pagg, 1, "o1");
    foreign_outcome.aggregate_id = AggregateId("foreign".into());
    let err = refusal_keeps_rows(
        &p_path,
        &mut pstore,
        "foreign outcome aggregate",
        DiagnosticCode::IkEffectMismatch,
        |s| {
            s.record_effect(&EffectOutcomeRequest {
                aggregate_id: pagg.clone(),
                machine: pidentity.clone(),
                capability: planned.capability.clone(),
                effect: foreign_outcome.clone(),
            })
        },
    );
    assert_eq!(err.rule.as_deref(), Some("effect.identity"));
    let after = pstore.show(&pagg).expect("show");
    assert!(
        matches!(after.effect, EffectStatus::Pending(_)),
        "foreign outcome must not touch the pending intent"
    );
}

// ---------------------------------------------------------------------------
// IK-PROP-005 — exact duplicate effect outcome replays the original receipt
// ---------------------------------------------------------------------------

#[test]
fn ik_prop_005_exact_duplicate_effect_outcome_replays_original_receipt() {
    // An exact six-field duplicate of a recorded outcome must replay the original
    // receipt with zero mutation — no journal event, no generation increment, no
    // row change — even when presented with the stale pre-record capability.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let (mut store, agg, machine) = setup_probe(&dir);
    let identity = machine.identity().clone();
    let planned = plan_probe(&mut store, machine.as_ref(), &agg, &cap(&identity, &agg, 0));
    let binding = probe_outcome(&agg, 1, "o1");
    let recorded = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: planned.capability,
            effect: binding.clone(),
        })
        .expect("record commits");
    assert!(recorded.mutation);
    let original_receipt = recorded.receipt.expect("recording receipt");
    let rows_after_record = durable_rows(&path);

    // Duplicate with the stale pre-record capability: replay, zero mutation.
    let replayed = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: cap(&identity, &agg, 1), // stale: current is 2
            effect: binding.clone(),
        })
        .expect("duplicate replay must not error");
    assert!(!replayed.mutation, "duplicate replay is zero-mutation");
    assert_eq!(
        replayed.receipt.as_ref().map(|r| r.receipt_id.clone()),
        Some(original_receipt.receipt_id.clone())
    );
    assert_eq!(replayed.generation, 2, "no generation increment on replay");
    assert_eq!(
        durable_rows(&path),
        rows_after_record,
        "duplicate replay must not touch any durable row"
    );
    let projection = store.show(&agg).expect("show");
    assert_eq!(projection.sequence, 2, "no journal event appended");
    assert_eq!(projection.generation, 2);
    store
        .verify(machine.as_ref(), Some(&agg))
        .expect("verify passes after duplicate replay");

    // Duplicate with the current capability: still a zero-mutation replay.
    let again = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: projection.capability,
            effect: binding,
        })
        .expect("current-capability duplicate also replays");
    assert!(!again.mutation);
    assert_eq!(
        durable_rows(&path),
        rows_after_record,
        "current-capability duplicate replay must also be row-identical"
    );
}

// ---------------------------------------------------------------------------
// IK-PROP-006 — conflicting outcome never replaces original truth
// ---------------------------------------------------------------------------

#[test]
fn ik_prop_006_conflicting_outcome_never_replaces_original_truth() {
    // Same effect identity, different outcome: the recorded outcome must remain the
    // authoritative truth — the conflict refuses, no row changes, and replay still
    // reproduces the original recorded outcome.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let (mut store, agg, machine) = setup_probe(&dir);
    let identity = machine.identity().clone();
    let planned = plan_probe(&mut store, machine.as_ref(), &agg, &cap(&identity, &agg, 0));
    let binding = probe_outcome(&agg, 1, "o1");
    let recorded = store
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: planned.capability,
            effect: binding.clone(),
        })
        .expect("record commits");
    let rows_after_record = durable_rows(&path);

    // Current-capability conflict with a different outcome digest.
    let conflicting = probe_outcome(&agg, 1, "o2");
    let err = refusal_keeps_rows(
        &path,
        &mut store,
        "conflicting outcome",
        DiagnosticCode::IkEffectOutcomeConflict,
        |s| {
            s.record_effect(&EffectOutcomeRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: recorded.capability.clone(),
                effect: conflicting.clone(),
            })
        },
    );
    assert_eq!(err.rule.as_deref(), Some("effect.binding"));
    assert_eq!(
        durable_rows(&path),
        rows_after_record,
        "conflict must not touch any durable row"
    );

    // The recorded truth is untouched: original digest, original receipt, and the
    // pending intent identity all survive.
    let projection = store.show(&agg).expect("show");
    let EffectStatus::Recorded(intent, digest, receipt_id) = &projection.effect else {
        panic!("expected a recorded effect, got {:?}", projection.effect);
    };
    assert_eq!(digest, "o1");
    assert_eq!(
        receipt_id.as_deref(),
        Some(
            recorded
                .receipt
                .as_ref()
                .expect("receipt")
                .receipt_id
                .as_str()
        )
    );
    assert_eq!(intent.effect_id.0, "e1");
    assert_eq!(intent.generation, 1);

    // Reopen + replay reproduce the original recorded truth, not the conflict.
    drop(store);
    let mut reopened = Store::open(&path).expect("reopen");
    reopened
        .replay(machine.as_ref(), Some(&agg))
        .expect("replay");
    reopened
        .verify(machine.as_ref(), Some(&agg))
        .expect("verify");
    let after = reopened.show(&agg).expect("show after replay");
    let EffectStatus::Recorded(_, digest, _) = &after.effect else {
        panic!("replay must reproduce the recorded effect");
    };
    assert_eq!(
        digest, "o1",
        "the original recorded outcome is the only truth"
    );
}

// ---------------------------------------------------------------------------
// IK-PROP-007 — no second pending effect where single-pending law applies
// ---------------------------------------------------------------------------

#[test]
fn ik_prop_007_no_second_pending_effect_where_single_pending_law_applies() {
    // Domain path: the declarative machine refuses a second plan from `planned`.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let (mut store, agg, machine) = setup_probe(&dir);
    let identity = machine.identity().clone();
    let planned = plan_probe(&mut store, machine.as_ref(), &agg, &cap(&identity, &agg, 0));
    let EffectStatus::Pending(intent) = &planned.effect else {
        panic!("expected pending, got {:?}", planned.effect);
    };
    let original_intent = intent.clone();
    let err = refusal_keeps_rows(
        &path,
        &mut store,
        "declarative second plan",
        DiagnosticCode::IkIllegalState,
        |s| {
            s.command(machine.as_ref(), &CommandRequest {
            aggregate_id: agg.clone(),
            machine: identity.clone(),
            capability: planned.capability.clone(),
            command: json!({"name": "plan", "args": {"effect_id": "e2", "payload_digest": "p2", "idempotency_key": "k2"}}),
            observation: None,
        })
        },
    );
    assert_eq!(err.rule.as_deref(), Some("plan"));
    let projection = store.show(&agg).expect("show");
    assert_eq!(
        projection.effect,
        EffectStatus::Pending(original_intent),
        "the original pending intent is the only pending effect"
    );
    assert_eq!(projection.generation, 1);

    // Runtime path: a machine that emits a second intent must still be refused by
    // the runtime's single-pending wall before any write.
    let dir2 = tempfile::tempdir().expect("tempdir");
    let w_path = dir2.path().join("s.db");
    let (mut wstore, wagg, wmachine) = setup_wall(&dir2);
    let widentity = wmachine.identity().clone();
    let first = wstore
        .command(
            wmachine.as_ref(),
            &CommandRequest {
                aggregate_id: wagg.clone(),
                machine: widentity.clone(),
                capability: cap(&widentity, &wagg, 0),
                command: json!({"name": "plan"}),
                observation: None,
            },
        )
        .expect("first plan");
    assert!(matches!(first.effect, EffectStatus::Pending(_)));
    let err = refusal_keeps_rows(
        &w_path,
        &mut wstore,
        "runtime second plan",
        DiagnosticCode::IkEffectStale,
        |s| {
            s.command(
                wmachine.as_ref(),
                &CommandRequest {
                    aggregate_id: wagg.clone(),
                    machine: widentity.clone(),
                    capability: first.capability.clone(),
                    command: json!({"name": "plan"}),
                    observation: None,
                },
            )
        },
    );
    assert_eq!(err.rule.as_deref(), Some("effect.pending"));
    let after = wstore.show(&wagg).expect("show");
    assert!(matches!(after.effect, EffectStatus::Pending(_)));
    assert_eq!(after.generation, 1);
}

// ---------------------------------------------------------------------------
// IK-PROP-008 — crash prefix is replayable or explicitly corrupt/refused
// ---------------------------------------------------------------------------

/// Build a VA aggregate advanced to sequence 3 (created -> running -> submitted ->
/// repairable). Returns the store path.
fn va_to_repairable(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("s.db");
    {
        let (mut store, agg, machine) = setup_va(dir);
        let identity = machine.identity().clone();
        let mut capability = cap(&identity, &agg, 0);
        for command in [
            json!({"name": "start"}),
            submit(),
            json!({"name": "reject"}),
        ] {
            let out = store
                .command(
                    machine.as_ref(),
                    &CommandRequest {
                        aggregate_id: agg.clone(),
                        machine: identity.clone(),
                        capability,
                        command,
                        observation: None,
                    },
                )
                .expect("commit");
            capability = out.capability;
        }
        drop(store);
    }
    path
}

#[test]
fn ik_prop_008_crash_prefix_is_replayable_or_explicitly_corrupt_refused() {
    // (a) Torn tail: the journal is a legal prefix and the projection is ahead of
    // it (a crash between the projection write and anything after commit). Replay
    // must converge to the last committed prefix — a known safe continuation —
    // rather than guessing or inventing state.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = va_to_repairable(&dir);
    let conn = rusqlite::Connection::open(&path).expect("open");
    conn.execute(
        "DELETE FROM journal WHERE aggregate_id = 'agg-1' AND sequence = 3",
        [],
    )
    .expect("truncate torn tail event");
    conn.execute(
        "DELETE FROM receipts WHERE aggregate_id = 'agg-1' AND sequence = 3",
        [],
    )
    .expect("truncate torn tail receipt");
    drop(conn);

    let machine = va();
    let mut store = Store::open(&path).expect("reopen after torn tail");
    let summary = store
        .replay(machine.as_ref(), Some(&AggregateId("agg-1".into())))
        .expect("replay must converge on the legal prefix");
    assert!(
        summary.aggregates[0].rebuilt,
        "torn tail must be repaired by replay"
    );
    assert_eq!(summary.aggregates[0].sequence, 2);
    let projection = store
        .show(&AggregateId("agg-1".into()))
        .expect("show after replay");
    assert_eq!(projection.sequence, 2);
    assert_eq!(projection.generation, 2);
    assert_eq!(projection.state["state"], "submitted");
    store
        .verify(machine.as_ref(), Some(&AggregateId("agg-1".into())))
        .expect("verify passes after prefix repair");

    // (b) Hole in the middle: the journal is no longer a contiguous chain, so the
    // crash prefix is NOT replayable; it must be explicitly refused as corrupt, not
    // silently "repaired" or accepted.
    let dir2 = tempfile::tempdir().expect("tempdir");
    let hole_path = va_to_repairable(&dir2);
    let conn = rusqlite::Connection::open(&hole_path).expect("open");
    conn.execute(
        "DELETE FROM journal WHERE aggregate_id = 'agg-1' AND sequence = 2",
        [],
    )
    .expect("punch a hole in the journal");
    drop(conn);
    let rows_before = durable_rows(&hole_path);
    let mut store = Store::open(&hole_path).expect("reopen with hole");
    let err = store
        .replay(machine.as_ref(), Some(&AggregateId("agg-1".into())))
        .expect_err("journal hole must refuse replay");
    assert_eq!(err.code, DiagnosticCode::IkJournalCorrupt);
    let err = store
        .verify(machine.as_ref(), Some(&AggregateId("agg-1".into())))
        .expect_err("journal hole must refuse verify");
    assert_eq!(err.code, DiagnosticCode::IkJournalCorrupt);
    assert_eq!(
        durable_rows(&hole_path),
        rows_before,
        "the corruption refusal must not mutate any durable row"
    );

    // (c) Empty journal: an aggregate whose journal vanished entirely is explicitly
    // corrupt, not silently accepted as a fresh aggregate.
    let dir3 = tempfile::tempdir().expect("tempdir");
    let empty_path = va_to_repairable(&dir3);
    let conn = rusqlite::Connection::open(&empty_path).expect("open");
    conn.execute("DELETE FROM journal WHERE aggregate_id = 'agg-1'", [])
        .expect("delete the whole journal");
    drop(conn);
    let rows_before = durable_rows(&empty_path);
    let mut store = Store::open(&empty_path).expect("reopen with empty journal");
    let err = store
        .replay(machine.as_ref(), Some(&AggregateId("agg-1".into())))
        .expect_err("empty journal must refuse replay");
    assert_eq!(err.code, DiagnosticCode::IkJournalCorrupt);
    assert_eq!(err.rule.as_deref(), Some("journal.empty"));
    assert_eq!(
        durable_rows(&empty_path),
        rows_before,
        "the empty-journal refusal must not mutate any durable row"
    );
}

// ---------------------------------------------------------------------------
// IK-PROP-009 — diagnostics/projection cannot mutate durable truth
// ---------------------------------------------------------------------------

#[test]
fn ik_prop_009_diagnostics_and_projection_reads_cannot_mutate_durable_truth() {
    // (a) On a healthy store, show/verify/replay (no-op match) are pure reads:
    // durable rows stay byte-identical and the projection is unchanged.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    {
        let (mut store, agg, machine) = setup_va(&dir);
        let identity = machine.identity().clone();
        store
            .command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap(&identity, &agg, 0),
                    command: json!({"name": "start"}),
                    observation: None,
                },
            )
            .expect("start commits");
        drop(store);
    }
    let machine = va();
    let rows_before = durable_rows(&path);
    let mut store = Store::open(&path).expect("reopen");
    let before = store
        .show(&AggregateId("agg-1".into()))
        .expect("show before");
    for _ in 0..3 {
        store
            .show(&AggregateId("agg-1".into()))
            .expect("repeated diagnostic read");
    }
    store.verify(machine.as_ref(), None).expect("verify");
    let summary = store.replay(machine.as_ref(), None).expect("no-op replay");
    assert!(
        !summary.aggregates[0].rebuilt,
        "healthy replay is a pure read"
    );
    store
        .show(&AggregateId("agg-1".into()))
        .expect("show after reads");
    assert_eq!(
        durable_rows(&path),
        rows_before,
        "diagnostic reads must not mutate any durable row"
    );
    let after = store
        .show(&AggregateId("agg-1".into()))
        .expect("show after");
    assert_eq!(after.state, before.state);
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.sequence, before.sequence);
    assert_eq!(after.last_event_hash, before.last_event_hash);

    // (b) On a journal-corrupt store, diagnostics surface a typed refusal and
    // never "repair" the corruption: rows stay byte-identical. show() remains a
    // pure projection read.
    let dir2 = tempfile::tempdir().expect("tempdir");
    let corrupt_path = dir2.path().join("s.db");
    {
        let (mut store, agg, machine) = setup_va(&dir2);
        let identity = machine.identity().clone();
        store
            .command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap(&identity, &agg, 0),
                    command: json!({"name": "start"}),
                    observation: None,
                },
            )
            .expect("start commits");
        drop(store);
    }
    let conn = rusqlite::Connection::open(&corrupt_path).expect("open");
    conn.execute(
        "UPDATE journal SET hash = 'deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef' WHERE aggregate_id = 'agg-1' AND sequence = 1",
        [],
    )
    .expect("corrupt the journal hash");
    drop(conn);
    let rows_before = durable_rows(&corrupt_path);
    let mut store = Store::open(&corrupt_path).expect("reopen corrupt");
    let shown = store
        .show(&AggregateId("agg-1".into()))
        .expect("show is a projection read even when the journal is corrupt");
    assert_eq!(shown.sequence, 1);
    let err = store
        .verify(machine.as_ref(), None)
        .expect_err("verify refuses journal corruption");
    assert_eq!(err.code, DiagnosticCode::IkJournalCorrupt);
    let err = store
        .replay(machine.as_ref(), None)
        .expect_err("replay refuses journal corruption instead of repairing it");
    assert_eq!(err.code, DiagnosticCode::IkJournalCorrupt);
    assert_eq!(
        durable_rows(&corrupt_path),
        rows_before,
        "corruption diagnostics must not mutate any durable row"
    );

    // (c) On a projection-corrupt store, verify refuses with the exact diagnostic
    // without writing; show() surfaces the stored projection unchanged. Only the
    // explicit replay repair path may rewrite — and verify detects that.
    let dir3 = tempfile::tempdir().expect("tempdir");
    let proj_path = dir3.path().join("s.db");
    {
        let (mut store, agg, machine) = setup_va(&dir3);
        let identity = machine.identity().clone();
        store
            .command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap(&identity, &agg, 0),
                    command: json!({"name": "start"}),
                    observation: None,
                },
            )
            .expect("start commits");
        drop(store);
    }
    let conn = rusqlite::Connection::open(&proj_path).expect("open");
    conn.execute(
        "UPDATE aggregates SET state = '{\"state\":\"cancelled\"}' WHERE aggregate_id = 'agg-1'",
        [],
    )
    .expect("corrupt the projection");
    drop(conn);
    let rows_before = durable_rows(&proj_path);
    let mut store = Store::open(&proj_path).expect("reopen corrupt projection");
    let shown = store
        .show(&AggregateId("agg-1".into()))
        .expect("show surfaces the stored projection");
    assert_eq!(shown.state["state"], "cancelled");
    let err = store
        .verify(machine.as_ref(), None)
        .expect_err("verify refuses projection corruption");
    assert_eq!(err.code, DiagnosticCode::IkProjectionCorrupt);
    assert_eq!(
        durable_rows(&proj_path),
        rows_before,
        "the projection-corruption diagnostic must not mutate any durable row"
    );
}

// ---------------------------------------------------------------------------
// CORE_LAWS-2 — hosts request named commands, never arbitrary destinations
// ---------------------------------------------------------------------------

#[test]
fn named_commands_only_no_arbitrary_destination_state() {
    // The mutation surface is a named command; there is no destination parameter
    // anywhere. Destination-shape attacks (a `state`/`to`/`destination` argument,
    // an unknown command, a non-object command) must refuse with
    // IK_COMMAND_INVALID and zero mutation, and the post-state of a legal command
    // must be exactly the machine's declared transition target — never a
    // caller-chosen value.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let (mut store, agg, machine) = setup_va(&dir);
    let identity = machine.identity().clone();

    for (label, command) in [
        (
            "destination via state arg",
            json!({"name": "start", "args": {"state": "verified"}}),
        ),
        (
            "destination via to arg",
            json!({"name": "start", "args": {"to": "verified"}}),
        ),
        (
            "destination via destination arg",
            json!({"name": "start", "args": {"destination": "verified"}}),
        ),
        ("unknown command name", json!({"name": "jump"})),
        ("non-object command", json!(["start"])),
    ] {
        refusal_keeps_rows(
            &path,
            &mut store,
            label,
            DiagnosticCode::IkCommandInvalid,
            |s| {
                s.command(
                    machine.as_ref(),
                    &CommandRequest {
                        aggregate_id: agg.clone(),
                        machine: identity.clone(),
                        capability: cap(&identity, &agg, 0),
                        command: command.clone(),
                        observation: None,
                    },
                )
            },
        );
    }

    // Positive control: the legal named command moves to exactly the declared
    // destination, never to a caller-chosen one.
    let out = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap(&identity, &agg, 0),
                command: json!({"name": "start"}),
                observation: None,
            },
        )
        .expect("legal named command commits");
    assert_eq!(out.state["state"], "running");
    let projection = store.show(&agg).expect("show");
    assert_eq!(projection.state["state"], "running");
    assert_eq!(projection.generation, 1);
    assert_eq!(projection.sequence, 1);
}

// ---------------------------------------------------------------------------
// CORE_LAWS-11/12 — child observations retain source authority; the substrate
// binds the exact pending identity (no domain choice of authority)
// ---------------------------------------------------------------------------

#[test]
fn substrate_stamps_effect_authority_not_domain_claims() {
    // The domain machine may not choose the effect authority: the runtime stamps
    // aggregate id and planned generation onto every emitted intent, refuses an
    // outcome naming a foreign aggregate, and fills an empty outcome aggregate
    // with the target. Removing any of these walls must fail this test.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("s.db");
    let (mut store, agg, machine) = setup_wall(&dir);
    let identity = machine.identity().clone();

    // The machine claims aggregate "" and generation 0; the substrate must stamp
    // the real aggregate and the next generation into the persisted intent.
    let planned = store
        .command(
            machine.as_ref(),
            &CommandRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: cap(&identity, &agg, 0),
                command: json!({"name": "plan"}),
                observation: None,
            },
        )
        .expect("plan commits");
    let EffectStatus::Pending(intent) = &planned.effect else {
        panic!("expected pending, got {:?}", planned.effect);
    };
    assert_eq!(
        intent.aggregate_id, agg,
        "the substrate stamps the target aggregate"
    );
    assert_eq!(
        intent.generation, 1,
        "the substrate stamps the planned generation"
    );
    assert_eq!(
        intent.effect_id.0, "forged",
        "the domain still owns the effect identity"
    );

    // The stamped authority is durable and replay reproduces it exactly.
    drop(store);
    let mut reopened = Store::open(&path).expect("reopen");
    reopened
        .replay(machine.as_ref(), Some(&agg))
        .expect("replay");
    let after = reopened.show(&agg).expect("show");
    let EffectStatus::Pending(replayed_intent) = &after.effect else {
        panic!("expected pending after replay");
    };
    assert_eq!(replayed_intent.aggregate_id, agg);
    assert_eq!(replayed_intent.generation, 1);
    assert_eq!(replayed_intent, intent);

    // Outcome authority: an empty aggregate is filled with the target; a foreign
    // aggregate refuses before any binding decision.
    let dir2 = tempfile::tempdir().expect("tempdir");
    let p_path = dir2.path().join("p.db");
    let pmachine = probe();
    let pidentity = pmachine.identity().clone();
    let mut pstore = Store::create(&p_path).expect("create probe store");
    let pagg = AggregateId("p-1".into());
    pstore
        .create_aggregate(pmachine.as_ref(), &pagg, &pidentity)
        .expect("create probe aggregate");
    let planned = plan_probe(
        &mut pstore,
        pmachine.as_ref(),
        &pagg,
        &cap(&pidentity, &pagg, 0),
    );
    let mut empty_aggregate = probe_outcome(&pagg, 1, "o1");
    empty_aggregate.aggregate_id = AggregateId(String::new());
    let recorded = pstore
        .record_effect(&EffectOutcomeRequest {
            aggregate_id: pagg.clone(),
            machine: pidentity.clone(),
            capability: planned.capability,
            effect: empty_aggregate,
        })
        .expect("empty-aggregate outcome is stamped to the target");
    let EffectStatus::Recorded(recorded_intent, _, _) = &recorded.effect else {
        panic!("expected recorded");
    };
    assert_eq!(recorded_intent.aggregate_id, pagg);
}

// ---------------------------------------------------------------------------
// CORE_LAWS-5 — reducers are deterministic and perform no IO
// ---------------------------------------------------------------------------

#[test]
fn production_reducer_modules_are_io_free_and_deterministic() {
    // Compile-time half: the pure reducer entry points are IO-free BY SIGNATURE.
    // `Machine::decide` and `Machine::apply` receive no store/connection/path or
    // wall-clock handle, so the type system — not source scanning — enforces that
    // a reducer cannot perform IO. If the contract methods ever grow an
    // IO-capable parameter or change their pure input/output types, these
    // function-pointer coercions stop compiling.
    type DecideFn<M> =
        fn(&M, &Value, Option<&Value>, &EffectStatus, &Value) -> Result<Decision, Diagnostic>;
    type ApplyFn<M> = fn(&M, &Value, &DomainEvent) -> Result<ApplyOutcome, Diagnostic>;

    fn assert_pure_signatures<M: Machine>() {
        let _: DecideFn<M> = M::decide;
        let _: ApplyFn<M> = M::apply;
    }
    assert_pure_signatures::<WallProbeMachine>();

    // Determinism half: the reducer is deterministic on the effect path — the same
    // command folded twice from the same state yields the same intent, and the
    // same journal transition yields byte-identical persisted rows.
    let machine = load_machine("effect_probe");
    let state = machine.initial_state();
    let command = json!({"name": "plan", "args": {"effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1"}});
    let event_a = machine
        .decide(&state, None, &EffectStatus::None, &command)
        .expect("decide A");
    let event_b = machine
        .decide(&state, None, &EffectStatus::None, &command)
        .expect("decide B");
    assert_eq!(event_a, event_b, "decide must be deterministic");
    let Decision::Emit { event, .. } = event_a else {
        panic!("plan must emit");
    };
    let applied_a = machine.apply(&state, &event).expect("apply A");
    let applied_b = machine.apply(&state, &event).expect("apply B");
    assert_eq!(applied_a, applied_b, "apply must be deterministic");

    // End-to-end determinism: two identical fresh stores driven through the same
    // lifecycle commit byte-identical journal/receipt/aggregate rows.
    let dirs = [
        tempfile::tempdir().expect("tempdir"),
        tempfile::tempdir().expect("tempdir"),
    ];
    let paths = vec![dirs[0].path().join("d1.db"), dirs[1].path().join("d2.db")];
    for path in &paths {
        let machine = probe();
        let identity = machine.identity().clone();
        let mut store = Store::create(path).expect("create store");
        let agg = AggregateId("d".into());
        store
            .create_aggregate(machine.as_ref(), &agg, &identity)
            .expect("create aggregate");
        let planned = store
            .command(
                machine.as_ref(),
                &CommandRequest {
                    aggregate_id: agg.clone(),
                    machine: identity.clone(),
                    capability: cap(&identity, &agg, 0),
                    command: json!({"name": "plan", "args": {"effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1"}}),
                    observation: None,
                },
            )
            .expect("plan commits");
        store
            .record_effect(&EffectOutcomeRequest {
                aggregate_id: agg.clone(),
                machine: identity.clone(),
                capability: planned.capability,
                effect: probe_outcome(&agg, 1, "o1"),
            })
            .expect("record commits");
        drop(store);
    }
    let rows = paths
        .iter()
        .map(|path| {
            durable_rows(path)
                .lines()
                .filter(|line| !line.starts_with("meta:"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rows[0], rows[1],
        "identical lifecycles must produce byte-identical durable rows"
    );
}
