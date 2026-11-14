# IK-14 pilot: blind agent runs on a new project (n = 1 per arm)

**Status: pilot. This is not the IK-14 experiment and supports no comparative claim.** It exercises the
method on a new project (a refund-approval library, not the `publish-approval` fixture) with fresh agents.

## Setup

- Project: `SPEC.md` (11 rules, fixed Python API). Hidden acceptance suites: `test_refunds.py` (build) and
  `test_two_approvers.py` (change). The agents never saw the suites (instructed not to read them; honor system).
- Arms, one fresh agent each, same agent type/configuration:
  - *kernel arm*: given the Ikaros binary, Python client, docs and the `publish-approval` example (from the
    repository checkout at the time of the run, not a release artifact); told to keep rule enforcement in a machine.
  - *plain arm*: standard library only (sqlite3 allowed).
- Neither agent was told about any comparison. They were not blind to which toolchain they were given.
- Change: `CHANGE1.md` (two distinct approvers), applied by a fresh agent per arm to that arm's delivered build.

## Results (raw)

| Measure | Kernel arm | Plain arm |
|---|---|---|
| Build: hidden suite | 11/11 | 11/11 |
| Build: agent wall time | 13m54s | 14m13s |
| Build: `app.py` lines (+ machine files) | 355 (+ 7 machine files, 273 lines, 6 of them scratch) | 247 |
| Change: hidden suite | 3/3 | 3/3 |
| Change: previous suite (6 tests legitimately assume one approval) | 6 failed, 5 passed | 6 failed, 5 passed |
| Change: agent wall time | 5m12s | 4m41s |
| Change: changed `app.py` lines / machine lines | 9 / 20 | 45 / n/a |
| Human interventions | 0 | 0 |

## Observations (not scored)

- The kernel arm's own report disclosed a weakness the suite did not catch: its operation ids are scoped per
  refund rather than global, and a failed second step can leave a recorded payout effect with public state still
  `payout_pending` until a same-digest retry. The plain arm treated operation ids as global.
- Both arms read "expected_version is part of operation identity" the same way (a spec ambiguity).
- The kernel arm left six scratch machine files in its directory.
- In the change, the rule moved into one machine file plus nine host lines for the kernel arm; the plain arm
  rewrote the approval path (45 changed lines).

## Why this is not evidence of a difference

n = 1 per arm; one change; one baseline built by an agent not checked against the contract's competence checklist
(outbox, uniqueness, replay check) and no independent parity review; the hidden suite is my own and was not
validated for sensitivity; wall time is dominated by model latency. Ties on correctness are what was observed.
`docs/maintenance-comparison-contract.md` still governs the real experiment.
Agent deliverables are stored as `*.py.txt` so they are archived, not analyzed or run by the repository gates.
