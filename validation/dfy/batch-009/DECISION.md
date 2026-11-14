# Batch dfy-009: batch 008 rerun with an unrestricted agent (frozen instruments, families reused, not blind)

## Why this batch exists

Batch 008 produced builds that never used the Ikaros runtime (build 3/6, change 2/6). Batch-005 builds had used it. The
difference was suspected to be the agent condition: batch 008's builder definition carries a "do not run tests" rule and
the agents ran about two minutes each. This batch changes only the agent: the general-purpose agent (no such rule, about
6 to 11 minutes per build), same candidate, same frozen instruments (`../batch-008/instrument-manifest.json`, unchanged),
same briefs, change texts, cases, and harness. Nothing was edited and nothing was rescored. The families were seen in
batch 005, and the agent is a different model from earlier batches, so this is neither blind nor a like-for-like replica
of batch 005. n=2 per family is not a rate.

Declared before scoring and reported beside the unchanged evaluator scores: a static scan for runtime client references
in each project.

## Results

| Stage | Pass |
|---|---|
| Build | 6 of 6 |
| Change | 6 of 6 |
| Diagnose | 18 of 18 |
| Continuity (existing-instance, after the change) | 6 of 6 |

Runtime use: all six builds reference the runtime in application code, ship machines the program loads, and keep
authoritative state in the runtime store (`ikaros.sqlite`, `desk.sqlite`, or equivalent). Every change run added a
second machine version, kept the first for existing instances, and routed each stored aggregate by `Aggregate.owner`
(the accessor added in `a600592`); `ik check <v2> --prior <v1>` accepted in the changes that report it. Existing-instance
continuity therefore passes on runtime-backed stores, which batch 008's passes could not show.

## What this establishes, and what it does not

- The batch-008 runtime-use gap tracked the agent condition (restricted definition, short runs), not the product, and
  the guides plus the owner accessor carry cold agents to correct builds, changes, and diagnoses on three families in two
  languages with instruments frozen in advance. This follows the batch-007 result that `owner` was necessary.
- It does not meet a blind frozen pass: the families are reused, the agent model is not the batch-005 model, and nothing
  was held out. It is not a qualification and not a rate.
- One change run (N9 r1) ended with an open question to the requester (whether `extend-deadline` should reach
  pre-change instances) and chose to refuse on them; the evaluator accepts that choice. It is recorded here as a
  behaviour decision the agent surfaced, not a defect.
- No human or owner gate, no Linux x64 evidence, no new-family blind batch. Those remain open.

## Disposition

D4 (change with behaviour deltas and continuity) has frozen-instrument evidence on a runtime-backed population:
6 of 6 with continuity 6 of 6. D8 stays open for the reasons above. Nothing in any submission was edited after the agent
finished.
