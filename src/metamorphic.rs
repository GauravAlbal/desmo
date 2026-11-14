//! Kernel-owned metamorphic relations.
//!
//! These checks compare authoritative Store/Machine observations. They do not
//! implement a second reducer and do not depend on Hegel.

use crate::canonical::digest_json;
use crate::diagnostic::Diagnostic;
use crate::effect::EffectStatus;
use crate::identity::{AggregateId, Capability};
use crate::machine::{Decision, DomainEvent, Machine};
use crate::store::{CommandRequest, EffectOutcomeRequest, Store};
use serde_json::Value;

/// Canonical digest of durable projection truth for one aggregate.
pub fn durable_truth(store: &mut Store, aggregate: &AggregateId) -> Result<String, Diagnostic> {
    let shown = store.show(aggregate)?;
    Ok(digest_json(&shown))
}

/// META-001: a refused mutation leaves durable truth unchanged.
pub fn refusal_preserves_truth(before: &str, after: &str) -> bool {
    before == after
}

/// META-002: live projection matches post-replay projection.
pub fn replay_matches_live(live: &str, replayed: &str) -> bool {
    live == replayed
}

/// META-003: an exact duplicate commit does not change truth.
pub fn duplicate_is_idempotent(before: &str, after: &str) -> bool {
    before == after
}

/// META-004: a stale capability is never current after a later generation.
pub fn stale_never_current(old: &Capability, current: &Capability) -> bool {
    old != current && old.generation < current.generation
}

/// META-005: the same live state+command folds twice identically.
pub fn deterministic_fold(
    machine: &dyn Machine,
    state: &Value,
    effect: &EffectStatus,
    command: &Value,
) -> Result<bool, Diagnostic> {
    let first = fold_once(machine, state, effect, command)?;
    let second = fold_once(machine, state, effect, command)?;
    Ok(first == second)
}

/// META-006: applying a frozen event does not re-run live guard selection.
pub fn frozen_event_stable(
    machine: &dyn Machine,
    original_state: &Value,
    mutated_state: &Value,
    event: &DomainEvent,
) -> Result<bool, Diagnostic> {
    let original = machine.apply(original_state, event)?;
    let mutated = machine.apply(mutated_state, event)?;
    Ok(original.state.get("state") == mutated.state.get("state")
        && original.terminal == mutated.terminal)
}

/// Snapshot truth, run a refusing command, snapshot again.
pub fn check_refusal_immutability(
    store: &mut Store,
    machine: &dyn Machine,
    req: &CommandRequest,
) -> Result<bool, Diagnostic> {
    let before = durable_truth(store, &req.aggregate_id)?;
    let refused = store.command(machine, req).is_err();
    let after = durable_truth(store, &req.aggregate_id)?;
    Ok(refused && refusal_preserves_truth(&before, &after))
}

/// Snapshot truth, replay, snapshot again.
pub fn check_replay_equivalence(
    store: &mut Store,
    machine: &dyn Machine,
    aggregate: &AggregateId,
) -> Result<bool, Diagnostic> {
    let live = durable_truth(store, aggregate)?;
    store.replay(machine, Some(aggregate))?;
    let replayed = durable_truth(store, aggregate)?;
    Ok(replay_matches_live(&live, &replayed))
}

/// Snapshot truth, record an exact duplicate, snapshot again.
pub fn check_duplicate_idempotence(
    store: &mut Store,
    req: &EffectOutcomeRequest,
) -> Result<bool, Diagnostic> {
    let before = durable_truth(store, &req.aggregate_id)?;
    let result = store.record_effect(req)?;
    let after = durable_truth(store, &req.aggregate_id)?;
    Ok(!result.mutation && duplicate_is_idempotent(&before, &after))
}

fn fold_once(
    machine: &dyn Machine,
    state: &Value,
    effect: &EffectStatus,
    command: &Value,
) -> Result<(Value, bool), Diagnostic> {
    let Decision::Emit { event, .. } = machine.decide(state, None, effect, command)? else {
        return Err(Diagnostic::internal("expected emit"));
    };
    let applied = machine.apply(state, &event)?;
    Ok((applied.state, applied.terminal))
}
