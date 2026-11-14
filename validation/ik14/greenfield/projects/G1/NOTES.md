# Freeze notes

Project: local media-render supervisor. One machine, one local store, one worker HTTP service. No distributed transactions.

## Requirement ids

Initial brief (`brief.md`): R1, R2, R3, R4, R5, R6, R7, R8, R9, R10, R11, R12.

Follow-on change (`change.md`): C1, C2, C3, C4. These revise only the rule that input and revision are fixed, and add the `replace` command. R1 through R12 are preserved.

`evaluator/run.sh` defaults to `EVAL_SCOPE=brief` and prints a JSON object with pass or fail per brief id. That default suite checks only requirements stated in `brief.md`.

`EVAL_SCOPE=all` also runs C1 through C4. Those checks are stated in `change.md`, not only in the evaluator. Use that scope when scoring the follow-on. Do not treat a brief-only builder's C failures as initial-brief failures.

## Crash points

The process must `SIGKILL` itself at the named boundary when `CRASH_AT` is that label. Unknown labels, and labels not on the command path, are ignored.

* `after_accept`: after the accept or replacement revision is committed and visible, lock released, before any byte of the worker request is written.
* `after_send`: after the worker request is fully written and response headers have been received. The worker records the admission before it writes headers. The body is not yet parsed, and the result is not committed. Not reached if the connection drops before headers.
* `before_apply_result`: after the body is fully read and parsed, before that result is committed, lock not held.
* `after_cancel`: after cancellation is committed and visible, lock released, before stdout is written.

There is no worker cancel call. Cancellation is local.

## Deliberate ambiguities

* Two overlapping `submit` calls for the same new job id: one record must remain, but which caller receives `already_exists` is unspecified if neither has committed yet.
* Any nonzero exit code is a refusal. The particular number is unspecified.
* stderr text is unspecified.
* The HTTP timeout length is unspecified. A timeout after the request was sent is a lost reply. A reply received while the command is still waiting must follow the cancel and apply rules.
* A worker HTTP error with no parsed result object, after the request was sent, is a lost reply rather than a job failure. Job failure is only a parsed worker state of `failed`. This is not separately tested.
* Bytes inside `STATE_DIR` are unspecified.
* A worker success with a null `output_ref` does not occur on the supplied worker and is unspecified.
* Overlapping `reconcile` of the same non-terminal job must both use the canonical submission id, so the worker will not double-admit. Which local write lands last is unspecified, except that a committed terminal state must not be overwritten by an older in-flight result.
* Whether a repeated cancel of an already cancelled job honors `after_cancel` is unspecified. The tested case is the transition into cancelled.
* Reading the worker callback list is optional. Lookup of the current submission id is enough if the apply rules are followed.

## Controls

Five directories, each with an executable `run`. The assignment text named these five and also said "six"; no sixth defect was named, so none was added.

* `late-overwrite`: a late or old result overwrites a newer local state.
* `duplicate-effect`: recovery from a lost or unapplied reply admits a second render under a new submission id.
* `lost-reply-misattribution`: after a lost or unapplied reply, reports `not_sent` and also admits another render.
* `always-refuse`: every command exits nonzero with `{"error":"refused"}`.
* `reference-toy`: minimal passer for the checks. Not a model solution.

The four behavioral controls share `controls/_shared/engine.cjs` and differ by a defect flag. That is an authoring convenience, not part of the builder interface.

A failing control proves only that the check can detect that defect. It does not prove that every bad implementation fails, and a passing control is not evidence of general correctness.

## Control-run table

Default evaluator, `EVAL_SCOPE=brief`, Node 22.17.1. `reference-toy` passes every brief check.

| control | failed ids |
| --- | --- |
| reference-toy | none |
| late-overwrite | R6, R7, R11 |
| duplicate-effect | R4, R8 |
| lost-reply-misattribution | R4, R8 |
| always-refuse | R1, R2, R3, R4, R5, R6, R7, R8, R9, R10, R11, R12 |

Change suite, `EVAL_SCOPE=all`, same controls. `reference-toy` passes every brief and change check. Failures below are the full set, including the brief ids above.

| control | failed ids |
| --- | --- |
| reference-toy | none |
| late-overwrite | R6, R7, R11, C1, C3 |
| duplicate-effect | R4, R8, C4 |
| lost-reply-misattribution | R4, R8, C4 |
| always-refuse | R1 through R12 and C1 through C4 |

`late-overwrite` still passes C2, so the change suite can tell "old output attached" from "replacement cannot finish". `duplicate-effect` and `lost-reply-misattribution` still pass the cancel checks, so those checks are not just "any defect fails".
