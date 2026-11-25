# G6 evaluator fixture contract

This document describes the private evaluator fixture under `evaluator/fixture/`. It is not a builder input. Builders receive `brief.md`, `change.md` when applicable, `stub/CONTRACT.md`, `stub/server.mjs`, and the arm-specific kit; they do not receive this directory or its private routes.

## Public service surface

`server.mjs <ready-file>` starts a loopback public service and a separate loopback control sidecar. The ready file and startup stdout are `{"publicPort":N,"controlPort":N}`. The evaluator gives only `http://127.0.0.1:<publicPort>` to the submission as `SERVICE_URL`; the `controlPort` is private.

| Request | Fixture response / effect | Difference from published `stub/server.mjs` |
|---|---|---|
| `POST /v1/deliveries`, valid new id and payload | Record one input/effect, then HTTP 200 JSON `{delivery_id,provider_id:"provider-"+delivery_id,delivered:true}` | Same request, response, commit-before-response order, and idempotent effect behavior. Fixture additionally appends a request trace. |
| Same POST id and same serialized JSON payload | HTTP 200 with the recorded result; no second effect | Same response and effect behavior. Equality uses `JSON.stringify` on the parsed object, so property order participates in equality. Fixture additionally appends a request trace. |
| Same POST id with a different serialized JSON payload | HTTP 409 JSON `{error:"conflict"}`; existing effect is unchanged | Same status/body/effect behavior. The fixture returns before consuming the configured drop/hold/delay for that request. |
| POST with malformed JSON | HTTP 400 JSON `{error:"invalid"}` | Same status/body. |
| `GET /v1/deliveries/<encoded-id>` for a recorded id | Decode the suffix and return HTTP 200 with the recorded POST result; no effect | Same lookup and response. Fixture additionally records `{method:"GET",delivery_id}`. |
| GET for an unrecorded id | HTTP 404 JSON `{error:"not_found"}`; no effect | Same status/body. |
| Any other method/path | HTTP 404 JSON `{error:"not_found"}` | Same status/body. |

All JSON responses use `content-type: application/json` and an explicit content length. As with the published stub, the fixture does not validate the types or presence of fields in otherwise parseable JSON; client requests covered by the contract use the declared delivery schema.

The fixture has no public fault-control route. Its private test configuration affects every valid non-conflicting POST after effect lookup/creation: `drop_next` decrements and destroys the response connection after the effect is recorded or retrieved; otherwise `hold:true` leaves the response pending; otherwise a nonzero `delay_ms` waits before the ordinary HTTP 200. A held response returns the same recorded HTTP 200 result when released. A dropped response has no HTTP status or body, but its effect remains available to the GET route. These settings do not change request or JSON response shapes.

## Private control sidecar

The evaluator uses the sidecar's loopback HTTP port only; the submission never receives it. JSON responses use `content-type: application/json` and an explicit content length. Routes match on the URL pathname; query parameters do not change routing.

| Request | Response |
|---|---|
| `GET /control/health` | HTTP 200 `{"ok":true}` |
| `GET /control/state` | HTTP 200 `{"requests":[...],"effects":[...]}`. A POST trace is the parsed request object with `method:"POST"`; a GET trace is `{"method":"GET","delivery_id":string}`. `effects` is an array of the stored POST input objects. |
| `POST /control/config` with a valid JSON body (empty means `{}`) | HTTP 200 `{"ok":true}`. For object bodies, supplied `drop_next`, `delay_ms`, and `hold` values shallow-overlay the current settings; values are not validated. Setting `hold:false` releases all held responses. |
| `POST /control/release` with a valid JSON body (empty means `{}`) | HTTP 200 `{"released":number}` and release every held response. |
| `POST /control/reset` with a valid JSON body (empty means `{}`) | HTTP 200 `{"ok":true}`; clear effects, request traces, and held responses; restore `drop_next:0`, `delay_ms:0`, `hold:false`. |
| Malformed JSON on a POST control route | HTTP 400 `{"error":"invalid"}` |
| Any other method/path | HTTP 404 `{"error":"not_found"}` |

The parent fixture and sidecar use private IPC: startup sends `{"ready":true,"port":N}`; evaluator requests send `{"id":N,"action":"state"|"reset"|"config"|"release","value":object}`; replies are `{"replyTo":N,"value":object}`. IPC is not part of `SERVICE_URL` or the builder contract.

## Endpoint parity summary

For `POST /v1/deliveries`, `GET /v1/deliveries/<encoded-id>`, malformed POST JSON, conflicts, misses, and unmatched methods/paths, the fixture and published stub return the same HTTP statuses and JSON bodies and preserve the same effect semantics. The only public-service differences are fixture-only request tracing and the evaluator-injected POST connection faults above. Startup readiness differs only because the private fixture adds `controlPort`; both public URL fields identify the same loopback service contract. The sidecar routes, traces, settings, and IPC have no counterpart in the published stub and are never staged.
