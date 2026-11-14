# Batch dfy-008: frozen instruments, final candidate, families reused (not blind)

## What this is

The three families from batch 005 (N7 warehouse shipments, N8 subscription renewals, N9 e-signature envelopes) rerun on
the final candidate built from public-source export of `736b6eb`, with the evaluators, briefs, change texts, diagnose
cases and harness hashed **before any agent ran** (`instrument-manifest.json`, 99 files plus artifact digests). No
instrument was edited during the batch and nothing was rescored. The authors had seen these families in batch 005, so
this is a frozen-instrument pass, **not** a blind one.

Candidate: binary digest prefix `0a0a2794` (unchanged since batch 004; its authority-negative controls apply), TypeScript
runtime and Python wheel packaged from `736b6eb` and qualified on macOS arm64 (`qualify.json`). Before dispatch, the
control battery on all three families was rerun: reference toys pass, every change requirement C1 to C5 has a control
that fails it. The change stage now reinstalls the candidate's skill into the copied project (a staging defect found in
batch 006); the continuity check from batch 005 is part of the instrument.

Agents: GPT-5.6 Luna builder, fresh context per run, two replicas per family. n=2 per family is not a rate.

## Attempt 1 (invalid condition, kept)

The first dispatch used the batch-005 task wording. The agent definition says "do not run tests, formatters, linters"
and permits commands only for hashing; all six builders obeyed it (runs of 81 to 102 seconds, hash-only output, five
without installing the kit, none running the provider stub), and each reported that it had not wired Ikaros in. That
condition is not the one the batch is meant to measure, so those six builds are kept in `submissions.tar.gz` under
`attempt1-restricted/` **unscored**, and the batch was rerun from clean staging with an explicit override notice in the
task context. Attempt 2 is the scored attempt. (Filed as a tooling snag.)

## Results (attempt 2)

| Stage | Pass | Failures |
|---|---|---|
| BUILD | 3 of 6 | N8 r1 R8, N8 r2 R8, N9 r2 R8 and R10 |
| CHANGE | 2 of 6 | N7 r2 R3 and C1; N8 r1 R8 and C5; N8 r2 R8, C1, C5; N9 r2 R10 (N7 r1, N9 r1 pass) |
| DIAGNOSE | 17 of 18 | N9 r2 recover: classification and action |
| Continuity | 6 of 6 | none |

Failures were read, not assumed to be instrument defects. N8 R8 (both replicas, same requirement): r1's build has no
`after-charge-saved-before-request` crash label at all; r2 returns `charge_id: null` from the renew that adopts the saved
id, though the brief defines the success object with `charge_id`. Both are builder defects against the brief. The
change-stage failures carry the build-stage failures forward (a change applied to a program that already fails R8 or R10
still fails them) plus the new-requirement failures listed.

## Findings that matter more than the counts

1. **No build routes lifecycle decisions through the Ikaros runtime.** Of six builds, five contain no reference to the
   runtime client and one imports a helper it never calls. Every build ships a `.ik.toml` that its program does not read
   and keeps authoritative state in its own JSON or SQLite store. The behavioural evaluators pass three of them anyway,
   because they test commands, not who owns the state. A pass here therefore does not show that Ikaros was used. This
   is **not** the batch-005 pattern: all six batch-005 builds import and use `Kernel` from the runtime (4 to 12
   references each). The likely cause is the condition, not the product: these agents still carried the
   focused-worker rule in their definition, ran about two minutes each, and several reported leaving the integration
   undone for lack of time or tool access. That cause is an inference; it was not isolated. Next lever: make runtime
   authority observable in the evaluator, then rerun builds under an unrestricted agent.
2. **Continuity 6/6 is weak evidence.** The agents kept their own stores and extended them with default-valued fields, so
   old instances stay readable without needing the `owner` accessor that batch 007 showed was necessary when the runtime
   is the store. It confirms nothing about the runtime path.
3. First-pass rates are far below the batch-005 final numbers (6/6, 6/6, 18/18), which were reached after instrument
   repairs. With instruments frozen the figures for this condition are build 3/6, change 2/6, diagnose 17/18; the
   missing runtime use (finding 1) confounds comparison with batch 005.
4. Diagnose is the strongest stage: 17 of 18 with unmodified instruments, including both unattended-recovery modes.

## Disposition

Not a qualification. D8 stays open. Alpha criteria A1 to A3 remain partial and A4 is not met. No human or owner gate,
no x64 evidence, no blind frozen pass on new families. Nothing was repaired by hand; no file in any submission was
edited after the agent finished.
