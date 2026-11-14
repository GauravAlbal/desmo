# batch-001 decision (descriptive evidence: 3 projects x 2 arms x 2 replicas)

Status: first tranche with two independent replicas of the preregistered portfolio. Descriptive only; no comparative
claim is supported; publication is not cleared. The pilot (project P0) is excluded.

## Observed
| Project | Replica | Ordinary stack: initial / change | Ikaros arm: initial / change |
|---|---|---|---|
| G1 render supervisor | r1 | pass / pass | pass / pass |
| G1 | r2 | pass / pass | pass / pass |
| G2 config rollout desk | r1 | pass / pass | pass / pass |
| G2 | r2 | pass / pass | pass / pass |
| G3 import worker | r1 | pass / pass | **fail R6** / change passes, R6 still fails (rescue-and-change) |
| G3 | r2 | pass / pass | **fail R2** / pass (rescue-and-change; R2 passes after the change) |

Initial build: ordinary 6/6, Ikaros 4/6. Post-change: all 12 submissions satisfied the change requirements. Complete
trajectory (both stages autonomous, all assigned runs; a rescue does not rewrite the initial result): ordinary 6/6,
Ikaros 4/6 (G3 in both replicas). No unsafe persisted outcomes or unauthorized effect requests and no wrongly refused
legitimate operations were observed in the stated checks. Human interventions: 0.

The two Ikaros-arm initial failures are both on G3 and on different requirements: r1 R6 (a named crash boundary did
not fire on the evaluator's path) and r2 R2 (a local refusal's error shape). They are observed outcomes, not
harness defects. G3 differs from the other projects in having a retry-counted finite schedule and three crash labels.

## Procedural defect and handling
In replica 1, the first change-stage dispatch used a bare task string and two maintainers (G1-A, G3-A) behaved as code
reviewers and changed nothing. Those attempts are invalid; the maintainer prompt was made explicit and G1 and G3 were
rerun in both arms. All first attempts are kept in results.jsonl. Replica 2 used the corrected prompt throughout.

## Product observations (from Ikaros-arm builder reports, unscored)
Effect-record payload (EffectOutcomeV1) not documented in the TypeScript guide (reported in 3 of 6 Ikaros-arm
builds and the pilot); `ik check` refuses a machine without a corpus; facts cannot be deleted so "forget approval" needs
a sentinel; `ik create` cannot set facts so acceptance is a second command; editing a machine invalidates stores
created under the old definition (expected, no migration in these fixtures); a missing store file is reported as
corruption.

## Limitations
n = 2 replicas per cell; one runtime-resolved model, settings unknown; all runs dispatched in parallel (arm order not
counterbalanced); builders could read the stub source, which contains evaluator control routes; evaluator validates
only stated requirements and was not shown to be sensitive beyond its four defect controls; no unblinded source review
yet; token/cost data unavailable; wall times are agent wall time. A 4/6 vs 6/6 difference on this sample, with both
failures concentrated in one project, is not a statistically supported difference, and it does not support a
superiority claim for Ikaros. It narrows the claim: on this fixture family, ordinary-stack agents were at least as
reliable at first build, and Ikaros-arm change work (maintenance) succeeded in every case.

## Next
Unblinded source review of the G3 Ikaros-arm failures; add documentation for the effect-record payload and corpus-less
`ik check`; replicate again on new project briefs before drawing any design conclusion.
