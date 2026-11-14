# Batch dfy-003: fresh families, blind track

Candidate: public-source export of source commit `e2a3343`, packed to host-target artifacts with the release script;
`pack_npm.py` and `qualify_artifacts.py` exited 0 on the packed bytes. Digest prefixes: binary archive `a716eb93`,
wheel `ed48cf26`, runtime tarball `1542cea2`, platform tarball `3d16cae1`. Two skill edits made afterwards
(`build.md` error-mapping bullet, `SKILL.md` node-direct fallback) are **not** in these bytes, so this batch does not
qualify them.

Track: **blind.** Families N4 (conference seat holds, TypeScript), N5 (payout desk with a clock, TypeScript) and N6
(clinic appointments, Python) were authored after batch-002 and were not used to shape the product. Their
evaluators and controls were validated by the orchestrator before use and corrected during the batch (below), so
this is not a zero-fix instrument.

Configuration: GPT-5.6 Luna builder context with a shell, fresh context per run. The stub is stated exempt from the
port rule in the prompt. The agent definition's standing no-tests instruction was overridden in the task text.

| Stage | Result | Failures |
|---|---|---|
| BUILD | 5 of 6 | N4 r2 fails R12: an unknown hold surfaced the raw store-not-found error as the application error. |
| CHANGE | 5 of 6 | N4 r2 fails R12, inherited from its build. |
| DIAGNOSE | 16 of 18 | N5 r1 and N5 r2 control case: both agents classified `not_sent`; the ground truth is `no_incident`. |

## Instrument defects found and fixed during the batch

Every one is a defect in the evaluation, not the builders. First-pass scores are kept beside the rescored ones in
`results/` (`*-instrument-v1.json`, `*-pass1`).

- N4 id summary used a prefix match; N6 evaluator passed an argument to `status` that its brief omits (build, then
  again in the diagnose evaluator); N6 build scores were re-run after the fix.
- N4 change requirement C4 asserted an outcome for releasing the last seat that the change text states two ways.
  The assertion was removed; the reference programs now agree. Both independent builds had failed C4 identically,
  which is how it was found. Reference toys now pass every change requirement; defect controls still fail what they claim,
  except C1 and C4 for N4 (no control fails them).
- Diagnose briefs: N4 omitted the hold id and its ids named the answer; N5 stated the provider effect count and its
  control stated that the refusal is not an incident; N6 bundled both variants. Staging was corrected and affected
  cases restaged.
- N4 and N5 diagnose briefs did not state the `diagnosis.json` fields or the classification vocabulary, so all 12
  first-pass diagnoses failed on format. The output section was added, and the 16 affected cases were rerun with
  fresh agents. **DIAGNOSE scores above are the rerun.** The first pass is kept as failed.
- The evaluation script signalled its own process group when a ground file lacked a stub pid; guarded.

## Orchestrator interventions

Two N4 agents asked for identifiers the report omitted; the six N4 runs were cancelled and rerun on corrected
staging. No agent output was edited. The N5 control case's neutral wording ("whether a payment was sent") may have
invited `not_sent`; it is scored as a failure regardless.

## Authority-negative controls (packed binary, observed)

- `scenario run --store` is refused with exit 2; no store is created.
- `--out` into a directory holding a store is refused with exit 2; store bytes are unchanged.

## Disposition

Not a qualification. Mandatory requirements do not all pass (N4 r2 R12; N5 control diagnosis), instrument
defects were fixed after first scoring, the candidate predates two skill edits, and no Linux or human-owner gate was
run. The N5 `reconcile` command refused `payment_mismatch` on an exact saved id in one first-pass run while
`apply` succeeded; that is a builder finding recorded, not investigated. Nothing was repaired by hand. Batches
dfy-001 and dfy-002 are unchanged.
