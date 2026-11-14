//! Stable, machine-derived schema metadata for clients and code generation.
//!
//! Generators must read this projection instead of parsing `.ik.toml` themselves: Rust owns
//! the machine semantics, and a client that re-interprets them would be a second semantic
//! engine. The export names shapes (states, facts, command arguments, successor states); it
//! carries no guard or effect logic and grants no authority.

use super::def::Definition;
use crate::identity::MachineIdentity;
use serde_json::{json, Value};

pub const SCHEMA_ID: &str = "ikaros.machine-schema.v1";

/// Project a definition and its identity into the versioned schema document.
pub fn machine_schema(def: &Definition, identity: &MachineIdentity) -> Value {
    json!({
        "schema": SCHEMA_ID,
        "machine": {
            "machine_id": identity.machine_id,
            "version": identity.version,
            "spec_hash": identity.spec_hash,
        },
        "states": states(def),
        "facts": facts(def),
        "commands": commands(def),
        "invariants": def.invariants.keys().collect::<Vec<_>>(),
    })
}

fn states(def: &Definition) -> Vec<Value> {
    def.states
        .iter()
        .map(|(name, state)| {
            json!({"name": name, "initial": state.initial, "terminal": state.terminal})
        })
        .collect()
}

/// A fact is `required` only when the definition gives it an initial value, so it exists from
/// genesis. Every other fact may be absent and must stay optional in generated types.
fn facts(def: &Definition) -> Vec<Value> {
    def.facts
        .iter()
        .map(|(name, fact)| {
            json!({
                "name": name,
                "type": fact.ty,
                "policy": fact.policy,
                "required": fact.initial.is_some(),
            })
        })
        .collect()
}

fn commands(def: &Definition) -> Vec<Value> {
    def.commands
        .iter()
        .map(|(name, cmd)| {
            let args: Vec<Value> = cmd
                .args
                .iter()
                .flatten()
                .map(|(arg, ty)| json!({"name": arg, "type": ty}))
                .collect();
            let successors: Vec<&str> = match &cmd.outcomes {
                Some(outcomes) => outcomes.iter().map(|o| o.to.as_str()).collect(),
                None => cmd.to.iter().map(String::as_str).collect(),
            };
            json!({
                "name": name,
                "from": cmd.from,
                "args": args,
                "successors": successors,
                "conditional_successor": cmd.outcomes.as_ref().is_some_and(|o| o.len() > 1),
                "plans_effect": cmd.effect.is_some(),
            })
        })
        .collect()
}
