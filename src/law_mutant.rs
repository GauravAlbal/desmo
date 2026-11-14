//! Test-support constitutional mutants. Not compiled into production builds.
//!
//! A mutant deletes one kernel wall so the verification substrate can try to
//! kill it. There are no environment flags and no production `Store` fields.
//!
//! Evidence that a mutant was killed is only meaningful if the deleted wall was
//! actually reached. Every query of the active mutant at its seam is counted:
//! [`with_law_mutant`] refuses to return when the seam never fired, and
//! [`observe_law_mutant`] reports the count for controls that expect zero.

use std::cell::Cell;

/// One deleted constitutional wall.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LawMutant {
    AcceptStaleCapability,
    SkipGenerationIncrement,
    SkipNonceRotation,
    AllowTerminalMutation,
    SkipInvariantValidation,
    AllowSecondPendingEffect,
    AllowEffectConflict,
    MutateOnDuplicate,
    RerunLiveGuardsOnReplay,
    PersistOnRefusal,
    /// Commit the already-applied prefix of a transaction when a later step fails.
    CommitPartialTransaction,
    /// Commit a mutation without writing the caller operation row that names it.
    SkipOperationBinding,
    /// Run a mutation with an already-committed operation id through currentness instead of
    /// answering the exact repeat from its recorded history (command, transaction, record and
    /// create-with-command all share this gate).
    SkipDuplicateGate,
    /// Fingerprint a mutation without its capability, so a refreshed capability reusing the id
    /// is mistaken for an exact repeat (command, transaction and record fingerprints).
    OmitCapabilityFromFingerprint,
    /// Admit an aggregate whose initial state already violates a declared hard law.
    SkipGenesisValidation,
}

thread_local! {
    static ACTIVE: Cell<Option<LawMutant>> = const { Cell::new(None) };
    static FIRED: Cell<u32> = const { Cell::new(0) };
}

/// Run `f` with one law deleted and report how many times its seam was reached.
pub fn observe_law_mutant<R>(mutant: LawMutant, f: impl FnOnce() -> R) -> (R, u32) {
    ACTIVE.with(|cell| cell.set(Some(mutant)));
    FIRED.with(|cell| cell.set(0));
    let result = f();
    ACTIVE.with(|cell| cell.set(None));
    let fired = FIRED.with(Cell::get);
    (result, fired)
}

/// Run `f` with one law deleted. Panics if the deleted wall was never reached:
/// a mutant that did not fire cannot have been killed.
pub fn with_law_mutant<R>(mutant: LawMutant, f: impl FnOnce() -> R) -> R {
    let (result, fired) = observe_law_mutant(mutant, f);
    assert!(
        fired > 0,
        "{mutant:?} never reached its seam: any 'kill' evidence from this run is invalid"
    );
    result
}

/// Is this mutant active on the current thread? Counts as a firing when it is.
pub fn is(mutant: LawMutant) -> bool {
    let active = ACTIVE.with(|cell| cell.get() == Some(mutant));
    if active {
        FIRED.with(|cell| cell.set(cell.get() + 1));
    }
    active
}
