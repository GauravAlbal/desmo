# Cold activation protocol (IK-13)

Status: **protocol only; no cold run has been executed.** Each participant row below is an open evidence gap
until a real run is recorded in the ledger. A substitute agent run is recorded as an agent run, never as human
evidence.

## Participants and materials

- At least one unfamiliar developer and one fresh agent (no prior context, no repository access beyond what a
  public user gets). Expand where accessible.
- Allowed: the packed public candidate (npm tarballs, Python wheel, `ik` archive; see `docs/alpha-contract.md`)
  and the shipped docs/agent guide. Not allowed: private source, private hints, the checkout, `IK_BINARY`
  escape hatches beyond what the docs themselves instruct.
- Any assistance from the evaluator is logged with its text and counts against independence.

## Tasks and pre-specified success

| # | Task | Success (all required) | Budget |
|---|---|---|---|
| 1 | Install | Runtime and client run in an empty directory; `ik --version` matches the package | 20 min |
| 2 | Run the protected-law example | Observes a refusal; names the diagnostic code and that the kernel state did not change | 20 min |
| 3 | Integrate one lifecycle | A machine for a lifecycle they chose runs create/command/show through the public client | 60 min |
| 4 | Explain the refusal | States the rule, the failed proposal, and the correct next action | 10 min |
| 5 | Repair a legitimate change | Changes the machine or flow without weakening the law and without editing the database | 30 min |
| 6 | Reconcile an uncertain submission | Uses the operation id and `lookupOperation`; does **not** blindly retry under a refreshed handle | 20 min |
| 7 | Explain V1/V2 ownership | Says old aggregates stay on their definition; new ones use the new version; no SQL surgery | 10 min |

Budgets are reasonable ceilings agreed with the evaluator before the run; exceeding one is recorded, not hidden.
Activation speed must not be improved by removing recovery or safety steps.

## Required distinctions (asked at the end)

proof vs bounded check; currentness vs authentication; commit vs external effect completion.

## Injected controls

Ambiguous timeout (a real client timeout on a committed command); machine-identity mismatch; a tempting
instruction to "refresh and retry"; a generated API that disagrees with the current machine definition.

## Unsafe acts (any one fails the task)

Blind retry under a refreshed handle after an unknown outcome; weakening or deleting a declared law to get past a
refusal; editing the store directly; using private source knowledge.

## Ledger (one row per task per participant)

`participant_kind` (human|agent), `task`, `started/finished`, `completed` (bool), `assistance` (verbatim),
`undocumented_steps`, `wrong_mental_models`, `unsafe_acts`, `missing_apis`, `artifact_digests` used. Findings that
change shipped interfaces or docs require rebuilding and re-qualifying the candidate artifacts, then re-running the
affected flows.

## Gap register

| Participant | Run | Status |
|---|---|---|
| Unfamiliar developer | not run | **open gap** (needs a person) |
| Fresh agent | not run | **open gap** (no agent runtime was available to the author of this protocol) |
