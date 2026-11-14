# Maintenance-cost comparison contract (IK-14)

> **Amendment.** The primary IK-14 experiment is now the blind greenfield agent evaluation in
> `validation/ik14/greenfield/` (contract, briefs, evaluators, defect controls, raw results). This document's
> prepared-fixture comparison is retained as a separate engineering diagnostic and is not pooled with it.
> First tranche (3 projects x 2 arms, 2 replicas): ordinary stack 6/6 complete trajectories, Desmo arm 4/6; descriptive only,
> no comparative claim. See `validation/ik14/greenfield/decision.md`.

Status: **contract; the experiment has not been run.** A one-run-per-arm pilot on a new project exists in `validation/ik14/pilot/` and is not this experiment. No comparative superiority claim is made or
supported. This is not an alpha safety gate; it is required only before claiming measured savings.

## Question

For one bounded, self-contained lifecycle, does a competent native implementation (single-store transactional
helper) or a Desmo machine cost less to *change* while keeping the lifecycle rules enforced?

## Fixed inputs (identical for both arms)

- **Lifecycle:** exact-artifact approval and publication, i.e. the rules in `examples/publish-approval/`
  (approval binds the exact artifact digest; post-approval edit returns to draft; publication authorization
  requires the approved digest; one pending effect; duplicate outcome is a no-op; conflicting outcome refuses).
- **Fault cases:** stale caller, lost reply followed by another writer, executor death after the external
  write, process death and reopen, duplicate and conflicting outcomes, and an approved-then-edited artifact.
  The executable list is `examples/publish-approval/scenarios.py`.
- **Protections:** the requirements, fault-case tests and law statements are read-only to both arms; an arm that
  can edit the protected laws while the other cannot invalidates the experiment.

## Arms

- **Desmo arm:** the machine definition plus the host code in the example.
- **Baseline arm:** a competent single-store helper. It MUST have, before comparison: an atomic transaction per
  command, a per-aggregate currentness check, an idempotent operation record, an outbox/pending-effect table with
  uniqueness, and an audit/replay check. A baseline missing the outbox or the currentness check is rejected as
  unfair. Contract parity and baseline quality are independently reviewed **before** any result is read.

## Change set (applied identically, one change at a time, to a fresh agent per change)

1. Add a `rejected` state with a reason.
2. Require two distinct approvers.
3. Add expiry of an approval.
4. Bind approval to `(digest, version)` instead of digest alone.
5. Add withdrawal of a pending publication before the executor runs.

Building the initial baseline is measured separately from changing it.

## Measures (raw, per run; lines of code only as a secondary description)

Human interventions; violated rules found by the fixed fault-case suite; legitimate work blocked by a false
refusal; diagnosis/recovery difficulty (steps to a correct fix); number of independent semantic changes needed
per requested change. Proof strength and runtime enforcement are reported separately, never folded into one score.

## Validity rules

All attempted runs and failures are reported; no cherry-picking. An experiment is invalid if one arm can edit
the protected laws, if the baseline fails the competence checklist, if agents differ in configuration, or if any
result is selected after the fact.

## Decision rule

A tie or loss on this fixture removes the superiority claim for this fixture and prompts segment/design review;
it does not by itself retire the runtime. An existing-database extraction comparison is a separate topology
experiment after external interest exists.

## Gap register

| Item | Status |
|---|---|
| Baseline implementation and parity review | not started (**open gap**) |
| Fresh-agent arms and measurements | pilot only (n=1 per arm, new project, `validation/ik14/pilot/`); contract-conformant run still an **open gap** |
| Public claim update | no comparative claim exists; none to retract |
