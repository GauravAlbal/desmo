# Device service contract

The desk calls this service at `SERVICE_URL` (no trailing slash). The runnable server is `stub/server.mjs`:

```text
node server.mjs
```

It listens on `127.0.0.1` and prints one JSON line on stdout, then serves until killed:

```json
{"service_url":"http://127.0.0.1:PORT","control_url":"http://127.0.0.1:CPORT"}
```

State is in memory for that process only. Restarting the server drops every install.

## Data plane

### `POST /v1/install`

`Content-Type: application/json`

```json
{"request_id":"...","device_id":"...","digest":"...","config_b64":"..."}
```

`request_id` and `device_id` are non-empty strings. `digest` is lowercase hex SHA-256 of the raw bytes decoded from `config_b64`.

Responses:

- HTTP 400 `{"error":"digest_mismatch"}` if the digest does not match the bytes, or if a required field is missing. Nothing is stored.
- HTTP 200 if this `request_id` is new, or if it was already accepted with the same digest and the same bytes:

```json
{"request_id":"...","device_id":"...","digest":"...","outcome":"installed","config_b64":"..."}
```

The first acceptance stores the bytes as the latest install for that device and counts one installation effect. A repeat of the same id and the same bytes returns the stored result and does not count another effect.

- HTTP 409 `{"error":"conflict"}` if this `request_id` was already accepted with a different digest or different bytes. The stored effect is unchanged.

A different `request_id` always counts as another effect, even if the bytes are identical, and becomes the latest install for that device.

### `GET /v1/requests/<request_id>`

HTTP 200 with the stored install object, or HTTP 404 `{"error":"not_found"}` if that id was never accepted.

### `GET /v1/devices/<device_id>`

HTTP 200 with the latest accepted install for that device, or HTTP 404 `{"error":"not_found"}` if none.

Ids in paths are from `A-Za-z0-9._-`, so they need no extra encoding. Other characters are not part of the desk contract.

## Callback shape

The service does not call the desk. An operator (or an acceptance harness) delivers this JSON in a file:

```json
{"callback_id":"...","request_id":"...","device_id":"...","digest":"...","outcome":"installed"}
```

`outcome` is `installed` or `rejected`. The same object may be delivered more than once. A later object may name a different `outcome` or `digest` for a request the service already accepted.

On each new effect the server keeps one callback body, available from the control port, with `callback_id` set to `cb-` plus the request id and `outcome` `installed`. That list is a convenience for the harness. The desk never reads the control port.

## Idempotency and lookup

Identity of an effect is `request_id`. Lookup by request id returns the acceptance even when the HTTP client never saw the response. Lookup by device id returns only the latest acceptance, so it is the wrong check for whether a second effect happened.

## Evaluator fault injection

This section is for the acceptance harness, not for the desk. The desk brief does not describe these controls. The desk is only required to cope with a dropped reply, a late reply, and a callback delivered twice or in conflict with a recorded outcome.

The control base URL is `control_url` from the startup line. Faults apply only to `POST /v1/install` on the data plane. `GET` routes are not delayed or dropped.

`POST /fault` with JSON, for example `{"drop_reply":true,"delay_ms":0}`.

- `drop_reply`: after the effect is stored, the server destroys the socket and writes no HTTP response. A repeat POST of the same id is still stored only once; if `drop_reply` is still set, that repeat is also dropped.
- `delay_ms`: non-negative integer. After the effect is stored, the server waits this many milliseconds before the response or the drop.

`GET /effects` returns `{"count":N,"effects":[{"request_id","device_id","digest","config_b64"}]}` in acceptance order.

`GET /callbacks` returns `{"callbacks":[...]}` for effects this process accepted.

`GET /health` returns `{"ok":true}`.

The harness may also hand the desk a callback file the server never stored (a conflicting or duplicate delivery). That does not require a control-port call.
