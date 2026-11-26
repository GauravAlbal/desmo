//! Payload-opaque pure authority state machine shared by `execute_command`,
//! `execute_outcome_binding`, and bounded Kani proofs.
//!
//! Zero `serde_json::Value`, zero `String` formatting, zero `Sha256`, and zero
//! `#[cfg(kani)]` cutpoints: CBMC verifies the exact production decision table
//! exhaustively over arbitrary `u64` generations/sequences and all status/effect
//! combinations.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectStateKind {
    None,
    Pending,
    Recorded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingClass {
    Resolve,
    ExactDuplicate,
    Stale,
    Foreign,
    Conflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandStage {
    DecideRefusal,
    Noop,
    InvalidEvent,
    ApplyRefusal,
    OutcomeStampRefusal,
    Emit {
        plans_effect: bool,
        outcome: Option<BindingClass>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthoritySnapshotView {
    pub generation: u64,
    pub sequence: u64,
    pub nonce_matches: bool,
    pub terminal: bool,
    pub effect_state: EffectStateKind,
    pub has_recorded_receipt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorityPolicy {
    pub accept_stale_capability: bool,
    pub allow_terminal_mutation: bool,
    pub allow_second_pending_effect: bool,
    pub allow_effect_conflict: bool,
    pub mutate_on_duplicate: bool,
    pub skip_generation_increment: bool,
}

impl Default for AuthorityPolicy {
    fn default() -> Self {
        Self {
            accept_stale_capability: false,
            allow_terminal_mutation: false,
            allow_second_pending_effect: false,
            allow_effect_conflict: false,
            mutate_on_duplicate: false,
            skip_generation_increment: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityError {
    CapabilityStale,
    NonceInvalid,
    DomainRefusal,
    TerminalMutation,
    SecondPendingEffect,
    BothEffectAndOutcome,
    EffectStale,
    EffectForeign,
    EffectConflict,
    MissingRecordedReceipt,
    CounterOverflowSequence,
    CounterOverflowGeneration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityStep {
    Noop,
    DuplicateReplay,
    Commit {
        next_sequence: u64,
        next_generation: u64,
    },
}

#[inline]
pub fn check_capability(
    snapshot: AuthoritySnapshotView,
    supplied_generation: u64,
    policy: AuthorityPolicy,
) -> Result<(), AuthorityError> {
    if policy.accept_stale_capability {
        return Ok(());
    }
    if supplied_generation != snapshot.generation {
        return Err(AuthorityError::CapabilityStale);
    }
    if !snapshot.nonce_matches {
        return Err(AuthorityError::NonceInvalid);
    }
    Ok(())
}

#[inline]
fn next_counters(
    snapshot: AuthoritySnapshotView,
    policy: AuthorityPolicy,
) -> Result<(u64, u64), AuthorityError> {
    let next_sequence = snapshot
        .sequence
        .checked_add(1)
        .ok_or(AuthorityError::CounterOverflowSequence)?;
    let next_generation = if policy.skip_generation_increment {
        snapshot.generation
    } else {
        snapshot
            .generation
            .checked_add(1)
            .ok_or(AuthorityError::CounterOverflowGeneration)?
    };
    Ok((next_sequence, next_generation))
}

/// Pure authority gate for `execute_command`.
pub fn authorize_command(
    snapshot: AuthoritySnapshotView,
    supplied_generation: u64,
    requires_preflight: bool,
    stage: CommandStage,
    policy: AuthorityPolicy,
) -> Result<AuthorityStep, AuthorityError> {
    if requires_preflight {
        check_capability(snapshot, supplied_generation, policy)?;
    }
    let (plans_effect, outcome) = match stage {
        CommandStage::DecideRefusal
        | CommandStage::InvalidEvent
        | CommandStage::ApplyRefusal
        | CommandStage::OutcomeStampRefusal => {
            check_capability(snapshot, supplied_generation, policy)?;
            return Err(AuthorityError::DomainRefusal);
        }
        CommandStage::Noop => {
            check_capability(snapshot, supplied_generation, policy)?;
            return Ok(AuthorityStep::Noop);
        }
        CommandStage::Emit {
            plans_effect,
            outcome,
        } => (plans_effect, outcome),
    };

    // G-002 precedence exception: exact duplicate of a recorded effect replays before
    // capability currentness and terminal refusal.
    if snapshot.effect_state == EffectStateKind::Recorded
        && outcome == Some(BindingClass::ExactDuplicate)
        && !policy.mutate_on_duplicate
    {
        return if snapshot.has_recorded_receipt {
            Ok(AuthorityStep::DuplicateReplay)
        } else {
            Err(AuthorityError::MissingRecordedReceipt)
        };
    }

    check_capability(snapshot, supplied_generation, policy)?;

    if snapshot.terminal && !policy.allow_terminal_mutation {
        return Err(AuthorityError::TerminalMutation);
    }

    if plans_effect && outcome.is_some() {
        return Err(AuthorityError::BothEffectAndOutcome);
    }
    if plans_effect {
        if snapshot.effect_state == EffectStateKind::Pending
            && !policy.allow_second_pending_effect
        {
            return Err(AuthorityError::SecondPendingEffect);
        }
        // Effect intent stamping advances generation before commit_plan advances sequence.
        snapshot
            .generation
            .checked_add(1)
            .ok_or(AuthorityError::CounterOverflowGeneration)?;
    }
    if let Some(class) = outcome {
        match class {
            BindingClass::Resolve => {}
            BindingClass::ExactDuplicate => {
                // Only reachable when `policy.mutate_on_duplicate` is set.
            }
            BindingClass::Stale => return Err(AuthorityError::EffectStale),
            BindingClass::Foreign => return Err(AuthorityError::EffectForeign),
            BindingClass::Conflict if policy.allow_effect_conflict => {}
            BindingClass::Conflict => return Err(AuthorityError::EffectConflict),
        }
    }

    let (next_sequence, next_generation) = next_counters(snapshot, policy)?;
    Ok(AuthorityStep::Commit {
        next_sequence,
        next_generation,
    })
}

/// Pure authority gate for `execute_outcome_binding` (kernel `record`).
pub fn authorize_outcome_binding(
    snapshot: AuthoritySnapshotView,
    supplied_generation: u64,
    stamp_foreign: bool,
    binding: BindingClass,
    policy: AuthorityPolicy,
) -> Result<AuthorityStep, AuthorityError> {
    if stamp_foreign {
        return Err(AuthorityError::EffectForeign);
    }
    // G-002 precedence exception: exact recorded duplicate replays before capability currentness.
    if snapshot.effect_state == EffectStateKind::Recorded
        && binding == BindingClass::ExactDuplicate
        && !policy.mutate_on_duplicate
    {
        return if snapshot.has_recorded_receipt {
            Ok(AuthorityStep::DuplicateReplay)
        } else {
            Err(AuthorityError::MissingRecordedReceipt)
        };
    }

    check_capability(snapshot, supplied_generation, policy)?;

    // Terminal states reject ordinary mutation, but an outstanding pending effect may resolve.
    if snapshot.terminal
        && snapshot.effect_state == EffectStateKind::None
        && !policy.allow_terminal_mutation
    {
        return Err(AuthorityError::TerminalMutation);
    }

    match binding {
        BindingClass::Resolve => {}
        BindingClass::ExactDuplicate => {
            if !policy.mutate_on_duplicate {
                return if snapshot.has_recorded_receipt {
                    Ok(AuthorityStep::DuplicateReplay)
                } else {
                    Err(AuthorityError::MissingRecordedReceipt)
                };
            }
        }
        BindingClass::Stale => return Err(AuthorityError::EffectStale),
        BindingClass::Foreign => return Err(AuthorityError::EffectForeign),
        BindingClass::Conflict if policy.allow_effect_conflict => {}
        BindingClass::Conflict => return Err(AuthorityError::EffectConflict),
    }

    let (next_sequence, next_generation) = next_counters(snapshot, policy)?;
    Ok(AuthorityStep::Commit {
        next_sequence,
        next_generation,
    })
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    fn any_effect_state() -> EffectStateKind {
        match kani::any::<u8>() % 3 {
            0 => EffectStateKind::None,
            1 => EffectStateKind::Pending,
            _ => EffectStateKind::Recorded,
        }
    }

    fn any_binding() -> BindingClass {
        match kani::any::<u8>() % 5 {
            0 => BindingClass::Resolve,
            1 => BindingClass::ExactDuplicate,
            2 => BindingClass::Stale,
            3 => BindingClass::Foreign,
            _ => BindingClass::Conflict,
        }
    }

    fn any_stage() -> CommandStage {
        match kani::any::<u8>() % 6 {
            0 => CommandStage::DecideRefusal,
            1 => CommandStage::Noop,
            2 => CommandStage::InvalidEvent,
            3 => CommandStage::ApplyRefusal,
            4 => CommandStage::OutcomeStampRefusal,
            _ => CommandStage::Emit {
                plans_effect: kani::any(),
                outcome: if kani::any() { Some(any_binding()) } else { None },
            },
        }
    }

    fn any_snapshot() -> AuthoritySnapshotView {
        AuthoritySnapshotView {
            generation: kani::any(),
            sequence: kani::any(),
            nonce_matches: kani::any(),
            terminal: kani::any(),
            effect_state: any_effect_state(),
            has_recorded_receipt: kani::any(),
        }
    }

    /// Full production `authorize_command` contract: invalid capability NEVER commits or noops;
    /// terminal states NEVER commit; second pending effect ALWAYS refuses; counter ceilings NEVER wrap.
    #[kani::proof]
    fn prove_command_authority_end_to_end() {
        let snap = any_snapshot();
        let supplied_gen: u64 = kani::any();
        let preflight: bool = kani::any();
        let stage = any_stage();
        let policy = AuthorityPolicy::default();

        let res = authorize_command(snap, supplied_gen, preflight, stage, policy);
        let cap_valid = supplied_gen == snap.generation && snap.nonce_matches;

        // Law 1: Invalid capability can never Commit or Noop.
        if !cap_valid {
            assert!(!matches!(res, Ok(AuthorityStep::Commit { .. }) | Ok(AuthorityStep::Noop)));
        }

        // Law 2: Terminal aggregate can never Commit on a command.
        if snap.terminal {
            assert!(!matches!(res, Ok(AuthorityStep::Commit { .. })));
        }

        // Law 3: Second pending effect can never Commit.
        if let CommandStage::Emit { plans_effect: true, .. } = stage {
            if snap.effect_state == EffectStateKind::Pending {
                assert!(!matches!(res, Ok(AuthorityStep::Commit { .. })));
            }
        }

        // Law 4: Counter ceiling can never wrap or commit.
        if snap.generation == u64::MAX || snap.sequence == u64::MAX {
            assert!(!matches!(res, Ok(AuthorityStep::Commit { .. })));
        }

        // Law 5: Every Commit strictly advances both sequence and generation by +1.
        if let Ok(AuthorityStep::Commit { next_sequence, next_generation }) = res {
            assert!(cap_valid);
            assert!(!snap.terminal);
            assert_eq!(next_sequence, snap.sequence + 1);
            assert_eq!(next_generation, snap.generation + 1);
        }

        // Law 6: Exact duplicate replay only occurs under Recorded + ExactDuplicate + receipt.
        if matches!(res, Ok(AuthorityStep::DuplicateReplay)) {
            assert_eq!(snap.effect_state, EffectStateKind::Recorded);
            assert!(snap.has_recorded_receipt);
            assert!(matches!(
                stage,
                CommandStage::Emit { outcome: Some(BindingClass::ExactDuplicate), .. }
            ));
        }

        kani::cover!(matches!(res, Ok(AuthorityStep::Commit { .. })), "command commits");
        kani::cover!(matches!(res, Ok(AuthorityStep::DuplicateReplay)), "command replays exact duplicate");
        kani::cover!(matches!(res, Ok(AuthorityStep::Noop)), "command noops");
        kani::cover!(matches!(res, Err(AuthorityError::CapabilityStale)), "command stale gen refusal");
        kani::cover!(matches!(res, Err(AuthorityError::NonceInvalid)), "command bad nonce refusal");
        kani::cover!(matches!(res, Err(AuthorityError::TerminalMutation)), "command terminal refusal");
        kani::cover!(matches!(res, Err(AuthorityError::SecondPendingEffect)), "command second pending refusal");
        kani::cover!(matches!(res, Err(AuthorityError::CounterOverflowGeneration)), "command gen ceiling refusal");
    }

    /// Full production `authorize_outcome_binding` contract: invalid capability NEVER commits;
    /// foreign/stale/conflicting bindings ALWAYS refuse; terminal with no effect refuses; ceilings never wrap.
    #[kani::proof]
    fn prove_outcome_binding_authority_end_to_end() {
        let snap = any_snapshot();
        let supplied_gen: u64 = kani::any();
        let stamp_foreign: bool = kani::any();
        let binding = any_binding();
        let policy = AuthorityPolicy::default();

        let res = authorize_outcome_binding(snap, supplied_gen, stamp_foreign, binding, policy);
        let cap_valid = supplied_gen == snap.generation && snap.nonce_matches;

        // Law 1: Invalid capability can never Commit.
        if !cap_valid {
            assert!(!matches!(res, Ok(AuthorityStep::Commit { .. })));
        }

        // Law 2: Foreign stamp always refuses EffectForeign.
        if stamp_foreign {
            assert_eq!(res, Err(AuthorityError::EffectForeign));
        }

        // Law 3: Non-Resolve binding (Stale/Foreign/Conflict) can never Commit.
        if matches!(binding, BindingClass::Stale | BindingClass::Foreign | BindingClass::Conflict) {
            assert!(!matches!(res, Ok(AuthorityStep::Commit { .. })));
        }

        // Law 4: Terminal aggregate with no pending/recorded effect can never Commit.
        if snap.terminal && snap.effect_state == EffectStateKind::None {
            assert!(!matches!(res, Ok(AuthorityStep::Commit { .. })));
        }

        // Law 5: Counter ceiling can never wrap or commit.
        if snap.generation == u64::MAX || snap.sequence == u64::MAX {
            assert!(!matches!(res, Ok(AuthorityStep::Commit { .. })));
        }

        // Law 6: Every Commit requires Resolve, valid capability, and advances counters by +1.
        if let Ok(AuthorityStep::Commit { next_sequence, next_generation }) = res {
            assert!(cap_valid);
            assert!(!stamp_foreign);
            assert_eq!(binding, BindingClass::Resolve);
            assert_eq!(next_sequence, snap.sequence + 1);
            assert_eq!(next_generation, snap.generation + 1);
        }

        kani::cover!(matches!(res, Ok(AuthorityStep::Commit { .. })), "outcome commits");
        kani::cover!(matches!(res, Ok(AuthorityStep::DuplicateReplay)), "outcome replays exact duplicate");
        kani::cover!(matches!(res, Err(AuthorityError::CapabilityStale)), "outcome stale gen refusal");
        kani::cover!(matches!(res, Err(AuthorityError::NonceInvalid)), "outcome bad nonce refusal");
        kani::cover!(matches!(res, Err(AuthorityError::EffectConflict)), "outcome conflict refusal");
        kani::cover!(matches!(res, Err(AuthorityError::EffectForeign)), "outcome foreign refusal");
        kani::cover!(matches!(res, Err(AuthorityError::TerminalMutation)), "outcome terminal refusal");
        kani::cover!(matches!(res, Err(AuthorityError::CounterOverflowSequence)), "outcome seq ceiling refusal");
    }
}
