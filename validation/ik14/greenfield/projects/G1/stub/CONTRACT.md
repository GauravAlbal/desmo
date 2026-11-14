# Render worker contract

This program is the fake worker. Builders call the public HTTP API. The control port is for the evaluator only. The brief describes worker faults only as: the worker may drop a reply, respond late, or report a result more than once, including a conflicting output. It does not describe this control port.

Start the public API with `PORT=8080 node server.mjs`. `SERVICE_URL` is `http://127.0.0.1:8080`. If `CONTROL_PORT` is unset, only the public API starts. State lives in memory for the life of the process.

Successful output strings are deterministic: `out:` concatenated with the `submission_id`.

## Public API

### POST /v1/renders

Request JSON object with required fields `submission_id` (string), `job_id` (string), `input_ref` (string), and `revision` (number).

Example: `{"submission_id":"job1:r1","job_id":"job1","input_ref":"media/clip","revision":1}`

* A new `submission_id` admits one render. That admission is the external effect.
* Repeating POST with the same `submission_id` returns the current record and does not admit another render, even if other fields differ.
* Another `submission_id` is a new render, even for the same `job_id`.
* `submission_id` is an opaque idempotency key. The supervisor must use the form required by the brief (`{job_id}:r{revision}`). The worker does not enforce that form.
* Unless the evaluator is holding completions, the admitted render is already `succeeded` when the response is produced, with `output_ref` set to `out:` plus `submission_id`.
* While completions are held, the admitted render stays `state` `rendering` and `output_ref` null until the evaluator completes it.

Response 200 uses the same fields plus `state`, `output_ref`, and `progress`. `state` is `rendering`, `succeeded`, or `failed`. Service `progress` is `rendering`, `done`, or `failed`. `output_ref` is a string when `state` is `succeeded`, otherwise null.

Example success body: `{"submission_id":"job1:r1","job_id":"job1","input_ref":"media/clip","revision":1,"state":"succeeded","output_ref":"out:job1:r1","progress":"done"}`

Malformed JSON or missing fields returns 400 and `{"error":"invalid"}`.

### GET /v1/renders/<submission_id>

Lookup of one admission. The path segment is URL-encoded. A colon may appear as `%3A`.

* 200 with the same object shape as POST if that `submission_id` was admitted, including when the POST reply was dropped.
* 404 and `{"error":"not_found"}` if it was never admitted.
* Lookup does not admit a render and does not change the record.

Use lookup to recover a lost reply. Do not invent a new `submission_id` because a reply was missing.

### GET /v1/callbacks?job_id=<job_id>

Ordered result reports the worker has produced. The body is `{"events":[...]}`. Each event has `event_id`, `submission_id`, `job_id`, `revision`, `state`, `output_ref`, and `seq`.

`seq` increases for each report. The worker may append a duplicate report or a later report with a different `output_ref` for the same `submission_id`. Reports for an older `submission_id` remain after a newer submission exists. There is no cancel operation. A local cancellation does not remove admissions or stop a later report.

Applying reports is the supervisor's job. The safe rule is in the brief: ignore a report whose `submission_id` is not the job's current one, and do not let any report change a terminal job.

## Dropped replies, lateness, duplicates

The public API does not document how to trigger faults. Behavior builders must tolerate:

* The worker may admit the render and then drop the TCP connection without a response body.
* The worker may wait before sending the response. The admission is recorded before that wait.
* Lookup and the callback list can show a result that the original caller never received.
* The callback list can contain two reports for one admission, including a later conflicting `output_ref`.
* Response headers are written only after the admission is recorded. A caller that has seen response headers can rely on lookup finding that admission.

GET is not dropped and is not delayed.

## Evaluator control port

Not part of the supervisor interface. Bind address is `127.0.0.1` and the port is `CONTROL_PORT`.

* `GET /health` returns `{"ok":true}`.
* `GET /effects` returns `{"count":N,"items":[...]}` in admission order. Each item has `submission_id`, `job_id`, `input_ref`, and `revision`. `count` is the number of distinct admissions, not the number of POST attempts.
* `GET /callbacks?job_id=` returns the same event list as the public callback route.
* `POST /fault` accepts a JSON object. Any subset of fields may be set.
* `drop_replies` (number): how many subsequent POST /v1/renders responses to destroy after the admission is recorded. GET is unaffected. Idempotent repeats still consume a drop and still do not create a second admission.
* `delay_ms` (number): wait this long after recording a POST admission, before the response or the drop. Zero clears the wait.
* `delay_job_id` (string or null): if set, only POSTs for that `job_id` wait. Null waits every POST.
* `hold` (boolean): when true, new admissions stay `rendering` until `complete`. When false, new admissions are created already succeeded.
* `complete` (string submission_id): mark that admission succeeded with `output_ref` equal to `out:` plus the id, and append a callback. Unknown id returns 404.
* `conflict` is an object with `submission_id` and `output_ref`. It sets that admission's current output, forces succeeded, and appends a callback. Unknown id returns 404.

Faults do not rewrite the supervisor's store. They only change what the worker will say next.
