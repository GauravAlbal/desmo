# Label printing spooler

A local spooler accepts label jobs for one label printer, can hold or cancel a job before that job is dispatched, and asks a printer service to print the captured label. A job that has been dispatched produces at most one physical print. The spooler keeps its records in one durable local store on this machine. It does not coordinate a fleet of printers, a quota, or a cross-machine transaction.

You own the program behind `./run`. The printer service is supplied. Its HTTP API is specified below and, with the same rules, in `stub/CONTRACT.md`.

## Scope

- One machine, one state directory, one spooler process at a time.
- One printer id for the life of that state directory. A second printer id is refused.
- Many label jobs may exist in that state directory. Each job is dispatched on its own.
- The operator does not run two `./run` processes at once. An unrelated process may write other files in the state directory between commands, and may do so while a command runs.
- No fleet-wide transaction, no printer quota, no remote database.
- Clocks are not part of the contract. Do not depend on the wall clock.
- A held job stays held until it is cancelled. There is no unhold command.

## How to run

`./run` is executable. Each invocation is one process:

```text
./run <command> [args]
```

The process prints exactly one JSON object on stdout and nothing else. Exit 0 means the command completed. A refusal is a nonzero exit and a JSON object with a non-empty string field `error`. Extra JSON fields are allowed. Field names and values named below are required. stderr is ignored by acceptance checks.

Environment:

- `STATE_DIR` — directory for the durable store. The spooler may create it. A new process with the same `STATE_DIR` sees every record the previous process committed. The spooler chooses its own filenames inside `STATE_DIR` and must leave every other file there unchanged.
- `SERVICE_URL` — printer service base URL, with no trailing slash. Example: `http://127.0.0.1:9`.
- `CRASH_AT` — optional. When the running command reaches the named boundary, the process must kill itself with `SIGKILL` (`process.kill(process.pid, "SIGKILL")` or the equivalent uncatchable kill). If the command never reaches that boundary, it must not kill itself.

Committed means a later process with the same `STATE_DIR` can read the record after `SIGKILL`. Flush the record to disk before treating it as committed.

### Crash boundaries

`after-dispatch-saved-before-request`
On `dispatch`, after the dispatch record (dispatch id, printer id, job id, and captured label bytes) is committed, and before any byte of the print HTTP request is sent. Not on a later `dispatch` that finds the dispatch already saved.

`after-accept-before-record`
On `dispatch`, after the printer has accepted the print and this process has read the complete HTTP response body, and before the durable printed outcome is committed. Not on a lookup that only reads an acceptance performed by an earlier process.

`after-record-before-exit`
On `dispatch` or `reconcile`, after a printed outcome is committed, and before the success JSON is written and the process exits. Not on a command that found `printed` already committed and did not write it again.

## Identifiers and bytes

- Printer id: 1 to 64 characters from `A-Za-z0-9._-`.
- Job id and dispatch id: non-empty, 1 to 64 characters from `A-Za-z0-9._-`, chosen by the spooler.
- Label file: raw bytes, non-empty, at most 1 MiB. A larger or empty file is refused.
- Digest: lowercase hex SHA-256 of the raw label bytes, not of the base64 text.
- `label_b64`: standard base64 of those raw bytes.

Paths are used as given. Relative paths are relative to the process working directory.

Stored job states are `queued`, `held`, `cancelled`, `dispatch_saved`, and `printed`. `not_sent` is not a stored state.

## Commands

### `./run submit <printer_id> <label_path>`

Capture the file bytes into a new job and, on the first successful submit, bind the store to `printer_id`.

Success:

```json
{"job_id":"...","printer_id":"...","digest":"...","state":"queued"}
```

Refuse if `printer_id` is not a printer id, if it differs from the printer already bound to this store, or if the file is missing, empty, or larger than 1 MiB. A refusal must not create a job.

### `./run hold <job_id>`

Allowed only when the stored state of that job is `queued`. Commit `held`. Do not contact the printer. Do not allocate a dispatch id.

Success:

```json
{"job_id":"...","state":"held"}
```

Refuse if the job does not exist or is not `queued`. A refusal must not change that job and must not contact the printer.

### `./run cancel <job_id>`

Allowed only when the stored state of that job is `queued` or `held`. Commit `cancelled`. Do not contact the printer. Do not allocate a dispatch id.

Success:

```json
{"job_id":"...","state":"cancelled"}
```

Refuse if the job does not exist, or if its state is `dispatch_saved` or `printed`. A refusal must not change that job, must not contact the printer, and must not add a print effect.

### `./run status <job_id>`

Print the committed local record for that job. Do not contact the printer.

Success:

```json
{"job_id":"...","printer_id":"...","digest":"...","label_b64":"...","state":"queued","dispatch_id":null,"printed_digest":null}
```

`state` is the stored state. `dispatch_id` is null until a dispatch record has been saved, and is the saved id after that. `printed_digest` is the digest recorded as printed, or null when `state` is not `printed`. `label_b64` is the captured label, including after the source file has changed or been deleted. Refuse if the job does not exist.

### `./run dispatch <job_id>`

Ask the service to print the captured label for that job.

Refuse, and do not contact the service, if the job does not exist or if its state is `held` or `cancelled`.

If the state is already `printed`, succeed without contacting the service again:

```json
{"job_id":"...","printer_id":"...","dispatch_id":"...","digest":"...","state":"printed","printed_digest":"..."}
```

Otherwise the state is `queued` or `dispatch_saved`:

1. If no dispatch is saved yet, allocate a dispatch id, commit a dispatch record (`state` = `dispatch_saved`, plus dispatch id, printer id, job id, and the captured label bytes), then honor `after-dispatch-saved-before-request`.
2. If a dispatch is already saved, keep that dispatch id. Query `GET /v1/dispatches/<dispatch_id>`. If the service has accepted it, commit `printed` and do not POST again. If the service answers 404, POST the saved dispatch id.
3. POST once, as specified in the service section. After the complete response body is read, honor `after-accept-before-record`, then commit `printed` if the response digest is the captured digest. `printed_digest` is that digest.
4. Honor `after-record-before-exit`, then print the success object above.

The bytes in the POST are the captured label bytes. `dispatch` must not read the original file again. The POST uses this job id and the saved dispatch id.

If the connection fails or closes before a complete response, `dispatch` must exit nonzero with an `error` string, must leave the dispatch record in place, and must not POST a different dispatch id in that invocation. It may return without a lookup; `reconcile` will query the service. It must not report that the printer has no record unless a lookup returned HTTP 404. A later `dispatch`, with `CRASH_AT` unset, must recover as in step 2 rather than by allocating a new dispatch id.

A HTTP 400 or HTTP 409 from the service is an error for that invocation: leave the dispatch record in place and do not allocate a different dispatch id in that invocation.
### `./run reconcile <job_id>`

Query the service for the saved dispatch id and adopt a printed outcome that the local store missed.

- Unknown job: refuse.
- No dispatch saved (`queued`, `held`, or `cancelled`): exit 0 with that stored `state`, `dispatch_id` null, and `printed_digest` null. Do not contact the service. Do not POST.
- Lookup HTTP 200 with `outcome` `printed` and `digest` equal to the captured digest: commit `printed`, honor `after-record-before-exit` if this invocation committed that outcome, exit 0 with `"state":"printed"` and that digest in `printed_digest`. Do not POST.
- Lookup HTTP 404: exit 0 with `"state":"not_sent"` and the saved `dispatch_id`. Leave the stored state as `dispatch_saved`. `not_sent` means only that this lookup returned 404. It is not stored.
- Lookup HTTP 200 whose digest differs from the captured label: do not change the local outcome; exit nonzero.
- The service cannot be contacted: exit nonzero with `error`. Do not change the dispatch record. Do not report `not_sent`.

If `state` is already `printed`, `reconcile` exits 0 and leaves that outcome, `dispatch_id`, and `printed_digest` unchanged. It must not POST.

Success object:

```json
{"job_id":"...","state":"...","dispatch_id":"...","digest":"...","printed_digest":null}
```

`dispatch_id` is null only when no dispatch has been saved. `printed_digest` is null when `state` is not `printed`.

### `./run notice <notice_path>`

The file contains one JSON notice object (shape below). A well-formed notice exits 0 and prints one JSON object.

- If `state` is already `printed`, do not change `state`, `dispatch_id`, or `printed_digest`, whether the notice repeats the same outcome and digest or names a different outcome or digest.
- A notice must not cause a POST, and must not allocate a new dispatch id.
- A notice for another printer id, or for a dispatch id that is not the saved dispatch of a local job, does not change the store.
- If `state` is `dispatch_saved` and the notice names that dispatch id and printer id with `outcome` `printed` and the captured digest, the spooler may look up that dispatch id and commit the service result. It must not mark `printed` when the service has no matching acceptance.
- The spooler may ignore a notice entirely. Ignoring a well-formed notice still exits 0.

A malformed file (not one JSON object, or missing a required notice field) exits nonzero and must not change the store.

## Printer service

`SERVICE_URL` has no trailing slash. The spooler uses these routes. The same rules are in `stub/CONTRACT.md`.

`POST /v1/print` with `Content-Type: application/json`:

```json
{"dispatch_id":"...","printer_id":"...","job_id":"...","digest":"...","label_b64":"..."}
```

- If `digest` is not the lowercase hex SHA-256 of the decoded `label_b64`, the service returns HTTP 400 `{"error":"digest_mismatch"}` and prints nothing.
- The first time a `dispatch_id` is accepted, the service prints those bytes on that printer and counts one print effect. Response HTTP 200:

```json
{"dispatch_id":"...","printer_id":"...","job_id":"...","digest":"...","outcome":"printed","label_b64":"..."}
```

- The same `dispatch_id` with the same digest and the same bytes returns the stored result and does not count another effect. A differing `job_id` on that repeat does not count another effect and does not change the stored job id.
- The same `dispatch_id` with a different digest or different bytes returns HTTP 409 `{"error":"conflict"}` and does not change the stored effect.
- A different `dispatch_id` is a different physical print, even when the bytes and the job id match, and becomes the latest print for that printer.

`GET /v1/dispatches/<dispatch_id>` returns that 200 object, or HTTP 404 `{"error":"not_found"}` if that id was never accepted.

`GET /v1/printers/<printer_id>` returns the latest accepted print for that printer in the same object shape, or HTTP 404 if the printer has none.

The service may drop the HTTP reply after it has accepted and counted the effect, may answer the POST late, and may cause the same notice to be presented twice or a later notice to conflict with an outcome already recorded. Those conditions are properties of the service. The spooler does not open a listening port for notices; the operator delivers a notice by running `./run notice <file>`.

Notice file:

```json
{"notice_id":"...","dispatch_id":"...","printer_id":"...","job_id":"...","digest":"...","outcome":"printed"}
```

`outcome` is `printed` or `rejected`.

A physical print is one print effect counted by the service. Lookup by dispatch id returns the acceptance even when the HTTP client never saw the response. Lookup by printer id returns only the latest acceptance, so it is the wrong check for whether a second effect happened.

## Requirements

R1. Persistence. After `submit`, a new process with the same `STATE_DIR` sees the same job id, captured digest, captured label bytes, and stored state `queued`.

R2. Hold and cancel before dispatch. `hold` of a queued job commits `held` and a new process still sees `held`. `dispatch` of that held job is refused, leaves the job `held` with no dispatch id, and does not add a print effect. `cancel` of another queued job commits `cancelled`. `dispatch` of that cancelled job is refused and leaves it `cancelled`. `cancel` of the held job commits `cancelled`. None of these commands adds a print effect, and the printer has no accepted print afterward.

R3. Legitimate progress. With a healthy service, `submit` and then `dispatch` of that queued job each exit 0. `status` then reports `printed` and `printed_digest` equal to the digest of the bytes captured at `submit`. The bytes stored by the service for that printer equal those captured bytes, not whatever the source file contains at dispatch time. The service has counted one print effect.

R4. A printed job is not printed again. After `printed` is committed, a further `dispatch` of that job exits 0, keeps the same dispatch id and `printed_digest`, and does not add a print effect. `cancel` of that printed job is refused and does not change `state`, `dispatch_id`, or `printed_digest`, and does not add a print effect. If `dispatch` is killed at `after-record-before-exit`, a later `status` shows `printed` and the captured digest, and a further `dispatch` of that job does not add a print effect.

R5. Crash after accept. If `dispatch` is killed at `after-accept-before-record`, the service has already counted one effect for the saved dispatch id, and `status` shows `dispatch_saved` and that dispatch id. A later `dispatch` of the same job, with `CRASH_AT` unset, must end with `status` `printed` for the captured digest, must keep that same dispatch id, and must not cause the service to count a second effect.

R6. Lost reply. If the service accepts a print and drops the reply, that acceptance has already counted one effect. `dispatch` may exit nonzero. It must leave the dispatch record in place and must not POST a different dispatch id. The saved effect keeps that job id. A following `reconcile` must record `printed` for the captured digest on that same job, must not report `not_sent`, and must not add an effect. A further `reconcile` of that printed job must exit 0, leave the recorded result unchanged, and must not add an effect. `not_sent` is legal only when the dispatch lookup returns HTTP 404.

R7. Late or conflicting notice. After `state` is `printed`, a later notice for that dispatch with a different `outcome` or a different `digest` exits 0, must not change `state`, `dispatch_id`, or `printed_digest`, and must not add a print effect.

R8. Result protocol. A successful `submit` exits 0 and prints one JSON object. A refusal (for example `status` of a job id that was never submitted) exits nonzero and prints one JSON object whose `error` value is a non-empty string.

R9. Unrelated writer. A file in `STATE_DIR` written by another process, which the spooler did not create, is still present with the same bytes after later spooler commands, including `submit`, `hold`, and `dispatch` of a different job. Those commands still see the records owned by the spooler, including the held job and the printed job.

## Out of scope

Concurrent `./run` processes, a second printer in the same `STATE_DIR`, unholding a job, service rejection of a well-formed print POST (the supplied service accepts a well-formed POST), and any fleet or quota feature are outside this brief.
