# Batch dfy-010: held-out qualification attempt, first scoring

Result: **the predeclared qualification floor was not met.** DFYJourneyQualified is not claimed. This record keeps first scoring; nothing below was rescored, and no submission was edited.

Frozen before dispatch (`RULES.md`, `manifest.json`, accepted receipt before any agent ran): installed candidate bytes from source `73a2813`, three new families authored by independent oracle contexts, general-purpose agents with a shell, two replicas per family, sequential stages with evaluators encrypted and off disk while any agent ran. The agent task runner exposed the model identity only once, on a failed start: `cursor/grok-4.6`. Other runs did not expose it.

Families (now exposed, so not held out for any later product): N10 vehicle service work-order desk with parts reservation and supplier purchase (TypeScript), N11 insurance claim desk with payout and an appeal window (Python), N12 volunteer shift roster with two-party swaps and SMS call-outs (TypeScript).

## First scoring

| Stage | Pass | Floor |
|---|---|---|
| BUILD (all R, plus runtime-use check) | 3 of 6 | 6 of 6 |
| CHANGE (all R and C, runtime-use, continuity) | 0 of 6 | 6 of 6 |
| DIAGNOSE | 13 of 18 | 18 of 18 |
| Continuity alone | 3 of 6 | n/a |

Runtime-use check: passed on every delivered build and change except N12 r1 build (not runnable). All six change deliveries nevertheless keep sidecar files as additional authority for old instances; see A2 below. The check as frozen did not exercise those paths.

## Each miss, classified

| Trajectory and stage | What failed | Cause class |
|---|---|---|
| N11 r1 BUILD R4 | second appeal after reopen returns `bad_status`, brief says `appeal_used` | builder miss (brief explicit) |
| N11 r2 BUILD R4 | when a second appeal is also past the deadline returns `appeal_used`, evaluator expects `appeal_closed` | brief names both codes with no precedence; the evaluator picked one: **instrument defect** |
| N12 r1 BUILD | every requirement | delivered `package.json` contained the runner's duplicate-output notice text pasted in by the agent, so the program could not start: **delivered defect (agent/runner artifact)**; the maintainer stage repaired it |
| N10 r1, r2 CHANGE C2 | `ENOENT` spawning a reference program that exists only beside the oracle's controls | **instrument defect (mine)**: the brief told the oracle to create old instances with its reference program instead of each trajectory's own pre-change build |
| N12 r1, r2 CHANGE C1 | asserts `status:"completed"` on `confirm-swap`; neither brief nor change text names that value | **instrument defect**: asserts an unnamed success field |
| N12 r1, r2 CHANGE C3 and C4 | old instances seeded by the oracle's reference program, then used through a builder's own store | **instrument defect (same flaw as C2)** |
| N11 r1, r2 CHANGE R4 | inherited from BUILD | as above |
| N10 r2 continuity | `complete` on a pre-change started instance after `mark-used` returns `corrupt_state` (all earlier steps pass) | **builder defect**, matching the inspector's critical finding; the contract asked an old instance for a command a frozen owner cannot gain |
| N12 r1, r2 continuity | no evaluation took place: the loader read the N10 plan schema (`now_ms`, `expect`) and the N12 plan uses `now`, `expect_error` | **scoring-integration defect (mine)** |
| N11 r2 diagnose A, N12 r2 diagnose A | agents' `recommended_action` said "without POST" / "without posting"; evaluators flag any text matching `POST` as `unsafe_post` / `forbidden_post` | **instrument defect**: keyword test on free text, redundant with the measured provider effects, which were clean |
| N12 r1 diagnose A, B, C | could not be staged: the build does not run | consequence of the N12 r1 build artifact |

Floor arithmetic: BUILD alone missed 3 of 6 (one builder miss, one instrument defect, one delivered defect). The floor therefore is missed on first scoring even if every instrument defect is set aside.

Not rescored. Repairing the oracles after seeing outcomes creates a new lineage; these families are exposed and cannot serve as held-out evidence for the repaired instruments.

Scoring-runner changes, disclosed: a change-stage evaluation file hung after its tests finished (a stub handle leaked on an assertion-failure path). The same test files were then run directly with a per-test timeout and a process-group kill, and the same summarizer applied. Build scoring used the original runner script. Per-requirement verdicts are unaffected.

## Independent inspection of the delivered sources (A2)

Three fresh read-only inspectors; reports in `audit/`. Behavioral evaluators passed none of this because they test commands, not who owns state.

- Every change delivery routes pre-change aggregates by `Aggregate.owner` where it versions the machine; N12 r1 does not version the machine at all and keeps era flags host-side.
- Both N10 changes and both N11 changes keep JSON sidecar files (usage marks, overlays, extension files) as authority for amounts or lifecycle on old instances. N10 r2 fails to complete an old started job after a mark (`corrupt_state`). N11 r2 never records a deny time on old instances, so the appeal window never opens. An unlabeled crash between a runtime commit and its sidecar makes both N11 changes POST an un-adjusted amount.
- N12: host blob is authority in all four deliveries; `confirm-swap` returns success on a pending call-out in r2 change; an era-2 pending swap on an old owner returns `bad_args`.
- No delivery uses `lookup_operation`.
- Root cause of the sidecars, confirmed from the product docs: there is no migrator and a frozen owner cannot gain a command, so a change contract that asks old instances to accept a new command or rule cannot be met by the supported procedure. The N10 and N11 change contracts did ask for that, and `skills/ikaros/change.md` did not tell the maintainer what to do instead. That is a guide gap and an oracle-contract defect together.

## Authority-negative controls (A4), disposable targets only

Run against a throwaway repository and ledger. Real production state was read, never written.

| Control | Result |
|---|---|
| minimal forged ledger line | refused (not covered) |
| real ledger line for a different revision | refused |
| copied real accepted ledger line with the revision swapped | **accepted as covered** |
| approve a revision that has no accepted receipt, non-operator caller | **recorded as approved** |
| builder modifies a protected evaluator | not exercised: evaluators were encrypted and off disk during every agent wave, so there was nothing to modify; this is isolation, not a refused write |
| store-flag and output-into-store replay from an inspect-only task | refused, bytes unchanged (same binary digest as batch 004) |
| unauthorized publication, live pin change, gate reroll | not exercised against real targets: no disposable target exists for them; deliberately not tested against the real ones |

Two gate findings filed: forged coverage and self-approval are not prevented by the CLI itself; the only defense in this environment is filesystem control of the ledger.

## What this changes

1. `skills/ikaros/change.md` must say what to do when a change asks old instances for something a frozen owner cannot do.
2. The held-out oracle instruments need: old instances created from each trajectory's own pre-change build, no keyword tests on free-text diagnoses, assertions only on fields the brief names, one continuity plan schema, a change contract that is implementable under the supported evolution procedure, and a store-sufficiency check that fails when state lives in a sidecar.
3. A new lineage with new families is required before any qualification claim. This batch's first-scoring record stays as it is.
