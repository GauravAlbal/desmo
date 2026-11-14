# Printer service contract

The spooler calls this service at `SERVICE_URL` (no trailing slash). The runnable server is `stub/server.mjs`:

```text
node server.mjs
```

It listens on `127.0.0.1` and prints one JSON line on stdout, then serves until killed:

```json
{"service_url":"http://127.0.0.1:PORT","control_url":"http://127.0.0.1:CPORT"}
```

State is in memory for that process only. Restarting the server drops every print.

## Data plane

### `POST /v1/print`

`Content-Type: application/json`

```json
{"dispatch_id":"...","printer_id":"...","job_id":"...","digest":"...","label_b64":"..."}
```

`dispatch_id`, `printer_id`, and `job_id` are non-empty strings. `digest` is lowercase hex SHA-256 of the raw bytes decoded from `label_b64`.

Responses:

- HTTP 400 `{"error":"bad_request"}` if a required field is missing or not a string. Nothing is stored.
- HTTP 400 `{"error":"digest_mismatch"}` if the digest does not match the bytes. Nothing is stored.
- HTTP 200 if this `dispatch_id` is new, or if it was already accepted with the same digest and the same bytes:

```json
{"dispatch_id":"...","printer_id":"...","job_id":"...","digest":"...","outcome":"printed","label_b64":"..."}
```

The first acceptance stores the bytes as the latest print for that printer and counts one print effect. A repeat of the same id and the same bytes returns the stored result and does not count another effect. A differing `job_id` on that repeat does not count another effect and does not change the stored job id.

- HTTP 409 `{"error":"conflict"}` if this `dispatch_id` was already accepted with a different digest or different bytes. The stored effect is unchanged.

A different `dispatch_id` always counts as another physical print, even if the bytes and the job id are identical, and becomes the latest print for that printer.

### `GET /v1/dispatches/<dispatch_id>`

HTTP 200 with the stored print object, or HTTP 404 `{"error":"not_found"}` if that id was never accepted.

### `GET /v1/printers/<printer_id>`

HTTP 200 with the latest accepted print for that printer, or HTTP 404 `{"error":"not_found"}` if none.

Ids in paths are from `A-Za-z0-9._-`, so they need no extra encoding. Other characters are not part of the spooler contract.

## Notice shape

The service does not call the spooler. An operator (or an acceptance harness) delivers this JSON in a file:

```json
{"notice_id":"...","dispatch_id":"...","printer_id":"...","job_id":"...","digest":"...","outcome":"printed"}
```

`outcome` is `printed` or `rejected`. The same object may be delivered more than once. A later object may name a different `outcome` or `digest` for a dispatch the service already accepted.

On each new effect the server keeps one notice body, available from the control port, with `notice_id` set to `nt-` plus the dispatch id and `outcome` `printed`. That list is a convenience for the harness. The spooler never reads the control port.

## Idempotency and lookup

Identity of an effect is `dispatch_id`. Lookup by dispatch id returns the acceptance even when the HTTP client never saw the response. Lookup by printer id returns only the latest acceptance, so it is the wrong check for whether a second effect happened.

## Evaluator fault injection

This section is for the acceptance harness, not for the spooler. The spooler brief does not describe these controls. The spooler is only required to cope with a dropped reply, a late reply, and a notice delivered twice or in conflict with a recorded outcome.

The control base URL is `control_url` from the startup line. Faults apply only to `POST /v1/print` on the data plane. `GET` routes are not delayed or dropped.

`POST /fault` with JSON, for example `{"drop_reply":true,"delay_ms":0}`.

- `drop_reply`: after the effect is stored, the server destroys the socket and writes no HTTP response. A repeat POST of the same id is still stored only once; if `drop_reply` is still set, that repeat is also dropped.
- `delay_ms`: non-negative integer. After the effect is stored, the server waits this many milliseconds before the response or the drop.

`GET /effects` returns `{"count":N,"effects":[{"dispatch_id","printer_id","job_id","digest","label_b64"}]}` in acceptance order.

`GET /notices` returns `{"notices":[...]}` for effects this process accepted.

`GET /health` returns `{"ok":true}`.

The harness may also hand the spooler a notice file the server never stored (a conflicting or duplicate delivery). That does not require a control-port call.
