# Destination stub

The stub is a single Node process. It stores deliveries in memory. Restarting it clears every delivery. Builders use the public port only. The evaluator uses the control port to inject faults. The import-desk brief does not name the control port; the desk must not call it.

## Start

```text
node server.mjs <ready-file>
```

When both ports are listening on `127.0.0.1`, the process writes one JSON object to `ready-file` and to stdout:

```json
{"publicPort":1234,"controlPort":1235}
```

Public base URL: `http://127.0.0.1:<publicPort>`. Control base URL: `http://127.0.0.1:<controlPort>`.

## Public API

### `POST /v1/deliveries`

```json
{"idempotency_key":"imp1","record":"alpha"}
```

Optional `callback_url` is ignored by the effect rules. If present, fault modes below may POST a notification to it. The desk is not required to send `callback_url` or to listen for notifications.

Responses:

- `200` `{"outcome":"stored","destination_id":"dst_1","idempotency_key":"imp1"}` — the record is stored. `destination_id` is allocated on the first store and reused for every later identical POST.
- `422` `{"outcome":"rejected","error":"rejected","reason":"invalid_record"}` — the destination rejected the attempt. In the default reject fault, nothing is stored. In the conflicting-reply fault, the record is stored and the body still says rejected.
- `409` `{"outcome":"conflict","error":"conflict"}` — this key is already stored with a different `record`. The stored row is unchanged. No second effect is created.
- `400` — body is not the required JSON, or `idempotency_key` or `record` is missing or empty. Nothing is stored.

### `GET /v1/deliveries/<idempotency_key>`

- `200` `{"found":true,"idempotency_key":"imp1","record":"alpha","destination_id":"dst_1"}` if that key was stored, including when the POST reply was dropped or has not been released yet.
- `404` `{"found":false}` if it was never stored.
- `500` `{"error":"unavailable"}` when the evaluator has disabled lookup. Nothing about storage changes.

Idempotency: the effect is the stored row, not the HTTP call. Same key and same record: one row, same `destination_id`, later POSTs do not add a row. Same key and different record: `409`, original row kept. A new key is a new row. Lookup sees a row as soon as the POST handler has stored it, even if the client never receives the POST reply.

## Control API

All control routes are on the control port. They are how the evaluator injects faults. They are not part of the desk's job.

`POST /control/config` replaces the fault configuration. Omitted flags reset to the defaults (`false`, `0`, or `null`).

| Field | Effect |
| --- | --- |
| `drop_reply` | Perform the POST outcome (store, reject, or conflict), then destroy the socket without a response. |
| `hold` | Do not send the POST response until `POST /control/release`. |
| `hold_key` | With `hold`, hold only POSTs whose `idempotency_key` equals this string. Other POSTs answer immediately. `null` holds every POST. |
| `delay_ms` | Wait this many milliseconds before sending the POST response, unless the reply is dropped or held. |
| `reject` | Do not store. Respond `422` with the rejection body (unless the reply is dropped or held). |
| `late_conflict` | Store the row (unless it is a record conflict or `reject` / `force_status` applies), but respond `422` with the rejection body. Lookup still finds the row. |
| `duplicate_reply` | After the POST response is sent, POST that same JSON body to `callback_url` twice if the request included one. |
| `callback_conflict` | After the POST response is sent, POST a rejection body to `callback_url` if the request included one. |
| `fail_lookup` | `GET /v1/deliveries/<key>` returns 500. Storage is unchanged. |
| `force_status` | Nonzero HTTP status for POST, body `{"outcome":"error","error":"unavailable"}`, and do not store. |

`reject` wins over store. `force_status` wins over store and over `reject`. `drop_reply` and `hold` apply to whatever response would have been sent. `duplicate_reply` and `callback_conflict` run only when a response is actually written, including when a held response is released. If those flags are set and the request has no `callback_url`, the control state records a skipped notification and sends nothing.

`POST /control/release` sends every held POST response and returns `{"released":<count>}`.

`POST /control/reset` clears rows, the request log, held responses (without sending them), and faults.

`POST /control/seed` with `{"idempotency_key":"...","record":"..."}` inserts one row without counting as a client POST. `409` if the key exists.

`GET /control/state` returns:

```json
{
  "config": {},
  "effects": [{"idempotency_key":"imp1","record":"alpha","destination_id":"dst_1"}],
  "requests": [{"method":"POST","path":"/v1/deliveries","idempotency_key":"imp1","record":"alpha","status":200}],
  "held": 0,
  "callback_notes": []
}
```

GET lookups are included in `requests` with `method` `"GET"`. Seeded rows appear in `effects` and do not add a request.

The desk brief's only fault statement is: the service may drop a reply after acting, answer late, or answer twice, and a reply may disagree with lookup. Those behaviors are `drop_reply`, `hold` plus `release` (and `delay_ms`), `duplicate_reply`, and `late_conflict`. The other switches exist so the evaluator can separate a confirmed rejection, an unavailable lookup, and an unexpected status from a dropped reply.
