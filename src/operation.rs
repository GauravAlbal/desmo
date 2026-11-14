//! Caller-stable operation identity (IK-03; `docs/operation-identity.md`).
//!
//! An `operation_id` names one logical caller intent across transport attempts.
//! It is distinct from the per-attempt `request_id` and from external effect
//! identity. Attribution is written in the same SQLite transaction as the
//! mutation it names and is not part of the journal hash chain.

use crate::canonical::digest_json;
use crate::diagnostic::{Diagnostic, DiagnosticCode, NO_RULE};
use crate::identity::{AggregateId, Capability, MachineIdentity};
use crate::store::{CommandRequest, EffectOutcomeRequest, TransactionRequest};
use serde::Serialize;
use serde_json::{json, Value};

/// The capability as it enters a mutation's fingerprint. A test-support mutant deletes it.
fn fingerprinted_capability(capability: &Capability) -> Option<&Capability> {
    #[cfg(feature = "test-support")]
    if crate::law_mutant::is(crate::law_mutant::LawMutant::OmitCapabilityFromFingerprint) {
        return None;
    }
    Some(capability)
}

/// Upper bound on an operation id, in bytes.
pub const MAX_OPERATION_ID_BYTES: usize = 128;

/// A validated caller operation id: 1..=128 bytes of `[A-Za-z0-9._:-]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationId(String);

impl OperationId {
    pub fn parse(raw: &str) -> Result<Self, Diagnostic> {
        let well_formed = !raw.is_empty()
            && raw.len() <= MAX_OPERATION_ID_BYTES
            && raw
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'));
        if well_formed {
            return Ok(Self(raw.to_string()));
        }
        Err(Diagnostic::new(
            DiagnosticCode::IkProtocolInvalid,
            Some("request.operation_id"),
            format!("operation_id must be 1..={MAX_OPERATION_ID_BYTES} bytes of [A-Za-z0-9._:-]"),
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The kind of mutation an operation row attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Command,
    Transaction,
    Record,
    CreateWithCommand,
}

impl OperationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Transaction => "transaction",
            Self::Record => "record",
            Self::CreateWithCommand => "create_with_command",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "command" => Some(Self::Command),
            "transaction" => Some(Self::Transaction),
            "record" => Some(Self::Record),
            "create_with_command" => Some(Self::CreateWithCommand),
            _ => None,
        }
    }
}

/// What the kernel durably recorded when the operation committed. It carries no
/// capability: a historical result is never a current handle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OperationRecord {
    pub operation_id: String,
    pub kind: OperationKind,
    pub fingerprint: String,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub generation: u64,
    pub state_digest: String,
    pub event_hashes: Vec<String>,
    pub receipt_ids: Vec<String>,
}

/// A recorded operation together with the aggregate's coordinates as of the read,
/// so a caller can see that the historical result is not current.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OperationHistory {
    pub record: OperationRecord,
    pub current_generation: u64,
    pub current_sequence: u64,
}

/// Result of a mutation that may carry an operation id.
#[derive(Clone, Debug)]
pub enum OperationOutcome<T> {
    /// The request executed (committed, no-op, or an existing non-attributed duplicate).
    Committed(T),
    /// An exact repeat of an already committed operation. Nothing executed.
    Historical(OperationHistory),
}

/// Read-only answer to "did this operation commit", as of this read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Lookup {
    CommittedExact(OperationHistory),
    Conflict {
        kind: OperationKind,
        first_sequence: u64,
        last_sequence: u64,
        current_generation: u64,
        current_sequence: u64,
    },
    /// No record as of this read. This is not a refusal, a cancellation, or
    /// permission to retry under a new id.
    NotRecorded {
        current_generation: Option<u64>,
        current_sequence: Option<u64>,
    },
}

/// The semantic request an operation id names. Borrowed from the mutation or
/// lookup request so both fingerprint identically.
pub enum OperationRequest<'a> {
    Command(&'a CommandRequest),
    Transaction(&'a TransactionRequest),
    Record(&'a EffectOutcomeRequest),
    CreateWithCommand {
        machine: &'a MachineIdentity,
        aggregate_id: &'a AggregateId,
        command: &'a Value,
        observation: Option<&'a Value>,
    },
}

impl OperationRequest<'_> {
    pub fn kind(&self) -> OperationKind {
        match self {
            Self::Command(_) => OperationKind::Command,
            Self::Transaction(_) => OperationKind::Transaction,
            Self::Record(_) => OperationKind::Record,
            Self::CreateWithCommand { .. } => OperationKind::CreateWithCommand,
        }
    }

    pub fn aggregate_id(&self) -> &AggregateId {
        match self {
            Self::Command(req) => &req.aggregate_id,
            Self::Transaction(req) => &req.aggregate_id,
            Self::Record(req) => &req.aggregate_id,
            Self::CreateWithCommand { aggregate_id, .. } => aggregate_id,
        }
    }

    pub fn machine(&self) -> &MachineIdentity {
        match self {
            Self::Command(req) => &req.machine,
            Self::Transaction(req) => &req.machine,
            Self::Record(req) => &req.machine,
            Self::CreateWithCommand { machine, .. } => machine,
        }
    }

    /// Digest of the semantic request. Excludes the per-attempt `request_id`,
    /// runtime/protocol versions, local paths, and reducer launch details. The
    /// original capability is included: a refreshed capability is a different
    /// request.
    pub fn fingerprint(&self) -> String {
        let body = match self {
            Self::Command(req) => json!({
                "kind": self.kind(),
                "machine": req.machine,
                "aggregate_id": req.aggregate_id,
                "capability": fingerprinted_capability(&req.capability),
                "command": req.command,
                "observation": req.observation,
            }),
            Self::Transaction(req) => json!({
                "kind": self.kind(),
                "machine": req.machine,
                "aggregate_id": req.aggregate_id,
                "capability": fingerprinted_capability(&req.capability),
                "steps": req.steps,
            }),
            Self::Record(req) => json!({
                "kind": self.kind(),
                "machine": req.machine,
                "aggregate_id": req.aggregate_id,
                "capability": fingerprinted_capability(&req.capability),
                "effect": req.effect,
            }),
            Self::CreateWithCommand {
                machine,
                aggregate_id,
                command,
                observation,
            } => json!({
                "kind": self.kind(),
                "machine": machine,
                "aggregate_id": aggregate_id,
                "command": command,
                "observation": observation,
            }),
        };
        digest_json(&body)
    }
}

/// An operation id bound to one fingerprinted request, ready to be recorded.
pub(crate) struct Binding {
    pub operation_id: OperationId,
    pub kind: OperationKind,
    pub fingerprint: String,
}

impl Binding {
    pub fn new(operation_id: &OperationId, request: &OperationRequest<'_>) -> Self {
        Self {
            operation_id: operation_id.clone(),
            kind: request.kind(),
            fingerprint: request.fingerprint(),
        }
    }

    /// Build the row for a committed run of consecutive journal events.
    pub fn record(
        &self,
        committed: &[(&crate::runtime::JournalEvent, &crate::runtime::Receipt)],
    ) -> Result<OperationRecord, Diagnostic> {
        let (Some((first, _)), Some((last, _))) = (committed.first(), committed.last()) else {
            return Err(Diagnostic::new(
                DiagnosticCode::IkInternal,
                NO_RULE,
                "an operation row requires at least one committed event",
            ));
        };
        Ok(OperationRecord {
            operation_id: self.operation_id.as_str().to_string(),
            kind: self.kind,
            fingerprint: self.fingerprint.clone(),
            first_sequence: first.sequence,
            last_sequence: last.sequence,
            generation: last.generation,
            state_digest: last.state_digest.clone(),
            event_hashes: committed.iter().map(|(e, _)| e.hash.clone()).collect(),
            receipt_ids: committed
                .iter()
                .map(|(_, r)| r.receipt_id.clone())
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_id_accepts_the_documented_charset_and_bounds() {
        assert!(OperationId::parse("op-1.a_b:c").is_ok());
        assert!(OperationId::parse(&"a".repeat(MAX_OPERATION_ID_BYTES)).is_ok());
    }

    #[test]
    fn operation_id_rejects_empty_oversized_and_unlisted_bytes() {
        assert!(OperationId::parse("").is_err());
        assert!(OperationId::parse(&"a".repeat(MAX_OPERATION_ID_BYTES + 1)).is_err());
        assert!(OperationId::parse("has space").is_err());
        assert!(OperationId::parse("slash/path").is_err());
        assert!(OperationId::parse("naïve").is_err());
    }
}
