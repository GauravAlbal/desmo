# IK-14 blind greenfield agent evaluation: contract

Status: prepared; no comparative results exist. Supersedes the prepared-example question as the primary IK-14
experiment; the earlier pilot (`validation/ik14/pilot/`) and the prepared-fixture comparison are engineering
diagnostics and are never pooled with these results.

## Question
Given the same user-facing requirements and resource budget, does a fresh coding agent produce a useful, correctly
recovering lifecycle application more reliably, or with less intervention, when it uses Ikaros than with ordinary tools?
Conditional on the assigned toolchain and workload. Not measured: whether developers choose Ikaros, market size,
universal correctness, human usability.

## Supported topology (deliberate inclusion criterion)
One machine, one persistent local store, one active process at a time with restarts, a possible unrelated second
writer process, external effects through a supplied fake service. Results do not generalize beyond this.

## Roles and blindness
- Brief/evaluator authors: fresh contexts; given the workload envelope and evaluation purpose only.
- Builders: one project, one arm, one fresh directory/context; ordinary documentation of the assigned toolchain; the
  same fake service and budget. Not given: the thesis, the opposite arm, prior scores, evaluator schedules,
  private guidance. Builders know their toolchain: this is NOT double-blind.
- Assessors: behavior results first; normalized A/B presentation. Source review is labeled unblinded.
- A single context playing several roles is not independent execution.

## Arms
- A (ordinary stack): blank Node/TypeScript project; any reasonable local libraries, SQLite transactions, outbox, etc.
- B (Ikaros): same blank project, requirements, host language, stub and budget, plus the installed frozen candidate
  and the shipped guide; told to use Ikaros for the critical lifecycle; ordinary libraries allowed elsewhere.
  The kernel package is immutable; machine definitions and application code are implementation artifacts.

## Frozen inputs
See `batch-001/MANIFEST.md` for candidate tarball digests. No silent rebuild between arms; a product change opens a new
batch and previous outcomes are retained.

## Protections
Briefs, change requests, stub, evaluator are read-only to builders and maintainers for both arms. Neither arm may weaken
them or edit dependency/runtime code. Builders must not read the evaluator or other runs. Machine/schema edits are
legitimate implementation work in both arms.

## Sequence
1. Briefs/evaluators/controls authored by independent agents; frozen. 2. Evaluator validated on defect controls
(late overwrite, duplicate effect, lost-reply misattribution, always-refuse) plus a passing reference toy.
3. Pilot pair (excluded from comparison). 4. Tranche 1: 3 projects x 2 arms = 6 builders, same model and settings,
counterbalanced order, isolated paths. 5. Freeze each submission at completion/budget; evaluate all. 6. Fresh
maintainer per submission applies the follow-on change (rescue-and-change if the initial build failed; not assessable
if no usable submission). 7. Replicate the whole portfolio only within approved budget. 8. Internal register only.

## Outcomes (separate, never combined)
Initial build pass; post-change pass (maintenance / rescue-and-change / not assessable); complete trajectory (both
stages, all assigned runs in the denominator); unsafe persisted outcomes or unauthorized effect requests (counts);
legitimate operations wrongly refused; uncertain operations reconciled correctly. Secondary costs: elapsed time,
tokens/cost where available, tool calls, setup failures, repair attempts, interventions, files touched. Unknown stays
unknown. Lines of code descriptive only.

## Validity
Run invalid only for predefined procedural defects: unequal access or budgets, leaked answers, broken common fixtures,
a builder reading protected material. Poor baseline construction is an observed outcome. Failures are never filtered.
All attempts are reported, including failures and invalid runs with reasons.

## Decision rule
Proportional. One tranche is descriptive evidence only. A tie or loss narrows the claim for this fixture family and
prompts design review; it does not by itself retire the runtime. Public claims stay unchanged; publication needs owner
clearance.
