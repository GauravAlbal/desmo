# Batches dfy-005 to dfy-007: fresh families, continuity, and two targeted retests

## dfy-005: fresh families on a rebuilt candidate (blind track)

Candidate: public-source export of source commit `21256fb`, built with the release script on macOS arm64, TypeScript
compiled, npm packages packed, `qualify_artifacts.py` exited 0 (`qualify.json`). Digest prefixes: binary archive
`0328f998`, wheel `15dd4ca4`, source distribution `6efa16c5`, platform package `a0e5cd5c`, runtime package `bcc69b84`,
extracted binary `0a0a2794`. The Rust binary is byte-identical to batch-004's; the authority-negative controls recorded
there apply to these bytes (same digest).

Families (new, authored for this batch by builder-class agents under a contract that listed the defects found in
earlier families, then audited and repaired by the orchestrator): N7 warehouse shipments (TypeScript, a close structural
relative of N4: facility capacity, deadlines, and an order that already exposes a list-shaped `parcel_ids` field),
N8 subscription renewals (TypeScript, clock-driven), N9 e-signature envelopes (Python). Every change requirement C1 to
C5 has a defect control that fails it, and reference toys differ before and after the change.

Configuration: GPT-5.6 Luna builder context with a shell, fresh context per run, commands authorized, provider stub
exempt from the port rule.

| Stage | First scoring | Final, after the repairs below |
|---|---|---|
| BUILD | 4 of 6 | 6 of 6 |
| CHANGE | 0 of 6 | 6 of 6 |
| DIAGNOSE | 14 of 18 | 18 of 18 |
| Existing-instance continuity (added after first scoring) | not run | 4 of 6 |

Every first-scoring failure except one was an evaluator or brief defect, found by noticing that both replicas of a
family failed the same requirement, and confirmed by reading the brief and reproducing the command:

- N8 R4 required a refusal code (`bad_status`) the brief never defines; N8 R5 expected a second `renew` to succeed while
  R3 says a not-yet-due renewal is refused `not_due`. Both builders gave the brief-consistent answer.
- N7 C5 required `not_packed`, defined nowhere; N8 C2 required `bad_plan` for an upgrade the change text does not name.
- N9 change text said `create-envelope` "now has" a new form while R1 to R14 stay in force with the old form; the text was
  amended to keep the six-argument form, and the N9 change stage was rerun with fresh agents. Attempt 1 stays on record.
  One N9 replica then followed the change text's "collision with either signer" literally while the evaluator reused
  signers across envelopes; the evaluator now uses distinct signers per envelope.
- Diagnose: the N8 evaluator re-ran `reconcile` after the agent had already recovered and demanded exit 0, though the brief
  says a second `reconcile` with no saved charge is `no_charge`; N9's control case did not say what `committed` means for
  a refused later operation. Both were fixed and the cases rerun with fresh agents.
- One diagnose case remains unexplained: an N7 investigate agent reported a 404 for a request id the ground truth says the
  provider holds. A direct lookup on a restaged copy returned 200 three times and a fresh agent classified it correctly;
  the first attempt stays recorded as a failure.

First-scoring results are kept beside the final ones in `results/` (`*-instrument-v1`, `*-attempt1-*`, `*-eval1-*`).

### Existing-instance continuity

The evaluators did not test continuity, though change texts say existing instances stay valid. After first scoring I added
a check: create an instance with the pre-change build, then run the changed program on that same store (`status`, then the
next legitimate command), with the provider effect counted. N7 and N9 pass in both replicas (old instances routed to the
retained definition). **N8 fails in both replicas:** `renew` on a pre-change subscription returns `corrupt_state`; in one
replica the provider charge had already been made, leaving the local record `past_due` with the charge not adopted.

## dfy-006: N8 change retest, candidate with the continuity step in the change guide (not blind)

Candidate: the same binary, with a change-guide step that tells the agent to create an instance with the old program and
run the changed program on it. Change requirements: 2 of 2 pass. Continuity: **0 of 2** (the same failure). One agent
declared old stores "outside continuity support" despite the step; the other tested only a hand-made old-format store,
not the old program's own output. Note: the change stage does not reinstall the skill into the project it copies from the
build stage; the new skill was installed by hand after a first dispatch with stale text was cancelled before it did work.

## dfy-007: N8 change retest, candidate with the owner accessor (not blind)

Product change `a600592`: `Aggregate.owner` (TypeScript, Python) and `Aggregate.Owner()` (Go) return the identity that
owns an aggregate, so routing an old instance to its retained definition is a documented call rather than hand-rolled
index files; the guide names it. Change requirements: 2 of 2 pass. Continuity: **2 of 2**. One agent's report says it
routed by recorded identity because `show` through the new definition did not refuse. Two replicas on one family are
evidence that the change helped, not proof of a rate.

## Findings not scored

- One of six builds authored a witness corpus and reached a bounded `ACCEPT` (and kept it through its change); the other
  five stopped at the refusal that names the commands to run.

## Disposition

Not a qualification. First-scoring failure rates were high, so the instruments were not frozen before seeing outcomes;
the continuity check was added afterward; the retests on patched candidates (batches 006 and 007) cover one family and
are not blind; no human or owner gate, and Linux evidence is a container source and packaged-artifact check, not a cold
batch. Nothing was repaired by hand. Batches 001 to 004 are unchanged.
