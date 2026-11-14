//! Minimal external effect identity (K2.5 / G-002).
//!
//! The runtime effect state of an aggregate is exactly NONE, PENDING(intent), or
//! RECORDED(intent, outcome_digest). A committed transition may create at most one
//! pending intent. An outcome command binds the exact pending identity
//! (aggregate, effect_id, planned_generation, payload_digest, idempotency_key) plus
//! `outcome_digest`. Exact duplicates replay the original receipt with zero mutation;
//! conflicts, stale, and foreign outcomes refuse. The runtime never executes, retries,
//! schedules, leases, times out, classifies, or selects effects, and never interprets
//! `effect_kind` or `payload`.

use crate::identity::{AggregateId, EffectId};
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// Durable identity of a planned external effect. `effect_kind` and `payload` are
/// opaque to the runtime; the domain machine decides which kinds are legal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectIntentV1 {
    pub effect_id: EffectId,
    pub aggregate_id: AggregateId,
    /// Generation of the state in which this intent is pending (stamped by the runtime).
    pub generation: u64,
    /// Opaque domain kind selected by the bound domain machine.
    pub effect_kind: String,
    /// Digest of the effect payload.
    pub payload_digest: String,
    /// Stable idempotency key supplied by the domain.
    pub idempotency_key: String,
    /// Opaque effect payload, optionally persisted; covered by `payload_digest`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

/// Outcome binding: the exact pending identity plus the recorded outcome digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectOutcomeV1 {
    pub effect_id: EffectId,
    pub aggregate_id: AggregateId,
    pub planned_generation: u64,
    pub payload_digest: String,
    pub idempotency_key: String,
    pub outcome_digest: String,
}

impl EffectOutcomeV1 {
    /// Does this outcome bind the exact identity of the given pending intent?
    pub fn binds(&self, intent: &EffectIntentV1) -> bool {
        self.effect_id == intent.effect_id
            && self.aggregate_id == intent.aggregate_id
            && self.planned_generation == intent.generation
            && self.payload_digest == intent.payload_digest
            && self.idempotency_key == intent.idempotency_key
    }

    /// Exact duplicate of an already-recorded outcome (all six binding fields).
    pub fn exact_duplicate_of(&self, intent: &EffectIntentV1, outcome_digest: &str) -> bool {
        self.binds(intent) && self.outcome_digest == outcome_digest
    }

    /// Foreign identity: a different effect or a different aggregate.
    pub fn is_foreign_to(&self, intent: &EffectIntentV1) -> bool {
        self.effect_id != intent.effect_id || self.aggregate_id != intent.aggregate_id
    }
}

/// Runtime effect state of an aggregate. Persisted with the projection and rebuilt
/// on replay from journal effect deltas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectStatus {
    None,
    Pending(EffectIntentV1),
    /// intent, recorded outcome digest, receipt id of the recording transition
    Recorded(EffectIntentV1, String, Option<String>),
}

impl EffectStatus {
    /// The pending intent, if any.
    pub fn pending_intent(&self) -> Option<&EffectIntentV1> {
        match self {
            EffectStatus::None => None,
            EffectStatus::Pending(i) => Some(i),
            EffectStatus::Recorded(i, _, _) => Some(i),
        }
    }
}

impl Serialize for EffectStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            EffectStatus::None => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("status", "none")?;
                map.end()
            }
            EffectStatus::Pending(intent) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("status", "pending")?;
                map.serialize_entry("intent", intent)?;
                map.end()
            }
            EffectStatus::Recorded(intent, outcome_digest, receipt_id) => {
                let mut map = serializer.serialize_map(Some(4))?;
                map.serialize_entry("status", "recorded")?;
                map.serialize_entry("intent", intent)?;
                map.serialize_entry("outcome_digest", outcome_digest)?;
                if let Some(id) = receipt_id {
                    map.serialize_entry("receipt_id", id)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for EffectStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            status: String,
            intent: Option<EffectIntentV1>,
            outcome_digest: Option<String>,
            receipt_id: Option<String>,
        }
        let wire = Wire::deserialize(deserializer)?;
        match wire.status.as_str() {
            "none" => Ok(EffectStatus::None),
            "pending" => Ok(EffectStatus::Pending(wire.intent.ok_or_else(|| {
                DeError::custom("pending effect status requires intent")
            })?)),
            "recorded" => Ok(EffectStatus::Recorded(
                wire.intent
                    .ok_or_else(|| DeError::custom("recorded effect status requires intent"))?,
                wire.outcome_digest.ok_or_else(|| {
                    DeError::custom("recorded effect status requires outcome_digest")
                })?,
                wire.receipt_id,
            )),
            other => Err(DeError::custom(format!("unknown effect status: {other}"))),
        }
    }
}
