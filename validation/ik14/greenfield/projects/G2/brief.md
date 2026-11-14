# Configuration rollout desk

A local desk prepares one configuration for one device, presents that configuration for approval, and asks a device service to install it. The bytes the service installs must be the bytes that were approved. The desk keeps its records in a single durable store on this machine. It does not coordinate a fleet, a quota, or a cross-device transaction.

You own the program behind `./run`. The device service is supplied. Its HTTP API is specified below and, with the same rules, in `stub/CONTRACT.md`.

## Scope

- One machine, one state directory, one desk process at a time.
- One device id for the life of that state directory. A second device id is refused.
- One proposal in that state directory. Starting over means a new state directory.
- The operator does not run two `./run` processes at once. An unrelated process may write other files in the state directory between commands, and may do so while a command runs.
- No fleet-wide transaction, no cross-device budget, no remote database.
- Clocks are not part of the contract. Do not depend on the wall clock.

## How to run

`./run` is executable. Each invocation is one process:

```text
./run <command> [args]
```

The process prints exactly one JSON object on stdout and nothing else. Exit 0 means the command completed. A refusal is a nonzero exit and a JSON object with a non-empty string field `error`. Extra JSON fields are allowed. Field names and values named below are required. stderr is ignored by acceptance checks.

Environment:

- `STATE_DIR` — directory for the durable store. The desk may create it. A new process with the same `STATE_DIR` sees every record the previous process committed. The desk chooses its own filenames inside `STATE_DIR` and must leave every other file there unchanged.
- `SERVICE_URL` — device service base URL, with no trailing slash. Example: `http://127.0.0.1:9`.
- `CRASH_AT` — optional. When the running command reaches the named boundary, the process must kill itself with `SIGKILL` (`process.kill(process.pid, "SIGKILL")` or the equivalent uncatchable kill). If the command never reaches that boundary, it must not kill itself.

Committed means a later process with the same `STATE_DIR` can read the record after `SIGKILL`. Flush the record to disk before treating it as committed.

### Crash boundaries

`after-attempt-saved-before-request`
On `install`, after the attempt record (request id, device id, and approved bytes) is committed, and before any byte of the install HTTP request is sent. Not on a later `install` that finds the attempt already saved.

`after-accept-before-record`
On `install`, after the service has accepted the install and this process has read the complete HTTP response body, and before the durable outcome is committed. Not on a lookup that only reads an acceptance performed by an earlier process.

`after-record-before-exit`
On `install` or `reconcile`, after a terminal outcome (`installed` or `rejected`) is committed, and before the success JSON is written and the process exits.

## Identifiers and bytes

- Device id: 1 to 64 characters from `A-Za-z0-9._-`.
- Proposal id and request id: non-empty, 1 to 64 characters from `A-Za-z0-9._-`, chosen by the desk.
- Configuration file: raw bytes, non-empty, at most 1 MiB. A larger or empty file is refused.
- Digest: lowercase hex SHA-256 of the raw configuration bytes, not of the base64 text.
- `bytes_b64` / `config_b64`: standard base64 of those raw bytes.

Paths are used as given. Relative paths are relative to the process working directory.

## Commands

### `./run prepare <device_id> <config_path>`

Capture the file bytes into a new proposal and bind the store to `device_id`.

Success:

```json
{"proposal_id":"...","device_id":"...","digest":"...","approval":"none","install":"none"}
```

Refuse if a proposal already exists, if `device_id` differs from the device already bound to this store, or if the file is missing, empty, or larger than 1 MiB.

### `./run show`

Present the captured proposal for approval.

Success:

```json
{"proposal_id":"...","device_id":"...","digest":"...","bytes_b64":"...","approval":"none","approved_digest":null,"install":"none"}
```

`approval` is `none` or `approved`. `approved_digest` is the approved digest, or `null` when approval is `none`. `install` is `none`, `attempt_saved`, `installed`, or `rejected`. Refuse if no proposal exists.

### `./run approve <proposal_id> <digest>`

Approve the presented proposal only when `proposal_id` is the current proposal and `digest` equals the captured bytes. Success:

```json
{"proposal_id":"...","digest":"...","approval":"approved","approved_digest":"..."}
```

Refuse if the id or digest does not match. A refused approve must not record an approval.

### `./run install`

Ask the service to install the approved captured bytes.

Refuse, and do not contact the service, if the proposal is not approved or if `approved_digest` is not the digest of the captured bytes.

If the outcome `installed` is already committed for those bytes, succeed without contacting the service again:

```json
{"proposal_id":"...","device_id":"...","request_id":"...","digest":"...","install":"installed","installed_digest":"..."}
```

Otherwise:

1. If no attempt is saved yet, allocate a request id, commit an attempt record (`install` = `attempt_saved`, plus request id, device id, and the approved bytes), then honor `after-attempt-saved-before-request`.
2. If an attempt is already saved, keep that request id. Query `GET /v1/requests/<request_id>`. If the service has accepted it, commit that outcome and do not POST again. If the service answers 404, POST the saved request id.
3. POST once, as specified in the service section. After the complete response body is read, honor `after-accept-before-record`, then commit `installed` if the response digest is the approved digest.
4. Honor `after-record-before-exit`, then print the success object above.

The bytes in the POST are the captured approved bytes. `install` must not read the original file again.

If the connection fails or closes before a complete response, `install` must exit nonzero with an `error` string, must leave the attempt record in place, and must not POST a different request id in that invocation. It may return without a lookup; `reconcile` will query the service. It must not report that the service has no record unless a lookup returned HTTP 404.

### `./run status`

Print the committed local record. Do not contact the service. On an empty store, exit 0 with null ids and `install` `none`:

```json
{"device_id":null,"proposal_id":null,"digest":null,"approval":"none","approved_digest":null,"request_id":null,"install":"none","installed_digest":null}
```

After work exists, the same fields are present. `installed_digest` is the digest of the bytes recorded as installed, or `null` when `install` is not `installed`.

### `./run reconcile`

Query the service for the saved request id and adopt a terminal outcome that the local store missed.

- No attempt saved: exit 0, `{"install":"none","request_id":null,"digest":null,"installed_digest":null}`.
- Lookup HTTP 200 with `outcome` `installed` and `digest` equal to the captured proposal digest: commit `installed`, honor `after-record-before-exit` if this invocation committed that outcome, exit 0 with `"install":"installed"` and that digest in `installed_digest`. Do not POST.
- Lookup HTTP 404: exit 0 with `"install":"not_sent"` and the saved `request_id`. Leave the attempt record in place. `not_sent` means only that this lookup returned 404.
- Lookup HTTP 200 whose digest differs from the captured proposal: do not change the local outcome; exit nonzero.
- The service cannot be contacted: exit nonzero with `error`. Do not change the attempt record. Do not report `not_sent`.

If `install` is already `installed` or `rejected`, `reconcile` exits 0 and leaves that outcome and `installed_digest` unchanged. It must not POST.

### `./run callback <callback_path>`

The file contains one JSON callback object (shape below). A well-formed callback exits 0.

- If `install` is already `installed` or `rejected`, do not change `install` or `installed_digest`, whether the callback repeats the same outcome and digest or names a different outcome or digest.
- A callback must not cause a POST, and must not allocate a new request id.
- A callback for another device id, or for a request id that is not the saved attempt, does not change the store.
- If `install` is `attempt_saved` and the callback names that request id and device id with `outcome` `installed` and the approved digest, the desk may look up that request id and commit the service result. It must not mark `installed` when the service has no matching acceptance.
- The desk may ignore a callback entirely. Ignoring a well-formed callback still exits 0.

A malformed file (not one JSON object, or missing a required callback field) exits nonzero.

## Device service

`SERVICE_URL` has no trailing slash. The desk uses these routes. The same rules are in `stub/CONTRACT.md`.

`POST /v1/install` with `Content-Type: application/json`:

```json
{"request_id":"...","device_id":"...","digest":"...","config_b64":"..."}
```

- If `digest` is not the lowercase hex SHA-256 of the decoded `config_b64`, the service returns HTTP 400 `{"error":"digest_mismatch"}` and installs nothing.
- The first time a `request_id` is accepted, the service installs those bytes on that device and counts one installation effect. Response HTTP 200:

```json
{"request_id":"...","device_id":"...","digest":"...","outcome":"installed","config_b64":"..."}
```

- The same `request_id` with the same digest and the same bytes returns the same result and does not count another effect.
- The same `request_id` with a different digest or different bytes returns HTTP 409 `{"error":"conflict"}` and does not change the stored effect.
- A different `request_id` is a different effect, even when the bytes match, and becomes the latest install for that device.

`GET /v1/requests/<request_id>` returns that 200 object, or HTTP 404 `{"error":"not_found"}` if that id was never accepted.

`GET /v1/devices/<device_id>` returns the latest accepted install for that device in the same object shape, or HTTP 404 if the device has none.

The service may drop the HTTP reply after it has accepted and counted the effect, may answer the POST late, and may cause the same callback to be presented twice or a later callback to conflict with an outcome already recorded. Those conditions are properties of the service. The desk does not open a listening port for callbacks; the operator delivers a callback by running `./run callback <file>`.

Callback file:

```json
{"callback_id":"...","request_id":"...","device_id":"...","digest":"...","outcome":"installed"}
```

`outcome` is `installed` or `rejected`.

## Requirements

R1. Persistence. After `prepare` or any later committed record, a new process with the same `STATE_DIR` sees the same proposal, digest, approval, attempt, and outcome. A committed record survives `SIGKILL`.

R2. Prepare and present. `prepare` captures the file bytes. A later `show` returns those exact bytes and their digest. Changing or deleting the source file after `prepare` does not change the presented bytes.

R3. Approval binds the presented digest. `approve` succeeds only for the current proposal id and the captured digest, and it records that digest. A wrong digest is refused and leaves approval unset.

R4. Installed bytes match the approved configuration. After a successful install against a healthy service, `status` reports `install` `installed` and `installed_digest` equal to the approved digest. The bytes stored by the service for that device equal the captured approved bytes, not whatever the source file contains at install time.

R5. Legitimate progress. With a healthy service, `prepare`, `approve` of the presented digest, and `install` each exit 0, and `status` then reports `installed`. A valid sequence must not be refused.

R6. Repeated callback. After a successful install, delivering the same well-formed installed callback twice exits 0 both times, leaves `install` and `installed_digest` unchanged, and does not add an installation effect.

R7. Late or conflicting callback. After `install` is `installed`, a later callback for that request with a different `outcome` or a different `digest` must not change `install` or `installed_digest`, and must not add an installation effect.

R8. Crash after accept. If `install` is killed at `after-accept-before-record`, the service has already counted one effect for the saved request id. A later `install`, with `CRASH_AT` unset, must end with `status` `installed` for the approved digest, must not say the request was never sent, and must not cause the service to count a second effect or a different request id.

R9. Crash before request. If `install` is killed at `after-attempt-saved-before-request`, the service has counted no effect, and `status` shows `attempt_saved` and the saved request id. A later `install`, with `CRASH_AT` unset, must reach `installed` and the service must count exactly one effect.

R10. Crash after the durable record. If `install` is killed at `after-record-before-exit`, a later `status` shows `installed` and the approved digest, and a further `install` must not add an effect.

R11. Single device. The first successful `prepare` binds the store to that device id. A `prepare` for any other device id is refused, and the bound proposal remains. The desk has no command that installs more than one device.

R12. Result protocol. A successful `prepare` exits 0 and prints one JSON object. A refusal (for example `approve` with no proposal) exits nonzero and prints one JSON object whose `error` value is a non-empty string.

R13. Unrelated writer. A file in `STATE_DIR` written by another process, which the desk did not create, is still present with the same bytes after later desk commands. Those commands still see the records owned by the desk.

R14. Lost reply. If the service accepts an install and drops the reply, that acceptance has already counted one effect. `install` may exit nonzero. It must leave the attempt record in place and must not POST a different request id. A following `reconcile` must record `installed` for the approved digest, must not report `not_sent`, and must not add an effect. `not_sent` is legal only when the request lookup returns HTTP 404.

## Out of scope

A second rollout in the same `STATE_DIR`, concurrent `./run` processes, service rejection of a well-formed install POST (the supplied service accepts a well-formed POST), and any fleet or budget feature are outside this brief.
