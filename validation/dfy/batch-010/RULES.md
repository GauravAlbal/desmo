# Batch dfy-010: held-out qualification, frozen before dispatch

Status: frozen. Written and committed before any builder, maintainer or diagnostic agent for this batch ran.

## Subject

Installed candidate bytes and shipped guidance at the digests in `manifest.json`: public-source export of source commit
`73a281360dae0f25664e3a3f950593ad2cc0e668` (macOS arm64 binary archive, Python wheel, npm packages, installed skill tree).
Three families, N10, N11 and N12, authored by independent oracle contexts after batch 009 and never exposed to a builder.
Two replicas per family. Names of the domains are withheld until the batch completes.

## Roles and agent profile

- Builders, maintainers, diagnosticians: the general-purpose agent, fresh context per trajectory stage, shell and edit
  tools, no rule against running code, tests, installers or the supplied stub. Model, version and sampling settings are not
  exposed by the runner and are recorded as unknown. Builder stages receive only the run PROMPT, the brief, the stub, the
  supplied packages and the installed guidance. Maintainer stages get the same project plus the change text and the
  reinstalled guide. Diagnostic stages get the case directory and the built project. Prompts are hashed in the manifest.
- Oracle: independent context, authored the evaluators, controls, incident cases, continuity plans and the runtime-use check;
  it had no candidate implementation and no builder submission.
- Inspector: candidate-aware, read-only; runs after the batch over the delivered sources. Findings are reported; they do not
  change first scores.
- Gate: the repository's mechanical acceptance receipt for any shipped record; no persona approves.

## Isolation

Evaluators, controls, authority check, continuity plans and incident truth are in encrypted archives (hashes in the
manifest) that are off the builder-readable filesystem while any agent runs; they are decrypted only between agent waves
and removed again before the next wave. The key is held in the lead's process memory only. This does not stop an agent
from reading other trajectories' directories or the oracle agents' session records through the runner's own URLs; the
prompts forbid it and compliance is not measured beyond the audit of transcripts. No general sandbox is claimed.

## Pass definitions (first scoring is the record)

- BUILD passes when every requirement R1..R14 (or the family's R-set) passes AND the runtime-use check passes.
- CHANGE passes when every R and every C passes, the runtime-use check passes on the changed project, AND the
  existing-instance continuity plan passes on pre-change instances from that same trajectory.
- DIAGNOSE passes when the family's diagnosis evaluator reports ok for the case.
- Runtime-use check: shared generic checker validated on toys, shadow-state and bypass controls, six real positive and six
  real negative submissions before this freeze. It covers the discovered store, history and state growth on exercised
  mutating commands, crash persistence, and fact agreement. It does not exclude a bypass that also writes the runtime.

## Qualification floor (predeclared, finite)

All of: 6 of 6 BUILD, 6 of 6 CHANGE with continuity, 18 of 18 DIAGNOSE under FIRST scoring. Any miss means the floor is not
met and the batch is reported as such with per-criterion analysis. Meeting the floor is a finite contract result for this
agent configuration and environment. It is not a success-rate estimate, a comparison with any other stack, or a
public-release claim.

## Invalidation and repair rules

- Submissions freeze at the agent's declared completion or at actual exhaustion. Every assigned submission is evaluated.
- A defect in an instrument (brief ambiguity, evaluator error) found after scoring keeps the first scoring on record, is
  disclosed, and any rescoring is reported separately as v2; the qualification floor is judged on first scoring, and a
  product failure is never relabeled as a harness defect for convenience.
- A candidate fix, an oracle revision or a leak of held-out content creates a new lineage; exposed families cannot be
  represented as held out for the repaired product.
- No instrument, brief, candidate byte or agent profile changes during the batch.

## Retention

Keep all raw outcomes, submissions (sanitized of local paths), receipts, first-scoring results and this manifest. Disposable
scratch is removed only after required evidence is retained.

## Not claimed

Human review or cold use, Linux x64, consumer acceptance, publication and any owner decision are separate readiness rows.
