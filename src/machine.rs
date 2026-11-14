//! Domain machine contracts for the invariant runtime.
//!
//! Commands are not events. [`Machine::decide`] is the live-only command path; it
//! validates intent and freezes every authoritative observation into a versioned
//! [`DomainEvent`]. [`Machine::apply`] is the replayable fold and is the sole source of
//! post-state, terminality, and domain-produced effect intent/outcome.

use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::effect::{EffectIntentV1, EffectOutcomeV1, EffectStatus};
use crate::identity::MachineIdentity;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// A versioned, self-sufficient domain fact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainEvent {
    pub event_type: String,
    pub event_version: u64,
    pub payload: Value,
}

impl DomainEvent {
    pub fn new(event_type: impl Into<String>, event_version: u64, payload: Value) -> Self {
        Self {
            event_type: event_type.into(),
            event_version,
            payload,
        }
    }
}

/// Live command decision. A no-op is never journaled.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    Emit {
        event: DomainEvent,
        note: Option<Value>,
    },
    Noop {
        note: Option<Value>,
    },
}

impl Decision {
    pub fn emit(event: DomainEvent) -> Self {
        Self::Emit { event, note: None }
    }

    pub fn noop() -> Self {
        Self::Noop { note: None }
    }

    pub fn with_note(mut self, note: Value) -> Self {
        match &mut self {
            Self::Emit { note: slot, .. } | Self::Noop { note: slot } => *slot = Some(note),
        }
        self
    }
}

/// Authoritative result of folding one domain event.
#[derive(Clone, Debug, PartialEq)]
pub struct ApplyOutcome {
    pub state: Value,
    pub terminal: bool,
    pub effect: Option<EffectIntentV1>,
    pub outcome: Option<EffectOutcomeV1>,
}

impl ApplyOutcome {
    pub fn new(state: Value) -> Self {
        Self {
            state,
            terminal: false,
            effect: None,
            outcome: None,
        }
    }

    pub fn with_terminal(mut self, terminal: bool) -> Self {
        self.terminal = terminal;
        self
    }

    pub fn with_effect(mut self, effect: EffectIntentV1) -> Self {
        self.effect = Some(effect);
        self
    }

    pub fn with_outcome(mut self, outcome: EffectOutcomeV1) -> Self {
        self.outcome = Some(outcome);
        self
    }
}

/// Uniform machine contract used by the runtime.
pub trait Machine: Send + Sync {
    fn identity(&self) -> &MachineIdentity;
    fn initial_state(&self) -> Value;

    /// Admission check run before any genesis row, receipt, or capability is
    /// produced. A machine that declares hard laws MUST refuse an initial state
    /// that violates them. The default accepts: the generic runtime cannot
    /// invent domain invariants a machine has not declared, so a machine that
    /// does not override this keeps its initial state outside the valid-genesis
    /// claim.
    fn validate_initial(&self) -> Result<(), Diagnostic> {
        Ok(())
    }

    /// External machines require capability validation before invoking their
    /// reducer, so a failed worker cannot race currentness or mutate state.
    fn requires_capability_preflight(&self) -> bool {
        false
    }

    /// Validate a live command and freeze its authoritative inputs into an event.
    /// This method is never called during replay.
    fn decide(
        &self,
        state: &Value,
        observation: Option<&Value>,
        effect: &EffectStatus,
        command: &Value,
    ) -> Result<Decision, Diagnostic>;

    /// Fold an event into state. This must be deterministic and must not consult
    /// observations, current time, external state, or command guards.
    fn apply(&self, state: &Value, event: &DomainEvent) -> Result<ApplyOutcome, Diagnostic>;
}

/// Capability proving that a machine was consumed by one exact registry.
///
/// Both fields are private: callers can carry or clone the capability, but only
/// [`MachineRegistry::register`] can issue one. The private registry id prevents
/// a capability issued by one registry from authorizing resolution in another.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use = "a registered machine capability must be carried to the resolution boundary"]
pub struct RegisteredMachine {
    registry_id: u64,
    identity: MachineIdentity,
}

impl RegisteredMachine {
    pub fn identity(&self) -> &MachineIdentity {
        &self.identity
    }
}

static NEXT_REGISTRY_ID: AtomicU64 = AtomicU64::new(1);

pub struct MachineRegistry {
    registry_id: u64,
    machines: HashMap<(String, u64, String), Box<dyn Machine>>,
}

impl Default for MachineRegistry {
    fn default() -> Self {
        let registry_id = NEXT_REGISTRY_ID.fetch_add(1, Ordering::Relaxed);
        assert_ne!(registry_id, 0, "machine registry id space exhausted");
        Self {
            registry_id,
            machines: HashMap::new(),
        }
    }
}

impl MachineRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, machine: Box<dyn Machine>) -> Result<RegisteredMachine, Diagnostic> {
        let identity = machine.identity().clone();
        let key = (
            identity.machine_id.clone(),
            identity.version.0,
            identity.spec_hash.0.clone(),
        );
        if self.machines.contains_key(&key) {
            return Err(Diagnostic::new(
                DiagnosticCode::IkMachineMismatch,
                Some("registry.duplicate"),
                format!(
                    "machine {}@{}#{} is already registered",
                    identity.machine_id, identity.version.0, identity.spec_hash.0
                ),
            ));
        }
        self.machines.insert(key, machine);
        Ok(RegisteredMachine {
            registry_id: self.registry_id,
            identity,
        })
    }

    /// Resolve an identity through this registry's issued capability. Exact
    /// identity succeeds; cross-registry capabilities and every identity
    /// deviation fail closed.
    pub fn resolve(
        &self,
        registration: &RegisteredMachine,
        requested: &MachineIdentity,
    ) -> Result<&dyn Machine, Diagnostic> {
        if registration.registry_id != self.registry_id {
            return Err(Diagnostic::new(
                DiagnosticCode::IkMachineMismatch,
                Some("registry.capability"),
                "machine registration capability belongs to another registry",
            ));
        }
        let exact = (
            requested.machine_id.clone(),
            requested.version.0,
            requested.spec_hash.0.clone(),
        );
        if registration.identity == *requested {
            if let Some(machine) = self.machines.get(&exact) {
                return Ok(machine.as_ref());
            }
        }
        if !self
            .machines
            .keys()
            .any(|(id, _, _)| id == &requested.machine_id)
        {
            return Err(Diagnostic::new(
                DiagnosticCode::IkMachineUnknown,
                crate::diagnostic::NO_RULE,
                format!("unknown machine {}", requested.machine_id),
            ));
        }
        if !self
            .machines
            .keys()
            .any(|(id, version, _)| id == &requested.machine_id && *version == requested.version.0)
        {
            return Err(Diagnostic::new(
                DiagnosticCode::IkMachineVersionUnknown,
                crate::diagnostic::NO_RULE,
                format!(
                    "unknown version {} for {}",
                    requested.version.0, requested.machine_id
                ),
            ));
        }
        Err(Diagnostic::new(
            DiagnosticCode::IkMachineHashMismatch,
            crate::diagnostic::NO_RULE,
            format!("unknown spec hash for {}", requested.machine_id),
        ))
    }
}

/// Explicit codec for native domain event schemas.
pub trait EventCodec: Sized {
    fn event_type(&self) -> &'static str;
    fn event_version(&self) -> u64;
    fn payload(&self) -> Value;
    fn decode(event: &DomainEvent) -> Result<Option<Self>, Diagnostic>;
}

/// Typed pure domain reducer. Adapted to [`Machine`] by [`NativeAdapter`].
pub trait NativeMachine: Send + Sync + 'static {
    type State: Serialize + DeserializeOwned + Clone;
    type Command: DeserializeOwned;
    type Observation: DeserializeOwned;
    type Event: EventCodec;
    type Diagnostic: Into<Diagnostic>;

    fn identity(&self) -> &MachineIdentity;
    fn initial_state(&self) -> Self::State;
    /// Declared initial-law validation. The default accepts; override to put
    /// this machine's initial state inside the valid-genesis claim.
    fn validate_initial(&self, _state: &Self::State) -> Result<(), Self::Diagnostic> {
        Ok(())
    }
    fn decide(
        &self,
        state: &Self::State,
        observation: Option<&Self::Observation>,
        effect: &EffectStatus,
        command: &Self::Command,
    ) -> Result<NativeDecision<Self::Event>, Self::Diagnostic>;
    fn apply(
        &self,
        state: &Self::State,
        event: &Self::Event,
    ) -> Result<NativeApply<Self::State>, Self::Diagnostic>;
}

#[derive(Clone, Debug)]
pub enum NativeDecision<E> {
    Emit { event: E, note: Option<Value> },
    Noop { note: Option<Value> },
}

impl<E> NativeDecision<E> {
    pub fn emit(event: E) -> Self {
        Self::Emit { event, note: None }
    }

    pub fn noop() -> Self {
        Self::Noop { note: None }
    }

    pub fn with_note(mut self, note: Value) -> Self {
        match &mut self {
            Self::Emit { note: slot, .. } | Self::Noop { note: slot } => *slot = Some(note),
        }
        self
    }
}

#[derive(Clone, Debug)]
pub struct NativeApply<S> {
    pub state: S,
    pub terminal: bool,
    pub effect: Option<EffectIntentV1>,
    pub outcome: Option<EffectOutcomeV1>,
}

impl<S> NativeApply<S> {
    pub fn new(state: S) -> Self {
        Self {
            state,
            terminal: false,
            effect: None,
            outcome: None,
        }
    }

    pub fn with_terminal(mut self, terminal: bool) -> Self {
        self.terminal = terminal;
        self
    }

    pub fn with_effect(mut self, effect: EffectIntentV1) -> Self {
        self.effect = Some(effect);
        self
    }

    pub fn with_outcome(mut self, outcome: EffectOutcomeV1) -> Self {
        self.outcome = Some(outcome);
        self
    }
}

pub struct NativeAdapter<M: NativeMachine> {
    inner: M,
}

impl<M: NativeMachine> NativeAdapter<M> {
    pub fn new(inner: M) -> Self {
        Self { inner }
    }
}

impl<M: NativeMachine> Machine for NativeAdapter<M> {
    fn identity(&self) -> &MachineIdentity {
        self.inner.identity()
    }

    fn initial_state(&self) -> Value {
        serde_json::to_value(self.inner.initial_state())
            .expect("native initial state must be JSON-serializable")
    }

    fn validate_initial(&self) -> Result<(), Diagnostic> {
        self.inner
            .validate_initial(&self.inner.initial_state())
            .map_err(Into::into)
    }

    fn decide(
        &self,
        state: &Value,
        observation: Option<&Value>,
        effect: &EffectStatus,
        command: &Value,
    ) -> Result<Decision, Diagnostic> {
        let state: M::State = serde_json::from_value(state.clone()).map_err(|error| {
            Diagnostic::internal(format!("native state decode failed: {error}"))
        })?;
        let command: M::Command = serde_json::from_value(command.clone()).map_err(|error| {
            Diagnostic::new(
                DiagnosticCode::IkCommandInvalid,
                crate::diagnostic::NO_RULE,
                format!("native command decode failed: {error}"),
            )
        })?;
        let observation = observation
            .map(|value| {
                serde_json::from_value(value.clone()).map_err(|error| {
                    Diagnostic::new(
                        DiagnosticCode::IkCommandInvalid,
                        Some("observation.decode"),
                        format!("native observation decode failed: {error}"),
                    )
                })
            })
            .transpose()?;
        match self
            .inner
            .decide(&state, observation.as_ref(), effect, &command)
            .map_err(Into::into)?
        {
            NativeDecision::Emit { event, note } => Ok(Decision::Emit {
                event: DomainEvent::new(event.event_type(), event.event_version(), event.payload()),
                note,
            }),
            NativeDecision::Noop { note } => Ok(Decision::Noop { note }),
        }
    }

    fn apply(&self, state: &Value, event: &DomainEvent) -> Result<ApplyOutcome, Diagnostic> {
        let state: M::State = serde_json::from_value(state.clone()).map_err(|error| {
            Diagnostic::internal(format!("native state decode failed: {error}"))
        })?;
        let event = M::Event::decode(event)?.ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCode::IkEventSchemaUnknown,
                Some("event.schema"),
                "native machine does not recognize the event type/version",
            )
        })?;
        let out = self.inner.apply(&state, &event).map_err(Into::into)?;
        Ok(ApplyOutcome {
            state: serde_json::to_value(out.state)
                .expect("native post-state must be JSON-serializable"),
            terminal: out.terminal,
            effect: out.effect,
            outcome: out.outcome,
        })
    }
}

pub fn native_identity(
    machine_id: impl Into<String>,
    version: u64,
    spec_hash: impl Into<String>,
) -> MachineIdentity {
    MachineIdentity {
        machine_id: machine_id.into(),
        version: crate::identity::MachineVersion(version),
        spec_hash: crate::identity::MachineSpecHash(spec_hash.into()),
    }
}
