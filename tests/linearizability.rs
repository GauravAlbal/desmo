//! Linearizability and generation/capability property tests (K1/K2.5 concurrency).
//!
//! Defends: concurrent command/record operations through multiple real SQLite
//! connections linearize against a sequential reference model; every committed
//! mutation increments generation exactly once with contiguous receipts; refusals,
//! no-ops, and duplicate replays preserve generation; stale and foreign capabilities
//! never mutate; replay after the concurrent history reproduces the live projection
//! exactly, including across reopen.
//!
//! The randomized traces are generated with a deterministic PRNG (xorshift64) so a
//! seed reproduces the same trace; assertions hold for any thread interleaving, so
//! the test is deterministic in outcome. The generator covers accepted, refused,
//! no-op, duplicate, effect (pending/recorded), reopen, old-nonce, same-generation
//! wrong-nonce, and cross-aggregate capability transitions; the driver materializes
//! capabilities from the observed history so each class deterministically fires.
//!
//! Schedule seeds derive from a fixed base seed and the run index (see `BASE_SEED`
//! and `run_seed`), so repeated default runs explore exactly the same schedules.
//! `IK_LINEARIZABILITY_RUNS` overrides the schedule count; an explicit `0` or an
//! unparseable value is normalized to the documented minimum of `DEFAULT_RUNS` so
//! coverage can never pass after zero histories.

use ik::canonical::digest_json;
use ik::declarative::compile::create_command;
use ik::diagnostic::{Diagnostic, DiagnosticCode, NO_RULE};
use ik::effect::{EffectIntentV1, EffectOutcomeV1, EffectStatus};
use ik::identity::{AggregateId, Capability, EffectId, MachineIdentity};
use ik::machine::{
    DomainEvent, EventCodec, Machine, NativeAdapter, NativeApply, NativeDecision, NativeMachine,
};
use ik::runtime::{self, AggregateSnapshot, Receipt, Step};
use ik::store::{CommandRequest, CommandResult, EffectOutcomeRequest, Projection, Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// A deterministic native machine: counter with commit, no-op, domain refusal,
// effect plan, and terminal transitions.
// ---------------------------------------------------------------------------

const MACHINE_ID: &str = "lin_counter";
const MACHINE_SPEC_HASH: &str = "lin-counter-spec-v1";
const EFFECT_ID: &str = "lin-effect";
const PAYLOAD_DIGEST: &str = "lin-payload";
const IDEMPOTENCY_KEY: &str = "lin-key";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct LinState {
    count: i64,
    done: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "name", rename_all = "lowercase")]
enum LinCommand {
    Inc,
    Noop,
    Refuse,
    Plan,
    Stop,
}

#[derive(Clone, Debug, Deserialize)]
struct NoObservation;

#[derive(Clone, Debug)]
enum LinEvent {
    Inc,
    Stop,
    Plan,
}

impl EventCodec for LinEvent {
    fn event_type(&self) -> &'static str {
        match self {
            LinEvent::Inc => "inc",
            LinEvent::Stop => "stop",
            LinEvent::Plan => "plan",
        }
    }

    fn event_version(&self) -> u64 {
        1
    }

    fn payload(&self) -> Value {
        json!({})
    }

    fn decode(event: &DomainEvent) -> Result<Option<Self>, Diagnostic> {
        if event.event_version != 1 {
            return Ok(None);
        }
        match event.event_type.as_str() {
            "inc" => Ok(Some(LinEvent::Inc)),
            "stop" => Ok(Some(LinEvent::Stop)),
            "plan" => Ok(Some(LinEvent::Plan)),
            _ => Ok(None),
        }
    }
}

enum LinDiag {
    Refuse(&'static str),
}

impl From<LinDiag> for Diagnostic {
    fn from(d: LinDiag) -> Self {
        match d {
            LinDiag::Refuse(rule) => Diagnostic::new(
                DiagnosticCode::Domain("LIN_REFUSE".to_string()),
                Some(rule),
                "domain refusal",
            ),
        }
    }
}

struct LinCounter {
    identity: MachineIdentity,
}

impl LinCounter {
    fn new() -> Self {
        Self {
            identity: ik::machine::native_identity(MACHINE_ID, 1, MACHINE_SPEC_HASH),
        }
    }
}

impl NativeMachine for LinCounter {
    type State = LinState;
    type Command = LinCommand;
    type Observation = NoObservation;
    type Event = LinEvent;
    type Diagnostic = LinDiag;

    fn identity(&self) -> &MachineIdentity {
        &self.identity
    }

    fn initial_state(&self) -> Self::State {
        LinState {
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
            return Err(LinDiag::Refuse("terminal"));
        }
        match command {
            LinCommand::Inc => Ok(NativeDecision::emit(LinEvent::Inc)),
            LinCommand::Noop => Ok(NativeDecision::noop()),
            LinCommand::Refuse => Err(LinDiag::Refuse("lin.refuse")),
            LinCommand::Plan => {
                if effect.pending_intent().is_some() {
                    return Err(LinDiag::Refuse("lin.effect.pending"));
                }
                Ok(NativeDecision::emit(LinEvent::Plan))
            }
            LinCommand::Stop => Ok(NativeDecision::emit(LinEvent::Stop)),
        }
    }

    fn apply(
        &self,
        state: &Self::State,
        event: &Self::Event,
    ) -> Result<NativeApply<Self::State>, Self::Diagnostic> {
        match event {
            LinEvent::Inc => Ok(NativeApply::new(LinState {
                count: state.count + 1,
                done: state.done,
            })),
            LinEvent::Stop => Ok(NativeApply::new(LinState {
                count: state.count,
                done: true,
            })
            .with_terminal(true)),
            LinEvent::Plan => {
                let intent = EffectIntentV1 {
                    effect_id: EffectId(EFFECT_ID.to_string()),
                    aggregate_id: AggregateId(String::new()),
                    generation: 0,
                    effect_kind: "LIN".to_string(),
                    payload_digest: PAYLOAD_DIGEST.to_string(),
                    idempotency_key: IDEMPOTENCY_KEY.to_string(),
                    payload: None,
                };
                Ok(NativeApply::new(state.clone()).with_effect(intent))
            }
        }
    }
}

fn lin_machine() -> Arc<dyn Machine> {
    Arc::new(NativeAdapter::new(LinCounter::new()))
}

fn agg_name(index: usize) -> String {
    format!("a{index}")
}

// ---------------------------------------------------------------------------
// Trace vocabulary: capability classes and operation specs.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapClass {
    /// The latest capability observed for the aggregate.
    Current,
    /// The genesis capability, guaranteed stale once the aggregate has committed.
    Old,
    /// The current generation with a bogus nonce (same-generation wrong nonce).
    WrongNonce,
    /// A capability derived for a different aggregate at the target's generation.
    Foreign,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CmdKind {
    Inc,
    Noop,
    Refuse,
    Plan,
    Stop,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutKind {
    Bind,
    Duplicate,
    Stale,
    ForeignEffect,
}

#[derive(Clone, Debug)]
enum OpSpec {
    Create {
        agg: usize,
    },
    Command {
        agg: usize,
        cap: CapClass,
        cmd: CmdKind,
    },
    Record {
        agg: usize,
        cap: CapClass,
        out: OutKind,
    },
    Show {
        agg: usize,
    },
    Reopen,
}

/// The materialized operation executed against a store connection.
#[derive(Clone, Debug)]
enum OpKind {
    Create {
        agg: String,
    },
    Command {
        agg: String,
        capability: Capability,
        command: Value,
        observation: Option<Value>,
    },
    Record {
        agg: String,
        capability: Capability,
        effect: EffectOutcomeV1,
    },
    Show {
        agg: String,
    },
}

fn op_agg(op: &HistoryOp) -> &str {
    match &op.kind {
        OpKind::Create { agg }
        | OpKind::Command { agg, .. }
        | OpKind::Record { agg, .. }
        | OpKind::Show { agg } => agg.as_str(),
    }
}

// ---------------------------------------------------------------------------
// Deterministic PRNG (xorshift64).
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn cap_class(rng: &mut Rng) -> CapClass {
    match rng.below(100) {
        0..=54 => CapClass::Current,
        55..=66 => CapClass::Old,
        67..=81 => CapClass::WrongNonce,
        _ => CapClass::Foreign,
    }
}

/// Generate a randomized trace covering every transition class: accepted commands,
/// domain refusals, no-ops, duplicate replays, effect plan/record, reopen markers,
/// old nonces, same-generation wrong nonces, and cross-aggregate capabilities.
fn generate(seed: u64) -> Vec<OpSpec> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let total = 34 + rng.below(7); // 34..=40 storm operations
    let mut reopen_every = 9 + rng.below(4);
    let mut since_reopen = 0;
    let mut specs = Vec::new();
    for _ in 0..total {
        if since_reopen >= reopen_every {
            specs.push(OpSpec::Reopen);
            since_reopen = 0;
            reopen_every = 9 + rng.below(4);
            continue;
        }
        since_reopen += 1;
        let roll = rng.below(100);
        let spec = if roll < 2 {
            OpSpec::Create { agg: 2 }
        } else if roll < 57 {
            let cap = cap_class(&mut rng);
            // Old-nonce ops must target an aggregate with at least two observed
            // generations so the genesis capability is guaranteed stale; the setup
            // phase guarantees that for a0/a1.
            let agg = if cap == CapClass::Old {
                rng.below(2)
            } else {
                rng.below(3)
            };
            let cmd = match rng.below(100) {
                0..=44 => CmdKind::Inc,
                45..=59 => CmdKind::Noop,
                60..=71 => CmdKind::Refuse,
                72..=84 => CmdKind::Plan,
                85..=94 => CmdKind::Stop,
                _ => CmdKind::Unknown,
            };
            OpSpec::Command { agg, cap, cmd }
        } else if roll < 71 {
            let cap = cap_class(&mut rng);
            let agg = if cap == CapClass::Old {
                rng.below(2)
            } else {
                rng.below(3)
            };
            let out = match rng.below(100) {
                0..=59 => OutKind::Bind,
                60..=74 => OutKind::Duplicate,
                75..=89 => OutKind::Stale,
                _ => OutKind::ForeignEffect,
            };
            OpSpec::Record { agg, cap, out }
        } else if roll < 83 {
            OpSpec::Show { agg: rng.below(3) }
        } else {
            OpSpec::Reopen
        };
        specs.push(spec);
    }
    specs
}

/// Deterministic setup: one connection, sequential, guarantees every transition
/// class fires exactly once (so coverage assertions are schedule-independent).
fn setup_specs() -> Vec<OpSpec> {
    vec![
        OpSpec::Create { agg: 0 },
        OpSpec::Create { agg: 1 },
        OpSpec::Command {
            agg: 0,
            cap: CapClass::Current,
            cmd: CmdKind::Inc,
        },
        OpSpec::Command {
            agg: 1,
            cap: CapClass::Current,
            cmd: CmdKind::Inc,
        },
        OpSpec::Command {
            agg: 0,
            cap: CapClass::Current,
            cmd: CmdKind::Plan,
        },
        OpSpec::Record {
            agg: 0,
            cap: CapClass::Current,
            out: OutKind::Bind,
        },
        OpSpec::Command {
            agg: 0,
            cap: CapClass::Current,
            cmd: CmdKind::Noop,
        },
        OpSpec::Command {
            agg: 0,
            cap: CapClass::Current,
            cmd: CmdKind::Refuse,
        },
        OpSpec::Command {
            agg: 0,
            cap: CapClass::WrongNonce,
            cmd: CmdKind::Inc,
        },
        OpSpec::Command {
            agg: 0,
            cap: CapClass::Old,
            cmd: CmdKind::Inc,
        },
        OpSpec::Command {
            agg: 0,
            cap: CapClass::Foreign,
            cmd: CmdKind::Inc,
        },
        OpSpec::Record {
            agg: 0,
            cap: CapClass::Current,
            out: OutKind::Duplicate,
        },
        OpSpec::Command {
            agg: 0,
            cap: CapClass::Current,
            cmd: CmdKind::Unknown,
        },
        OpSpec::Create { agg: 0 },
        OpSpec::Command {
            agg: 1,
            cap: CapClass::Current,
            cmd: CmdKind::Plan,
        },
        OpSpec::Record {
            agg: 1,
            cap: CapClass::Current,
            out: OutKind::Bind,
        },
        OpSpec::Command {
            agg: 1,
            cap: CapClass::Current,
            cmd: CmdKind::Stop,
        },
        OpSpec::Record {
            agg: 1,
            cap: CapClass::Current,
            out: OutKind::Duplicate,
        },
        OpSpec::Command {
            agg: 1,
            cap: CapClass::Current,
            cmd: CmdKind::Inc,
        },
        OpSpec::Show { agg: 0 },
    ]
}

/// Sequential continuation after a reopen: capabilities observed before the reopen
/// must remain valid, old/wrong/foreign capabilities must still refuse, and the
/// effect state must survive the reopen.
fn post_reopen_specs() -> Vec<OpSpec> {
    vec![
        OpSpec::Command {
            agg: 0,
            cap: CapClass::Current,
            cmd: CmdKind::Inc,
        },
        OpSpec::Record {
            agg: 1,
            cap: CapClass::Current,
            out: OutKind::Duplicate,
        },
        OpSpec::Command {
            agg: 0,
            cap: CapClass::Old,
            cmd: CmdKind::Plan,
        },
        OpSpec::Show { agg: 1 },
        OpSpec::Command {
            agg: 0,
            cap: CapClass::WrongNonce,
            cmd: CmdKind::Stop,
        },
        OpSpec::Command {
            agg: 2,
            cap: CapClass::Current,
            cmd: CmdKind::Inc,
        },
        OpSpec::Create { agg: 2 },
        OpSpec::Command {
            agg: 2,
            cap: CapClass::Current,
            cmd: CmdKind::Plan,
        },
        OpSpec::Command {
            agg: 2,
            cap: CapClass::Foreign,
            cmd: CmdKind::Inc,
        },
        OpSpec::Record {
            agg: 2,
            cap: CapClass::Current,
            out: OutKind::Bind,
        },
    ]
}

// ---------------------------------------------------------------------------
// Driver state: what capabilities/effect statuses have been observed per aggregate.
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct AggObs {
    exists: bool,
    current: Option<Capability>,
    /// Every distinct capability observed, in generation order.
    history: Vec<Capability>,
    effect: Option<EffectStatus>,
    pending_generation: Option<u64>,
}

#[derive(Default)]
struct DriverState {
    aggs: Vec<AggObs>,
    /// Freshness counter so Bind outcomes never accidentally duplicate a recorded digest.
    bind_seq: u64,
}

impl DriverState {
    fn new(aggregates: usize) -> Self {
        Self {
            aggs: (0..aggregates).map(|_| AggObs::default()).collect(),
            bind_seq: 0,
        }
    }
}

fn materialize_cap(
    machine: &dyn Machine,
    st: &DriverState,
    class: CapClass,
    target: usize,
) -> Capability {
    let a = &st.aggs[target];
    let current = a.current.clone().unwrap_or_else(|| Capability {
        generation: 0,
        nonce: runtime::derive_nonce(machine.identity(), &AggregateId(agg_name(target)), 0),
    });
    match class {
        CapClass::Current => current,
        CapClass::Old => {
            // The genesis capability is guaranteed stale once the aggregate has
            // committed at least once; otherwise fall back to the current one.
            if a.history.len() >= 2 {
                a.history[0].clone()
            } else {
                current
            }
        }
        CapClass::WrongNonce => Capability {
            generation: current.generation,
            nonce: "deadbeef-nonce".to_string(),
        },
        CapClass::Foreign => {
            let other = (0..st.aggs.len()).find(|&o| o != target).unwrap_or(target);
            Capability {
                generation: current.generation,
                nonce: runtime::derive_nonce(
                    machine.identity(),
                    &AggregateId(agg_name(other)),
                    current.generation,
                ),
            }
        }
    }
}

fn materialize_outcome(st: &mut DriverState, agg: usize, out: OutKind) -> EffectOutcomeV1 {
    let (planned_generation, recorded_digest) = {
        let a = &st.aggs[agg];
        match &a.effect {
            Some(EffectStatus::Pending(intent)) => (intent.generation, None),
            Some(EffectStatus::Recorded(intent, digest, _)) => {
                (intent.generation, Some(digest.clone()))
            }
            _ => (a.pending_generation.unwrap_or(0), None),
        }
    };
    let outcome_digest = match out {
        OutKind::Bind => {
            // Fresh digest: binds the pending intent but never accidentally equals
            // an already-recorded outcome.
            st.bind_seq += 1;
            format!("bind-{agg}-{planned_generation}-{}", st.bind_seq)
        }
        OutKind::Duplicate => {
            // Exact duplicate of the observed recorded outcome when one exists.
            recorded_digest.unwrap_or_else(|| format!("dup-{agg}-{planned_generation}"))
        }
        OutKind::Stale => "stale-outcome".to_string(),
        OutKind::ForeignEffect => "foreign-outcome".to_string(),
    };
    let effect_id = match out {
        OutKind::ForeignEffect => "other-effect".to_string(),
        _ => EFFECT_ID.to_string(),
    };
    EffectOutcomeV1 {
        effect_id: EffectId(effect_id),
        aggregate_id: AggregateId(agg_name(agg)),
        planned_generation,
        payload_digest: PAYLOAD_DIGEST.to_string(),
        idempotency_key: IDEMPOTENCY_KEY.to_string(),
        outcome_digest,
    }
}

fn materialize(machine: &dyn Machine, st: &mut DriverState, spec: &OpSpec) -> (OpKind, CapClass) {
    match spec {
        OpSpec::Create { agg } => (
            OpKind::Create {
                agg: agg_name(*agg),
            },
            CapClass::Current,
        ),
        OpSpec::Show { agg } => (
            OpKind::Show {
                agg: agg_name(*agg),
            },
            CapClass::Current,
        ),
        OpSpec::Command { agg, cap, cmd } => {
            let capability = materialize_cap(machine, st, *cap, *agg);
            let command = match cmd {
                CmdKind::Inc => json!({"name": "inc"}),
                CmdKind::Noop => json!({"name": "noop"}),
                CmdKind::Refuse => json!({"name": "refuse"}),
                CmdKind::Plan => json!({"name": "plan"}),
                CmdKind::Stop => json!({"name": "stop"}),
                CmdKind::Unknown => json!({"name": "bogus"}),
            };
            (
                OpKind::Command {
                    agg: agg_name(*agg),
                    capability,
                    command,
                    observation: None,
                },
                *cap,
            )
        }
        OpSpec::Record { agg, cap, out } => {
            // Duplicate outcomes replay before capability validation, so they are
            // always presented with the current capability; every other class must
            // deterministically refuse.
            let class = if *out == OutKind::Duplicate {
                CapClass::Current
            } else {
                *cap
            };
            let capability = materialize_cap(machine, st, class, *agg);
            let effect = materialize_outcome(st, *agg, *out);
            (
                OpKind::Record {
                    agg: agg_name(*agg),
                    capability,
                    effect,
                },
                class,
            )
        }
        OpSpec::Reopen => unreachable!("reopen markers are driver-level, not store ops"),
    }
}

fn update_state(st: &mut DriverState, kind: &OpKind, observed: &Observed) {
    let agg = match kind {
        OpKind::Create { agg }
        | OpKind::Command { agg, .. }
        | OpKind::Record { agg, .. }
        | OpKind::Show { agg } => agg.clone(),
    };
    let idx = agg[1..].parse::<usize>().expect("aggregate index");
    let a = &mut st.aggs[idx];
    match observed {
        Observed::CommandResult(result) => {
            a.exists = true;
            a.current = Some(result.capability.clone());
            if a.history.last().map(|c| c.generation) != Some(result.generation) {
                a.history.push(result.capability.clone());
            }
            a.effect = Some(result.effect.clone());
            if let Some(intent) = result.effect.pending_intent() {
                a.pending_generation = Some(intent.generation);
            }
        }
        Observed::Projection(projection) => {
            a.exists = true;
            a.current = Some(projection.capability.clone());
            if a.history.last().map(|c| c.generation) != Some(projection.generation) {
                a.history.push(projection.capability.clone());
            }
            a.effect = Some(projection.effect.clone());
            if let Some(intent) = projection.effect.pending_intent() {
                a.pending_generation = Some(intent.generation);
            }
        }
        Observed::Refused { .. } => {}
    }
}

// ---------------------------------------------------------------------------
// Observed responses, response equality, and one recorded history operation.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Observed {
    CommandResult(CommandResult),
    Projection(Projection),
    Refused {
        code: DiagnosticCode,
        rule: Option<String>,
    },
}

fn same_command_result(a: &CommandResult, b: &CommandResult) -> bool {
    a.receipt == b.receipt
        && a.state == b.state
        && a.terminal == b.terminal
        && a.generation == b.generation
        && a.capability == b.capability
        && a.effect == b.effect
        && a.mutation == b.mutation
        && a.note == b.note
}

fn same_projection(a: &Projection, b: &Projection) -> bool {
    a.aggregate_id == b.aggregate_id
        && a.machine == b.machine
        && a.state == b.state
        && a.terminal == b.terminal
        && a.generation == b.generation
        && a.capability == b.capability
        && a.effect == b.effect
        && a.sequence == b.sequence
        && a.last_event_hash == b.last_event_hash
        && a.last_receipt == b.last_receipt
}

fn same_observed(a: &Observed, b: &Observed) -> bool {
    match (a, b) {
        (Observed::CommandResult(x), Observed::CommandResult(y)) => same_command_result(x, y),
        (Observed::Projection(x), Observed::Projection(y)) => same_projection(x, y),
        (Observed::Refused { code: c1, rule: r1 }, Observed::Refused { code: c2, rule: r2 }) => {
            c1 == c2 && r1 == r2
        }
        _ => false,
    }
}

/// Does the model's predicted outcome match the observed response?
fn predicted_matches(pred: Result<Observed, Diagnostic>, observed: &Observed) -> bool {
    match pred {
        Ok(p) => same_observed(&p, observed),
        Err(d) => matches!(
            observed,
            Observed::Refused { code, rule } if &d.code == code && &d.rule == rule
        ),
    }
}

struct HistoryOp {
    kind: OpKind,
    class: CapClass,
    t_inv: u64,
    t_res: u64,
    observed: Observed,
}

// ---------------------------------------------------------------------------
// Sequential reference model: the store's transaction path, minus SQLite.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ModelAgg {
    exists: bool,
    machine: MachineIdentity,
    state: Value,
    terminal: bool,
    generation: u64,
    nonce: String,
    effect: EffectStatus,
    sequence: u64,
    last_event_hash: String,
    last_receipt_id: String,
    receipts: HashMap<String, Receipt>,
}

#[derive(Clone)]
struct Model {
    aggs: HashMap<String, ModelAgg>,
}

impl Model {
    fn new() -> Self {
        Self {
            aggs: HashMap::new(),
        }
    }

    fn snapshot(&self, agg: &str) -> Result<AggregateSnapshot, Diagnostic> {
        let a = self.aggs.get(agg).ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCode::IkIllegalState,
                Some("aggregate.not_found"),
                format!("aggregate `{agg}` not found"),
            )
        })?;
        if !a.exists {
            return Err(Diagnostic::new(
                DiagnosticCode::IkIllegalState,
                Some("aggregate.not_found"),
                format!("aggregate `{agg}` not found"),
            ));
        }
        Ok(AggregateSnapshot {
            aggregate_id: AggregateId(agg.to_string()),
            machine: a.machine.clone(),
            state: a.state.clone(),
            terminal: a.terminal,
            generation: a.generation,
            nonce: a.nonce.clone(),
            effect: a.effect.clone(),
            sequence: a.sequence,
            last_event_hash: a.last_event_hash.clone(),
            last_receipt_id: a.last_receipt_id.clone(),
        })
    }

    fn recorded_receipt(&self, agg: &str) -> Option<Receipt> {
        let a = self.aggs.get(agg)?;
        match &a.effect {
            EffectStatus::Recorded(_, _, Some(id)) => a.receipts.get(id).cloned(),
            _ => None,
        }
    }

    fn create(&mut self, machine: &dyn Machine, agg: &str) -> Result<CommandResult, Diagnostic> {
        let identity = machine.identity().clone();
        let exists = self.aggs.get(agg).map(|a| a.exists).unwrap_or(false);
        if exists {
            return Err(Diagnostic::new(
                DiagnosticCode::IkIllegalState,
                Some("create.duplicate_aggregate"),
                format!("aggregate `{agg}` already exists"),
            ));
        }
        let initial = machine.initial_state();
        let (event, receipt) = runtime::genesis_event(
            &identity,
            &AggregateId(agg.to_string()),
            initial.clone(),
            create_command(agg, &identity),
        );
        let nonce = runtime::derive_nonce(&identity, &AggregateId(agg.to_string()), 0);
        let mut receipts = HashMap::new();
        receipts.insert(receipt.receipt_id.clone(), receipt.clone());
        self.aggs.insert(
            agg.to_string(),
            ModelAgg {
                exists: true,
                machine: identity,
                state: initial.clone(),
                terminal: false,
                generation: 0,
                nonce: nonce.clone(),
                effect: EffectStatus::None,
                sequence: 0,
                last_event_hash: event.hash.clone(),
                last_receipt_id: receipt.receipt_id.clone(),
                receipts,
            },
        );
        Ok(CommandResult {
            receipt: Some(receipt),
            state: initial,
            terminal: false,
            generation: 0,
            capability: Capability {
                generation: 0,
                nonce,
            },
            effect: EffectStatus::None,
            mutation: true,
            note: None,
        })
    }

    fn command(
        &mut self,
        machine: &dyn Machine,
        req: &CommandRequest,
    ) -> Result<CommandResult, Diagnostic> {
        let agg = req.aggregate_id.0.clone();
        let snapshot = self.snapshot(&agg)?;
        if req.machine != snapshot.machine || machine.identity() != &snapshot.machine {
            return Err(Diagnostic::new(
                DiagnosticCode::IkMachineMismatch,
                NO_RULE,
                "presented machine identity does not match the aggregate's recorded identity",
            ));
        }
        let recorded = self.recorded_receipt(&agg);
        let step = runtime::execute_command(
            machine,
            &snapshot,
            recorded.as_ref(),
            &req.capability,
            &req.command,
            req.observation.as_ref(),
        )?;
        self.finish(snapshot, step)
    }

    fn record(&mut self, req: &EffectOutcomeRequest) -> Result<CommandResult, Diagnostic> {
        let agg = req.aggregate_id.0.clone();
        let snapshot = self.snapshot(&agg)?;
        if req.machine != snapshot.machine {
            return Err(Diagnostic::new(
                DiagnosticCode::IkMachineMismatch,
                NO_RULE,
                "presented machine identity does not match the aggregate's recorded identity",
            ));
        }
        let recorded = self.recorded_receipt(&agg);
        let step = runtime::execute_outcome_binding(
            &snapshot,
            recorded.as_ref(),
            &req.capability,
            &req.effect,
        )?;
        self.finish(snapshot, step)
    }

    fn show(&self, agg: &str) -> Result<Projection, Diagnostic> {
        let a = self.aggs.get(agg).ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCode::IkIllegalState,
                Some("aggregate.not_found"),
                format!("aggregate `{agg}` not found"),
            )
        })?;
        if !a.exists {
            return Err(Diagnostic::new(
                DiagnosticCode::IkIllegalState,
                Some("aggregate.not_found"),
                format!("aggregate `{agg}` not found"),
            ));
        }
        let last_receipt = if a.last_receipt_id.is_empty() {
            None
        } else {
            a.receipts.get(&a.last_receipt_id).cloned()
        };
        Ok(Projection {
            aggregate_id: AggregateId(agg.to_string()),
            machine: a.machine.clone(),
            state: a.state.clone(),
            terminal: a.terminal,
            generation: a.generation,
            capability: Capability {
                generation: a.generation,
                nonce: a.nonce.clone(),
            },
            effect: a.effect.clone(),
            sequence: a.sequence,
            last_event_hash: a.last_event_hash.clone(),
            last_receipt,
        })
    }

    fn finish(
        &mut self,
        snapshot: AggregateSnapshot,
        step: Step,
    ) -> Result<CommandResult, Diagnostic> {
        let current_capability = || Capability {
            generation: snapshot.generation,
            nonce: snapshot.nonce.clone(),
        };
        match step {
            Step::Noop { note } => Ok(CommandResult {
                receipt: None,
                state: snapshot.state,
                terminal: snapshot.terminal,
                generation: snapshot.generation,
                capability: current_capability(),
                effect: snapshot.effect,
                mutation: false,
                note,
            }),
            Step::DuplicateReplay(receipt) => Ok(CommandResult {
                receipt: Some(*receipt),
                state: snapshot.state,
                terminal: snapshot.terminal,
                generation: snapshot.generation,
                capability: current_capability(),
                effect: snapshot.effect,
                mutation: false,
                note: None,
            }),
            Step::Commit(plan) => {
                let agg = snapshot.aggregate_id.0.clone();
                let nonce = runtime::derive_nonce(
                    &snapshot.machine,
                    &snapshot.aggregate_id,
                    plan.event.generation,
                );
                let a = self
                    .aggs
                    .get_mut(&agg)
                    .expect("committing aggregate exists");
                a.state = plan.state.clone();
                a.terminal = plan.event.terminal;
                a.generation = plan.event.generation;
                a.nonce = nonce.clone();
                a.effect = plan.effect_after.clone();
                a.sequence = plan.event.sequence;
                a.last_event_hash = plan.event.hash.clone();
                a.last_receipt_id = plan.receipt.receipt_id.clone();
                a.receipts
                    .insert(plan.receipt.receipt_id.clone(), plan.receipt.clone());
                Ok(CommandResult {
                    receipt: Some(plan.receipt),
                    state: plan.state,
                    terminal: plan.event.terminal,
                    generation: plan.event.generation,
                    capability: Capability {
                        generation: plan.event.generation,
                        nonce,
                    },
                    effect: plan.effect_after,
                    mutation: true,
                    note: plan.event.note,
                })
            }
        }
    }

    fn apply_op(&mut self, machine: &dyn Machine, kind: &OpKind) -> Result<Observed, Diagnostic> {
        match kind {
            OpKind::Create { agg } => self.create(machine, agg).map(Observed::CommandResult),
            OpKind::Command {
                agg,
                capability,
                command,
                observation,
            } => {
                let req = CommandRequest {
                    aggregate_id: AggregateId(agg.clone()),
                    machine: machine.identity().clone(),
                    capability: capability.clone(),
                    command: command.clone(),
                    observation: observation.clone(),
                };
                self.command(machine, &req).map(Observed::CommandResult)
            }
            OpKind::Record {
                agg,
                capability,
                effect,
            } => {
                let req = EffectOutcomeRequest {
                    aggregate_id: AggregateId(agg.clone()),
                    machine: machine.identity().clone(),
                    capability: capability.clone(),
                    effect: effect.clone(),
                };
                self.record(&req).map(Observed::CommandResult)
            }
            OpKind::Show { agg } => self.show(agg).map(Observed::Projection),
        }
    }

    /// Stable digest of the full model state (projection fields plus the receipt
    /// map) for linearizability-search memoization.
    fn digest(&self) -> String {
        #[derive(Serialize)]
        struct View<'a> {
            id: &'a str,
            exists: bool,
            state: &'a Value,
            terminal: bool,
            generation: u64,
            nonce: &'a str,
            effect: &'a EffectStatus,
            sequence: u64,
            last_event_hash: &'a str,
            last_receipt_id: &'a str,
            receipts: Vec<&'a Receipt>,
        }
        let mut views: Vec<View> = self
            .aggs
            .iter()
            .map(|(id, a)| {
                let mut receipts: Vec<&Receipt> = a.receipts.values().collect();
                receipts.sort_by_key(|r| (r.sequence, r.receipt_id.as_str()));
                View {
                    id,
                    exists: a.exists,
                    state: &a.state,
                    terminal: a.terminal,
                    generation: a.generation,
                    nonce: &a.nonce,
                    effect: &a.effect,
                    sequence: a.sequence,
                    last_event_hash: &a.last_event_hash,
                    last_receipt_id: &a.last_receipt_id,
                    receipts,
                }
            })
            .collect();
        views.sort_by(|x, y| x.id.cmp(y.id));
        digest_json(&json!({ "aggregates": views }))
    }
}

// ---------------------------------------------------------------------------
// Live-store target and the linearizability search.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct LiveAgg {
    state: Value,
    terminal: bool,
    generation: u64,
    nonce: String,
    effect: EffectStatus,
    sequence: u64,
    last_event_hash: String,
    last_receipt_id: String,
}

impl LiveAgg {
    fn from_projection(p: &Projection) -> Self {
        Self {
            state: p.state.clone(),
            terminal: p.terminal,
            generation: p.generation,
            nonce: p.capability.nonce.clone(),
            effect: p.effect.clone(),
            sequence: p.sequence,
            last_event_hash: p.last_event_hash.clone(),
            last_receipt_id: p
                .last_receipt
                .as_ref()
                .map(|r| r.receipt_id.clone())
                .unwrap_or_default(),
        }
    }
}

fn model_matches_live(model: &Model, live: &HashMap<String, LiveAgg>) -> bool {
    if model.aggs.len() != live.len() {
        return false;
    }
    for (id, target) in live {
        let Some(a) = model.aggs.get(id) else {
            return false;
        };
        if !a.exists
            || a.state != target.state
            || a.terminal != target.terminal
            || a.generation != target.generation
            || a.nonce != target.nonce
            || a.effect != target.effect
            || a.sequence != target.sequence
            || a.last_event_hash != target.last_event_hash
            || a.last_receipt_id != target.last_receipt_id
        {
            return false;
        }
    }
    true
}

/// Depth-first search over the partial order induced by the invocation/response
/// intervals. `pending` is the set of invoked-but-unlinearized operations; time
/// advances only to the next invocation. The search asks for a linearization
/// whose final model state equals the live store state (the real serial execution
/// is one such linearization, so a correct store always admits one). Memoization
/// keys on (time, pending set, model digest), which collapses the factorial blowup
/// of interchangeable refused operations.
#[allow(
    clippy::too_many_arguments,
    reason = "the explicit DFS state mirrors the linearizability search tuple and memoization key"
)]
fn dfs(
    machine: &dyn Machine,
    history: &[HistoryOp],
    live: &HashMap<String, LiveAgg>,
    visited: &mut HashSet<(u64, Vec<usize>, String)>,
    model: &mut Model,
    pending: Vec<usize>,
    time: u64,
    max_inv: u64,
) -> bool {
    // Prune: any operation whose response time has passed must already be completed.
    for (i, op) in history.iter().enumerate() {
        if op.t_res <= time && (op.t_inv > time || pending.contains(&i)) {
            return false;
        }
    }
    if pending.is_empty() && time >= max_inv {
        return model_matches_live(model, live);
    }
    let key = (time, pending.clone(), model.digest());
    if !visited.insert(key) {
        return false;
    }
    // Apply a pending operation at the current time.
    for &i in &pending {
        let op = &history[i];
        let mut probe = model.clone();
        let pred = probe.apply_op(machine, &op.kind);
        if predicted_matches(pred, &op.observed) {
            let mut next_pending = pending.clone();
            next_pending.retain(|&j| j != i);
            if dfs(
                machine,
                history,
                live,
                visited,
                &mut probe,
                next_pending,
                time,
                max_inv,
            ) {
                return true;
            }
        }
    }
    // Advance time to the next invocation (ticks are unique, so exactly one
    // operation becomes pending per advance).
    if let Some(t) = history
        .iter()
        .filter(|op| op.t_inv > time)
        .map(|op| op.t_inv)
        .min()
    {
        let mut next_pending = pending;
        for (i, op) in history.iter().enumerate() {
            if op.t_inv == t {
                next_pending.push(i);
            }
        }
        next_pending.sort_unstable();
        if dfs(
            machine,
            history,
            live,
            visited,
            model,
            next_pending,
            t,
            max_inv,
        ) {
            return true;
        }
    }
    false
}

fn linearizable_to_live(
    machine: &dyn Machine,
    history: &[HistoryOp],
    live: &HashMap<String, LiveAgg>,
) -> bool {
    let n = history.len();
    assert!(n < 64, "history too large for the search bitmask: {n}");
    let max_inv = history.iter().map(|op| op.t_inv).max().unwrap_or(0);
    let mut visited: HashSet<(u64, Vec<usize>, String)> = HashSet::new();
    let mut model = Model::new();
    dfs(
        machine,
        history,
        live,
        &mut visited,
        &mut model,
        Vec::new(),
        0,
        max_inv,
    )
}

// ---------------------------------------------------------------------------
// Store runner.
// ---------------------------------------------------------------------------

fn run_store(store: &mut Store, machine: &dyn Machine, kind: &OpKind) -> Observed {
    let identity = machine.identity().clone();
    match kind {
        OpKind::Create { agg } => {
            match store.create_aggregate(machine, &AggregateId(agg.clone()), &identity) {
                Ok(result) => Observed::CommandResult(result),
                Err(d) => Observed::Refused {
                    code: d.code,
                    rule: d.rule,
                },
            }
        }
        OpKind::Command {
            agg,
            capability,
            command,
            observation,
        } => {
            let req = CommandRequest {
                aggregate_id: AggregateId(agg.clone()),
                machine: identity,
                capability: capability.clone(),
                command: command.clone(),
                observation: observation.clone(),
            };
            match store.command(machine, &req) {
                Ok(result) => Observed::CommandResult(result),
                Err(d) => Observed::Refused {
                    code: d.code,
                    rule: d.rule,
                },
            }
        }
        OpKind::Record {
            agg,
            capability,
            effect,
        } => {
            let req = EffectOutcomeRequest {
                aggregate_id: AggregateId(agg.clone()),
                machine: identity,
                capability: capability.clone(),
                effect: effect.clone(),
            };
            match store.record_effect(&req) {
                Ok(result) => Observed::CommandResult(result),
                Err(d) => Observed::Refused {
                    code: d.code,
                    rule: d.rule,
                },
            }
        }
        OpKind::Show { agg } => match store.show(&AggregateId(agg.clone())) {
            Ok(projection) => Observed::Projection(projection),
            Err(d) => Observed::Refused {
                code: d.code,
                rule: d.rule,
            },
        },
    }
}

fn execute_op(
    store: &mut Store,
    machine: &dyn Machine,
    spec: &OpSpec,
    tick: &AtomicU64,
    history: &Mutex<Vec<HistoryOp>>,
    state: &Arc<Mutex<DriverState>>,
) {
    let (kind, class) = {
        let mut st = state.lock().expect("driver state");
        materialize(machine, &mut st, spec)
    };
    let t_inv = tick.fetch_add(1, Ordering::SeqCst);
    let observed = run_store(store, machine, &kind);
    let t_res = tick.fetch_add(1, Ordering::SeqCst);
    {
        let mut st = state.lock().expect("driver state");
        update_state(&mut st, &kind, &observed);
    }
    history.lock().expect("history").push(HistoryOp {
        kind,
        class,
        t_inv,
        t_res,
        observed,
    });
}

/// Split a trace at reopen markers; every segment runs through freshly opened
/// store connections, so each round is a reopen transition.
fn storm_rounds(specs: &[OpSpec]) -> Vec<Vec<OpSpec>> {
    let mut rounds: Vec<Vec<OpSpec>> = Vec::new();
    let mut current: Vec<OpSpec> = Vec::new();
    for spec in specs {
        if matches!(spec, OpSpec::Reopen) {
            if !current.is_empty() {
                rounds.push(std::mem::take(&mut current));
            }
        } else {
            current.push(spec.clone());
        }
    }
    if !current.is_empty() {
        rounds.push(current);
    }
    if rounds.is_empty() {
        rounds.push(Vec::new());
    }
    rounds
}

/// Run the randomized storm: `WORKERS` real store connections, reopened every round.
fn run_storm(
    path: &Path,
    machine: &Arc<dyn Machine>,
    rounds: &[Vec<OpSpec>],
    tick: &AtomicU64,
    history: &Mutex<Vec<HistoryOp>>,
    state: &Arc<Mutex<DriverState>>,
) {
    const WORKERS: usize = 3;
    for round in rounds {
        let barrier = Arc::new(Barrier::new(WORKERS));
        std::thread::scope(|scope| {
            for w in 0..WORKERS {
                let ops: Vec<OpSpec> = round
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| i % WORKERS == w)
                    .map(|(_, s)| s.clone())
                    .collect();
                let path = path.to_path_buf();
                let machine = machine.clone();
                let barrier = barrier.clone();
                let state = state.clone();
                scope.spawn(move || {
                    let mut store = Store::open(&path).expect("worker opens the store");
                    barrier.wait();
                    for spec in &ops {
                        execute_op(&mut store, machine.as_ref(), spec, tick, history, &state);
                    }
                    barrier.wait();
                });
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Direct, model-independent invariant checks on the observed history.
// ---------------------------------------------------------------------------

fn command_result(op: &HistoryOp) -> &CommandResult {
    match &op.observed {
        Observed::CommandResult(result) => result,
        _ => panic!("expected a command result"),
    }
}

/// Every committed mutation: contiguous receipt sequences 0..=n per aggregate, one
/// distinct receipt per commit, generation equal to sequence, and the presented
/// capability exactly the current capability at its linearization point.
fn check_commit_invariants(history: &[HistoryOp], machine: &dyn Machine) {
    let mut per_agg: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, op) in history.iter().enumerate() {
        if matches!(&op.observed, Observed::CommandResult(r) if r.mutation) {
            per_agg.entry(op_agg(op).to_string()).or_default().push(i);
        }
    }
    for (agg, idxs) in per_agg {
        let mut sequences: Vec<u64> = idxs
            .iter()
            .map(|&i| {
                command_result(&history[i])
                    .receipt
                    .as_ref()
                    .expect("commits carry receipts")
                    .sequence
            })
            .collect();
        sequences.sort_unstable();
        for (i, sequence) in sequences.iter().enumerate() {
            assert_eq!(
                *sequence, i as u64,
                "aggregate {agg}: committed receipts must be contiguous 0..=n"
            );
        }
        let mut receipt_ids: Vec<&str> = idxs
            .iter()
            .map(|&i| {
                command_result(&history[i])
                    .receipt
                    .as_ref()
                    .expect("receipt")
                    .receipt_id
                    .as_str()
            })
            .collect();
        receipt_ids.sort_unstable();
        receipt_ids.dedup();
        assert_eq!(
            receipt_ids.len(),
            idxs.len(),
            "aggregate {agg}: each commit must carry a distinct receipt id"
        );
        for &i in &idxs {
            let op = &history[i];
            let result = command_result(op);
            let receipt = result.receipt.as_ref().expect("receipt");
            assert_eq!(
                receipt.generation, receipt.sequence,
                "aggregate {agg}: receipt generation must equal its sequence"
            );
            assert_eq!(
                result.generation, receipt.generation,
                "aggregate {agg}: response generation must equal the receipt generation"
            );
            if let OpKind::Command { capability, .. } | OpKind::Record { capability, .. } = &op.kind
            {
                assert_eq!(
                    capability.generation + 1,
                    receipt.generation,
                    "aggregate {agg}: a committed mutation must be authorized by the exact preceding generation"
                );
                let expected = runtime::derive_nonce(
                    machine.identity(),
                    &AggregateId(agg.clone()),
                    capability.generation,
                );
                assert_eq!(
                    capability.nonce, expected,
                    "aggregate {agg}: a committed mutation must carry the exact current nonce"
                );
            }
        }
    }
}

/// Every required transition class must be observed, and stale/wrong/foreign
/// capability classes must never succeed (never mutate).
fn check_coverage(history: &[HistoryOp]) {
    let mut accepted = 0usize;
    let mut refused = 0usize;
    let mut noop = 0usize;
    let mut duplicate = 0usize;
    let mut seen_pending = false;
    let mut seen_recorded = false;
    let mut old = 0usize;
    let mut old_refused = 0usize;
    let mut wrong_nonce = 0usize;
    let mut wrong_nonce_refused = 0usize;
    let mut foreign = 0usize;
    let mut foreign_refused = 0usize;
    for op in history {
        match &op.observed {
            Observed::CommandResult(result) => {
                if result.mutation {
                    accepted += 1;
                } else if result.receipt.is_some() {
                    duplicate += 1;
                } else {
                    noop += 1;
                }
                if matches!(&result.effect, EffectStatus::Pending(_)) {
                    seen_pending = true;
                }
                if matches!(&result.effect, EffectStatus::Recorded(..)) {
                    seen_recorded = true;
                }
            }
            Observed::Projection(projection) => {
                if matches!(&projection.effect, EffectStatus::Pending(_)) {
                    seen_pending = true;
                }
                if matches!(&projection.effect, EffectStatus::Recorded(..)) {
                    seen_recorded = true;
                }
            }
            Observed::Refused { .. } => refused += 1,
        }
        match op.class {
            CapClass::Old => {
                old += 1;
                if matches!(&op.observed, Observed::Refused { .. }) {
                    old_refused += 1;
                }
            }
            CapClass::WrongNonce => {
                wrong_nonce += 1;
                if matches!(&op.observed, Observed::Refused { .. }) {
                    wrong_nonce_refused += 1;
                }
            }
            CapClass::Foreign => {
                foreign += 1;
                if matches!(&op.observed, Observed::Refused { .. }) {
                    foreign_refused += 1;
                }
            }
            CapClass::Current => {}
        }
    }
    assert!(accepted >= 1, "trace must observe an accepted commit");
    assert!(refused >= 1, "trace must observe a refusal");
    assert!(
        noop >= 1,
        "trace must observe a no-op preserving generation"
    );
    assert!(
        duplicate >= 1,
        "trace must observe a duplicate replay preserving generation"
    );
    assert!(
        seen_pending,
        "trace must observe a pending effect transition"
    );
    assert!(
        seen_recorded,
        "trace must observe a recorded effect transition"
    );
    assert!(
        old >= 1 && old == old_refused,
        "old-nonce capabilities must never succeed: {old} fired, {old_refused} refused"
    );
    assert!(
        wrong_nonce >= 1 && wrong_nonce == wrong_nonce_refused,
        "same-generation wrong nonces must never succeed: {wrong_nonce} fired, {wrong_nonce_refused} refused"
    );
    assert!(
        foreign >= 1 && foreign == foreign_refused,
        "cross-aggregate capabilities must never succeed: {foreign} fired, {foreign_refused} refused"
    );
}

// ---------------------------------------------------------------------------
// One full randomized run.
// ---------------------------------------------------------------------------

/// Default number of randomized schedules explored by the entry test when
/// `IK_LINEARIZABILITY_RUNS` is unset, zero, or unparseable. This is the documented
/// minimum: coverage is non-vacuous only when at least one history runs.
const DEFAULT_RUNS: usize = 40;

/// Fixed base seed for schedule derivation. Combined with the run index through
/// `run_seed`, every schedule is fully determined by `(BASE_SEED, index)` — repeated
/// default runs explore exactly the same schedules on every platform and environment.
const BASE_SEED: u64 = 0x006C_696E_5F6B_6572; // "lin_ker"

/// Deterministic per-index schedule seed: splitmix64 finalizer over
/// `BASE_SEED ^ index`, so consecutive indices yield well-separated, reproducible
/// seeds that never depend on the configured run count.
fn run_seed(index: usize) -> u64 {
    let mut z = BASE_SEED ^ (index as u64);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Effective number of schedules to run. `IK_LINEARIZABILITY_RUNS` overrides the
/// count, but `0` and unparseable values are normalized to `DEFAULT_RUNS` so the
/// entry test can never pass after running zero histories.
fn runs() -> usize {
    match std::env::var("IK_LINEARIZABILITY_RUNS") {
        Ok(raw) => match raw.trim().parse::<usize>() {
            Ok(0) | Err(_) => DEFAULT_RUNS,
            Ok(n) => n,
        },
        Err(_) => DEFAULT_RUNS,
    }
}

fn run_one(seed: u64, index: usize, total: usize, dir: &TempDir) {
    // Exposed in every assertion/panic below: which schedule failed, its seed, and
    // the base seed + schedule count that determine the whole default run.
    let ctx = format!("schedule {index}/{total}: seed {seed} (base seed {BASE_SEED:#x})");
    let path = dir.path().join(format!("run-{seed}.db"));
    let machine = lin_machine();
    let tick = AtomicU64::new(1);
    let history: Mutex<Vec<HistoryOp>> = Mutex::new(Vec::new());
    let state: Arc<Mutex<DriverState>> = Arc::new(Mutex::new(DriverState::new(3)));

    // Phase 1: deterministic setup on one connection (guarantees every transition
    // class fires).
    let expected_store_id = {
        let mut store = Store::create(&path)
            .unwrap_or_else(|e| panic!("{ctx}: create store at {}: {e}", path.display()));
        let id = store.store_id().clone();
        for spec in setup_specs() {
            execute_op(&mut store, machine.as_ref(), &spec, &tick, &history, &state);
        }
        id
    };

    // Phase 2: randomized storm; every round reopens through fresh connections.
    let specs = generate(seed);
    let rounds = storm_rounds(&specs);
    assert!(
        rounds.len() >= 2,
        "{ctx}: trace must exercise reopen transitions"
    );
    run_storm(&path, &machine, &rounds, &tick, &history, &state);

    // Phase 3: reopen once more; verify, replay, and compare with the live projection.
    let mut store = Store::open(&path).expect("final reopen");
    assert_eq!(
        store.store_id(),
        &expected_store_id,
        "{ctx}: store identity must survive reopen"
    );
    let summary = store.replay(machine.as_ref(), None).expect("replay");
    let verify = store.verify(machine.as_ref(), None).expect("verify");
    assert!(
        verify.verified && verify.journal_ok && verify.projection_ok,
        "{ctx}: store must verify after the concurrent history: {verify:?}"
    );
    assert!(
        !summary.aggregates.is_empty(),
        "{ctx}: the concurrent history must have created at least one aggregate"
    );
    // Live projections, one per replayed aggregate (a randomized trace may or may
    // not have created a2).
    let mut live: HashMap<String, Projection> = HashMap::new();
    for entry in &summary.aggregates {
        assert!(
            !entry.rebuilt,
            "{ctx}: the stored projection must already agree with replay"
        );
        let projection = store
            .show(&entry.aggregate_id)
            .unwrap_or_else(|e| panic!("{ctx}: show {}: {e}", entry.aggregate_id));
        assert_eq!(
            entry.sequence, projection.sequence,
            "{ctx}: replay sequence must equal the live projection"
        );
        assert_eq!(
            entry.generation, projection.generation,
            "{ctx}: replay generation must equal the live projection"
        );
        assert_eq!(
            entry.state, projection.state,
            "{ctx}: replay state must equal the live projection"
        );
        live.insert(entry.aggregate_id.0.clone(), projection);
    }
    drop(store);

    let history = history.into_inner().expect("history");
    // The observed history must linearize to the live state: a correct store's real
    // serial execution is itself a linearization, so this is the exact property.
    let live_target: HashMap<String, LiveAgg> = live
        .iter()
        .map(|(id, p)| (id.clone(), LiveAgg::from_projection(p)))
        .collect();
    assert!(
        linearizable_to_live(machine.as_ref(), &history, &live_target),
        "{ctx}: the concurrent history does not linearize to the live state"
    );
    check_commit_invariants(&history, machine.as_ref());
    check_coverage(&history);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn concurrent_command_record_histories_linearize() {
    // Structural guard: coverage is non-vacuous only if at least one schedule ran.
    // `runs()` normalizes `IK_LINEARIZABILITY_RUNS=0` and unparseable values to
    // `DEFAULT_RUNS`, so this can only trip if the normalization itself regresses.
    let total = runs();
    assert!(
        total >= 1,
        "linearizability coverage must run at least one schedule: effective count {total}, base seed {BASE_SEED:#x}"
    );
    let dir = tempfile::tempdir().expect("tempdir");
    for index in 0..total {
        run_one(run_seed(index), index, total, &dir);
    }
}

#[test]
fn sequential_reference_matches_store_across_reopen() {
    // Validate the reference model op-by-op against the real store, including a
    // mid-trace reopen. If this passes, any linearizability failure in the
    // concurrent test is a genuine store bug rather than a model drift.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("seq.db");
    let machine = lin_machine();
    let state: Arc<Mutex<DriverState>> = Arc::new(Mutex::new(DriverState::new(3)));
    let mut model = Model::new();

    let mut store = Store::create(&path).expect("create");
    let expected_store_id = store.store_id().clone();
    for spec in setup_specs() {
        step_compare(&mut store, &mut model, machine.as_ref(), &spec, &state);
    }
    drop(store);

    // Reopen: capabilities observed before the reopen must remain valid and the
    // effect state must survive.
    let mut store = Store::open(&path).expect("reopen");
    assert_eq!(store.store_id(), &expected_store_id);
    for spec in post_reopen_specs() {
        step_compare(&mut store, &mut model, machine.as_ref(), &spec, &state);
    }

    let verify = store.verify(machine.as_ref(), None).expect("verify");
    assert!(verify.verified, "sequential store must verify: {verify:?}");
    let summary = store.replay(machine.as_ref(), None).expect("replay");
    for entry in &summary.aggregates {
        assert!(
            !entry.rebuilt,
            "stored projection must already agree with replay"
        );
    }
    for name in ["a0", "a1", "a2"] {
        let projection = store.show(&AggregateId(name.to_string())).expect("show");
        let a = model.aggs.get(name).expect("model aggregate");
        assert_eq!(a.state, projection.state, "{name}: model state equals live");
        assert_eq!(
            a.generation, projection.generation,
            "{name}: model generation"
        );
        assert_eq!(a.nonce, projection.capability.nonce, "{name}: model nonce");
        assert_eq!(a.effect, projection.effect, "{name}: model effect");
        assert_eq!(a.terminal, projection.terminal, "{name}: model terminal");
        assert_eq!(a.sequence, projection.sequence, "{name}: model sequence");
        assert_eq!(
            a.last_event_hash, projection.last_event_hash,
            "{name}: model last event hash"
        );
        assert_eq!(
            a.last_receipt_id,
            projection
                .last_receipt
                .as_ref()
                .map(|r| r.receipt_id.clone())
                .unwrap_or_default(),
            "{name}: model last receipt id"
        );
    }
}

fn step_compare(
    store: &mut Store,
    model: &mut Model,
    machine: &dyn Machine,
    spec: &OpSpec,
    state: &Arc<Mutex<DriverState>>,
) {
    let (kind, _class) = {
        let mut st = state.lock().expect("driver state");
        materialize(machine, &mut st, spec)
    };
    let observed = run_store(store, machine, &kind);
    {
        let mut st = state.lock().expect("driver state");
        update_state(&mut st, &kind, &observed);
    }
    let predicted = match model.apply_op(machine, &kind) {
        Ok(p) => p,
        Err(d) => Observed::Refused {
            code: d.code,
            rule: d.rule,
        },
    };
    assert!(
        same_observed(&predicted, &observed),
        "sequential reference mismatch for {spec:?}:\n  predicted: {predicted:?}\n  observed:  {observed:?}"
    );
}
