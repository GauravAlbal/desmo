# Independent parity review of tranche 1 (batch-001)

Reviewer: a fresh review agent with no stake in the result (a model reviewer, not a human), given the tranche-1 contract, prompts, stubs, evaluators, results and decision. Date: 2026-11-15. It answered six closed questions with file and line evidence.

| # | Question | Verdict | Evidence summary |
|---|---|---|---|
| 1 | Contract parity between arms | PASS | Same briefs, change requests, fault cases and protections; only the stack supplement differs (`prompts/armA.md`, `prompts/armB.md`), plus the arm-B kit and two port windows on one change-stage prompt. |
| 2 | Baseline quality (the roadmap's negative control: a baseline missing an outbox or currentness check is unfair) | FAIL | `prompts/armA.md` offered outbox tables "as you see fit" and never required an effect-intent record, a currentness check, uniqueness or a replay check. The pre-result competence review required by `docs/maintenance-comparison-contract.md` was not done. The built arm-A sources are not in the checkout, so whether the submitted code had these checks is undetermined. |
| 3 | Protected laws | FAIL on read, PASS on edit asymmetry | Neither arm was given a write path to the scored evaluator, but both arms could read the evaluator control routes in the staged stub source (G1, G2, G3). Symmetric, not a one-arm edit, but a procedural defect under the tranche-1 contract. |
| 4 | Cherry-picking | PASS | All 28 attempts in `results/` are in `results.jsonl`, including the invalid first change attempts, and are named in `decision.md`. |
| 5 | Evaluator validity | FAIL | Four planted defect controls plus an always-refuse toy; no dedicated failing control for most requirements (G1 R1,R2,R3,R5,R9,R10,R12,C2; G2 R1-R6,R9-R13,R15,R17; G3 R1,R2,R4,R6,R9,R11-R13,R15,R16). G3 R6, the arm-B initial failure, has no planted control that fails it. |
| 6 | Proportionality | PASS | No Desmo superiority claim anywhere. One sentence in `decision.md` generalizes ("ordinary-stack agents were at least as reliable at first build") beyond what a 2-replica, 3-project sample supports; it is already hedged in the same paragraph. |

## Consequence

Tranche 1 stays in the record as descriptive evidence and a pilot. It cannot support any comparative statement in either direction: the baseline competence checklist was not required, builders could read evaluator control routes, and most requirements have no defect control. The roadmap negative controls "baseline missing an outbox/currentness check is rejected as unfair" and "cherry-pick only successful runs: experiment invalid" were checked: the first is tripped, the second is not.

Tranche 2 (`validation/ik14/greenfield-2/`) was built to remove exactly these defects: both arms are told the same competence baseline, the evaluator and control sidecar are never staged, every requirement has an isolated defect control, and an always-refuse and an always-accept stub are run through every evaluator. The independent review of tranche 2 is recorded in `validation/ik14/greenfield-2/validation.log`.
