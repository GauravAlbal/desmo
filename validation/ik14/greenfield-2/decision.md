# IK-14 tranche 2 decision (run 2)

Status: descriptive evidence. No comparative claim, in either direction, follows. Run 1 (`run-1-record.md`) is kept as a confounded record; this file covers run 2, which used the corrected evaluators, briefs and contracts.

## Design
Three new self-contained lifecycles (G4 equipment reservation, G5 inspection schedule, G6 notification preferences and delivery), each with a fake external service whose faults are injected by an evaluator-owned sidecar the builders never see. Two arms: A is an ordinary single-store transactional implementation and was told to include an outbox, a currentness check, idempotent operations, a re-derivable history and crash recovery; B used Desmo for the lifecycle and was told the same baseline. Two replicas per cell, a separate initial build and maintenance change (G4 add a hold phase, G5 add pause/resume, G6 add quiet mode), fresh agents of one model configuration for every build, arm order counterbalanced, and a protected evaluator with a defect control for every requirement (each fails exactly its own requirement) and a hang control. 24 scored submissions.

Before scoring, a model reviewer checked parity, isolation, control validity and concurrency, and a second model reviewer classified every failed requirement with file and line evidence. Neither is human evidence.

## Result (all requirements met)
| Project / replica | Arm A initial | Arm A change | Arm B initial | Arm B change |
|---|---|---|---|---|
| G4 r1 | fail (R3-R7) | pass | pass | pass |
| G4 r2 | pass | fail (app left with a syntax error) | pass | pass |
| G5 r1 | pass | pass | fail (R1) | fail (R1, inherited) |
| G5 r2 | pass | pass | pass | pass |
| G6 r1 | pass | pass | fail (R1-R7) | fail (R1-R6, C1) |
| G6 r2 | pass | pass | pass | pass |
| **Total** | **5 of 6** | **5 of 6** | **4 of 6** | **4 of 6** |

The change request itself (hold, pause, quiet) was satisfied in 5 of 6 arm-A and 5 of 6 arm-B maintenance runs. Human interventions: none. Every attempt is in `results.jsonl`, including two invalid batches (see Procedure).

## Why each failure happened (source review, run 2)
All eight failed cells were classified as builder or maintainer error, none as evaluator, brief or contract defects:
- G4 A r1: a SQL statement binds five values to six placeholders, so the new version is never stored.
- G4 A r2 (change): an unclosed template string left in the application.
- G5 B r1: a duplicate schedule is not refused (the application re-admits identically; the other arm-B replica refused by reading first).
- G6 B r1: the machine definition is rejected by the Desmo binary (a string and an expression where the language requires a literal or `$arg`); the maintenance run then read state from the wire envelope and saw undefined fields.

## Desmo product and documentation gaps this surfaced (arm B)
1. Boolean `then` literals must be written as the string `"true"`/`"false"`, which `docs/machine-language.md` does not state; both wrong spellings fail with messages that do not show the right one.
2. The not-a-literal error for `then` values does not show the required spelling (string literals need inner quotes, expressions must be `$arg`).
3. `createWith` of an identical admission succeeds under a new operation id while `create` refuses an existing aggregate; the TypeScript guide does not say so, and one builder's duplicate-refusal requirement failed on it.
Earlier tranche-1 observations are in `validation/ik14/greenfield/product-observations.md`.

## What this does and does not support
- Supports: on these three small lifecycles and this one model, agents given a competent ordinary-stack baseline and agents given Desmo finished the build and the change in similar numbers (5 and 4 of 6 on each stage). No safety failures, unauthorized effects or wrongly refused legitimate operations were checked beyond what the requirements encode.
- Does not support: any statement that Desmo is faster, cheaper, safer or more reliable than the ordinary baseline, or the reverse. n = 2 per cell, three projects, one model, and the 4-versus-5 difference is within the noise of a single failed cell.
- The arm-B failures came from documentation and error-message gaps, which are fixable, and from ordinary coding errors; none from the runtime's guarantees.

## Procedure, defects and limitations
- Run 1 was invalidated by evaluator, fixture and brief defects found by an independent source review; run 2 corrected them. Two further invalid attempts are recorded: an aborted first batch (an instruction to prepend Node to `PATH` was rejected by the agent shell) and a G5 initial brief that named the future change (builders and runs redone).
- The agent shell rejects `npm`: every agent was told to extract the Desmo runtime tarballs by hand, so arm B carried a small extra step. Agents ran minutes, not the 45 minutes the prompts allow.
- Agents could not exercise fault paths (the public stub has no fault controls); the written contract was their only guide to the fault requirements.
- Builders were told not to read protected directories; that is an instruction, not a sandbox. The evaluators, controls and fixtures for this tranche are now public in this repository, so a later comparison needs new fixtures.
- The reviewers and the evaluators' defect controls are model-written. The Desmo-arm reference implementations are controls, not builder submissions.

## Claim change record
No document claims Desmo outperforms a native implementation; the tranche-1 review found none, and this tranche adds none. Any comparative language still requires a new tranche with human-reviewed baselines. IK-14's acceptance is met as a bounded comparison with a proportional decision; it supports no superiority claim and none is made.
