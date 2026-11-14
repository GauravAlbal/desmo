# Batch dfy-011: second held-out lineage, first scoring

Result: **the predeclared qualification floor was met on first scoring.** Nothing was rescored, no submission was edited, and no instrument, candidate byte or agent profile changed during the batch (`RULES.md` and `manifest.json` were accepted and pushed before the first agent ran).

| Stage | Pass | Floor |
|---|---|---|
| BUILD (all R, runtime-use check with store sufficiency) | 6 of 6 | 6 of 6 |
| CHANGE (all R and C, runtime-use in old-instance mode, continuity) | 6 of 6 | 6 of 6 |
| DIAGNOSE | 18 of 18 | 18 of 18 |
| Continuity alone | 6 of 6 | n/a |

Subject: installed candidate bytes from source `3741e2b` (`manifest.json`), including `build.md` step 2 and `change.md` step 5b. Families N13 interlibrary loan desk (TypeScript), N14 pharmacy refill desk (Python), N15 property offer and escrow desk (TypeScript), authored by independent oracle contexts under the revised brief, validated before freeze, never shown to a builder. Two replicas per family, fresh agents per stage, general-purpose agent with a shell. The runner exposed a model identity only on a failed start in an earlier batch (`cursor/grok-4.6`); the other runs did not expose it.

## What had to be true for this to mean something, and what was measured

- **Evaluator isolation during agent waves**: evaluators, controls, authority check, continuity plans and incident truth were encrypted and off disk whenever an agent ran; an instrument self-test (reference positives pass, bypass, shadow-state and sidecar controls detected) ran in every decryption window before scoring and passed each time. This establishes absence from the builder-readable filesystem, not a sandbox.
- **Instrument validation before freeze**: each family's control battery was rerun by the lead (reference toy passes, changed toy passes, the unchanged toy fails exactly C1 to C5, every C has a control that fails it, 13 to 17 defect controls per family); the runtime-use check was validated per family on an Ikaros-backed positive control and on bypass, shadow-state and sidecar-authority controls, in both happy and old-instance modes; continuity plans pass with reference toys through the real harness.
- **Repairs from batch 010 were built in, not patched after the fact**: old instances come from each trajectory's own pre-change build, no keyword test over free text, asserted fields are named in the briefs, one continuity schema, change contracts implementable under the supported evolution procedure, store-sufficiency in the runtime-use check, failure-safe test teardown.
- **Dispatch accidents**: none in this batch. No agent start failed.

## Independent inspection of the delivered sources (A2), after scoring

Reports in `audit/`. They do not change first scoring. Findings that matter:

- No delivery keeps a JSON sidecar as lifecycle authority, and no old instance was handled by a sidecar (batch 010's dominant defect). N14's inspector confirmed the stated old-instance policy (`legacy_rx`, uncapped version-1 hold) is honored in both change deliveries; N15's confirmed original-instance behavior; N13's change deliveries attempt it at the CLI.
- Every delivery computes amounts, deadlines or spans in the host and passes them to the runtime as command arguments, because the guard and assignment language cannot add integers or assign string literals. Agents reported this in every family (`then` assignments accept only `$arg` or numeric-string literals; no `!exists`). The host therefore remains the authority for those derived values. This is a product expressiveness limit, not an agent error, and it is not caught by the store-sufficiency check.
- Seams the behavioral evaluators and the runtime-use check did not catch: N13's version-2 `checkin` is not guarded in the machine, so the host's `recall_required` gate is bypassable through the command interface (reproduced), and one replica's `file` path calls an undefined method (reproduced); N15's `promote` on one replica leaves deposit facts that make a later `reconcile` return `corrupt_state`, and both replicas compute the revised deadline in the host only; N14's reconcile in one replica requires a live provider even after the claim is terminal.
- No delivery uses `lookup_operation`.

## What this establishes and what it does not

Established: with a frozen instrument set, installed bytes and shipped guidance, a general-purpose agent with a shell and no other help delivered runtime-backed applications for three families it had never seen (6 of 6 builds), changed them with correct old-instance behavior (6 of 6 changes, 6 of 6 continuity checks), and diagnosed lost-reply and no-incident cases safely (18 of 18), on first scoring, with evaluator files absent from disk during every wave.

Not established, and not claimed:

- A rate. Two replicas per family is a finite floor, not an estimate.
- Correctness beyond the exercised paths: the inspectors found seams above that the instruments cannot see.
- That the first lineage's failure was an instrument problem alone. It was not: it also contained a builder miss and an unusable delivered file, and exposed a guide gap that is now fixed. Lineage 2 is new families, so the comparison is directional.
- Anything about other agent models, other hosts, or Linux x64.
- Authority controls beyond those in `authority-controls.json` and batch 010's gate controls; two of those failed against the gate CLI (see `../batch-010/DECISION.md`).

## Disposition

The journey qualification floor is met for the declared environment and agent configuration. Whether that becomes `DFYJourneyQualified` also depends on the audit criteria; see `../audits.md` round 4 for the per-criterion dispositions and the one open authority leaf.
