# Freeze

Project: resumable data-import desk. Materials are the brief, the follow-on change, the destination stub, the black-box evaluator, and six controls. A failing control proves only that the check can detect that defect. It does not prove the check catches every other defect, and a passing control is not a model solution.

## Requirement ids

Initial brief (`brief.md`), always scored by `evaluator/run.sh`:

- R1 persistence is per `STATE_DIR`
- R2 local refusals do not touch the destination
- R3 one destination effect
- R4 one confirmed rejection stays retryable
- R5 retry limit is three and survives restart; a later delivery does not decrease it
- R6 crash `after-record-before-dispatch` does not consume a retry and does not block delivery
- R7 crash `after-dispatch-before-result` stays uncertain and recovers once
- R8 a dropped reply is attributed by lookup, not reported as not sent, and is not repeated under a new key
- R9 unfinished work is not a confirmed failure (conflicting rejection, lookup unavailable, HTTP 500)
- R10 a late conflicting reply does not overwrite a newer delivered outcome
- R11 legitimate work is not refused and an earlier rejection does not block a later delivery
- R12 a second `./run` process sees committed state and can read status during an in-flight call
- R13 local and destination conflicts stay conflicts and do not create another effect
- R14 crash `after-result-before-commit` leaves the outcome uncommitted and recovers once

Follow-on (`change.md`), scored only when `EVAL_INCLUDE_CHANGE=1`. None of R1–R14 are revised.

- R15 pause blocks a new dispatch; resume continues with the same retry count
- R16 pause does not undo an in-flight dispatch; advance reconciles it without a second POST
- R17 pause and resume do not restore exhausted retries

## Crash points

The process must `SIGKILL` itself when `CRASH_AT` names one of these. The evaluator does not pick the moment.

1. `after-record-before-dispatch` — durable import record committed, phase still `ready`, dispatch still `none`, no destination byte written.
2. `after-dispatch-before-result` — request sent and the destination has produced response headers or the connection has failed after the send; durable record already `uncertain` / `in_flight`; body not applied.
3. `after-result-before-commit` — response body fully read; outcome not yet written. Durable record still `uncertain` / `in_flight`.

## Deliberate ambiguities

- The durable store format is not prescribed. Only `STATE_DIR` and the `./run` interface are.
- Extra JSON fields are allowed. Key order is not prescribed. Stdout is one JSON object and an optional trailing newline. stderr is ignored.
- A repeat `advance` of a delivered import may POST the same idempotency key again. It must not use a new key. The check is one stored effect and a stable lookup id, not a ban on the idempotent POST.
- Record length is Unicode code points. The shipped checks use ASCII, so a builder that counts UTF-16 code units still passes them.
- The desk is not required to listen for callbacks. "Answer twice" is an optional `callback_url` notification in the stub, plus an idempotent repeat of the same key. The automated late-reply check is a held synchronous response that disagrees with lookup.
- Drop combined with lookup unavailable is specified (stay `uncertain`) but is not its own check. Rejection plus lookup unavailable is checked.
- Restarting the destination process, which forgets in-memory rows, is out of scope.
- Two first submits of the same new id at the same moment are not separately checked. A committed import must stay consistent.
- `attempts_failed` after a later delivery stays at the confirmed-rejection count. It is not cleared to zero.
- Pause and resume are outside the initial brief. Default `evaluator/run.sh` does not score R15–R17.

## Control-run table

Default evaluator (`R1`–`R14`). `reference-toy` passes every default check.

| Control | Failed requirement ids |
| --- | --- |
| reference-toy | none |
| late-overwrite | R10 |
| duplicate-effect | R3, R7, R14 |
| lost-reply-misattribution | R8 |
| always-refuse | R1–R14 |
| retry-reset | R5 |

`duplicate-effect` fails R14 as well as R3 and R7 because recovery from the post-body crash also submits under a new key. That is the same duplicate-effect defect at the second post-dispatch boundary, not a separate planted bug.

With `EVAL_INCLUDE_CHANGE=1`, `reference-toy` also passes R15–R17. Additional failures beyond the table above: `always-refuse` fails R15–R17, and `retry-reset` fails R17 because the forgotten rejection count is still forgotten after pause and resume. The other controls pass R15–R17.

`retry-reset` is the sixth control. It drops the confirmed-rejection count at the start of each `advance`. The other five are the named controls.
