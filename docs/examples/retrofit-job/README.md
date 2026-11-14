# Retrofit: one job lifecycle

Migration unit: **one coherent lifecycle**, not a file.

Existing jobs finish under the old implementation. Newly created jobs belong
to Desmo. Never both owners for the same aggregate.

## Boundary

- Canonical fields: job state, generation, cancellation.
- Writers: API, worker, admin cancel. List them; a repo scan is not proof.
- External effects (emails, webhooks) stay host-owned.
- Store: local SQLite for Desmo. Application DB is a **read projection**.
  A Desmo commit and an application-database update are **not** one atomic
  transaction. Crash between them requires reconciliation, not a wrapper that
  hides the split.

## Preserve / prohibit / unresolved

| Kind | Example |
|---|---|
| Preserve | Newly created jobs start in `created`. |
| Prohibit | Late worker `submit` after `cancel`. |
| Unresolved | Historical jobs — do not fabricate journal rows. |

## Cutover

Route by an explicit ownership rule (`owner = desmo | legacy`). Rolling back
application code must **not** silently return authority to the old lifecycle.
Presentation rollback is allowed while Desmo remains in charge.

## Executable cutover

`python/tests/test_retrofit_jobs.py` runs new-aggregate adoption: legacy jobs
finish in the host map; new jobs are Desmo-owned; a late submit after cancel
is refused and `show` stays cancelled.

Broader failure rehearsal (concurrent commands, lost response, process death,
unresolved effect) remains a checklist, not coverage in this directory.


Do not insert rows into kernel tables. Snapshot/import of old aggregates is a
separate, validated procedure — not implied by this example.
