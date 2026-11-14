//! Stable runtime identities: store, aggregate, machine, capability, effect.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable identity of a persisted store. Generated at store creation and durable.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StoreId(pub String);

impl fmt::Display for StoreId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stable identity of an aggregate within a store.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AggregateId(pub String);

impl fmt::Display for AggregateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Integer version of a machine definition or implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MachineVersion(pub u64);

impl fmt::Display for MachineVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Hex sha256 of the machine spec. For declarative machines this is the digest of the
/// canonical machine definition; native machines provide their own immutable value.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MachineSpecHash(pub String);

impl fmt::Display for MachineSpecHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Full machine identity: `(machine_id, machine_version, machine_spec_hash)`.
/// Persisted aggregates retain the exact identity used to create them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MachineIdentity {
    pub machine_id: String,
    pub version: MachineVersion,
    pub spec_hash: MachineSpecHash,
}

/// Monotonic mutation counter for an aggregate. Increments exactly once per committed
/// mutation; the genesis aggregate sits at generation 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Generation(pub u64);

/// Rotation token for a generation. Derived deterministically from the aggregate and
/// machine identity (positive control, not a secret), so replay reproduces it and
/// normalized responses are byte-stable.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Nonce(pub String);

/// Mutation capability: valid only for the exact current `(generation, nonce)` pair.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Capability {
    pub generation: u64,
    pub nonce: String,
}

/// Stable identity of a planned external effect.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EffectId(pub String);

impl fmt::Display for EffectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
