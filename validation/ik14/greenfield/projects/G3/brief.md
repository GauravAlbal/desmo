# Record import desk

Build a local import desk. An operator submits one record at a time. The desk validates it, remembers it, and delivers it to the destination service. A delivery can be rejected, lost, delayed, or answered in conflict with what the destination actually stored. The desk must keep going, tell the truth about what is confirmed, and not do the same external work twice.

This version is a single-machine tool. It uses one durable store under `STATE_DIR`. One command runs in one process. Another process may run commands against the same `STATE_DIR` at the same time. There is no distributed transaction, no multi-host lock, and no network service to deploy. Pause, resume, and cancellation are not part of this version. There is no operation that undoes a request already sent to the destination.

The evaluator starts the destination and sets `SERVICE_URL`. You may run `stub/server.mjs` yourself while developing. Its HTTP API, idempotency, and lookup rules are in `stub/CONTRACT.md`. The desk must not depend on any control port, fault switch, or ready-file the contract describes for the evaluator. What the desk must assume is only this: the destination may drop the reply after it has acted, answer late, or answer twice, and a reply can disagree with lookup.

## Commands

Entry point: `./run` in the project directory, executable, one process per invocation.

```text
./run submit <import_id> <record>
./run status <import_id>
./run advance <import_id>
```

Environment:

- `STATE_DIR` — durable state directory. A new process with the same directory sees every committed result. A different directory does not.
- `SERVICE_URL` — destination base URL, such as `http://127.0.0.1:1234`.
- `CRASH_AT` — optional. When it names a boundary below, the process must terminate itself with `SIGKILL` at that boundary (`process.kill(process.pid, 'SIGKILL')` or the equivalent). No other label causes a crash. Unset or empty means do not crash.

If `STATE_DIR` or `SERVICE_URL` is missing, refuse before doing the command:

```json
{"error":"invalid_input","field":"env"}
```

Stdout must be exactly one JSON object, optionally followed by a single trailing newline. No other stdout. Exit 0 when the command completed, including when the import is terminally `failed`. Exit nonzero only for a refusal. A refusal body is `{"error":"<code>"}` plus the fields named below. Extra fields are allowed on every object. Missing required fields are not.

`status` and `submit` must not contact the destination. Only `advance` may contact it. `status` reads the local store only. It must return even if another process is waiting on the destination; it must not wait for that call to finish.

### Identity and validation

`import_id` must match `^[A-Za-z0-9_-]{1,64}$`. `record` must be a string of 1 to 256 Unicode code points. Anything else is a local refusal, creates no import, and does not contact the destination:

```json
{"error":"invalid_input","field":"import_id"}
{"error":"invalid_input","field":"record"}
```

An unknown command, including no command, is `{"error":"unknown_command"}`. `status` or `advance` of an id that was never submitted is `{"error":"not_found"}`.

Submitting an id that already exists with the same record returns the current status object and does not change phase, dispatch, attempt counts, destination id, or last error, and does not contact the destination. Submitting the same id with a different record is `{"error":"conflict"}`, changes nothing, and does not contact the destination.

### Status object

Successful `submit`, `status`, and `advance` print:

```json
{
  "import_id": "imp1",
  "phase": "ready",
  "dispatch": "none",
  "attempts_failed": 0,
  "attempts_remaining": 3,
  "destination_id": null,
  "last_error": null
}
```

- `attempts_failed` is the number of confirmed rejections. It never decreases. It is not reset by restart, by submitting the same record again, or by a later successful delivery.
- `attempts_remaining` is always `3 - attempts_failed`.
- `destination_id` is a non-empty string only when `phase` is `delivered`; otherwise it is null. When delivered, it is the destination id lookup returns for this import id.
- `last_error` is `null`, `"rejected"`, or `"conflict"`. It is `"rejected"` after a confirmed rejection, `"conflict"` after a destination conflict, and `null` after a confirmed delivery (including a delivery that follows an earlier rejection).

Phases and dispatch:

| Situation | phase | dispatch |
| --- | --- | --- |
| Accepted, request not sent | `ready` | `none` |
| Request sent, outcome not yet durable | `uncertain` | `in_flight` |
| Lookup or an applied success shows the same record stored | `delivered` | `confirmed` |
| Three confirmed rejections, record not stored | `failed` | `none` |
| Destination conflict | `failed` | `none` |
| Confirmed rejection, retries still left | `ready` | `none` |

## Destination use

Deliver with `POST /v1/deliveries` and look up with `GET /v1/deliveries/<import_id>`, as `stub/CONTRACT.md` describes.

Every delivery attempt for an import must use `idempotency_key` equal to that import's `import_id` and `record` equal to the submitted record. Do not invent another key to retry, to recover, or to get around a conflict.

A repeated POST of the same key and the same record is one stored effect. A different record under that key is a conflict. Lookup is how the desk tells whether the effect landed when the reply is missing, late, or in conflict with the stored row.

Confirmed outcomes:

- Lookup shows the same record: the import is `delivered`. A rejection reply, a late reply, or a second reply must not turn that into a failure and must not increase `attempts_failed`.
- Lookup shows a different record under this import id: terminal `failed`, `last_error` `"conflict"`, `attempts_failed` unchanged, `destination_id` null. Do not POST a different key or a different record to force a new effect. A later `advance` must not contact the destination.
- The destination returns a rejection and lookup shows the record is absent: one confirmed rejection. If `attempts_failed` is still below 3, phase returns to `ready` and dispatch to `none`. If it reaches 3, phase is `failed` and dispatch is `none`.
- The reply is missing, the destination returns an unexpected status (including 500), or lookup is unavailable: the attempt is unfinished. Phase stays `uncertain`, dispatch stays `in_flight`, `attempts_failed` does not change, and the import is not `failed`. Do not report this as not sent (`ready` / `none`).

`advance` on an already `delivered` or exhausted `failed` import returns the current status and must not create another effect. An exhausted `failed` import must not contact the destination again. A `delivered` import must not be submitted under a new key; repeating the same key is allowed only if the effect count stays one.

If lookup shows the record is absent and the previous send was unfinished, `advance` may POST the same key again. That POST is not a confirmed rejection by itself. It becomes a confirmed rejection only under the rule above.

When the destination is reachable and lookup shows the stored record, the `advance` that observes that fact must return `delivered` before it exits. It must not stop at `uncertain`, and it must not refuse a valid import that still has retries left.

## Crash boundaries

`CRASH_AT` is for the evaluator. The process kills itself. The evaluator does not guess the moment.

1. `after-record-before-dispatch` — the import's durable record has been committed (the import exists, `attempts_failed` is unchanged, phase is still `ready`, dispatch is still `none`). No byte of a destination request has been written. Then `SIGKILL`. After a new process starts, `status` shows that same ready import, and a later `advance` can still deliver it. This crash does not consume a retry.

2. `after-dispatch-before-result` — the delivery request has been sent and the destination has produced a response (response headers are visible to the process, or the connection has failed after the send). The durable record already says `uncertain` / `in_flight`. The response body has not been applied and no outcome write has happened. Then `SIGKILL`. `status` in a new process shows `uncertain` / `in_flight`, `attempts_failed` unchanged, `destination_id` null. The effect, if the destination stored one, is still one effect. A later `advance` must reconcile by lookup before any further submit, reach `delivered` when the record is there, and must not create a second effect.

3. `after-result-before-commit` — the response body has been fully read into memory. The outcome has not been written to the durable store. Then `SIGKILL`. The durable record is still the pre-outcome record (`uncertain` / `in_flight` if the send was marked first). Recovery is the same as boundary 2: lookup, one effect, no false confirmed failure.

The durable dispatch mark for boundary 2 is written before the request is sent, and the crash is after the destination has acted (or the connection has failed) and before the outcome is committed. Boundary 1 is before that mark and before any send.

## Requirements

**R1. Persistence is per state directory.** A valid `submit` returns `ready` / `none`, zero failed attempts, null destination id, null last error, and does not contact the destination. A later process with the same `STATE_DIR` sees that status. `status` does not contact the destination. A different `STATE_DIR` does not contain the import (`not_found`).

**R2. Local refusals do not touch the destination.** Invalid id, invalid record, unknown command, and missing `STATE_DIR` or `SERVICE_URL` refuse with the error codes above and make no destination call. Unknown id is `not_found` and makes no destination call.

**R3. One effect.** On a healthy destination, `advance` returns `delivered` / `confirmed` with the lookup destination id, null last error, and exactly one stored effect for that record under the import id. Another `advance`, or a same-record `submit`, does not create a second effect and does not change that destination id.

**R4. A confirmed rejection consumes one retry.** One rejection with lookup absent returns `ready` / `none`, `attempts_failed` 1, `last_error` `"rejected"`, null destination id, and stores nothing. It is not yet `failed`.

**R5. The retry limit is three and it sticks.** The third confirmed rejection returns `failed` / `none`, `attempts_failed` 3, `last_error` `"rejected"`. A new process still shows that. Further `advance` does not contact the destination. A same-record `submit` does not clear it. Separately, one confirmed rejection is still 1 after a new process, and a later successful delivery keeps `attempts_failed` at 1, clears `last_error`, and sets `delivered` / `confirmed`.

**R6. A crash before dispatch is not a failure and does not block progress.** `CRASH_AT=after-record-before-dispatch` kills the process with `SIGKILL`, sends nothing, and leaves `ready` / `none` with the previous `attempts_failed`. A later `advance` delivers exactly once.

**R7. A crash after dispatch is an uncertain send, not a lost effect and not a confirmed failure.** `CRASH_AT=after-dispatch-before-result` leaves one stored effect, `uncertain` / `in_flight`, `attempts_failed` unchanged, null destination id. Recovery `advance` returns `delivered` with the lookup id and still exactly one effect.

**R8. A dropped reply is attributed, not denied.** If the destination stores the record and drops the reply, that `advance` returns `delivered` / `confirmed` with the lookup id, `attempts_failed` unchanged, exactly one effect, and every POST for that record used `idempotency_key` equal to the import id. It must not return `ready` / `none`, must not return `failed`, and must not submit under another key.

**R9. Unfinished is not confirmed failure.** If the reply is a rejection but lookup shows the same record stored, the result is `delivered` with `attempts_failed` unchanged and one effect. If a rejection reply arrives and lookup is unavailable, stay `uncertain` / `in_flight` with `attempts_failed` unchanged. If the destination returns 500 and stores nothing, stay `uncertain` / `in_flight` with `attempts_failed` unchanged and no effect. Do not count any of these as a confirmed rejection.

**R10. A late conflicting reply does not overwrite a newer delivered outcome.** While one `advance` is waiting on a reply that will disagree with a stored record, another process's `advance` must be able to reconcile that import to `delivered` with `attempts_failed` still 0. When the late rejection arrives, it must not replace that delivered outcome, must not increase `attempts_failed`, and must not drop the stored record lookup finds under the import id.

**R11. Legitimate work is not blocked.** A confirmed rejection below the limit does not refuse later work. After the destination stops rejecting, `advance` delivers that import. A second valid import delivers on its own. An implementation that refuses every command fails this.

**R12. Another process sees committed state.** After `submit` returns, a second process's `status` shows the import. After a confirmed rejection is committed, a second process's `status` shows that `attempts_failed`. While one process is blocked in a destination call, another process's `status` returns `uncertain` / `in_flight` without waiting for that call.

**R13. Conflicts stay conflicts.** A different-record `submit` is a local `conflict` and does not contact the destination or change the stored import. If the destination already holds a different record under this import id, `advance` returns `failed` / `none`, `attempts_failed` unchanged, `last_error` `"conflict"`, null destination id, and does not store the import's record. A later `advance` does not create an effect and does not POST that record again.

**R14. A crash after the body is read and before the outcome is committed is still unfinished.** `CRASH_AT=after-result-before-commit` leaves `uncertain` / `in_flight` and the single stored effect. Recovery matches R7: `delivered`, lookup id, one effect, `attempts_failed` unchanged.

## Safety, progress, persistence, recovery

- Do not create a second destination effect for one import.
- Do not report a sent request as not sent.
- Do not treat an unfinished send as a confirmed rejection, and do not treat a confirmed rejection as unfinished.
- Do not let a late or conflicting reply replace a newer `delivered` outcome or reduce `attempts_failed`.
- Do not contact the destination for a local refusal, a `status`, or an exhausted import.
- Do not change the idempotency key or the record to get around a conflict or a lost reply.
- A valid import with retries left must be acceptable. When the destination has the record, one `advance` must finish at `delivered`. When the destination confirms a rejection and lookup agrees it is absent, that `advance` must record the confirmed rejection before it exits. Recovery after any named crash must be possible with a later command; the desk must not stick in a refusal or a permanent "try later" once the destination can answer lookup.
- Committed attempt counts, phase, and delivery identity survive process exit and a new process pointed at the same `STATE_DIR`.
