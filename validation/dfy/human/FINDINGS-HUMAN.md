# Human readout findings ledger

Current state is separated from history. Historical findings are kept verbatim as recorded against the packed 0.5.0 binary of batch 017/018; they are not deleted because a later text change addressed the symptom.

### Current open findings (final 0.5.0 text)

None for the 20-minute readout. `gen_readout.py` pastes only the product handoff. `ik handoff` leads with `Ikaros <kind>: <subject>` and a plain-language first block (change: declared delta before exploration/observation); cited codes sit under `Exact evidence`.

## Repaired in the final 0.5.0 text (historical record, kept)

Repaired by the report, handoff and check text changes (see `CHANGELOG.md`); the generated readout in this directory reflects the repaired text.

### Report and laws are not a 20-minute language

`ik report` embeds kernel JSON in `before` / `after` / `attempted`. `ik identity` and `ik laws` print only compact JSON. Those commands remain compact JSON; the readout no longer pastes them. The handoff first block is the domain summary; the evidence index stays in `Exact evidence`.

### Handoff does not state a safe next action

Recovery `next` lines were `inspect status committed_exact` and similar. The recovery first block now prints `Safe next action` from the evidence reading (`do not resend; the earlier result stands`).

### Existing-instance continuity has no human projection

`IK_MACHINE_MISMATCH` and the successful version-1 `recall` existed only as store JSON envelopes. A `prior_machine` scenario now records `present` observations; `ik report` and `ik handoff --kind change` project them as `continuity_observation` findings. Absent that evidence the handoff renders `continuity not demonstrated`.

### Recovery handoff marks a successful lookup `incomplete`

The lost-reply journey cites finding `out/sc-lost#item-lost/found` (`status=committed_exact`). `ik handoff --kind recovery` with the journey's untyped claim prints:

`R1 [incomplete]: lookup of the dropped commit is committed_exact`

Supporting statuses for an untyped claim are `committed`, `refused`, and `holds_in_scope` only. The same evidence with `claim: "reconciled"` prints `exercised`. The journey model does not set that claim, so the packed binary's recovery handoff as this journey invokes it does not tell a reader the lookup established history.

### Lookup report line says the operation did not commit

For that same finding, `ik report` prints `status: committed_exact` and `committed: not committed`, with `next: inspect status committed_exact`. Lookup is not a new commit; the original `tag` finding is `committed`. A reader answering "Did the operation commit?" from the lookup block can conclude no.

### Check results cannot be marked exercised in a handoff

`ik report` on an `ik check` document emits `status: witness_found`. Handoff does not treat that status as support for an untyped claim. BUILD B2 and CHANGE C1 therefore print `incomplete` even when the check rendered `VERDICT ACCEPT` and `witness_found`. Developer accept cannot appear as exercised delivery or change evidence.

### Check change text does not name old instances or permission

`ik check --prior` prints `NEW SEMANTIC DEBT` for a **FOUND** extend witness, `VERDICT ACCEPT`, and `baseline ... prior=witness_found`. It also prints `no change-local adequacy challenges ran (they need --prior; with --prior, none was generated for this change)`. It does not say that stored version-1 loans keep version 1, that `extend` is refused on them, or that a migrator would need reserved permission.

### Original expected-miss list (historical)

Under `SCORING.md`, these questions are likely to miss or go partial for a non-technical reader. Each miss is a readout finding:

- BUILD 1 may partial: scenario/report say `tag` committed to `tagged`; identity/laws JSON and check hashes add noise.
- BUILD 2 may partial: check exclusions are in English; `B2 [incomplete]` fights the `ACCEPT`.
- CHANGE 2: (historical) depended on decoding store JSON (`IK_MACHINE_MISMATCH`, `recall_placed`); the continuity scenario and change handoff now name kept version, working commands, and refused new commands.
- CHANGE 3: (historical) no product sentence about migration; the change handoff now states that moving existing instances needs an explicit migration which is not performed. Reserved permission is still not a product sentence.
- DIAGNOSE 1: handoff `incomplete` plus report `committed: not committed` on the lookup.
- DIAGNOSE 3: no safe-continuation sentence.
