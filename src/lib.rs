//! # invariant-kernel
//!
//! A reusable pure invariant runtime. Provides:
//!
//! - **machine contract** — a uniform pure `Machine` over canonical JSON values plus a
//!   typed `NativeMachine` adapter, so declarative and native reducers execute through
//!   the same runtime transaction path;
//! - **declarative machines** — compiled from inspectable `.ik.toml` definitions with
//!   strict static checks;
//! - **durable runtime** — a SQLite store with generation/nonce capabilities, an
//!   append-only hash-chained journal, a projection, durable receipts, replay, and
//!   verification with corruption detection;
//! - **minimal effect identity** — durable `EffectIntentV1` with
//!   none/pending/recorded states and exact outcome binding;
//! - **strict protocol** — versioned canonical JSON envelopes over a stdin/stdout CLI.
//!
//! Reusable modules contain no domain vocabulary and the runtime emits only
//! `IK_*` diagnostics.

pub mod analysis;
pub mod canonical;
pub mod census;
pub mod cli;
pub mod codegen;
pub mod declarative;
pub mod diagram;
pub mod diagnostic;
pub mod effect;
pub mod evidence_verify;
pub mod external;
pub mod graph_export;
pub mod handoff;
mod handoff_decision;
mod handoff_frame;
mod handoff_gaps;
mod handoff_origin;
mod handoff_witness;
mod readout;
pub mod identity;
#[cfg(feature = "test-support")]
pub mod law_mutant;
pub mod machine;
pub mod metamorphic;
pub mod operation;
pub mod operations;
pub mod protocol;
pub mod regression;
pub mod report;
pub mod runtime;
pub mod scenario;
pub mod simulate;
pub mod smt;
pub mod store;

pub mod prelude {
    pub use crate::canonical::{canonical_json, digest_json, sha256_hex};
    pub use crate::diagnostic::{Diagnostic, DiagnosticCode};
    pub use crate::effect::{EffectIntentV1, EffectOutcomeV1, EffectStatus};
    pub use crate::external::{ExternalMachine, ReducerIdentity, WorkerSpec};
    pub use crate::identity::{
        AggregateId, Capability, EffectId, Generation, MachineIdentity, MachineSpecHash,
        MachineVersion, Nonce, StoreId,
    };
    pub use crate::machine::{
        ApplyOutcome, Decision, DomainEvent, EventCodec, Machine, MachineRegistry, NativeAdapter,
        NativeApply, NativeDecision, NativeMachine, RegisteredMachine,
    };
    pub use crate::protocol::{Op, ProtocolVersion, RUNTIME_VERSION};
}
