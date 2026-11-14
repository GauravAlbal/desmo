# X09 usability and conformance tasks (installed packages, fresh contexts)

Seven task kinds were defined for qualification. This record covers four of them as single attempts, plus the
adapter-extension exercise recorded in `validation/x08/`. Each run used a fresh GPT-5.6 Luna builder context with a
shell, the installed 0.5.0 packages, the installed skill, and the shipped docs, adapters and examples. No hidden
evaluator or private strategy text was supplied. All attempts are in `x09-attempts.tar.gz` (prompts, reports, the
stale-handle scenario bundle, the new machine and its evidence). Paths are replaced.

| Task | Outcome checked by the orchestrator after the run |
|---|---|
| 3: stale command and lost reply, observation vs commit | `ik scenario reproduce` on the delivered bundle printed `reproduced`. The report's rows match the bundle: stale edit refused `IK_CAPABILITY_STALE`, state unchanged, lost reply reconciled `committed_exact`. |
| 4: backend for a named question | Dead-end question on the exported complete finite graph: `holds_in_scope`, exhaustive finite model, scoped to the corpus-restricted graph. Liveness question: `unsupported`, no verdict. Both stated by the report. The agent hit two integration frictions (a scratch-root refusal, and the TLC jar must be placed where the adapter expects it) and worked around them. |
| 5: no backend | The report states the dead-end check ran only natively over the corpus, that native obligations stayed `not_examined`, and that no external checker ran. |
| 6: add an adapter from the starter | See `validation/x08/RESULT.md`. |
| 7: evolve with old evidence present | The agent produced `archive` as version 4 with a new corpus and fresh evidence. `ik evidence verify` of the version 3 evidence against the new machine reports stale machine and corpus; the version 4 evidence verifies `current`. |
| 1, 2 | Not run in this form. The build and change stages of batch dfy-001 (`validation/dfy/`) cover authoring a lifecycle from a neutral brief and applying a feature change; they do not include detecting an unintended blocked normal path. |

Limits: one attempt per task, one model family, the agent's own reports were checked only on the points named above.
These are usability and conformance results. They are not human review, not a comparison, and not part of the
maintenance-comparison experiment.
