# Batch dfy-011: second held-out qualification lineage, frozen before dispatch

Status: frozen. Written and committed before any builder, maintainer or diagnostic agent for this batch ran.

## Why a second lineage

Batch 010 (first lineage) missed its predeclared floor on first scoring and exposed its three families. It also exposed
instrument defects (old instances seeded by a reference program, an unnamed asserted field, an undefined refusal
precedence, a keyword test over free text, a continuity-plan schema the loader did not read) and a guide gap (what to do when
a change asks old instances for something a frozen owner cannot do). Those are repaired here, so this is a new lineage on
new families. Nothing from batch 010 is rescored or reused as held-out evidence.

## Subject

Installed candidate bytes and shipped guidance at the digests in `manifest.json`: public-source export of source commit
`3741e2bb79102c10207db76a07bb5704c8076a5b`, which adds, relative to batch 010: `build.md` step 2 (one aggregate before a split
transaction; no saga or sidecar) and `change.md` step 5b (no migrator; old instances keep original rules; no sidecar).
Three new families, N13 to N15, authored by independent oracle contexts under a revised brief, validated before this freeze,
never exposed to a builder. Two replicas per family. Domain names are withheld until the batch completes.

## Roles and agent profile

- Builders, maintainers, diagnosticians: the general-purpose agent, fresh context per trajectory stage, shell and edit tools,
  no rule against running code, tests, installers or the supplied stub. Model, version and sampling settings are not exposed
  by the runner and are recorded as unknown, except where an error message names it. Prompts are hashed in the manifest.
- Oracle: independent context; no candidate implementation and no builder submission.
- Inspector: candidate-aware, read-only, after the batch; findings are reported and do not change first scores.
- Gate: the repository's mechanical acceptance receipt for any shipped record.

## Isolation

Evaluators, controls, authority check, continuity plans and incident truth are encrypted archives (hashes in the manifest)
and are off the builder-readable filesystem while any agent runs; decrypted only between agent waves, with the key held in
the lead's process memory only. Incident truth for staged diagnostic cases is encrypted separately before those agents run.
This does not stop an agent from reading other trajectories' directories or oracle session records through the runner; the
prompts forbid it. No general sandbox is claimed.

## Pass definitions (first scoring is the record)

- BUILD passes when every R passes AND the runtime-use check passes on the built project.
- CHANGE passes when every R and C passes, the runtime-use check passes on the changed project in old-instance mode (the
  pre-change instances come from that trajectory's own pre-change build), AND the continuity plan passes on pre-change
  instances from the same trajectory.
- DIAGNOSE passes when the family's diagnosis evaluator reports ok for the case.
- Runtime-use check (shared checker, scorer source `scorer.py`): discovers the runtime store, requires history and state
  growth on exercised mutating commands, crash persistence in the runtime store, named status facts present in the runtime
  record, and STORE SUFFICIENCY: the status reproduced from a clone containing only the runtime store files must equal the
  original. Covers exercised paths only; a bypass that also writes the runtime on those paths is not excluded; a sidecar on an
  unexercised path is not detected. Validated before freeze on reference toys, bypass, shadow-state and sidecar controls, and
  on real submissions from earlier batches.
- Continuity plans are less sensitive than the change evaluator to old-instance routing defects in two families; the change
  evaluator's own old-instance tests are the primary check.
- The scorer runs evaluation files under a per-test timeout and a process-group kill; per-requirement verdicts come from the
  family's own summarizer. Scorer, harness and prompt hashes are in the manifest.

## Qualification floor (predeclared, finite)

All of: 6 of 6 BUILD, 6 of 6 CHANGE (with continuity), 18 of 18 DIAGNOSE under FIRST scoring. Any miss means the floor is not
met and the batch is reported as such with per-criterion analysis. Meeting the floor is a finite contract result for this
agent configuration and environment: not a success-rate estimate, not a comparison, not a public-release claim.

## Invalidation and repair rules

- Submissions freeze at the agent's declared completion or at actual exhaustion; every assigned submission is evaluated.
- An agent start that fails with a runner network error before taking any action is redispatched fresh, once, and disclosed.
  It is not a trajectory.
- A diagnostic case that cannot be staged because the trajectory's own build does not run is a failed case for that trajectory.
- An instrument defect found after scoring keeps first scoring on record; any rescoring is reported separately as v2; the floor
  is judged on first scoring; a product failure is never relabeled as a harness defect for convenience.
- A candidate fix, an oracle revision or a leak creates a new lineage.
- No instrument, brief, candidate byte or agent profile changes during the batch.

## Retention

Keep all raw outcomes, submissions (sanitized), receipts, first-scoring results and this manifest. Remove disposable scratch only
after required evidence is retained.

## Not claimed

Human review or cold use, Linux x64, consumer acceptance, publication and any owner decision are separate readiness rows.
