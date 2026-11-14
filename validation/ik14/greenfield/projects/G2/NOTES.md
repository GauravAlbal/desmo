# Freeze

This directory is the experiment material for one single-device configuration rollout desk. Builders receive `brief.md` plus `stub/server.mjs` and `stub/CONTRACT.md`. A later maintainer receives `change.md`. The acceptance harness is `evaluator/`. The controls are not implementations to copy.

`reference-toy` passes the suite only to prove the harness can pass. It is not a model solution. A failing control proves only that the check can detect that defect. It does not prove the check detects every other defect, and a pass does not prove the control is otherwise correct.

The suite is the post-change contract: R1-R14 from `brief.md` and R15-R17 from `change.md`. A project that has not applied the change fails R15-R17. `change.md` revises none of R1-R14.

## Requirement ids

- R1 Persistence across process restart
- R2 Prepare captures bytes and show presents those bytes
- R3 Approval binds the presented digest
- R4 Installed bytes match the approved configuration
- R5 Legitimate progress is not refused
- R6 Repeated callback does not change a terminal outcome or add an effect
- R7 Late or conflicting callback does not overwrite a terminal outcome
- R8 Crash after accept is reconcilable without a second effect
- R9 Crash before the request still completes with one effect
- R10 Crash after the durable record keeps the outcome and does not add an effect
- R11 One device per state directory
- R12 Success and refusal are one JSON object
- R13 An unrelated writer file in STATE_DIR is preserved
- R14 Lost reply is reconciled and is not reported as not-sent
- R15 Replace after review clears approval and is refused after install
- R16 An old approval cannot authorize replacement bytes
- R17 Reapproval of the replacement installs the new bytes

## Crash points

Honored only when the running command reaches the boundary. Otherwise the command must not kill itself. The kill is SIGKILL.

- `after-attempt-saved-before-request`: on `install`, after the attempt record is committed, before any byte of the install HTTP request is sent. Not on a retry that finds the attempt already saved.
- `after-accept-before-record`: on `install`, after the complete HTTP response body has been read, before the durable outcome is committed. Not on a lookup of an acceptance from an earlier process.
- `after-record-before-exit`: on `install` or `reconcile`, after a terminal outcome is committed, before the success JSON is written.

## Deliberate ambiguities

- stderr is unconstrained.
- Refusal `error` strings are not a fixed vocabulary. A refusal is a nonzero exit and a non-empty `error` string.
- Extra JSON fields are allowed.
- Proposal ids and request ids are only constrained to 1-64 characters from `A-Za-z0-9._-`.
- After a dropped reply, `install` may return an error and leave `attempt_saved`, or it may look up the request and adopt `installed` before returning. It must not POST a different request id. The suite accepts either local state immediately after that `install`, then requires `reconcile` to report `installed`.
- A well-formed callback may be ignored, or used only as a hint to look up the request. The suite requires that a callback not change a terminal outcome and not add an effect. It does not require a callback alone to finish a lost reply. `reconcile` is the required recovery command.
- No numeric HTTP timeout is specified. A closed connection must make the command return rather than send a new request id.
- Overlapping `./run` processes are unsupported and unspecified.
- A second rollout in the same `STATE_DIR` is out of scope.
- Store filename and encoding are internal, provided committed records survive SIGKILL and unrelated files are left alone.
- The 1 MiB limit, empty-file refusal, and malformed-callback refusal are specified and not separately scored.
- Service delay is a harness fault and is not separately scored. The desk must not treat a late reply as a reason to allocate a new request id.
- The scored unrelated-writer check creates the file from another process and then runs desk commands. A command that deleted the file and recreated it before returning would not be distinguished.

## Control-run table

Observed with `PROJECT_DIR` set to each control and `STUB_DIR` set to `stub/`, via `evaluator/run.sh` logic (`node --test` on `evaluator/accept.test.mjs`).

| Control | Failed requirement ids | Passed |
| --- | --- | --- |
| reference-toy | none | R1-R17 |
| late-overwrite | R7 | every other id |
| duplicate-effect | R8 | every other id |
| lost-reply-misattribution | R14 | every other id |
| stale-approval | R16 | every other id |
| always-refuse | R1-R17 | none |

`stale-approval` is the sixth control. It keeps a hidden authorization after `replace` and installs the new bytes without a new approval. The four named defects plus `always-refuse` and `reference-toy` are the other five. `always-refuse` fails every id because each check includes a successful action; it does not, by itself, show that R7 detects a late overwrite. That isolation is the `late-overwrite` row.
