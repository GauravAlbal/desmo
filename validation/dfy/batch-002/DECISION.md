# Batch dfy-002: repackaged candidate, regression track

Candidate: public-source export of source commit `58dde26`, stand-in export commit `e8994778`, built to host-target
artifacts with the shipped release script; `qualify_artifacts.py` exited 0 on the packed bytes (`qualify-*.json`).
Packed digests: runtime tarball `20caff11...`, platform tarball `af899e34...`, wheel `555d28a3...`, binary archive
`afc87293...`. It includes the diagnose-guide patch and the handoff and specialist-routing text.

Track: **regression, not blind.** The three project families and their evaluators are the ones used in batch-001,
so they are trained-on fixtures for this product. Fresh blind evidence needs new families.

Configuration: every stage used a GPT-5.6 Luna builder context with a shell, one fresh context per run. The agent
definition carries a standing instruction not to run tests. Most runs ran code anyway, but the first N3 r1 build did
not, and delivered a project that needed an unbuilt virtual environment, so the configuration was not the same in
every run.

| Stage | Result | Failures |
|---|---|---|
| BUILD, first attempts | 4 of 6 | N2 r2 fails R14. N3 r1 attempt 1 fails all 14 (no virtual environment; the agent did not run the project). |
| BUILD, N3 r1 attempt 2 | passes 14/14 | The re-run is recorded as a second attempt; attempt 1 stays scored as a failure. |
| CHANGE | 4 of 6 | N2 r2 fails R14, inherited from its build (a rescue-and-change on a build that already failed). N3 r2 fails C4. |
| DIAGNOSE | 17 of 18 | N3 r1 recovery case: the agent performed `reconcile` correctly, then wrote the recommended action `stop`; the evaluator's recommended-action check failed. |

Orchestrator messages: three of the 18 diagnose runs (all N3) asked for the command-line argument shape and received
five messages in total (the status form twice and a NOW value for a read-only status, the reconcile form once, the
status form once more). These are not zero-intervention runs.

Cause analysis was not done for the N2 r2 R14 failure or the N3 r2 C4 failure.

## Disposition
Not a qualification: mandatory requirements do not all pass (N2 r2 build and change, N3 r2 change), the families
are not fresh, and the stage configuration was not the same in every run. Nothing was repaired by hand. The
batch-001 record is unchanged. No authority-negative control was run.
