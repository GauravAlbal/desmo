# Agent-delivered lifecycle engineering: source reconciliation, reuse map and causal audit (D0)

Status: engineering record for an additive track. It states what exists, what is reused, what is deliberately not
built, and what the earlier fresh-agent experiment does and does not show. It certifies nothing.

## 1. Source state at the time of writing

Reconciled against main after the lifecycle-workbench packages landed: check profiles and useful-path goals
(`ik check --profile`, `--plan`), the scenario driver (`ik scenario run|reproduce`), the pinned external-adapter
runner (`ik analysis describe|run|validate-result`), a Hegel executable adapter (example binary, sampled evidence),
a TLC adapter over an exported finite graph (`ik graph export`, real TLC 2.19), a unified report and regression
promotion (`ik report`, `ik regression`), change-aware evidence (`ik profile diff`, `ik evidence verify`,
`ik check --profile --prior`), and `desmo doctor` plus a profile/scenario scaffold. Not yet landed at that point: the
third-party adapter starter, a packaged-journey qualification, the agent skill/router, typed effect helpers in the
TypeScript client, a distinct "store not found" diagnostic, validated admission through the clients, and
documented invalidation patterns.

## 2. Reuse map

| Need | Existing facility reused | Gap closed by new work |
|---|---|---|
| Discover runtime, machine, laws, schema | `ik --version`, `ik identity|laws|schema`, `desmo doctor`, `desmo codegen` | one agent-facing skill that names them |
| Check usefulness | `ik check --profile`, explorer, witnesses | starter corpus from declared arg schemas; next-action text |
| Exercise adverse orderings | `ik scenario run|reproduce` | each requested fault reports whether its seam fired; an unfired fault fails |
| Explain and reproduce | `ik report`, trace v2, `CounterexampleV1` | none |
| Operation attribution and recovery | operation ids, `lookup_operation`, historical results | error classification vocabulary for domain adapters |
| Effects | recorded intents, `record` (Python typed helper) | TypeScript typed helper; validation before dispatch |
| Admission | `create_aggregate_with_command` in the store | exposure through the clients; "bare create then initialize is not atomic" stated |
| Invalidation | declared facts and laws | one tested pattern without sentinels or fact deletion |
| Change and versions | `--prior`, `profile diff`, `evidence verify`, machine identity | recipes only |
| Specialist checkers | adapter runner, Hegel and TLC adapters | none; they stay optional and subordinate |

Deliberately not built: a hosted agent, a scheduler, a general planner, a console or canvas, a second transition
interpreter, a universal checker IR, an optional-fact deletion primitive, an unchecked initial-facts setter, a
generic migrator, automatic retries or authority refresh.

## 3. Authority map

The runtime owns transition legality, currentness, declared laws, atomic durable state/history/receipt, effect-intent
bookkeeping, operation attribution and replay. The host application owns behavior outside that boundary,
authentication, scheduling, external I/O and executor cooperation. Analysis never runs against a production store
and never authorizes a mutation. Acceptance of changes is by the repository's mechanical gate; agents submit, they do
not approve their own work or override the gate.

Reserved for the owner and not decided here: irreducible behavior choices, licensing and disclosure, branding and
publication, production pin movement or destructive migration, new spend or data-sharing permission, and any human
evidence that only a human can supply.

## 4. Causal audit of the two blind-build failures on the import-worker project

The earlier descriptive experiment (6/6 versus 4/6 complete trajectories) is unchanged. Its two Desmo-arm initial
failures were audited causally from the submitted source and the evaluator, read-only:

* Replica 1, requirement R6 (a named crash label must kill the process on `advance` before any destination call).
  The submission fired the label only inside `submit`; `advance` never invoked it. Cause: builder logic. The
  shipped guidance has no crash-label API, so no product text steered the placement. The brief names the boundary
  by label and by effect ("before the durable dispatch mark and before any send") but not by command, which is a
  minor ambiguity that did not by itself cause the miss; the evaluator was correct. The ordinary-stack builders put
  the label on `advance` while the record was ready.
* Replica 2, requirement R2 (missing record argument must be refused as `invalid_input` with field `record`).
  The submission's argument-count check refused `submit imp1` with field `import_id`, so the record branch was
  unreachable for that input. Cause: builder CLI arity logic before any kernel call. No kernel or client error shape
  was involved.

Both are scored failures and stay scored. Neither is attributable to a product defect on the evidence read; the
audit did not execute the binaries, so the stdout of replica 2 is inferred from source. These findings do not show
the product is easier or harder to build with; they show where a typed, shipped integration path (this track) could
remove avoidable work, for example by making the reached-fault evidence and the error-shape mapping part of what an
agent receives.

## 5. Frozen contract for qualification (preparation, not execution)

Cold qualification uses three new project families: two TypeScript and one Python, each with two replicas, a build
stage, a change stage and a diagnose/recover stage, fresh context per stage, under existing explicit resource
authorization only. Briefs and acceptance cases are authored by contexts that have not seen the candidate
implementation, and the evaluators are read-only to builders. The earlier greenfield results are never rewritten or
pooled with these. Applicability of each criterion is declared before candidate outcomes are known; a failed or
unavailable required claim cannot be relabeled not applicable.

## 6. Honest gaps in this record

* The repository's grounding pass for plan-shaped work (an internal multi-reviewer process) was not run for this
  track; registering the work with it would change the shipping mode for these changes. Substituted: an independent
  read-only causal audit and a candidate-blind oracle plan. This substitution is not equivalent and is reported as a
  gap for the owner.
* Nothing here is evidence that any journey is qualified from installed artifacts.
