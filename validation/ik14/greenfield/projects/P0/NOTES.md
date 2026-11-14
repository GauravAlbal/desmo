# Freeze

This directory is the experiment material for one local label-printing spooler. Builders receive `brief.md` plus `stub/server.mjs` and `stub/CONTRACT.md`. A later maintainer receives `change.md`. The acceptance harness is `evaluator/`. The controls are not implementations to copy.

`reference-toy` passes the suite only to prove the harness can pass. It is not a model solution. A failing control proves only that the check can detect that defect. It does not prove the check detects every other defect, and a pass does not prove the control is otherwise correct.

The suite is the post-change contract: R1-R9 from `brief.md` and R10 from `change.md`. A project that has not applied the change fails R10. `change.md` revises none of R1-R9.

## Requirement ids

- R1 Persistence of a submitted job across process restart
- R2 Hold and cancel before dispatch add no print
- R3 Legitimate progress prints the captured label once
- R4 A printed job is not printed again, including after a crash once the record is durable
- R5 Crash after accept is recovered without a second print
- R6 Lost reply is reconciled onto the same job and is not reported as not-sent
- R7 Late or conflicting notice does not overwrite a printed result
- R8 Success and refusal are one JSON object
- R9 An unrelated writer file in STATE_DIR is preserved
- R10 Reprint creates a linked job, prints it once, and leaves the original result unchanged

## Crash points

Honored only when the running command reaches the boundary. Otherwise the command must not kill itself. The kill is SIGKILL.

- `after-dispatch-saved-before-request`: on `dispatch`, after the dispatch record is committed, before any byte of the print HTTP request is sent. Not on a retry that finds the dispatch already saved. Specified, not separately scored.
- `after-accept-before-record`: on `dispatch`, after the complete HTTP response body has been read, before the durable printed outcome is committed. Not on a lookup of an acceptance from an earlier process. Scored by R5.
- `after-record-before-exit`: on `dispatch` or `reconcile`, after a printed outcome is committed, before the success JSON is written. Not when the command found `printed` already committed. Scored by R4 for `dispatch`.

## Deliberate ambiguities

- stderr is unconstrained.
- Refusal `error` strings are not a fixed vocabulary. A refusal is a nonzero exit and a non-empty `error` string.
- Extra JSON fields are allowed. `reprint_of` is required on `status` only after the change.
- Job ids and dispatch ids are only constrained to 1-64 characters from `A-Za-z0-9._-`.
- After a dropped reply, `dispatch` may return an error and leave `dispatch_saved`, or it may look up the dispatch and adopt `printed` before returning. It must not POST a different dispatch id. The suite accepts either local state immediately after that `dispatch`, then requires `reconcile` to report `printed`.
- A well-formed notice may be ignored, or used only as a hint to look up the dispatch. The suite requires that a notice not change a printed result and not add an effect. It does not require a notice alone to finish a lost reply. `reconcile` is the required recovery command.
- No numeric HTTP timeout is specified. A closed connection must make the command return rather than send a new dispatch id.
- Overlapping `./run` processes are unsupported and unspecified.
- Store filename and encoding are internal, provided committed records survive SIGKILL and unrelated files are left alone.
- The 1 MiB limit, empty-file refusal, malformed-notice refusal, and HTTP 400 or 409 handling are specified and not separately scored.
- Binding the store to one printer id is specified and not separately scored.
- Service delay is a harness fault and is not separately scored. The spooler must not treat a late reply as a reason to allocate a new dispatch id.
- There is no unhold command. A held job stays held until cancel. That is specified. R2 scores the hold and the cancel, not a later dispatch of a formerly held job.
- The scored unrelated-writer check creates the file from another process and then runs spooler commands. A command that deleted the file and recreated it before returning would not be distinguished.
- `reprint` of a job that is not printed must not create a job. The scored check is a nonzero exit, no print effect, and an unchanged named job. A hidden extra job would not be visible without a list command.
- A printed reprint may itself be reprinted. That case is specified and not separately scored.
- R4 fails if either the repeat-dispatch path or the after-record crash path fails. No control isolates the after-record crash by itself.

## Control-run table

Observed with `PROJECT_DIR` set to each control directory and `STUB_DIR` set to `stub/`, via `evaluator/run.sh`.

| Control | Failed requirement ids | Passed |
| --- | --- | --- |
| reference-toy | none | R1-R10 |
| late-overwrite | R7 | every other id |
| duplicate-effect | R5 | every other id |
| lost-reply-misattribution | R6 | every other id |
| always-refuse | R1-R10 | none |

`late-overwrite` applies a conflicting notice onto a printed job. `duplicate-effect` allocates a new dispatch id when recovering a saved dispatch, so the crash-after-accept retry prints again. `lost-reply-misattribution` reports `not_sent` from `reconcile` after the printer has already accepted that dispatch. `always-refuse` fails every id because each check includes a successful action; it does not, by itself, show that R7 detects a late overwrite. That isolation is the `late-overwrite` row.
