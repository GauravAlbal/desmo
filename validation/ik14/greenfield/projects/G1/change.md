# Change: replace the input of an in-progress render

You are maintaining the local media-render supervisor from the initial brief. Keep every initial requirement unless this document explicitly revises it.

## What this revises

The initial brief says the input is fixed when the job is accepted, `revision` stays 1, and there is no command to change the input. That rule is withdrawn.

The command set gains `replace`. `replace` is no longer `unknown_command`. Every other unknown command is still `unknown_command`.

## What this preserves

R1 through R12 still hold, including useful progress, persistence across restarts, lost-reply recovery, and cancellation.

A cancelled job must still not become successful when a worker result arrives. A committed success must still keep its first output when a conflicting later report arrives. A lost reply must still be recovered by lookup of the current submission id, not reported as not sent, and must not admit a second render for that same submission id.

The crash labels are unchanged, and they also apply to `replace`:

* `after_accept` is after the new revision is committed and visible, the store lock is not held, and before any byte of the new worker request is written.
* `after_send` and `before_apply_result` apply to the worker request for the new submission id.
* `after_cancel` is unchanged.

Do not hold the store lock across the new worker call.

## New command

`./run replace JOB_ID INPUT_REF`

This is allowed only when the job exists and is not terminal. `accepted` and `rendering` are not terminal, including a job whose progress is `submit_uncertain`.

On success, in one commit before the new worker request:

* increment `revision` by 1
* set `input_ref` to the new input, even if the text is the same as before
* set `submission_id` to `JOB_ID:r` plus the new revision
* clear `output_ref` to null
* set `status` to `accepted` and `progress` to `accepted`

Then admit that new submission id once. Do not admit another render for the previous submission id. The previous admission, if it already exists, stays at the worker and may still report a result. That old result must not change the job.

If the new worker reply says the new submission is still rendering, return `status` `rendering`, `progress` `rendering`, `output_ref` null, the new revision, the new input, and the new submission id.

If that reply is lost, follow the same lost-reply rule as `submit`: exit 0, `status` `accepted`, `progress` `submit_uncertain`, `output_ref` null, and the new submission id. Do not report `not_sent`. A later `reconcile` looks up the new submission id, reports its real outcome, and does not admit a second render for it.

Refusals, with the stored record unchanged:

* unknown job id: `not_found`
* `succeeded`, `failed`, or `cancelled`: `terminal`
* input outside the grammar, or the wrong number of arguments: `invalid`

If `replace` and `cancel` overlap, commit order decides the next legal transition. A cancellation that commits first makes a later `replace` return `terminal`. A replacement that commits first can still be cancelled. An in-flight worker success must not overwrite a cancellation that committed during the wait, same as R11.

## Requirements added by this change

C1. `replace` on an in-progress job commits the new input and the next revision, admits exactly one render for the new submission id, and does not admit a second render for the old one. A later success of the previous submission id must not mark the job `succeeded` and must not attach an output. After `reconcile`, `status` is not `succeeded`, `output_ref` is null, `revision` is the replacement revision, `input_ref` is the new input, and `submission_id` is the new one. If the worker is still rendering the new submission, the `replace` response itself has `status` `rendering` and `output_ref` null.

C2. The replacement can still finish. After the worker succeeds the new submission id, `reconcile` returns `status` `succeeded`, `progress` `done`, `revision` equal to the replacement revision, `input_ref` equal to the new input, `submission_id` equal to the new id, and `output_ref` equal to `out:` plus that new submission id.

C3. Cancel still wins after a replacement. `cancel` of the replacement revision commits `cancelled` with `output_ref` null and keeps the new revision and new input. A further `replace` then exits nonzero with `terminal` and does not change the record. Completing the old submission, the new submission, or both, does not change that. `reconcile` and `status` stay `cancelled` with `output_ref` null.

C4. A dropped reply on the replacement submission is recovered without a second admission and without attaching the old output. `replace` returns exit 0 with `status` `accepted`, `progress` `submit_uncertain`, `output_ref` null, the new revision, the new input, and the new submission id. Exactly one admission exists for the old submission id and exactly one for the new one. A later `reconcile` returns `succeeded` with `output_ref` `out:` plus the new submission id, and the admission set is unchanged.
