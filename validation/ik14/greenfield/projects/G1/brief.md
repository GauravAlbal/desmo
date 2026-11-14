# Local media-render supervisor

Build a local supervisor that accepts render jobs, records their progress, and lets a person cancel a job. A separate worker program does the render. The worker may drop a reply, answer late, or report a result more than once, including a second report with a different output. A cancelled job must not become successful when an old worker result arrives. After a restart, accepted work must still be there, and a submission whose reply was lost must be reported as the worker actually recorded it, not as never sent.

This is one machine and one local store. There are no distributed transactions and no multi-host coordination. Each command is one process. A later process using the same store sees what earlier processes committed. Two processes may run at the same time. A second process may submit a different job, or cancel a job, while another process is waiting on the worker.

You are given the worker and its service contract (CONTRACT.md next to the worker program). Call that HTTP API. The worker may drop replies, respond late, or report twice. You do not need a way to cause those faults. There is no cancel operation on the worker. Cancellation is local. The worker may still finish after you cancel.

## How you are invoked

The entry point is an executable file named `run` in the project directory.

```
./run COMMAND [ARGUMENTS]
```

Each invocation is a new process. On success, exit 0. On refusal, exit nonzero. In both cases, write exactly one JSON object to stdout and nothing else. A trailing newline is fine. Diagnostics, if any, go to stderr only.

Environment:

* `STATE_DIR` is the durable store directory. Create it if it is missing. A new process with the same `STATE_DIR` must see every committed record. Required for every command.
* `SERVICE_URL` is the worker base URL, with no path. Required for every command, including commands that do not call the worker.
* `CRASH_AT`, when set to one of the labels below, means the process must kill itself at that boundary with `process.kill(process.pid, 'SIGKILL')` or an equivalent signal that cannot be caught. If the label is unset, unknown, or not on this command's path, ignore it and finish normally.

Do not depend on wall-clock time. You may bound one worker call with your own timeout. A timeout or a dropped connection after the request was sent is a lost reply, not proof that the render was never admitted. A reply that arrives while the command is still waiting must be handled under the rules below, including the cancellation rule.

## Identifiers and the worker

`job_id` matches the pattern `^[A-Za-z0-9_-]{1,64}$`. `input_ref` matches `^[A-Za-z0-9_.:/-]{1,256}$`. Anything else is invalid.

For this initial brief, the input is fixed when the job is accepted. `revision` is the integer 1 for the life of the job. There is no command to change the input.

The submission id you send the worker is exactly the job id, then `:r`, then the revision. Example: `job1:r1`. Send that id on the worker's admit call, as the service contract describes. Repeating the same submission id must not start a second render. A different submission id would start another render, so after a lost reply you must look up this same id. You must not invent a second id for the same revision.

When the worker reports success, it sets `output_ref` to `out:` concatenated with the submission id. Store that string unchanged. `output_ref` is JSON null unless `status` is `succeeded`.

The worker's lookup returns the admission even if the original reply was dropped. Lookup does not admit a render. The worker may also list result reports, in order, and may append a duplicate or a conflicting output for an admission. Apply a report only when both of the following are true:

* the report's submission id is the job's current submission id
* the job is not already terminal (`succeeded`, `failed`, or `cancelled`)

If the job is terminal, leave `status` and `output_ref` as they are. The first committed terminal outcome wins. A cancellation committed before you apply a result wins over that result, even if the worker result arrives later or the worker call was already in flight.

If the worker reports `failed` for the current submission and the job is not terminal, commit `status` `failed`, `progress` `failed`, and `output_ref` null. That is terminal. The supplied worker does not do this unless its record says so.

## Durable record

Success responses for `submit`, `cancel`, `status`, and `reconcile` are one object with at least these fields:

```json
{"job_id":"job1","status":"rendering","input_ref":"media/clip","revision":1,"output_ref":null,"progress":"rendering","submission_id":"job1:r1"}
```

`status` is one of `accepted`, `rendering`, `succeeded`, `failed`, `cancelled`.

`progress` is the recorded phase, one of `accepted`, `submit_uncertain`, `rendering`, `done`, `failed`, `cancelled`.

Map a worker result onto the current non-terminal job as follows:

* worker still rendering: `status` `rendering`, `progress` `rendering`, `output_ref` null
* worker succeeded: `status` `succeeded`, `progress` `done`, `output_ref` copied from the worker
* worker failed: `status` `failed`, `progress` `failed`, `output_ref` null

Extra fields are allowed. Missing required fields are not.

Refusal responses are one object with at least `{"error":"CODE"}`. Codes:

* `invalid` for a bad argument, the wrong number of arguments, or a missing `STATE_DIR` or `SERVICE_URL`
* `already_exists` when `submit` names a job id that is already stored
* `not_found` when the job id is not stored
* `terminal` when `cancel` is asked to cancel a `succeeded` or `failed` job
* `unknown_command` for a command name other than the four below
* `unavailable` when `reconcile` cannot reach the worker at all. Do not change the stored job, and do not report that as not sent.

`status` must not call the worker. It must succeed from the store alone, including when the worker process is gone.

Do not hold the store lock across a worker call. A crash must not leave the store locked. Another process must be able to read a committed record and commit a cancellation while this process is waiting on the worker.

## Commands

`./run submit JOB_ID INPUT_REF`

Commit the job as `accepted` with `progress` `accepted`, `revision` 1, `output_ref` null, and submission id `JOB_ID:r1` before opening the worker connection. Then admit that submission. If the reply is applied, return the mapped record. If the reply is lost after the request was sent, leave the job `accepted` with `progress` `submit_uncertain` and `output_ref` null, and exit 0. Do not return an error that claims the work was not sent. If the connection fails before the request is sent, leave `progress` `accepted` and exit 0. A later `reconcile` will admit the same submission id.

`./run cancel JOB_ID`

If the job is `accepted` or `rendering`, commit `cancelled`, `progress` `cancelled`, and `output_ref` null, then return that record with exit 0. Cancelling an already cancelled job returns the same record with exit 0 and does not change it. Cancelling `succeeded` or `failed` is `terminal` and must not change the stored record.

`./run status JOB_ID`

Return the stored record. Do not call the worker.

`./run reconcile JOB_ID`

If the job is terminal, return the stored record and do not change it, even if the worker now reports something else. If it is not terminal, look up the current submission id. If the worker has no admission, admit that same id once. If the worker has an admission, apply it under the rules above. A lost reply during this command follows the same `submit_uncertain` rule and must not create a second submission id.

## Crash boundaries

* `after_accept`: the accept has been committed and is visible to other processes. The store lock is not held. No byte of the worker request has been written.
* `after_send`: the worker request has been fully written, and the response headers have been received. The worker writes those headers only after it has recorded the admission, so the admission exists. The response body has not been parsed into a result, and that result has not been committed. If the worker drops the connection before headers, this boundary is not reached.
* `before_apply_result`: the response body has been fully read and parsed into a result in memory. That result has not been committed. The store lock is not held.
* `after_cancel`: cancellation has been committed and is visible to other processes. The store lock is not held. The command has not yet written its stdout response.

A killed process does not need to write a JSON object. The next process must be able to use the store.

## Requirements

R1. `submit` of a new well-formed job admits exactly one render for `JOB_ID:r1` and returns exit 0 with that job id, the given `input_ref`, `revision` 1, and submission id `JOB_ID:r1`. If the worker is still rendering, `status` is `rendering`, `progress` is `rendering`, and `output_ref` is null. If the worker already finished, `status` is `succeeded`, `progress` is `done`, and `output_ref` is `out:` plus the submission id. A second `submit` of the same job id exits nonzero with `already_exists` and does not admit another render.

R2. After a successful `submit`, a new process with the same `STATE_DIR` can `status` the job and observe the same record even if the worker process is gone. `status` reads only the durable store.

R3. If the process is killed at `after_accept`, the store contains the accepted job, `progress` is `accepted`, `output_ref` is null, and the worker has no admission yet. A later `reconcile` admits exactly one render for that same submission id and, when the worker completes immediately, reports `succeeded` with `output_ref` `out:` plus that submission id.

R4. If the worker drops the reply, or the process is killed at `after_send`, that submission was admitted exactly once. The handled drop returns exit 0 with `status` `accepted`, `progress` `submit_uncertain`, `output_ref` null, and the original submission id. It must not report the error `not_sent` or any other claim that the render was not sent. After either case, `reconcile` reports `succeeded` with `output_ref` `out:` plus the original submission id and does not create a second admission.

R5. `cancel` of an in-progress job commits `cancelled`, `progress` `cancelled`, and `output_ref` null, and returns that record with exit 0. A repeated cancel returns the same cancelled record with exit 0. A kill at `after_cancel` leaves the job cancelled for a later `status`.

R6. After cancel, a worker completion for that submission must not change the job. `reconcile` and `status` still show `cancelled` and `output_ref` null.

R7. After the job is `succeeded` with an `output_ref`, a conflicting later worker report for the same submission must not change `status` or `output_ref`. `reconcile` and `status` keep the first committed output.

R8. A kill at `before_apply_result` happens only after the worker has admitted the render. The result is not committed, so `status` does not show `succeeded`. A later `reconcile` commits that same admission's outcome once and does not admit a second render.

R9. A different job id can be submitted and reach `succeeded` with its own `out:` plus `JOB_ID:r1` while another job is `cancelled`, and while another job is only `accepted` because of an `after_accept` kill. Neither sibling is left unfinished because of the other. Each finished job has exactly one admission. The unsent job has none.

R10. With a healthy worker that completes immediately, `submit` returns `succeeded`, `progress` `done`, and `output_ref` `out:` plus `JOB_ID:r1`, with exit 0. `reconcile` leaves that result in place and does not admit a second render. An implementation that refuses the submission fails this requirement.

R11. If a second process cancels a job after the submission has been admitted and while `submit` is still waiting for the worker reply, the committed cancel wins. The waiting `submit` must not leave the job `succeeded`. After it finishes, `status` shows `cancelled` and `output_ref` null. The store must accept that cancellation during the wait.

R12. `status`, `cancel`, and `reconcile` of an unknown job id exit nonzero with `not_found`. A job id or input outside the grammar exits nonzero with `invalid`. `cancel` of a `succeeded` job exits nonzero with `terminal` and does not change the stored record. An unknown command exits nonzero with `unknown_command`. Every refusal and every success prints exactly one JSON object on stdout and nothing else.

