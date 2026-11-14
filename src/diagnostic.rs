//! Stable machine-readable diagnostics (`IK_*`). Every refusal carries a code; the
//! runtime itself only ever emits `IK_*` codes. Domain machines carry their own
//! codes in the `Domain` slot and keep them out of this registry.

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

/// Stable diagnostic codes, matching the DIAGNOSTICS registry.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticCode {
    /// Malformed or structurally invalid wire message.
    IkProtocolInvalid,
    /// Unknown protocol major or runtime version.
    IkProtocolVersionUnknown,
    /// Requested machine identity is not registered.
    IkMachineUnknown,
    /// Presented machine identity differs from the aggregate's recorded identity.
    IkMachineMismatch,
    /// Machine id is known but the version is not.
    IkMachineVersionUnknown,
    /// Machine id and version are known but the spec hash differs.
    IkMachineHashMismatch,
    /// Domain event type/version is not recognized by the bound machine identity.
    IkEventSchemaUnknown,
    /// Unknown persisted schema version.
    IkPersistedSchemaUnknown,
    /// Operation is not legal in the current state.
    IkIllegalState,
    /// Command is structurally invalid (unknown name, missing/wrong args).
    IkCommandInvalid,
    /// Capability generation is older than the current generation.
    IkCapabilityStale,
    /// Capability generation matches but the nonce is invalid.
    IkNonceInvalid,
    /// A domain invariant or guard failed; the rule names the failing condition.
    IkInvariantFailed,
    /// Effect outcome references a foreign effect identity.
    IkEffectMismatch,
    /// Effect outcome is stale (nothing pending / new effect while pending).
    IkEffectStale,
    /// Same effect identity with a different outcome or binding.
    IkEffectOutcomeConflict,
    /// A concurrent mutation was detected (write lock contention).
    IkConcurrentMutation,
    /// Present store file is unreadable or internally invalid. A missing file is
    /// [`DiagnosticCode::IkStoreNotFound`], not corruption.
    IkStoreCorrupt,
    /// The store path does not name an existing file. Read-only operations do not create it.
    IkStoreNotFound,
    /// Journal sequence/hash chain corruption.
    IkJournalCorrupt,
    /// Projection disagrees with journal replay.
    IkProjectionCorrupt,
    /// Journal events do not reproduce the recorded states.
    IkReplayMismatch,
    /// A monotone counter (generation or sequence) is at the `u64` ceiling; the
    /// next transition would overflow.
    IkCounterOverflow,
    /// A transaction would span an external effect boundary (an effect intent before its last step).
    IkTransactionBoundary,
    /// The same caller operation id was reused for a different semantic request.
    IkOperationConflict,
    /// Internal runtime or machine-contract violation.
    IkInternal,
    /// Authored chart disagrees with the lowered definition.
    IkChartInconsistent,
    /// Domain-owned code, opaque to the runtime.
    Domain(String),
}

impl DiagnosticCode {
    /// Stable string form, e.g. `IK_CAPABILITY_STALE` or the domain code verbatim.
    pub fn as_str(&self) -> String {
        match self {
            Self::IkProtocolInvalid => "IK_PROTOCOL_INVALID".into(),
            Self::IkProtocolVersionUnknown => "IK_PROTOCOL_VERSION_UNKNOWN".into(),
            Self::IkMachineUnknown => "IK_MACHINE_UNKNOWN".into(),
            Self::IkMachineMismatch => "IK_MACHINE_MISMATCH".into(),
            Self::IkMachineVersionUnknown => "IK_MACHINE_VERSION_UNKNOWN".into(),
            Self::IkMachineHashMismatch => "IK_MACHINE_HASH_MISMATCH".into(),
            Self::IkEventSchemaUnknown => "IK_EVENT_SCHEMA_UNKNOWN".into(),
            Self::IkPersistedSchemaUnknown => "IK_PERSISTED_SCHEMA_UNKNOWN".into(),
            Self::IkIllegalState => "IK_ILLEGAL_STATE".into(),
            Self::IkCommandInvalid => "IK_COMMAND_INVALID".into(),
            Self::IkCapabilityStale => "IK_CAPABILITY_STALE".into(),
            Self::IkNonceInvalid => "IK_NONCE_INVALID".into(),
            Self::IkInvariantFailed => "IK_INVARIANT_FAILED".into(),
            Self::IkEffectMismatch => "IK_EFFECT_MISMATCH".into(),
            Self::IkEffectStale => "IK_EFFECT_STALE".into(),
            Self::IkEffectOutcomeConflict => "IK_EFFECT_OUTCOME_CONFLICT".into(),
            Self::IkConcurrentMutation => "IK_CONCURRENT_MUTATION".into(),
            Self::IkStoreCorrupt => "IK_STORE_CORRUPT".into(),
            Self::IkStoreNotFound => "IK_STORE_NOT_FOUND".into(),
            Self::IkJournalCorrupt => "IK_JOURNAL_CORRUPT".into(),
            Self::IkProjectionCorrupt => "IK_PROJECTION_CORRUPT".into(),
            Self::IkReplayMismatch => "IK_REPLAY_MISMATCH".into(),
            Self::IkCounterOverflow => "IK_COUNTER_OVERFLOW".into(),
            Self::IkTransactionBoundary => "IK_TRANSACTION_BOUNDARY".into(),
            Self::IkOperationConflict => "IK_OPERATION_CONFLICT".into(),
            Self::IkInternal => "IK_INTERNAL".into(),
            Self::IkChartInconsistent => "IK_CHART_INCONSISTENT".into(),
            Self::Domain(s) => s.clone(),
        }
    }
}

/// Explicitly typed `None` for the optional rule slot (helps type inference).
pub const NO_RULE: Option<&str> = None;

/// A refusal. `rule` names the failing rule/condition when the failure is rule-scoped
/// (IK-LAW-008); `message` is human prose for operators, never the only interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub rule: Option<String>,
    pub message: String,
}

impl Diagnostic {
    pub fn new(
        code: DiagnosticCode,
        rule: Option<impl Into<String>>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            rule: rule.map(Into::into),
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(DiagnosticCode::IkInternal, NO_RULE, message)
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.code.as_str())?;
        if let Some(rule) = &self.rule {
            write!(f, " rule={rule}")?;
        }
        write!(f, ": {}", self.message)
    }
}

impl std::error::Error for Diagnostic {}

impl Serialize for Diagnostic {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("Diagnostic", 3)?;
        st.serialize_field("code", &self.code.as_str())?;
        if let Some(rule) = &self.rule {
            st.serialize_field("rule", rule)?;
        }
        st.serialize_field("message", &self.message)?;
        st.end()
    }
}
