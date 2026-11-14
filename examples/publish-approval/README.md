# Protected law: approval binds the artifact

Rule: a publish authorization must reference the artifact that was approved.

Reasonable feature: edit an approved artifact without repeating the workflow.

Unprotected host map: keep `approved=true`, replace the digest, publish.

Desmo: `revise` returns the document to `draft`. `publish` from a stale
approved handle is refused. Approve the new digest, then publish succeeds.

Authoritative declaration: `machines/publish_approval.ik.toml`.
Inventory: `ik laws machines/publish_approval.ik.toml` (advisory).
Tests: `python/tests/test_publish_approval.py`.

This refuses **kernel publication authorization**. It does not intercept an
external publishing API called outside the boundary.

## Complete worked example (IK-09)

One self-contained app around immutable bytes:

- `approval_app.py`: content-addressed `ArtifactStore`, a mutable `DraftStore` the executor
  never reads, `ApprovalApp`, `VersionRouter`, `Projection`, and a `Trace` recorder.
- `machines/publish_approval_v3.ik.toml`: publication is an effect bound to the approved digest
  (`authorize_publish` -> executor -> `record` -> `complete`).
- `machines/publish_approval_defective.ik.toml`: same law `pub_inv_001`, but `revise` keeps
  approval. `ik check` refuses it with a bounded witness (`edit approve revise publish`) and the
  runtime refuses `publish` with `IK_INVARIANT_FAILED`.
- `machines/publish_approval_all_refuse.ik.toml`: `approve` can never fire; `ik check` refuses it
  and `release_refused` is reported.
- `scenarios.py`: executable runner. `python3 examples/publish-approval/scenarios.py --out DIR`
  prints every observed code and writes `observed.trace.json` and `witness.trace.json`;
  render with `desmo trace DIR/observed.trace.json --out observed.html`.

```
ik check examples/publish-approval/machines/publish_approval_defective.ik.toml \
  --corpus examples/publish-approval/machines/publish_approval_defective.cases.json
```

What is and is not claimed:

- External execution is **at-least-once** against an idempotent sink. After executor death the
  intent stays `pending`; a retry writes the same bytes (`duplicate=True`) and `record` is
  idempotent. Exactly-once external execution is not claimed.
- The declarative machine does not force `record` before `complete`; the app does. The kernel's
  own guard is `$published_digest == approved_digest`.
- A client timeout is `submission-unknown`; `lookup_operation` is the only way to learn the answer.
- The corpus (`*.cases.json`) is hand-written; `ik check` evidence is relative to it.
- No real publishing API, payment, or workflow engine is involved.

Tests: `python/tests/test_publish_approval_app.py`, `tests/flat_invariant_witness.rs`.
