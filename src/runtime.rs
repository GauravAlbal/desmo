//! Pure transition engine (no IO).
//!
//! Implements the transactional transition semantics: capability validation,
//! deterministic reduction, effect-state rules, generation increment, nonce rotation,
//! journal event construction with hash continuity, and receipt derivation. The SQLite
//! store wraps these pure steps in a `BEGIN IMMEDIATE` transaction so that event
//! append, projection update, generation/nonce rotation, effect identity, and receipt
//! commit atomically.

use crate::canonical::{canonical_json, digest_json, digest_parts, sha256_hex};
use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::effect::{EffectIntentV1, EffectOutcomeV1, EffectStatus};
use crate::identity::{AggregateId, Capability, MachineIdentity};
use crate::machine::{ApplyOutcome, Decision, DomainEvent};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Hash of the (nonexistent) event before genesis.
pub const GENESIS_PREV: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Current view of an aggregate, as loaded from the projection.
#[derive(Clone, Debug)]
pub struct AggregateSnapshot {
    pub aggregate_id: AggregateId,
    pub machine: MachineIdentity,
    pub state: Value,
    pub terminal: bool,
    pub generation: u64,
    pub nonce: String,
    pub effect: EffectStatus,
    pub sequence: u64,
    pub last_event_hash: String,
    pub last_receipt_id: String,
}

/// Durable transition receipt. Fields equal the committed event identity/state/
/// generation fields (REQ-IK-011). Receipts are derived from events and never mutated.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub receipt_id: String,
    pub aggregate_id: AggregateId,
    pub sequence: u64,
    pub generation: u64,
    pub machine_id: String,
    pub machine_version: u64,
    pub machine_spec_hash: String,
    pub state_digest: String,
    pub event_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<EffectDelta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Value>,
}

/// Effect change recorded by one journal event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EffectDelta {
    /// No effect change.
    None,
    /// A new effect intent becomes pending. The runtime stamps `intent.generation`.
    Plan { intent: EffectIntentV1 },
    /// The pending effect is resolved with a recorded outcome.
    Record { outcome: EffectOutcomeV1 },
}

/// Hash-protected origin of a journal transition. Replay dispatches by this closed
/// discriminator rather than by inspecting attacker-controlled command shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Genesis,
    DomainCommand,
    KernelRecord,
}

/// Append-only, hash-chained journal event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JournalEvent {
    pub aggregate_id: AggregateId,
    pub sequence: u64,
    pub generation: u64,
    pub machine_id: String,
    pub machine_version: u64,
    pub machine_spec_hash: String,
    pub event_kind: EventKind,
    /// Versioned domain fact. The enclosing machine identity binds its schema.
    pub event_type: String,
    pub event_version: u64,
    pub payload: Value,
    pub command: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation: Option<Value>,
    pub state_digest: String,
    pub terminal: bool,
    pub effect_delta: EffectDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Value>,
    pub prev_hash: String,
    pub hash: String,
}

/// Fields participating in the hash chain (everything except `hash`).
#[derive(Serialize)]
struct EventCore<'a> {
    aggregate_id: &'a str,
    sequence: u64,
    generation: u64,
    machine_id: &'a str,
    machine_version: u64,
    machine_spec_hash: &'a str,
    event_kind: EventKind,
    event_type: &'a str,
    event_version: u64,
    payload: &'a Value,
    command: &'a Value,
    observation: &'a Option<Value>,
    state_digest: &'a str,
    terminal: bool,
    effect_delta: &'a EffectDelta,
    note: &'a Option<Value>,
    prev_hash: &'a str,
}

/// Deterministic rotation token: positive control for staleness detection, not a
/// secret. Derivable from the aggregate and machine identity so replay reproduces it
/// and normalized responses are byte-stable.
pub fn derive_nonce(
    identity: &MachineIdentity,
    aggregate_id: &AggregateId,
    generation: u64,
) -> String {
    digest_parts(&[
        b"ik:v1:nonce",
        aggregate_id.0.as_bytes(),
        identity.machine_id.as_bytes(),
        identity.version.0.to_string().as_bytes(),
        identity.spec_hash.0.as_bytes(),
        generation.to_string().as_bytes(),
    ])
}

/// Recompute the chain hash of an event from its own fields.
pub fn recompute_event_hash(event: &JournalEvent) -> String {
    digest_json(&EventCore {
        aggregate_id: &event.aggregate_id.0,
        sequence: event.sequence,
        generation: event.generation,
        machine_id: &event.machine_id,
        machine_version: event.machine_version,
        machine_spec_hash: &event.machine_spec_hash,
        event_kind: event.event_kind,
        event_type: &event.event_type,
        event_version: event.event_version,
        payload: &event.payload,
        command: &event.command,
        observation: &event.observation,
        state_digest: &event.state_digest,
        terminal: event.terminal,
        effect_delta: &event.effect_delta,
        note: &event.note,
        prev_hash: &event.prev_hash,
    })
}

/// Derive the receipt for a committed event (receipt fields equal event identity/state/
/// generation fields, REQ-IK-011).
pub fn receipt_from_event(event: &JournalEvent) -> Receipt {
    Receipt {
        receipt_id: event.hash.clone(),
        aggregate_id: event.aggregate_id.clone(),
        sequence: event.sequence,
        generation: event.generation,
        machine_id: event.machine_id.clone(),
        machine_version: event.machine_version,
        machine_spec_hash: event.machine_spec_hash.clone(),
        state_digest: event.state_digest.clone(),
        event_hash: event.hash.clone(),
        effect: Some(event.effect_delta.clone()),
        note: event.note.clone(),
    }
}

/// Do two effect intents match modulo the runtime-stamped fields (aggregate, generation)?
pub fn intents_match_stamped(recorded: &EffectIntentV1, emitted: &EffectIntentV1) -> bool {
    recorded.effect_id == emitted.effect_id
        && recorded.effect_kind == emitted.effect_kind
        && recorded.payload_digest == emitted.payload_digest
        && recorded.idempotency_key == emitted.idempotency_key
}

/// Validate-or-stamp the aggregate on an outcome: an empty aggregate is filled with the
/// target; a non-empty mismatch refuses as foreign.
pub fn stamp_outcome_aggregate(
    outcome: &EffectOutcomeV1,
    target: &AggregateId,
) -> Result<EffectOutcomeV1, Diagnostic> {
    let mut stamped = outcome.clone();
    if stamped.aggregate_id.0.is_empty() {
        stamped.aggregate_id = target.clone();
    } else if stamped.aggregate_id != *target {
        return Err(Diagnostic::new(
            DiagnosticCode::IkEffectMismatch,
            Some("effect.identity"),
            "effect outcome aggregate does not match the target aggregate",
        ));
    }
    Ok(stamped)
}

/// Result of the pure execution of a mutation request.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// A transition to persist (event + receipt + new effect status).
    Commit(Box<CommitPlan>),
    /// Deterministic no-mutation response (no journal event, no rotation).
    Noop { note: Option<Value> },
    /// Exact duplicate of a recorded outcome: replay the original receipt with zero
    /// mutation and zero error.
    DuplicateReplay(Box<Receipt>),
}

/// Everything the store must persist for one committed transition.
#[derive(Clone, Debug, PartialEq)]
pub struct CommitPlan {
    pub event: JournalEvent,
    pub receipt: Receipt,
    pub effect_after: EffectStatus,
    /// Complete post-state for the current projection; never journaled.
    pub state: Value,
    /// Canonical bytes reused by state digesting and projection persistence.
    pub canonical_state: String,
}

/// Genesis event plus its sole full-state projection representation.
#[derive(Clone, Debug, PartialEq)]
pub struct GenesisPlan {
    pub event: JournalEvent,
    pub receipt: Receipt,
    pub state: Value,
    pub canonical_state: String,
}

/// Mutation data that has crossed the capability gate. Only this typestate can be
/// turned into a commit plan.
struct AuthorizedTransition {
    event_type: String,
    event_version: u64,
    payload: Value,
    command: Value,
    observation: Option<Value>,
    state: Value,
    terminal: bool,
    delta: EffectDelta,
    note: Option<Value>,
}

fn validate_capability(
    snapshot: &AggregateSnapshot,
    capability: &Capability,
) -> Result<(), Diagnostic> {
    #[cfg(feature = "test-support")]
    if crate::law_mutant::is(crate::law_mutant::LawMutant::AcceptStaleCapability) {
        return Ok(());
    }
    if capability.generation != snapshot.generation {
        return Err(Diagnostic::new(
            DiagnosticCode::IkCapabilityStale,
            Some("capability.generation"),
            format!(
                "capability generation {} is stale; current generation is {}",
                capability.generation, snapshot.generation
            ),
        ));
    }
    if capability.nonce != snapshot.nonce {
        return Err(Diagnostic::new(
            DiagnosticCode::IkNonceInvalid,
            Some("capability.nonce"),
            "nonce does not match the current generation",
        ));
    }
    Ok(())
}

enum Binding {
    /// Exact duplicate of a recorded outcome (includes the stored receipt).
    Replay(Box<Receipt>),
    /// Matches the pending intent: record the outcome.
    Resolve,
    /// No pending or recorded effect.
    Stale,
    /// Different effect identity (foreign).
    Foreign,
    /// Same identity, different binding or outcome.
    Conflict,
}

fn classify_binding(
    snapshot: &AggregateSnapshot,
    recorded_receipt: Option<&Receipt>,
    outcome: &EffectOutcomeV1,
) -> Result<Binding, Diagnostic> {
    match &snapshot.effect {
        EffectStatus::None => Ok(Binding::Stale),
        EffectStatus::Pending(intent) => {
            if outcome.binds(intent) {
                Ok(Binding::Resolve)
            } else if outcome.is_foreign_to(intent) {
                Ok(Binding::Foreign)
            } else {
                Ok(Binding::Conflict)
            }
        }
        EffectStatus::Recorded(intent, digest, _) => {
            if outcome.exact_duplicate_of(intent, digest) {
                let receipt = recorded_receipt
                    .ok_or_else(|| Diagnostic::internal("recorded effect missing stored receipt"))?
                    .clone();
                Ok(Binding::Replay(Box::new(receipt)))
            } else if outcome.is_foreign_to(intent) {
                Ok(Binding::Foreign)
            } else {
                Ok(Binding::Conflict)
            }
        }
    }
}

fn refuse_binding(binding: Binding) -> Result<Binding, Diagnostic> {
    match binding {
        Binding::Replay(r) => Ok(Binding::Replay(r)),
        Binding::Resolve => Ok(Binding::Resolve),
        Binding::Stale => Err(Diagnostic::new(
            DiagnosticCode::IkEffectStale,
            Some("effect.none"),
            "no effect is pending for this aggregate",
        )),
        Binding::Foreign => Err(Diagnostic::new(
            DiagnosticCode::IkEffectMismatch,
            Some("effect.identity"),
            "effect outcome references a foreign effect identity",
        )),
        Binding::Conflict => {
            #[cfg(feature = "test-support")]
            if crate::law_mutant::is(crate::law_mutant::LawMutant::AllowEffectConflict) {
                return Ok(Binding::Resolve);
            }
            Err(Diagnostic::new(
                DiagnosticCode::IkEffectOutcomeConflict,
                Some("effect.binding"),
                "effect outcome conflicts with the recorded binding or outcome",
            ))
        }
    }
}

/// Execute a domain command against the snapshot. Pure and deterministic.
///
/// `decide` is live-only. `apply` folds the emitted event before mutation authority
/// is consumed. Native machines preserve exact-outcome replay as the sole
/// pre-capability exception; external machines opt into capability preflight so
/// no stale command can start semantic code.
pub fn execute_command(
    machine: &dyn crate::machine::Machine,
    snapshot: &AggregateSnapshot,
    recorded_receipt: Option<&Receipt>,
    capability: &Capability,
    command: &Value,
    observation: Option<&Value>,
) -> Result<Step, Diagnostic> {
    if machine.requires_capability_preflight() {
        validate_capability(snapshot, capability)?;
    }
    let decision = decide_command(machine, snapshot, capability, command, observation)?;
    let (event, note) = match decision {
        Decision::Noop { note } => return no_op_step(snapshot, capability, note),
        Decision::Emit { event, note } => (event, note),
    };
    validate_event_or_capability(snapshot, capability, &event)?;
    let ApplyOutcome {
        state,
        terminal,
        effect,
        outcome,
    } = apply_command(machine, snapshot, capability, &event)?;
    let outcome = stamp_outcome_or_capability(snapshot, capability, outcome)?;

    if let Some(step) = duplicate_replay_step(snapshot, recorded_receipt, outcome.as_ref())? {
        return Ok(step);
    }
    validate_capability(snapshot, capability)?;
    if snapshot.terminal {
        #[cfg(feature = "test-support")]
        let allow = crate::law_mutant::is(crate::law_mutant::LawMutant::AllowTerminalMutation);
        #[cfg(not(feature = "test-support"))]
        let allow = false;
        if !allow {
            return Err(Diagnostic::new(
                DiagnosticCode::IkIllegalState,
                Some("terminal"),
                "terminal states reject ordinary mutation",
            ));
        }
    }
    let delta = derive_effect_delta(snapshot, recorded_receipt, effect, outcome)?;
    let authorized = AuthorizedTransition {
        event_type: event.event_type,
        event_version: event.event_version,
        payload: event.payload,
        command: command.clone(),
        observation: observation.cloned(),
        state,
        terminal,
        delta,
        note,
    };
    Ok(Step::Commit(Box::new(commit_plan(
        snapshot,
        EventKind::DomainCommand,
        authorized,
    )?)))
}

fn decide_command(
    machine: &dyn crate::machine::Machine,
    snapshot: &AggregateSnapshot,
    capability: &Capability,
    command: &Value,
    observation: Option<&Value>,
) -> Result<Decision, Diagnostic> {
    match machine.decide(&snapshot.state, observation, &snapshot.effect, command) {
        Ok(decision) => Ok(decision),
        Err(domain_refusal) => {
            validate_capability(snapshot, capability)?;
            Err(domain_refusal)
        }
    }
}

fn no_op_step(
    snapshot: &AggregateSnapshot,
    capability: &Capability,
    note: Option<Value>,
) -> Result<Step, Diagnostic> {
    validate_capability(snapshot, capability)?;
    Ok(Step::Noop { note })
}

fn validate_event_or_capability(
    snapshot: &AggregateSnapshot,
    capability: &Capability,
    event: &DomainEvent,
) -> Result<(), Diagnostic> {
    if let Err(domain_refusal) = validate_domain_event(event) {
        validate_capability(snapshot, capability)?;
        return Err(domain_refusal);
    }
    Ok(())
}

fn apply_command(
    machine: &dyn crate::machine::Machine,
    snapshot: &AggregateSnapshot,
    capability: &Capability,
    event: &DomainEvent,
) -> Result<ApplyOutcome, Diagnostic> {
    match machine.apply(&snapshot.state, event) {
        Ok(applied) => Ok(applied),
        Err(domain_refusal) => {
            validate_capability(snapshot, capability)?;
            Err(domain_refusal)
        }
    }
}

fn stamp_outcome_or_capability(
    snapshot: &AggregateSnapshot,
    capability: &Capability,
    outcome: Option<EffectOutcomeV1>,
) -> Result<Option<EffectOutcomeV1>, Diagnostic> {
    match outcome.map(|value| stamp_outcome_aggregate(&value, &snapshot.aggregate_id)) {
        Some(Ok(outcome)) => Ok(Some(outcome)),
        Some(Err(domain_refusal)) => {
            validate_capability(snapshot, capability)?;
            Err(domain_refusal)
        }
        None => Ok(None),
    }
}

/// Exact-duplicate replay guard: an outcome that exactly duplicates a recorded
/// binding replays the stored receipt before capability/terminal refusal (G-002
/// precedence exception). Returns `Ok(None)` when no duplicate replay applies.
fn duplicate_replay_step(
    snapshot: &AggregateSnapshot,
    recorded_receipt: Option<&Receipt>,
    outcome: Option<&EffectOutcomeV1>,
) -> Result<Option<Step>, Diagnostic> {
    if let Some(outcome) = outcome {
        if let EffectStatus::Recorded(intent, digest, _) = &snapshot.effect {
            if outcome.exact_duplicate_of(intent, digest) {
                #[cfg(feature = "test-support")]
                if crate::law_mutant::is(crate::law_mutant::LawMutant::MutateOnDuplicate) {
                    return Ok(None);
                }
                let receipt = recorded_receipt
                    .ok_or_else(|| Diagnostic::internal("recorded effect missing stored receipt"))?
                    .clone();
                return Ok(Some(Step::DuplicateReplay(Box::new(receipt))));
            }
        }
    }
    Ok(None)
}

fn validate_domain_event(event: &DomainEvent) -> Result<(), Diagnostic> {
    if event.event_type.is_empty()
        || event.event_version == 0
        || event.event_type == "genesis"
        || event.event_type.starts_with("kernel.")
    {
        return Err(Diagnostic::new(
            DiagnosticCode::IkEventSchemaUnknown,
            Some("event.namespace"),
            "domain event type/version is empty, zero, or reserved by the kernel",
        ));
    }
    Ok(())
}

fn derive_effect_delta(
    snapshot: &AggregateSnapshot,
    recorded_receipt: Option<&Receipt>,
    effect: Option<EffectIntentV1>,
    outcome: Option<EffectOutcomeV1>,
) -> Result<EffectDelta, Diagnostic> {
    if effect.is_some() && outcome.is_some() {
        return Err(Diagnostic::internal(
            "machine event fold produced both an effect intent and an outcome binding",
        ));
    }
    if let Some(mut intent) = effect {
        let already_pending = matches!(snapshot.effect, EffectStatus::Pending(_));
        #[cfg(feature = "test-support")]
        let already_pending = already_pending
            && !crate::law_mutant::is(crate::law_mutant::LawMutant::AllowSecondPendingEffect);
        if already_pending {
            return Err(Diagnostic::new(
                DiagnosticCode::IkEffectStale,
                Some("effect.pending"),
                "a new effect cannot be planned while one is pending",
            ));
        }
        intent.aggregate_id = snapshot.aggregate_id.clone();
        intent.generation = checked_increment(snapshot.generation, "generation")?;
        return Ok(EffectDelta::Plan { intent });
    }
    if let Some(outcome) = outcome {
        return match refuse_binding(classify_binding(snapshot, recorded_receipt, &outcome)?)? {
            Binding::Resolve => Ok(EffectDelta::Record { outcome }),
            Binding::Replay(_) => Err(Diagnostic::internal(
                "duplicate effect replay escaped the pre-capability path",
            )),
            Binding::Stale | Binding::Foreign | Binding::Conflict => unreachable!(),
        };
    }
    Ok(EffectDelta::None)
}

/// Execute a kernel-level outcome binding (the runtime `record` operation).
/// Duplicate replay precedes stale-capability and terminal refusal.
pub fn execute_outcome_binding(
    snapshot: &AggregateSnapshot,
    recorded_receipt: Option<&Receipt>,
    capability: &Capability,
    outcome: &EffectOutcomeV1,
) -> Result<Step, Diagnostic> {
    let outcome = stamp_outcome_aggregate(outcome, &snapshot.aggregate_id)?;
    // G-002 grants one narrow precedence exception: only an exact six-field duplicate
    // replays before capability currentness. Every nonduplicate still requires current
    // mutation authority before its binding diagnostic is considered.
    if let EffectStatus::Recorded(intent, digest, _) = &snapshot.effect {
        if outcome.exact_duplicate_of(intent, digest) {
            #[cfg(feature = "test-support")]
            if crate::law_mutant::is(crate::law_mutant::LawMutant::MutateOnDuplicate) {
                // fall through to a mutating record
            } else {
                let receipt = recorded_receipt
                    .ok_or_else(|| Diagnostic::internal("recorded effect missing stored receipt"))?
                    .clone();
                return Ok(Step::DuplicateReplay(Box::new(receipt)));
            }
            #[cfg(not(feature = "test-support"))]
            {
                let receipt = recorded_receipt
                    .ok_or_else(|| Diagnostic::internal("recorded effect missing stored receipt"))?
                    .clone();
                return Ok(Step::DuplicateReplay(Box::new(receipt)));
            }
        }
    }

    validate_capability(snapshot, capability)?;

    // Terminal states reject ordinary mutation, but an outstanding pending effect can
    // always be resolved.
    if snapshot.terminal && matches!(snapshot.effect, EffectStatus::None) {
        #[cfg(feature = "test-support")]
        let allow = crate::law_mutant::is(crate::law_mutant::LawMutant::AllowTerminalMutation);
        #[cfg(not(feature = "test-support"))]
        let allow = false;
        if !allow {
            return Err(Diagnostic::new(
                DiagnosticCode::IkIllegalState,
                Some("terminal"),
                "terminal states reject ordinary mutation",
            ));
        }
    }
    let binding = refuse_binding(classify_binding(snapshot, recorded_receipt, &outcome)?)?;
    let delta = match binding {
        Binding::Resolve => EffectDelta::Record {
            outcome: outcome.clone(),
        },
        Binding::Replay(receipt) => {
            #[cfg(feature = "test-support")]
            if crate::law_mutant::is(crate::law_mutant::LawMutant::MutateOnDuplicate) {
                EffectDelta::Record {
                    outcome: outcome.clone(),
                }
            } else {
                return Ok(Step::DuplicateReplay(receipt));
            }
            #[cfg(not(feature = "test-support"))]
            {
                return Ok(Step::DuplicateReplay(receipt));
            }
        }
        Binding::Stale | Binding::Foreign | Binding::Conflict => unreachable!(),
    };

    Ok(Step::Commit(Box::new(commit_plan(
        snapshot,
        EventKind::KernelRecord,
        AuthorizedTransition {
            event_type: "kernel.effect_recorded".into(),
            event_version: 1,
            payload: serde_json::to_value(&outcome)
                .expect("effect outcome is always JSON-serializable"),
            command: serde_json::to_value(&outcome)
                .expect("effect outcome is always JSON-serializable"),
            observation: None,
            state: snapshot.state.clone(),
            terminal: snapshot.terminal,
            delta,
            note: None,
        },
    )?)))
}

/// Stable refusal for a monotone counter at the `u64` ceiling.
fn counter_overflow_diagnostic(counter: &str, value: u64) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::IkCounterOverflow,
        Some(counter),
        format!("{counter} {value} is at the u64 ceiling; the next transition cannot increment it"),
    )
}

/// Checked `counter + 1` for the monotone generation/sequence counters.
///
/// A counter at `u64::MAX` cannot advance: an unchecked increment would panic in
/// debug builds and silently wrap in release, corrupting capability currentness,
/// nonce derivation, and the journal chain. The refusal is deterministic, typed
/// (`IK_COUNTER_OVERFLOW`), and happens before any journal/receipt/projection write.
fn checked_increment(counter: u64, rule: &str) -> Result<u64, Diagnostic> {
    counter
        .checked_add(1)
        .ok_or_else(|| counter_overflow_diagnostic(rule, counter))
}

fn commit_plan(
    snapshot: &AggregateSnapshot,
    event_kind: EventKind,
    authorized: AuthorizedTransition,
) -> Result<CommitPlan, Diagnostic> {
    let AuthorizedTransition {
        event_type,
        event_version,
        payload,
        command,
        observation,
        state: new_state,
        terminal,
        delta,
        note,
    } = authorized;
    let sequence = checked_increment(snapshot.sequence, "sequence")?;
    #[cfg(feature = "test-support")]
    let generation = if crate::law_mutant::is(crate::law_mutant::LawMutant::SkipGenerationIncrement)
    {
        snapshot.generation
    } else {
        checked_increment(snapshot.generation, "generation")?
    };
    #[cfg(not(feature = "test-support"))]
    let generation = checked_increment(snapshot.generation, "generation")?;
    let canonical_state = canonical_json(&new_state);
    let state_digest = sha256_hex(canonical_state.as_bytes());
    let identity = &snapshot.machine;
    let core = EventCore {
        aggregate_id: &snapshot.aggregate_id.0,
        sequence,
        generation,
        machine_id: &identity.machine_id,
        machine_version: identity.version.0,
        machine_spec_hash: &identity.spec_hash.0,
        event_kind,
        event_type: &event_type,
        event_version,
        payload: &payload,
        command: &command,
        observation: &observation,
        state_digest: &state_digest,
        terminal,
        effect_delta: &delta,
        note: &note,
        prev_hash: &snapshot.last_event_hash,
    };
    let hash = digest_json(&core);
    let event = JournalEvent {
        aggregate_id: snapshot.aggregate_id.clone(),
        sequence,
        generation,
        machine_id: identity.machine_id.clone(),
        machine_version: identity.version.0,
        machine_spec_hash: identity.spec_hash.0.clone(),
        event_kind,
        event_type,
        event_version,
        payload,
        command,
        observation,
        state_digest,
        terminal,
        effect_delta: delta.clone(),
        note: note.clone(),
        prev_hash: snapshot.last_event_hash.clone(),
        hash: hash.clone(),
    };
    let receipt = Receipt {
        receipt_id: hash,
        aggregate_id: snapshot.aggregate_id.clone(),
        sequence,
        generation,
        machine_id: identity.machine_id.clone(),
        machine_version: identity.version.0,
        machine_spec_hash: identity.spec_hash.0.clone(),
        state_digest: event.state_digest.clone(),
        event_hash: event.hash.clone(),
        effect: Some(delta),
        note,
    };
    let effect_after =
        apply_effect_delta(&snapshot.effect, &event.effect_delta, &receipt.receipt_id);
    Ok(CommitPlan {
        event,
        receipt,
        effect_after,
        state: new_state,
        canonical_state,
    })
}

/// Build genesis plus its current projection state.
pub fn genesis_plan(
    machine: &MachineIdentity,
    aggregate_id: &AggregateId,
    initial_state: Value,
    create_command: Value,
) -> GenesisPlan {
    let canonical_state = canonical_json(&initial_state);
    let state_digest = sha256_hex(canonical_state.as_bytes());
    let core = EventCore {
        aggregate_id: &aggregate_id.0,
        sequence: 0,
        generation: 0,
        machine_id: &machine.machine_id,
        machine_version: machine.version.0,
        machine_spec_hash: &machine.spec_hash.0,
        event_kind: EventKind::Genesis,
        event_type: "genesis",
        event_version: 1,
        payload: &serde_json::json!({
            "aggregate_id": aggregate_id,
            "machine": machine,
        }),
        command: &create_command,
        observation: &None,
        state_digest: &state_digest,
        terminal: false,
        effect_delta: &EffectDelta::None,
        note: &None,
        prev_hash: GENESIS_PREV,
    };
    let hash = digest_json(&core);
    let event = JournalEvent {
        aggregate_id: aggregate_id.clone(),
        sequence: 0,
        generation: 0,
        machine_id: machine.machine_id.clone(),
        machine_version: machine.version.0,
        machine_spec_hash: machine.spec_hash.0.clone(),
        event_kind: EventKind::Genesis,
        event_type: "genesis".into(),
        event_version: 1,
        payload: serde_json::json!({
            "aggregate_id": aggregate_id,
            "machine": machine,
        }),
        command: create_command,
        observation: None,
        state_digest,
        terminal: false,
        effect_delta: EffectDelta::None,
        note: None,
        prev_hash: GENESIS_PREV.to_string(),
        hash: hash.clone(),
    };
    let receipt = Receipt {
        receipt_id: hash,
        aggregate_id: aggregate_id.clone(),
        sequence: 0,
        generation: 0,
        machine_id: machine.machine_id.clone(),
        machine_version: machine.version.0,
        machine_spec_hash: machine.spec_hash.0.clone(),
        state_digest: event.state_digest.clone(),
        event_hash: event.hash.clone(),
        effect: Some(EffectDelta::None),
        note: None,
    };
    GenesisPlan {
        event,
        receipt,
        state: initial_state,
        canonical_state,
    }
}

/// Build the public genesis event and receipt pair.
pub fn genesis_event(
    machine: &MachineIdentity,
    aggregate_id: &AggregateId,
    initial_state: Value,
    create_command: Value,
) -> (JournalEvent, Receipt) {
    let plan = genesis_plan(machine, aggregate_id, initial_state, create_command);
    (plan.event, plan.receipt)
}

/// Apply an effect delta to the prior effect status, yielding the status after the
/// transition. `receipt_id` is the recording event's receipt for `Record` deltas.
pub fn apply_effect_delta(
    effect: &EffectStatus,
    delta: &EffectDelta,
    receipt_id: &str,
) -> EffectStatus {
    match delta {
        EffectDelta::None => effect.clone(),
        EffectDelta::Plan { intent } => EffectStatus::Pending(intent.clone()),
        EffectDelta::Record { outcome } => {
            let intent = match effect {
                EffectStatus::Pending(i) => i.clone(),
                _ => EffectIntentV1 {
                    effect_id: outcome.effect_id.clone(),
                    aggregate_id: outcome.aggregate_id.clone(),
                    generation: outcome.planned_generation,
                    effect_kind: String::new(),
                    payload_digest: outcome.payload_digest.clone(),
                    idempotency_key: outcome.idempotency_key.clone(),
                    payload: None,
                },
            };
            EffectStatus::Recorded(
                intent,
                outcome.outcome_digest.clone(),
                Some(receipt_id.to_string()),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::digest_json;
    use crate::diagnostic::{Diagnostic, DiagnosticCode};
    use crate::effect::{EffectIntentV1, EffectOutcomeV1, EffectStatus};
    use crate::identity::{
        AggregateId, Capability, EffectId, MachineIdentity, MachineSpecHash, MachineVersion,
    };
    use crate::machine::{ApplyOutcome, Decision, DomainEvent, Machine};
    use serde_json::{json, Value};

    /// Minimal pure machine: `bump` folds a plain state change (no effect); `plan`
    /// emits one effect intent (aggregate/generation stamped by the runtime).
    struct ProbeMachine {
        identity: MachineIdentity,
    }

    impl ProbeMachine {
        fn new() -> Self {
            Self {
                identity: MachineIdentity {
                    machine_id: "probe".into(),
                    version: MachineVersion(1),
                    spec_hash: MachineSpecHash("probe-spec".into()),
                },
            }
        }
    }

    impl Machine for ProbeMachine {
        fn identity(&self) -> &MachineIdentity {
            &self.identity
        }

        fn initial_state(&self) -> Value {
            json!({"count": 0})
        }

        fn decide(
            &self,
            _state: &Value,
            _observation: Option<&Value>,
            _effect: &EffectStatus,
            command: &Value,
        ) -> Result<Decision, Diagnostic> {
            match command["name"].as_str() {
                Some("bump") => Ok(Decision::emit(DomainEvent::new(
                    "probe.bump",
                    1,
                    json!({"delta": 1}),
                ))),
                Some("plan") => Ok(Decision::emit(DomainEvent::new(
                    "probe.plan",
                    1,
                    json!({"effect_id": "e1"}),
                ))),
                _ => Err(Diagnostic::new(
                    DiagnosticCode::IkCommandInvalid,
                    Some("command.unknown"),
                    "unknown probe command",
                )),
            }
        }

        fn apply(&self, state: &Value, event: &DomainEvent) -> Result<ApplyOutcome, Diagnostic> {
            match event.event_type.as_str() {
                "probe.bump" => {
                    let mut next = state.clone();
                    let count = next["count"].as_u64().unwrap_or(0) + 1;
                    next["count"] = json!(count);
                    Ok(ApplyOutcome::new(next))
                }
                "probe.plan" => Ok(
                    ApplyOutcome::new(state.clone()).with_effect(EffectIntentV1 {
                        effect_id: EffectId("e1".into()),
                        aggregate_id: AggregateId(String::new()), // stamped by the runtime
                        generation: 0,                            // stamped by the runtime
                        effect_kind: "probe".into(),
                        payload_digest: "p".into(),
                        idempotency_key: "k".into(),
                        payload: None,
                    }),
                ),
                _ => Err(Diagnostic::internal("unexpected probe event")),
            }
        }
    }

    fn snapshot(
        machine: &ProbeMachine,
        aggregate: &AggregateId,
        generation: u64,
        sequence: u64,
        effect: EffectStatus,
    ) -> AggregateSnapshot {
        AggregateSnapshot {
            aggregate_id: aggregate.clone(),
            machine: machine.identity().clone(),
            state: machine.initial_state(),
            terminal: false,
            generation,
            nonce: derive_nonce(machine.identity(), aggregate, generation),
            effect,
            sequence,
            last_event_hash: GENESIS_PREV.to_string(),
            last_receipt_id: String::new(),
        }
    }

    fn capability(machine: &ProbeMachine, aggregate: &AggregateId, generation: u64) -> Capability {
        Capability {
            generation,
            nonce: derive_nonce(machine.identity(), aggregate, generation),
        }
    }

    #[test]
    fn generation_at_ceiling_refuses_commit_deterministically() {
        let machine = ProbeMachine::new();
        let aggregate = AggregateId("ceiling-gen".into());
        let snap = snapshot(&machine, &aggregate, u64::MAX, 41, EffectStatus::None);
        let cap = capability(&machine, &aggregate, u64::MAX);
        // Current capability: the refusal must be the counter ceiling, never a
        // stale-capability or nonce diagnostic, and never a Commit step.
        let err = execute_command(&machine, &snap, None, &cap, &json!({"name": "bump"}), None)
            .expect_err("generation at the ceiling must refuse");
        assert_eq!(err.code, DiagnosticCode::IkCounterOverflow);
        assert_eq!(err.rule.as_deref(), Some("generation"));
        // Deterministic retry: the identical request refuses identically.
        let retry = execute_command(&machine, &snap, None, &cap, &json!({"name": "bump"}), None)
            .expect_err("retry must refuse identically");
        assert_eq!(retry, err);
    }

    #[test]
    fn sequence_at_ceiling_refuses_commit_deterministically() {
        let machine = ProbeMachine::new();
        let aggregate = AggregateId("ceiling-seq".into());
        let snap = snapshot(&machine, &aggregate, 7, u64::MAX, EffectStatus::None);
        let cap = capability(&machine, &aggregate, 7);
        let err = execute_command(&machine, &snap, None, &cap, &json!({"name": "bump"}), None)
            .expect_err("sequence at the ceiling must refuse");
        assert_eq!(err.code, DiagnosticCode::IkCounterOverflow);
        assert_eq!(err.rule.as_deref(), Some("sequence"));
        let retry = execute_command(&machine, &snap, None, &cap, &json!({"name": "bump"}), None)
            .expect_err("retry must refuse identically");
        assert_eq!(retry, err);
    }

    #[test]
    fn both_counters_at_ceiling_refuse_without_capability_confusion() {
        let machine = ProbeMachine::new();
        let aggregate = AggregateId("ceiling-both".into());
        let snap = snapshot(&machine, &aggregate, u64::MAX, u64::MAX, EffectStatus::None);
        let cap = capability(&machine, &aggregate, u64::MAX);
        // commit_plan checks sequence before generation, so the deterministic rule
        // names sequence when both are at the ceiling.
        let err = execute_command(&machine, &snap, None, &cap, &json!({"name": "bump"}), None)
            .expect_err("both counters at the ceiling must refuse");
        assert_eq!(err.code, DiagnosticCode::IkCounterOverflow);
        assert_eq!(err.rule.as_deref(), Some("sequence"));
        // No Step::Commit may ever be produced: there is nothing to persist.
        assert!(
            execute_command(&machine, &snap, None, &cap, &json!({"name": "bump"}), None).is_err()
        );
    }

    #[test]
    fn effect_planning_stamp_refuses_at_generation_ceiling() {
        let machine = ProbeMachine::new();
        let aggregate = AggregateId("ceiling-plan".into());
        let snap = snapshot(&machine, &aggregate, u64::MAX, 0, EffectStatus::None);
        let cap = capability(&machine, &aggregate, u64::MAX);
        // The planning boundary stamps intent.generation = generation + 1; checked
        // arithmetic refuses before any plan delta exists.
        let err = execute_command(&machine, &snap, None, &cap, &json!({"name": "plan"}), None)
            .expect_err("planning at the generation ceiling must refuse");
        assert_eq!(err.code, DiagnosticCode::IkCounterOverflow);
        assert_eq!(err.rule.as_deref(), Some("generation"));
    }

    #[test]
    fn outcome_binding_refuses_at_generation_ceiling() {
        let machine = ProbeMachine::new();
        let aggregate = AggregateId("ceiling-record".into());
        let intent = EffectIntentV1 {
            effect_id: EffectId("e1".into()),
            aggregate_id: aggregate.clone(),
            generation: u64::MAX,
            effect_kind: "probe".into(),
            payload_digest: "p".into(),
            idempotency_key: "k".into(),
            payload: None,
        };
        let snap = snapshot(
            &machine,
            &aggregate,
            u64::MAX,
            2,
            EffectStatus::Pending(intent.clone()),
        );
        let cap = capability(&machine, &aggregate, u64::MAX);
        let outcome = EffectOutcomeV1 {
            effect_id: intent.effect_id.clone(),
            aggregate_id: aggregate.clone(),
            planned_generation: intent.generation,
            payload_digest: intent.payload_digest.clone(),
            idempotency_key: intent.idempotency_key.clone(),
            outcome_digest: "o".into(),
        };
        let err = execute_outcome_binding(&snap, None, &cap, &outcome)
            .expect_err("kernel record at the generation ceiling must refuse");
        assert_eq!(err.code, DiagnosticCode::IkCounterOverflow);
        assert_eq!(err.rule.as_deref(), Some("generation"));
    }

    #[test]
    fn near_max_transition_commits_then_ceiling_refuses() {
        let machine = ProbeMachine::new();
        let aggregate = AggregateId("ceiling-near-max".into());
        let snap = snapshot(
            &machine,
            &aggregate,
            u64::MAX - 1,
            u64::MAX - 1,
            EffectStatus::None,
        );
        let cap = capability(&machine, &aggregate, u64::MAX - 1);
        let step = execute_command(&machine, &snap, None, &cap, &json!({"name": "bump"}), None)
            .expect("near-max transition must commit");
        let Step::Commit(plan) = step else {
            panic!("near-max transition must produce a commit plan");
        };
        // The last legal transition lands exactly on the ceiling values.
        assert_eq!(plan.event.generation, u64::MAX);
        assert_eq!(plan.event.sequence, u64::MAX);
        assert_eq!(plan.receipt.generation, u64::MAX);
        assert_eq!(plan.receipt.sequence, u64::MAX);
        assert_eq!(plan.event.prev_hash, snap.last_event_hash);

        // The post-transition snapshot is exactly at the ceiling: the next mutation
        // must refuse with the overflow diagnostic and never wrap.
        let after = AggregateSnapshot {
            aggregate_id: aggregate.clone(),
            machine: snap.machine.clone(),
            state: plan.state.clone(),
            terminal: plan.event.terminal,
            generation: plan.event.generation,
            nonce: derive_nonce(&snap.machine, &aggregate, u64::MAX),
            effect: plan.effect_after.clone(),
            sequence: plan.event.sequence,
            last_event_hash: plan.event.hash.clone(),
            last_receipt_id: plan.receipt.receipt_id.clone(),
        };
        let next_cap = capability(&machine, &aggregate, u64::MAX);
        let err = execute_command(
            &machine,
            &after,
            Some(&plan.receipt),
            &next_cap,
            &json!({"name": "bump"}),
            None,
        )
        .expect_err("post-ceiling transition must refuse");
        assert_eq!(err.code, DiagnosticCode::IkCounterOverflow);
    }

    #[test]
    fn near_max_plan_stamps_exact_ceiling_generation() {
        let machine = ProbeMachine::new();
        let aggregate = AggregateId("ceiling-near-max-plan".into());
        let snap = snapshot(&machine, &aggregate, u64::MAX - 1, 0, EffectStatus::None);
        let cap = capability(&machine, &aggregate, u64::MAX - 1);
        let step = execute_command(&machine, &snap, None, &cap, &json!({"name": "plan"}), None)
            .expect("near-max plan must commit");
        let Step::Commit(plan) = step else {
            panic!("near-max plan must produce a commit plan");
        };
        assert_eq!(plan.event.generation, u64::MAX);
        assert_eq!(plan.event.sequence, 1);
        let EffectDelta::Plan { intent } = &plan.event.effect_delta else {
            panic!("plan delta expected");
        };
        // The runtime-stamped intent generation equals the event generation.
        assert_eq!(intent.generation, u64::MAX);
        assert_eq!(intent.aggregate_id, aggregate);
        // The recorded status after the transition carries the exact intent.
        let EffectStatus::Pending(pending) = &plan.effect_after else {
            panic!("pending effect expected");
        };
        assert_eq!(pending.generation, u64::MAX);
    }

    #[test]
    fn duplicate_replay_at_ceiling_is_zero_mutation() {
        let machine = ProbeMachine::new();
        let aggregate = AggregateId("ceiling-replay".into());
        let intent = EffectIntentV1 {
            effect_id: EffectId("e1".into()),
            aggregate_id: aggregate.clone(),
            generation: 1,
            effect_kind: "probe".into(),
            payload_digest: "p".into(),
            idempotency_key: "k".into(),
            payload: None,
        };
        let outcome = EffectOutcomeV1 {
            effect_id: intent.effect_id.clone(),
            aggregate_id: aggregate.clone(),
            planned_generation: intent.generation,
            payload_digest: intent.payload_digest.clone(),
            idempotency_key: intent.idempotency_key.clone(),
            outcome_digest: "o".into(),
        };
        // Build the stored receipt exactly as a real record transition would.
        let snap = snapshot(
            &machine,
            &aggregate,
            2,
            2,
            EffectStatus::Recorded(
                intent.clone(),
                outcome.outcome_digest.clone(),
                Some("receipt-ceiling-replay".into()),
            ),
        );
        let recorded = Receipt {
            receipt_id: "receipt-ceiling-replay".into(),
            aggregate_id: aggregate.clone(),
            sequence: 2,
            generation: 2,
            machine_id: machine.identity().machine_id.clone(),
            machine_version: machine.identity().version.0,
            machine_spec_hash: machine.identity().spec_hash.0.clone(),
            state_digest: digest_json(&snap.state),
            event_hash: "receipt-ceiling-replay".into(),
            effect: Some(EffectDelta::Record {
                outcome: outcome.clone(),
            }),
            note: None,
        };
        // G-002: an exact duplicate replays before capability currentness and never
        // increments, so it still replays even when the counters are at the ceiling.
        let max_snap = AggregateSnapshot {
            generation: u64::MAX,
            sequence: u64::MAX,
            nonce: derive_nonce(machine.identity(), &aggregate, u64::MAX),
            ..snap.clone()
        };
        let stale = capability(&machine, &aggregate, 2);
        let step = execute_outcome_binding(&max_snap, Some(&recorded), &stale, &outcome)
            .expect("exact duplicate must replay at the ceiling");
        assert_eq!(step, Step::DuplicateReplay(Box::new(recorded.clone())));
        // A non-duplicate at the ceiling still goes through the normal binding
        // rules first: a different outcome on the same identity refuses as a
        // conflict (before any counter arithmetic).
        let current = capability(&machine, &aggregate, u64::MAX);
        let mut conflict = outcome.clone();
        conflict.outcome_digest = "different".into();
        let err = execute_outcome_binding(&max_snap, Some(&recorded), &current, &conflict)
            .expect_err("a conflicting binding at the ceiling must refuse as conflict");
        assert_eq!(err.code, DiagnosticCode::IkEffectOutcomeConflict);
    }

    #[test]
    fn near_max_outcome_binding_commits_exactly_at_ceiling() {
        let machine = ProbeMachine::new();
        let aggregate = AggregateId("ceiling-near-max-record".into());
        let intent = EffectIntentV1 {
            effect_id: EffectId("e1".into()),
            aggregate_id: aggregate.clone(),
            generation: u64::MAX,
            effect_kind: "probe".into(),
            payload_digest: "p".into(),
            idempotency_key: "k".into(),
            payload: None,
        };
        let snap = snapshot(
            &machine,
            &aggregate,
            u64::MAX - 1,
            u64::MAX - 1,
            EffectStatus::Pending(intent.clone()),
        );
        let cap = capability(&machine, &aggregate, u64::MAX - 1);
        let outcome = EffectOutcomeV1 {
            effect_id: intent.effect_id.clone(),
            aggregate_id: aggregate.clone(),
            planned_generation: intent.generation,
            payload_digest: intent.payload_digest.clone(),
            idempotency_key: intent.idempotency_key.clone(),
            outcome_digest: "o".into(),
        };
        let step = execute_outcome_binding(&snap, None, &cap, &outcome)
            .expect("near-max record must commit");
        let Step::Commit(plan) = step else {
            panic!("near-max record must produce a commit plan");
        };
        assert_eq!(plan.event.generation, u64::MAX);
        assert_eq!(plan.event.sequence, u64::MAX);
        let EffectStatus::Recorded(_, digest, Some(receipt_id)) = &plan.effect_after else {
            panic!("recorded effect expected");
        };
        assert_eq!(digest, "o");
        assert_eq!(receipt_id, &plan.receipt.receipt_id);
    }
}
