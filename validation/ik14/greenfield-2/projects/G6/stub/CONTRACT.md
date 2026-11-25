# Notification provider contract

The harness starts this fake provider with `node server.mjs <ready-file>`. The provider listens on loopback and writes `{"publicPort":N}` to the ready file. `SERVICE_URL` is `http://127.0.0.1:<publicPort>`. Provider state is in memory until the provider exits.

Clients send `POST /v1/deliveries` with JSON `{ "delivery_id": string, "account_id": string, "message_id": string, "body": string, "channel": "email"|"sms", "version": positive integer }`. `delivery_id` is the exact idempotency key `<account_id>:<message_id>:v<version>`.

For a first request for an id, the provider records one effect before responding with HTTP 200 and JSON `{ "delivery_id": id, "provider_id": "provider-" + id, "delivered": true }`. A replay with the same serialized JSON object returns that same 200 response without a second effect. The provider compares the parsed object's JSON serialization, including property order. A request with the same id and a different payload returns HTTP 409 `{ "error": "conflict" }` and leaves the recorded effect unchanged. Malformed JSON returns HTTP 400 `{ "error": "invalid" }`.

`GET /v1/deliveries/<encoded-delivery-id>` performs a lookup without an effect. If an effect exists, it returns HTTP 200 with the same JSON result as POST; otherwise it returns HTTP 404 `{ "error": "not_found" }`. Other methods and paths return HTTP 404 `{ "error": "not_found" }`.

Under external fault injection, the provider may commit a POST and then close its response connection; the caller receives no HTTP status or body, while a later exact-id GET returns the committed result. It may instead keep the committed POST response pending while the caller changes preference, then return the same HTTP 200 result; a normal response may also be delayed. These are fault scenarios, not additional public routes or response shapes, and no public configuration endpoint exists. Applications MUST perform exact-id lookup to reconcile an uncertain result.
