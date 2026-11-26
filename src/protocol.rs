//! Strict versioned canonical JSON protocol (K2).
//!
//! One request envelope per line/process invocation over stdin, one response envelope
//! on stdout. Unknown-major protocol, unknown runtime version, and unknown fields fail
//! closed with stable diagnostics. All output is canonical JSON (sorted keys, no
//! whitespace, absent optionals omitted), so identical accepted requests produce
//! byte-stable normalized responses.

use crate::diagnostic::Diagnostic;
use crate::effect::EffectOutcomeV1;
use crate::identity::{Capability, MachineIdentity};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_MAJOR: u64 = 1;
pub const PROTOCOL_MINOR: u64 = 0;
pub const RUNTIME_VERSION: &str = "0.6.0";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub major: u64,
    pub minor: u64,
}

impl Default for ProtocolVersion {
    fn default() -> Self {
        ProtocolVersion {
            major: PROTOCOL_MAJOR,
            minor: PROTOCOL_MINOR,
        }
    }
}

/// Operations supported by the `ik` CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Create,
    Command,
    Record,
    Show,
    Replay,
    Verify,
    Transaction,
    /// Read-only: report whether a caller `operation_id` committed.
    #[serde(rename = "lookup_operation")]
    LookupOperation,
    /// Genesis plus the initializing command in one store transaction.
    /// Additive on protocol 1.0: older binaries reject the unknown op.
    #[serde(rename = "create_with_command")]
    CreateWithCommand,
}

impl Op {
    /// Stable lowercase wire name of this operation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Op::Create => "create",
            Op::Command => "command",
            Op::Record => "record",
            Op::Show => "show",
            Op::Replay => "replay",
            Op::Verify => "verify",
            Op::Transaction => "transaction",
            Op::LookupOperation => "lookup_operation",
            Op::CreateWithCommand => "create_with_command",
        }
    }
}

/// Strict request envelope. Unknown fields are rejected.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeRequest {
    pub protocol: ProtocolVersion,
    pub runtime_version: String,
    pub request_id: String,
    pub op: Op,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregate_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine: Option<MachineIdentity>,
    /// Optional external reducer launch specification. It travels in the
    /// request body so valid reducer state is not constrained by OS argv limits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reducer_worker: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<Capability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<EffectOutcomeV1>,
    /// `transaction` only: ordered commands applied atomically (all-or-nothing) in ONE store
    /// transaction. Mutually exclusive with `command`/`observation`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<Vec<TransactionStep>>,
    /// Caller-stable identity of one logical operation, separate from the
    /// per-attempt `request_id`. Valid on `command`, `transaction`, `record`,
    /// `lookup_operation`, and `create_with_command`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}

/// One command of a `transaction`, with its optional observation. Capabilities are never
/// supplied per step: the kernel threads the successor capability inside the transaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionStep {
    pub command: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<Value>,
}

/// Response envelope. `result` on success, `diagnostic` on refusal.
#[derive(Clone, Debug, Serialize)]
pub struct EnvelopeResponse {
    pub protocol: ProtocolVersion,
    pub runtime_version: String,
    pub request_id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<Diagnostic>,
}

impl EnvelopeResponse {
    pub fn success(request_id: &str, result: Value) -> Self {
        EnvelopeResponse {
            protocol: ProtocolVersion::default(),
            runtime_version: RUNTIME_VERSION.to_string(),
            request_id: request_id.to_string(),
            ok: true,
            result: Some(result),
            diagnostic: None,
        }
    }

    pub fn failure(request_id: &str, diagnostic: Diagnostic) -> Self {
        EnvelopeResponse {
            protocol: ProtocolVersion::default(),
            runtime_version: RUNTIME_VERSION.to_string(),
            request_id: request_id.to_string(),
            ok: false,
            result: None,
            diagnostic: Some(diagnostic),
        }
    }
}
