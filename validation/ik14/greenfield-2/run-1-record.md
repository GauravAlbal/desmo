# Tranche 2, run 1: as scored, and why it cannot be interpreted

Status: confounded by evaluator and document defects found in an independent source review. Kept in full as evidence; superseded by run 2 (`results.jsonl` rows tagged `"run":2`). Nothing here supports a comparative claim in either direction.

## What ran
Six projects-by-replica cells, each with arm A (ordinary stack, competence baseline required) and arm B (Desmo), initial build then a separate maintenance change; 24 scored submissions by fresh agents of one model configuration, arm order counterbalanced, at most six agents at once. Every attempt is in `results.jsonl`, including an aborted first batch (an envelope instruction that the agent shell rejected, so some builders could not run code) and one maintenance attempt that wrote no files because the shell rejected `npm`.

## Scores as evaluated (all requirements, initial and change stage)
| | Initial: all requirements met | Change: original and new requirements met | Change request itself met (C1/C2) |
|---|---|---|---|
| Arm A | 4 of 6 | 3 of 6 | 5 of 6 |
| Arm B | 3 of 6 | 1 of 6 | 1 of 6 |

## Why it cannot be read as builder or product performance
The source review (a model reviewer, not a human; file and line evidence per cell) classified each failed requirement:

- **G4 fixture diverged from the published stub.** The evaluator fixture answered a missing-operation lookup with HTTP 404, the published stub with 200 `{found:false}`. One arm-B submission followed the stub and the fixture made it fail R3-R8. Evaluator defect.
- **G4 conflict wire shape undocumented.** R8 depends on an HTTP 409 body the public contract never states. Contract gap.
- **G4 change request ambiguous.** "Add a `hold` phase ... advance on a held reservation refuses `held`" supports both phase values `hold` and `held`; the evaluator required `hold`; both arm-B maintainers used `held`, accounting for both arm-B C1 failures on G4. Brief ambiguity.
- **G5 R7 stricter than the brief.** The evaluator required a blocked in-flight command to exit 0 after a cancel; the brief only says cancellation wins. Evaluator strictness.
- **G6 re-subscribe semantics unstated.** R5 re-subscribes after unsubscribe; the brief never says that is allowed. Brief gap.
- **Genuine builder or maintainer errors**, not attributable to Desmo's runtime: G5 C1 in three cells (replayed idempotent pause, stale pre-I/O phase, a public phase `uncertain_paused`), G6-B-r1 initial (unhandled `StoreNotFoundError`, which `docs/typescript.md` documents) and its unimplemented delivery.
- **No arm-B failure was caused by a store lock held across the service call** (the one hypothesis the review specifically tested for G5 B r2: pause returned in under a second).

## Harness limitations recorded
The agent shell rejected `npm`, so every arm-B agent extracted the runtime tarballs by hand (one initial build aborted once and was resumed once); arm A was unaffected. Builders could in principle read the repository; the protection was an instruction, as in tranche 1. Agents wrote tests against a stub with no fault controls, so they reported fault paths as unexercised; they could only rely on the written contract.

## Consequence
The evaluator, the G4 contract, the G4 change request, the G5 brief and the G6 brief were corrected and audited requirement by requirement for run 2 (see `validation.log`). Run 1 results stay in the record unmodified.
