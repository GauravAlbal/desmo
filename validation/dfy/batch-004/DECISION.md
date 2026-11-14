# Batch dfy-004: retest on the repaired candidate and briefs

Candidate: public-source export of source commit `f5deb0a`, built on macOS arm64 with the release script, TypeScript
compiled, npm packages packed, and `qualify_artifacts.py` exited 0 on the packed bytes (`qualify.json`). Digest
prefixes: binary archive `438455ac`, wheel `abad16ec`, source distribution `7eaadc11`, platform package `a0e5cd5c`,
runtime package `dffb873b`, extracted binary `0a0a2794`. This candidate includes the post-batch-003 skill edits
(error mapping by class, node-direct command fallback) and the corpus-less `ik check` next-step hint.

Track: **retest, not blind.** The families N4 (seat holds, TypeScript), N5 (payout desk, TypeScript) and N6 (clinic
appointments, Python) were used in batch-003, and their briefs and diagnose staging were then repaired in response to
that batch (removed the future-change leaks, the undefined N5 field and the contradictory state-file sentence; added the
classification definitions and the `diagnosis.json` output section to N4 and N5). Those repairs came from seeing
batch-003 results, so this batch measures the new candidate on repaired instruments, not a fresh population.

Configuration: GPT-5.6 Luna builder context with a shell, fresh context per run, commands authorized in the task text,
the provider stub exempt from the port rule. The agent definition's standing no-tests instruction was overridden.

| Stage | Result | Failures |
|---|---|---|
| BUILD | 6 of 6 | none (batch-003 N4 r2 failed R12; the error-mapping guidance is in this candidate) |
| CHANGE | 5 of 6 | N5 r2 fails R4: after the change, `reject <id> no` returns `bad_args`, which the build accepted. |
| DIAGNOSE | 18 of 18 | none |

The N5 r2 failure was reproduced by hand against the build and change projects (reject exits 0 before, 1 after); it is
a regression the change introduced in a command it did not need to touch, not an instrument defect. It was not
repaired.

## Existing-instance continuity

Where a change altered the machine, N4 r2 and N5 r2 kept the original definition and routed persisted aggregates to it;
N5 r1 bumped the machine version and wrote that old-store compatibility was untested; N4 r1 left the machine unchanged
and kept partial-release seat membership in a sidecar file outside the lifecycle store, which splits state authority.
The evaluators do not score store continuity or authority placement, so these are observations from the agents' reports
and file listings, not scored results.

## Findings not scored by the evaluators

- None of the 12 builds and changes authored a witness corpus; every `ik check` stayed REFUSED and the agents reported
  it as an evidence gap. The build guidance already says to author cases and the check output now names the commands.
  Filed as a snag.
- Every N5 and N4 diagnose agent used the public provider lookup, and the control cases passed.

## Authority-negative controls (packed binary, observed in this batch)

`authority-controls.json`: `scenario run --store` is refused with exit 2 and no store is created; running into an output
directory that already holds a store is refused with exit 2 and the store bytes are unchanged; a fresh output directory
runs normally.

## Orchestrator interventions

None during the agent runs. Scoring of N5 r1's build began after the agent reported completion by message but before its
final result arrived; the delivered file hashes matched. One build scoring was run on the same tree as the agent's
report.

## Disposition

Not a qualification. Mandatory requirements do not all pass (N5 r2 R4), the families were reused and the briefs were
tuned from the previous batch, there is no human or owner gate, and Linux was run only as a source and packaged-artifact
check in a container (`validation/linux/`), not as a cold-agent batch. Nothing was repaired by hand. Batches 001 to 003
are unchanged.
