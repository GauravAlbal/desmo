# Equipment service contract

The application receives `SERVICE_URL`. All request and response bodies use JSON with `Content-Type: application/json`. The service uses `operation_id` as the idempotency key; clients MUST keep the same key for every retry and lookup and MUST NOT create a replacement key.

## `POST /reserve`

The request body is `{operation_id, member_id, item_id}`. On a recorded grant, the service returns HTTP 200 with `{"outcome":"granted","equipment_id":"<service-assigned string>"}`. The published stub assigns `equipment_id` as `equipment-<item_id>`; clients MUST return the value from the response rather than synthesize it. On a confirmed denial, the service returns HTTP 200 with `{"outcome":"denied"}` and retains that result for the operation id; repeating the same request returns the same denial, and it allocates no equipment.

A service conflict returns HTTP 409 with `{"error":"conflict"}`. The conflicting operation is not recorded; a following lookup returns `{"found":false}`. The reservation client treats this response as terminal `rejected` and MUST NOT retry under another operation id. Invalid JSON returns HTTP 400 with `{"error":"invalid"}`.

The service may apply and record a request, then drop the response by closing the connection without an HTTP status or body. A client-side timeout or dropped response therefore does not establish that the request was not applied. A response may also be delayed while the request is being processed; when it arrives, it has the same HTTP 200 body as the corresponding recorded grant or denial. In either case, the client MUST look up the original operation id before retrying.

## `GET /operations/<operation_id>`

The path operation id is URL-decoded. If no outcome is recorded, the service returns HTTP 200 with `{"found":false}`. A recorded grant returns HTTP 200 with `{"found":true,"outcome":"granted","equipment_id":"<service-assigned string>"}`; a recorded denial returns HTTP 200 with `{"found":true,"outcome":"denied"}`. A conflict is not recorded, so its lookup has the missing-operation shape. Unknown routes return HTTP 404 with `{"error":"not_found"}`.

The published stub serves only these two routes and returns the normal idempotent grant for valid requests. The denial, conflict, dropped-response, and delayed-response outcomes above are also valid service outcomes, but cannot be selected through a public endpoint.
