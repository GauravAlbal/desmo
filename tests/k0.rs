//! K0 — Pure machine substrate.
//!
//! Defends: declarative machine loads; native machine loads through one registry;
//! pure reducers use no IO; static checker rejects malformed fixtures; stable
//! diagnostic codes; deterministic reduction; machine identity/hash handshake; native
//! and declarative machines commit through the same capability/transaction path.

#[path = "common/mod.rs"]
mod common;

use common::*;
use ik::declarative::parse_and_compile;
use ik::diagnostic::{Diagnostic, DiagnosticCode};
use ik::effect::{EffectIntentV1, EffectOutcomeV1, EffectStatus};
use ik::identity::{AggregateId, Capability, EffectId};
use ik::machine::{
    Decision, DomainEvent, EventCodec, Machine, NativeAdapter, NativeApply, NativeDecision,
    NativeMachine,
};
use ik::prelude::MachineIdentity;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// A native test machine (typed reducer over a generic counter).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct CountState {
    count: i64,
    done: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "name", rename_all = "lowercase")]
enum CountCommand {
    Incr,
    Stop,
    Plan,
    Resolve { outcome_digest: String },
}

#[derive(Clone, Debug, Deserialize)]
struct NoObservation;

#[derive(Clone, Debug)]
enum CountDiagnostic {
    Illegal(&'static str),
}

impl From<CountDiagnostic> for Diagnostic {
    fn from(d: CountDiagnostic) -> Self {
        match d {
            CountDiagnostic::Illegal(rule) => Diagnostic::new(
                DiagnosticCode::Domain("TEST_ILLEGAL".to_string()),
                Some(rule),
                "domain refusal",
            ),
        }
    }
}

/// Versioned domain events for the counter machine. `decide` freezes every value
/// `apply` needs into the event; `apply` never consults commands or observations.
#[derive(Clone, Debug, PartialEq)]
enum CountEvent {
    Incr,
    Stop,
    Plan,
    Resolve { outcome_digest: String },
}

impl EventCodec for CountEvent {
    fn event_type(&self) -> &'static str {
        match self {
            CountEvent::Incr => "incr",
            CountEvent::Stop => "stop",
            CountEvent::Plan => "plan",
            CountEvent::Resolve { .. } => "resolve",
        }
    }

    fn event_version(&self) -> u64 {
        1
    }

    fn payload(&self) -> Value {
        match self {
            CountEvent::Incr | CountEvent::Stop | CountEvent::Plan => json!({}),
            CountEvent::Resolve { outcome_digest } => json!({ "outcome_digest": outcome_digest }),
        }
    }

    fn decode(event: &DomainEvent) -> Result<Option<Self>, Diagnostic> {
        // Unknown event type or version is not this machine's event schema.
        if event.event_version != 1 {
            return Ok(None);
        }
        match event.event_type.as_str() {
            "incr" => Ok(Some(CountEvent::Incr)),
            "stop" => Ok(Some(CountEvent::Stop)),
            "plan" => Ok(Some(CountEvent::Plan)),
            "resolve" => {
                let outcome_digest = event
                    .payload
                    .get("outcome_digest")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        Diagnostic::internal("count resolve event payload lacks outcome_digest")
                    })?;
                Ok(Some(CountEvent::Resolve {
                    outcome_digest: outcome_digest.to_string(),
                }))
            }
            _ => Ok(None),
        }
    }
}

struct CountMachine {
    identity: MachineIdentity,
}

impl CountMachine {
    fn new() -> Self {
        Self {
            identity: ik::machine::native_identity("native_counter", 1, "native-counter-spec-v1"),
        }
    }
}

impl NativeMachine for CountMachine {
    type State = CountState;
    type Command = CountCommand;
    type Observation = NoObservation;
    type Event = CountEvent;
    type Diagnostic = CountDiagnostic;

    fn identity(&self) -> &MachineIdentity {
        &self.identity
    }

    fn initial_state(&self) -> Self::State {
        CountState {
            count: 0,
            done: false,
        }
    }

    fn decide(
        &self,
        state: &Self::State,
        _observation: Option<&Self::Observation>,
        effect: &EffectStatus,
        command: &Self::Command,
    ) -> Result<NativeDecision<Self::Event>, Self::Diagnostic> {
        if state.done {
            return Err(CountDiagnostic::Illegal("terminal"));
        }
        match command {
            CountCommand::Incr => Ok(NativeDecision::emit(CountEvent::Incr)),
            CountCommand::Stop => Ok(NativeDecision::emit(CountEvent::Stop)),
            CountCommand::Plan => {
                if effect.pending_intent().is_some() {
                    return Err(CountDiagnostic::Illegal("effect.pending"));
                }
                Ok(NativeDecision::emit(CountEvent::Plan))
            }
            CountCommand::Resolve { outcome_digest } => {
                Ok(NativeDecision::emit(CountEvent::Resolve {
                    outcome_digest: outcome_digest.clone(),
                }))
            }
        }
    }

    fn apply(
        &self,
        state: &Self::State,
        event: &Self::Event,
    ) -> Result<NativeApply<Self::State>, Self::Diagnostic> {
        match event {
            CountEvent::Incr => Ok(NativeApply::new(CountState {
                count: state.count + 1,
                done: false,
            })),
            CountEvent::Stop => Ok(NativeApply::new(CountState {
                count: state.count,
                done: true,
            })
            .with_terminal(true)),
            CountEvent::Plan => {
                let intent = EffectIntentV1 {
                    effect_id: EffectId("native-effect".to_string()),
                    aggregate_id: AggregateId(String::new()),
                    generation: 0,
                    effect_kind: "COUNT".to_string(),
                    payload_digest: "native-payload".to_string(),
                    idempotency_key: "native-key".to_string(),
                    payload: None,
                };
                Ok(NativeApply::new(state.clone()).with_effect(intent))
            }
            CountEvent::Resolve { outcome_digest } => {
                let outcome = EffectOutcomeV1 {
                    effect_id: EffectId("native-effect".to_string()),
                    aggregate_id: AggregateId(String::new()),
                    planned_generation: 0,
                    payload_digest: "native-payload".to_string(),
                    idempotency_key: "native-key".to_string(),
                    outcome_digest: outcome_digest.clone(),
                };
                Ok(NativeApply::new(state.clone()).with_outcome(outcome))
            }
        }
    }
}

fn count_machine() -> Box<dyn ik::machine::Machine> {
    Box::new(NativeAdapter::new(CountMachine::new()))
}

/// Fold a command through the live-only `decide` path and the replayable `apply`
/// path, returning the authoritative `ApplyOutcome`.
fn fold(
    machine: &dyn ik::machine::Machine,
    state: &Value,
    command: &Value,
) -> Result<ik::machine::ApplyOutcome, Diagnostic> {
    let decision = machine.decide(state, None, &EffectStatus::None, command)?;
    let Decision::Emit { event, .. } = decision else {
        panic!("expected an emitted event");
    };
    machine.apply(state, &event)
}

fn va_machine() -> Box<dyn ik::machine::Machine> {
    Box::new(load_machine("verification_attempt"))
}

fn probe_machine() -> Box<dyn ik::machine::Machine> {
    Box::new(load_machine("effect_probe"))
}

// ---------------------------------------------------------------------------
// K0 tests
// ---------------------------------------------------------------------------

#[test]
fn declarative_machine_loads_with_stable_identity() {
    let machine = va_machine();
    let identity = machine.identity();
    assert_eq!(identity.machine_id, "verification_attempt");
    assert_eq!(identity.version.0, 1);
    assert_eq!(identity.spec_hash.0.len(), 64);
    // Loading the same definition twice yields the same spec hash.
    let again = va_machine();
    assert_eq!(again.identity(), machine.identity());
    // Initial state matches the declared initial state + initial facts.
    assert_eq!(
        machine.initial_state(),
        json!({"state": "created", "attempt_count": 0, "max_attempts": 3})
    );
}

#[test]
fn native_machine_loads_through_adapter_and_registry() {
    let mut registry = ik::machine::MachineRegistry::new();
    let identity = count_machine().identity().clone();
    let registration = registry.register(count_machine()).expect("register");
    assert!(registry.resolve(&registration, &identity).is_ok());
    // Unknown id / version / hash classification.
    let unknown_id = ik::machine::native_identity("nope", 1, "x");
    assert_eq!(
        registry_code(&registry, &registration, &unknown_id),
        DiagnosticCode::IkMachineUnknown
    );
    let unknown_version = ik::machine::native_identity("native_counter", 99, "x");
    assert_eq!(
        registry_code(&registry, &registration, &unknown_version),
        DiagnosticCode::IkMachineVersionUnknown
    );
    let wrong_hash = ik::machine::native_identity("native_counter", 1, "wrong");
    assert_eq!(
        registry_code(&registry, &registration, &wrong_hash),
        DiagnosticCode::IkMachineHashMismatch
    );
    // Duplicate registration refuses.
    assert!(registry.register(count_machine()).is_err());
}

/// A declarative machine registers under its exact identity and is resolved
/// from the registry for the same Store-class entrypoints as a native machine;
/// every identity deviation fails closed with the stable classification.
#[test]
fn declarative_machine_registers_and_resolves_by_exact_identity() {
    let mut registry = ik::machine::MachineRegistry::new();
    let machine = va_machine();
    let identity = machine.identity().clone();
    let registration = registry
        .register(machine)
        .expect("register compiled machine");

    // Exact identity resolves; the resolved machine IS the registered machine.
    let resolved = registry
        .resolve(&registration, &identity)
        .expect("exact identity resolves");
    assert_eq!(resolved.identity(), &identity);

    // Any deviation fails closed with the stable classification.
    let mut wrong_version = identity.clone();
    wrong_version.version = ik::identity::MachineVersion(identity.version.0 + 1);
    assert_eq!(
        registry_code(&registry, &registration, &wrong_version),
        DiagnosticCode::IkMachineVersionUnknown
    );
    let mut wrong_hash = identity.clone();
    wrong_hash.spec_hash = ik::identity::MachineSpecHash("deadbeef".to_string());
    assert_eq!(
        registry_code(&registry, &registration, &wrong_hash),
        DiagnosticCode::IkMachineHashMismatch
    );
    let wrong_id = ik::machine::native_identity("someone.else", 1, "x");
    assert_eq!(
        registry_code(&registry, &registration, &wrong_id),
        DiagnosticCode::IkMachineUnknown
    );

    // A capability issued by another registry cannot authorize this one even
    // when both contain byte-identical machine identities.
    let mut other = ik::machine::MachineRegistry::default();
    let foreign = other.register(va_machine()).expect("register in other");
    let diagnostic = match registry.resolve(&foreign, &identity) {
        Err(diagnostic) => diagnostic,
        Ok(_) => panic!("foreign registry capability must refuse"),
    };
    assert_eq!(diagnostic.code, DiagnosticCode::IkMachineMismatch);
    assert_eq!(diagnostic.rule.as_deref(), Some("registry.capability"));

    // Duplicate registration of the same identity refuses.
    assert!(registry.register(va_machine()).is_err());
}

fn registry_code(
    registry: &ik::machine::MachineRegistry,
    registration: &ik::machine::RegisteredMachine,
    identity: &MachineIdentity,
) -> DiagnosticCode {
    match registry.resolve(registration, identity) {
        Err(d) => d.code.clone(),
        Ok(_) => panic!("expected a registry refusal for {identity:?}"),
    }
}

#[test]
fn static_checker_rejects_malformed_fixtures() {
    let cases: Vec<(&str, &str)> = vec![
        ("unknown schema version", "schema_version = 2\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\n[states]\ns = { initial = true }\n[commands.c]\nfrom = [\"s\"]\nto = \"s\""),
        ("no initial state", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\n[states]\ns = {}\n[commands.c]\nfrom = [\"s\"]\nto = \"s\""),
        ("two initial states", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\n[states]\ns1 = { initial = true }\ns2 = { initial = true }\n[commands.c]\nfrom = [\"s1\"]\nto = \"s2\""),
        ("unknown source state", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\n[states]\ns = { initial = true }\n[commands.c]\nfrom = [\"ghost\"]\nto = \"s\""),
        ("unknown destination state", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\n[states]\ns = { initial = true }\n[commands.c]\nfrom = [\"s\"]\nto = \"ghost\""),
        ("unknown fact in guard", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\nx = { type = \"int\", policy = \"counter\", initial = 0 }\n[states]\ns = { initial = true }\nt = {}\n[commands.c]\nfrom = [\"s\"]\nto = \"t\"\nwhen = \"ghost == 1\""),
        ("type mismatch in guard", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\nx = { type = \"int\", policy = \"counter\", initial = 0 }\n[states]\ns = { initial = true }\nt = {}\n[commands.c]\nfrom = [\"s\"]\nto = \"t\"\nwhen = \"x == 'str'\""),
        ("unknown fact in then", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\nx = { type = \"int\", policy = \"counter\", initial = 0 }\n[states]\ns = { initial = true }\nt = {}\n[commands.c]\nfrom = [\"s\"]\nto = \"t\"\nthen = { ghost = \"1\" }"),
        ("kernel_only fact assigned", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\nk = { type = \"int\", policy = \"kernel_only\", initial = 0 }\n[states]\ns = { initial = true }\nt = {}\n[commands.c]\nfrom = [\"s\"]\nto = \"t\"\nthen = { k = \"1\" }"),
        ("counter non-increment", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\nx = { type = \"int\", policy = \"counter\", initial = 0 }\n[states]\ns = { initial = true }\nt = {}\n[commands.c]\nfrom = [\"s\"]\nto = \"t\"\nthen = { x = \"5\" }"),
        ("unknown arg in effect", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\n[states]\ns = { initial = true }\nt = {}\n[commands.c]\nfrom = [\"s\"]\nto = \"t\"\neffect = { kind = \"K\", id = \"$ghost\", payload_digest = \"p\", idempotency_key = \"i\" }"),
        ("initial state terminal", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\n[states]\ns = { initial = true, terminal = true }\n[commands.c]\nfrom = [\"s\"]\nto = \"s\""),
        ("terminal state as source", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\n[states]\ns = { initial = true }\nt = { terminal = true }\n[commands.c]\nfrom = [\"t\"]\nto = \"s\""),
        ("missing destination", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\n[states]\ns = { initial = true }\n[commands.c]\nfrom = [\"s\"]"),
        ("unknown invariant fact", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\nx = { type = \"int\", policy = \"counter\", initial = 0 }\n[states]\ns = { initial = true }\nt = {}\n[commands.c]\nfrom = [\"s\"]\nto = \"t\"\n[invariants.i]\nassert = \"ghost <= x\""),
        ("bad guard syntax", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\n[states]\ns = { initial = true }\nt = {}\n[commands.c]\nfrom = [\"s\"]\nto = \"t\"\nwhen = \"x ==\""),
        ("reserved genesis command", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\n[states]\ns = { initial = true }\n[commands.genesis]\nfrom = [\"s\"]\nto = \"s\""),
        ("reserved kernel command", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\n[states]\ns = { initial = true }\n[commands.\"kernel.effect\"]\nfrom = [\"s\"]\nto = \"s\""),
        ("invariant type mismatch", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\nx = { type = \"int\", policy = \"counter\", initial = 0 }\n[states]\ns = { initial = true }\n[commands.c]\nfrom = [\"s\"]\nto = \"s\"\n[invariants.i]\nassert = \"x != '5'\""),
        ("invariant boolean ordered cmp", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\nb = { type = \"bool\", policy = \"replace\", initial = false }\n[states]\ns = { initial = true }\n[commands.c]\nfrom = [\"s\"]\nto = \"s\"\n[invariants.i]\nassert = \"b < true\""),
        ("invariant arg reference", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\nx = { type = \"int\", policy = \"counter\", initial = 0 }\n[states]\ns = { initial = true }\n[commands.c]\nfrom = [\"s\"]\nto = \"s\"\n[invariants.i]\nassert = \"$x == 1\""),
        ("unknown state literal in guard", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\n[states]\ns = { initial = true }\n[commands.c]\nfrom = [\"s\"]\nto = \"s\"\nwhen = \"state == 'ghost'\""),
        ("unknown state literal in invariant", "schema_version = 1\nmachine_id = \"m\"\nmachine_version = 1\n[facts]\nx = { type = \"int\", policy = \"counter\", initial = 0 }\n[states]\ns = { initial = true }\n[commands.c]\nfrom = [\"s\"]\nto = \"s\"\n[invariants.i]\nwhen = \"state == 'ghost'\"\nassert = \"x >= 0\""),
    ];
    for (label, text) in cases {
        let result = parse_and_compile(text);
        assert!(result.is_err(), "static check must reject: {label}");
    }
}

#[test]
fn once_policy_allows_initial_write_and_refuses_overwrite() {
    let toml = r#"
schema_version = 1
machine_id = "once_guard"
machine_version = 1
[facts]
token = { type = "string", policy = "once" }
[states]
draft = { initial = true }
sealed = {}
done = { terminal = true }
[commands.seal]
from = ["draft", "sealed"]
to = "sealed"
args = { token = "string" }
then = { token = "$token" }
[commands.finish]
from = ["sealed"]
to = "done"
"#;
    let m = parse_and_compile(toml).expect("compile once machine");
    let st0 = m.initial_state();
    let st1 = fold(
        &m,
        &st0,
        &json!({ "name": "seal", "args": { "token": "first" } }),
    )
    .expect("first write to once fact succeeds")
    .state;
    assert_eq!(st1["token"], "first");

    let err = fold(
        &m,
        &st1,
        &json!({ "name": "seal", "args": { "token": "second" } }),
    )
    .expect_err("second write to once fact must refuse");
    assert_eq!(err.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(err.rule.as_deref(), Some("seal.then.token"));
}

#[test]
fn pure_reducers_ignore_process_environment() {
    // REQ-IK-014: reducer decisions must not derive from environment, cwd, or time.
    let machine = va_machine();
    let state = machine.initial_state();
    let command = json!({"name": "start"});
    let baseline = fold(machine.as_ref(), &state, &command).expect("fold");

    std::env::set_var("IK_NOISE_VAR", "changed");
    let cwd = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(repo_root().join("..")).expect("chdir");
    let after_env = fold(machine.as_ref(), &state, &command).expect("fold");
    std::env::set_current_dir(cwd).expect("chdir back");
    std::env::remove_var("IK_NOISE_VAR");

    assert_eq!(baseline, after_env);
}

#[test]
fn reduction_is_deterministic() {
    let machine = probe_machine();
    let state = machine.initial_state();
    let command = json!({"name": "plan", "args": {"effect_id": "e1", "payload_digest": "p1", "idempotency_key": "k1"}});
    let a = fold(machine.as_ref(), &state, &command).expect("fold");
    let b = fold(machine.as_ref(), &state, &command).expect("fold");
    assert_eq!(a, b);
}

#[test]
fn illegal_state_and_bad_commands_carry_stable_codes() {
    let machine = va_machine();
    let created = machine.initial_state();

    // Unknown command -> IK_COMMAND_INVALID.
    let err = machine
        .decide(
            &created,
            None,
            &EffectStatus::None,
            &json!({"name": "nope"}),
        )
        .unwrap_err();
    assert_eq!(err.code, DiagnosticCode::IkCommandInvalid);
    assert_eq!(err.rule.as_deref(), Some("command.unknown"));

    // Command not legal from the current state -> IK_ILLEGAL_STATE.
    let err = machine
        .decide(
            &created,
            None,
            &EffectStatus::None,
            &json!({"name": "submit", "args": {"artifact_digest": "a", "baseline_digest": "b"}}),
        )
        .unwrap_err();
    assert_eq!(err.code, DiagnosticCode::IkIllegalState);

    // Missing argument -> IK_COMMAND_INVALID.
    let err = machine
        .decide(
            &json!({"state": "running", "attempt_count": 0, "max_attempts": 3}),
            None,
            &EffectStatus::None,
            &json!({"name": "submit", "args": {"artifact_digest": "a"}}),
        )
        .unwrap_err();
    assert_eq!(err.code, DiagnosticCode::IkCommandInvalid);

    // Wrong verification binding -> IK_INVARIANT_FAILED with a rule.
    let err = machine
        .decide(
            &json!({"state": "submitted", "attempt_count": 0, "max_attempts": 3, "artifact_digest": "a1", "baseline_digest": "b1"}),
            None,
            &EffectStatus::None,
            &json!({"name": "verify", "args": {"receipt_digest": "r", "receipt_artifact_digest": "WRONG", "receipt_baseline_digest": "b1"}}),
        )
        .unwrap_err();
    assert_eq!(err.code, DiagnosticCode::IkInvariantFailed);
    assert!(err.rule.is_some());

    // Terminal states reject ordinary mutation.
    let verified = json!({"state": "verified", "attempt_count": 0, "max_attempts": 3, "artifact_digest": "a1", "baseline_digest": "b1", "receipt_digest": "r", "receipt_artifact_digest": "a1", "receipt_baseline_digest": "b1"});
    let err = machine
        .decide(
            &verified,
            None,
            &EffectStatus::None,
            &json!({"name": "start"}),
        )
        .unwrap_err();
    assert_eq!(err.code, DiagnosticCode::IkIllegalState);
}

#[test]
fn exhaustion_boundary_is_exact() {
    // attempt_count + 1 >= max_attempts(3) flips to EXHAUSTED exactly at 3.
    let machine = va_machine();
    let two = json!({"state": "submitted", "attempt_count": 2, "max_attempts": 3, "artifact_digest": "a", "baseline_digest": "b"});
    let outcome = fold(machine.as_ref(), &two, &json!({"name": "reject"})).expect("reject");
    assert!(outcome.terminal);
    assert_eq!(outcome.state["state"], "exhausted");
    assert_eq!(outcome.state["attempt_count"], 3);
}

#[test]
fn native_and_declarative_share_one_transaction_path() {
    // Both machine classes commit through the same store path: create + command.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut store = ik::store::Store::create(&dir.path().join("s.db")).expect("create store");

    // Declarative.
    let va = va_machine();
    let va_identity = va.identity().clone();
    let result = store
        .create_aggregate(va.as_ref(), &AggregateId("va".into()), &va_identity)
        .expect("va create");
    assert_eq!(result.generation, 0);

    // Native.
    let native = count_machine();
    let native_identity = native.identity().clone();
    let result = store
        .create_aggregate(
            native.as_ref(),
            &AggregateId("native".into()),
            &native_identity,
        )
        .expect("native create");
    assert_eq!(result.generation, 0);

    // Same command path for both.
    let cap = Capability {
        generation: 0,
        nonce: ik::runtime::derive_nonce(&va_identity, &AggregateId("va".into()), 0),
    };
    let out = store
        .command(
            va.as_ref(),
            &ik::store::CommandRequest {
                aggregate_id: AggregateId("va".into()),
                machine: va_identity.clone(),
                capability: cap,
                command: json!({"name": "start"}),
                observation: None,
            },
        )
        .expect("va command");
    assert_eq!(out.generation, 1);

    let cap = Capability {
        generation: 0,
        nonce: ik::runtime::derive_nonce(&native_identity, &AggregateId("native".into()), 0),
    };
    let out = store
        .command(
            native.as_ref(),
            &ik::store::CommandRequest {
                aggregate_id: AggregateId("native".into()),
                machine: native_identity,
                capability: cap,
                command: json!({"name": "incr"}),
                observation: None,
            },
        )
        .expect("native command");
    assert_eq!(out.generation, 1);
    assert_eq!(out.state["count"], 1);
}

#[test]
fn declarative_invariant_refuses_violating_transition_behaviorally() {
    let machine = load_machine("invariant_probe");
    let decision = machine
        .decide(
            &machine.initial_state(),
            None,
            &EffectStatus::None,
            &json!({"name": "violate"}),
        )
        .expect("the command resolves before post-state invariant validation");
    let Decision::Emit { event, .. } = decision else {
        panic!("violating command must emit a candidate event");
    };
    let err = machine
        .apply(&machine.initial_state(), &event)
        .expect_err("violating transition must be refused");
    assert_eq!(err.code, DiagnosticCode::IkInvariantFailed);
    assert_eq!(err.rule.as_deref(), Some("x_limit"));
}
