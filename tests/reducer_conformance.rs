//! Shared native/external reducer vectors.  The Python tests read the same JSON file;
//! this test intentionally uses the public `NativeMachine` contract rather than the
//! declarative compiler so both reducer implementations have an independent oracle.

use ik::diagnostic::{Diagnostic, DiagnosticCode};
use ik::effect::{EffectIntentV1, EffectStatus};
use ik::identity::{AggregateId, EffectId, MachineIdentity};
use ik::machine::{native_identity, EventCodec, NativeApply, NativeDecision, NativeMachine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ParityEvent {
    event_type: String,
    event_version: u64,
    payload: Value,
}

impl EventCodec for ParityEvent {
    fn event_type(&self) -> &'static str {
        match self.event_type.as_str() {
            "parity.accepted" => "parity.accepted",
            "parity.effect_planned" => "parity.effect_planned",
            "parity.effect_resolved" => "parity.effect_resolved",
            "parity.finished" => "parity.finished",
            "wedding.kiss_noah" => "wedding.kiss_noah",
            _ => "unknown",
        }
    }

    fn event_version(&self) -> u64 {
        self.event_version
    }

    fn payload(&self) -> Value {
        self.payload.clone()
    }

    fn decode(event: &ik::machine::DomainEvent) -> Result<Option<Self>, Diagnostic> {
        if event.event_version != 1
            || !matches!(
                event.event_type.as_str(),
                "parity.accepted"
                    | "parity.effect_planned"
                    | "parity.effect_resolved"
                    | "parity.finished"
                    | "wedding.kiss_noah"
            )
        {
            return Ok(None);
        }
        Ok(Some(Self {
            event_type: event.event_type.clone(),
            event_version: event.event_version,
            payload: event.payload.clone(),
        }))
    }
}

#[derive(Clone)]
struct ParityNative {
    identity: MachineIdentity,
}

impl ParityNative {
    fn new() -> Self {
        Self {
            identity: native_identity(
                "native-parity",
                1,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        }
    }
}

impl NativeMachine for ParityNative {
    type State = Value;
    type Command = Value;
    type Observation = Value;
    type Event = ParityEvent;
    type Diagnostic = Diagnostic;

    fn identity(&self) -> &MachineIdentity {
        &self.identity
    }

    fn initial_state(&self) -> Self::State {
        corpus()["initial_state"].clone()
    }

    fn decide(
        &self,
        _state: &Self::State,
        observation: Option<&Self::Observation>,
        _effect: &EffectStatus,
        command: &Self::Command,
    ) -> Result<NativeDecision<Self::Event>, Self::Diagnostic> {
        let name = command["name"].as_str().unwrap_or_default();
        match name {
            "accept" => {
                let amount = command["args"]["amount"].clone();
                let event = ParityEvent {
                    event_type: "parity.accepted".into(),
                    event_version: 1,
                    payload: json!({"amount": amount, "observed": observation.cloned()}),
                };
                Ok(NativeDecision::emit(event).with_note(json!({
                    "kind": "accepted",
                    "amount": amount
                })))
            }
            "reject" => Err(Diagnostic::new(
                DiagnosticCode::IkInvariantFailed,
                Some("parity.reject"),
                "command was rejected",
            )),
            "noop" => Ok(NativeDecision::noop().with_note(json!({
                "kind": "noop",
                "reason": "nothing_to_do"
            }))),
            "plan_effect" => {
                if !command["args"]["enabled"].as_bool().unwrap_or(false) {
                    return Ok(NativeDecision::noop().with_note(json!({
                        "kind": "noop",
                        "reason": "effect_disabled"
                    })));
                }
                Ok(NativeDecision::emit(ParityEvent {
                    event_type: "parity.effect_planned".into(),
                    event_version: 1,
                    payload: json!({"enabled": true}),
                }))
            }
            "resolve_effect" => Ok(NativeDecision::emit(ParityEvent {
                event_type: "parity.effect_resolved".into(),
                event_version: 1,
                payload: json!({}),
            })),
            "finish" => Ok(NativeDecision::emit(ParityEvent {
                event_type: "parity.finished".into(),
                event_version: 1,
                payload: json!({}),
            })),
            "kiss_noah" => Ok(NativeDecision::emit(ParityEvent {
                event_type: "wedding.kiss_noah".into(),
                event_version: 1,
                payload: json!({
                    "appraisal_delta": command["args"]["appraisal_delta"],
                    "clock": command["args"]["clock"],
                    "observation": observation.cloned(),
                    "obligation": "tell_noah_the_truth",
                    "route": command["args"]["route"],
                    "stage": command["args"]["stage"],
                    "trust_delta": command["args"]["trust_delta"]
                }),
            })
            .with_note(json!({
                "scene": "WEDDING-WEEKEND-01",
                "transition": "D_KISS_NOAH"
            }))),
            _ => Err(Diagnostic::new(
                DiagnosticCode::IkCommandInvalid,
                Some("parity.command"),
                "unknown command",
            )),
        }
    }

    fn apply(
        &self,
        state: &Self::State,
        event: &Self::Event,
    ) -> Result<NativeApply<Self::State>, Self::Diagnostic> {
        let mut result = state.clone();
        let object = result.as_object_mut().expect("fixture state object");
        let payload = &event.payload;
        match event.event_type.as_str() {
            "parity.accepted" => {
                object.insert(
                    "score".into(),
                    json!(state["score"].as_i64().unwrap() + payload["amount"].as_i64().unwrap()),
                );
                object.insert("observed".into(), payload["observed"].clone());
                Ok(NativeApply::new(result))
            }
            "parity.effect_planned" => {
                object.insert("effect_state".into(), json!("planned"));
                let effect = EffectIntentV1 {
                    effect_id: EffectId("parity-effect-1".into()),
                    aggregate_id: AggregateId("parity-aggregate".into()),
                    generation: 1,
                    effect_kind: "send.invitation".into(),
                    payload_digest:
                        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                    idempotency_key: "parity-effect-1".into(),
                    payload: Some(json!({"to": "mira"})),
                };
                Ok(NativeApply::new(result).with_effect(effect))
            }
            "parity.effect_resolved" => {
                object.insert("effect_state".into(), json!("resolved"));
                let outcome = ik::effect::EffectOutcomeV1 {
                    effect_id: EffectId("parity-effect-1".into()),
                    aggregate_id: AggregateId("parity-aggregate".into()),
                    planned_generation: 1,
                    payload_digest:
                        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                    idempotency_key: "parity-effect-1".into(),
                    outcome_digest:
                        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into(),
                };
                Ok(NativeApply::new(result).with_outcome(outcome))
            }
            "parity.finished" => {
                object.insert("finished".into(), json!(true));
                Ok(NativeApply::new(result).with_terminal(true))
            }
            "wedding.kiss_noah" => {
                object.insert("clock".into(), payload["clock"].clone());
                object["facts"]["kiss_noah"] = json!(true);
                object.insert("obligation".into(), payload["obligation"].clone());
                object.insert("observed".into(), payload["observation"].clone());
                object["relationship"]["appraisal"] = json!(
                    state["relationship"]["appraisal"].as_i64().unwrap()
                        + payload["appraisal_delta"].as_i64().unwrap()
                );
                object["relationship"]["stage"] = payload["stage"].clone();
                object["relationship"]["trust"] = json!(
                    state["relationship"]["trust"].as_i64().unwrap()
                        + payload["trust_delta"].as_i64().unwrap()
                );
                object.insert("route".into(), payload["route"].clone());
                Ok(NativeApply::new(result))
            }
            _ => Err(Diagnostic::new(
                DiagnosticCode::IkEventSchemaUnknown,
                Some("parity.event"),
                "unknown event",
            )),
        }
    }
}

fn corpus() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("conformance/reducers/native_parity_v1.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn native_machine_matches_every_shared_vector() {
    let vectors = corpus();
    let machine = ParityNative::new();
    assert_eq!(machine.initial_state(), vectors["initial_state"]);
    for case in vectors["cases"].as_array().unwrap() {
        let command = &case["command"];
        let observation = if case["observation"].is_null() {
            None
        } else {
            Some(&case["observation"])
        };
        let decision = machine.decide(
            &machine.initial_state(),
            observation,
            &EffectStatus::None,
            command,
        );
        if let Some(expected) = case.get("diagnostic") {
            let error = decision.unwrap_err();
            assert_eq!(error.code.as_str(), expected["code"].as_str().unwrap());
            assert_eq!(
                error.rule.as_deref(),
                Some(expected["rule"].as_str().unwrap())
            );
            continue;
        }
        let decision = decision.unwrap();
        match (&decision, case["decision"]["kind"].as_str().unwrap()) {
            (NativeDecision::Emit { event, note }, "emit") => {
                assert_eq!(
                    event.event_type,
                    case["decision"]["event"]["event_type"].as_str().unwrap()
                );
                assert_eq!(
                    event.event_version,
                    case["decision"]["event"]["event_version"].as_u64().unwrap()
                );
                assert_eq!(event.payload, case["decision"]["event"]["payload"]);
                assert_eq!(note.as_ref(), case["decision"].get("note"));
                if let Some(apply) = case.get("apply") {
                    let folded = machine.apply(&machine.initial_state(), event).unwrap();
                    assert_eq!(folded.state, apply["state"]);
                    assert_eq!(folded.terminal, apply["terminal"].as_bool().unwrap());
                    assert_eq!(
                        serde_json::to_value(&folded.effect).unwrap(),
                        apply.get("effect").cloned().unwrap_or(Value::Null)
                    );
                    assert_eq!(
                        serde_json::to_value(&folded.outcome).unwrap(),
                        apply.get("outcome").cloned().unwrap_or(Value::Null)
                    );
                }
            }
            (NativeDecision::Noop { note }, "noop") => {
                assert_eq!(note.as_ref(), case["decision"].get("note"));
            }
            _ => panic!("decision kind mismatch for {}", case["name"]),
        }
    }
}

#[test]
fn native_replay_is_deterministic_and_observation_free() {
    let vectors = corpus();
    let machine = ParityNative::new();
    let mut state = machine.initial_state();
    for raw in vectors["replay"].as_array().unwrap() {
        let event = ParityEvent {
            event_type: raw["event_type"].as_str().unwrap().into(),
            event_version: raw["event_version"].as_u64().unwrap(),
            payload: raw["payload"].clone(),
        };
        state = machine.apply(&state, &event).unwrap().state;
    }
    assert_eq!(state, vectors["replay_state"]);
    let first = machine
        .apply(
            &machine.initial_state(),
            &ParityEvent {
                event_type: "parity.accepted".into(),
                event_version: 1,
                payload: json!({"amount": 2, "observed": {"weather": "sunny", "witness": "mira"}}),
            },
        )
        .unwrap();
    let second = machine
        .apply(
            &machine.initial_state(),
            &ParityEvent {
                event_type: "parity.accepted".into(),
                event_version: 1,
                payload: json!({"amount": 2, "observed": {"weather": "sunny", "witness": "mira"}}),
            },
        )
        .unwrap();
    assert_eq!(first.state, second.state);
}
