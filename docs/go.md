# Go

Module: `github.com/GauravAlbal/desmo/go`.

The module lives in the `go/` subdirectory of the repository, so Go resolves a release from a tag
named `go/vX.Y.Z` (for example `go/v0.5.0`), not `vX.Y.Z`. Until such a tag exists, require a
pseudo-version of a commit.

```go
k, err := invariantkernel.Open(".state/kernel.db", invariantkernel.Options{})
m, err := k.Machine(ctx, "machine.ik.toml")
job, err := m.Create(ctx, "job-1")
admitted, historical, err := m.CreateWith(ctx, "admitted", "op-1", map[string]any{"name": "start"})
old := job
job, err = job.Command(ctx, "start", nil)
_, err = old.Command(ctx, "submit", map[string]any{
    "artifact_digest": "a", "baseline_digest": "b",
})
var stale *invariantkernel.StaleCapabilityError
errors.As(err, &stale) // IK_CAPABILITY_STALE
```

Product wrappers sit **above** this client:

```go
func (d Delegation) Advance(ctx context.Context, obs Observation) (Delegation, error) {
    next, err := d.agg.Command(ctx, "advance", args(obs))
    return Delegation{agg: next}, err
}
```

Do not re-implement envelopes, capability currentness, or diagnostics in
the product repo. Pin a released `ik` artifact; set `IK_BINARY`.

## Recovering an uncertain command (narrow, additive)

Existing `Command`/`Record` signatures and wire behavior are unchanged. The
attribution path is separate:

```go
operationID := invariantkernel.NewOperationID()  // persist before dispatch
wire := aggregate.WireCapability()               // authority: store like a secret
next, historical, err := aggregate.CommandWithOperation(ctx, operationID, "start", nil)
if err != nil && !invariantkernel.IsUnsent(err) {
    // submission-unknown: the command may have committed.
    found, lerr := machine.LookupOperation(ctx, "attempt-1", wire, operationID, "start", nil)
    // found.Outcome: "committed_exact" | "conflict" | "not_recorded"
    // conflict: the lookup fingerprint does not match the original request.
}
```

Restart-safe form (`go/invocation_test.go`): describe, persist, dispatch once, reconcile.

```go
inv, err := aggregate.Prepare("", "start", nil)  // snapshots the request; refuses an unserializable one
blob := inv.JSON()                               // persist in your durable job record before dispatch
next, historical, err := machine.Dispatch(ctx, inv)
// after a restart:
restored, _ := invariantkernel.InvocationFromJSON(blob)
found, _ := machine.Reconcile(ctx, restored)
```

`Classify(err, historical)` returns `not_sent`, `refused`,
`contract_mismatch`, `not_found`, `corrupt`, `historical`, or `uncertain`.
Pass a nil error and a non-nil `*Historical` for `historical`; that value is a
result, not an error. `*StoreNotFoundError` is `IK_STORE_NOT_FOUND`. `Rule` and
`Diagnostic` are preserved. See [error taxonomy](design/error-taxonomy.md).

`CommandWithOperation` returns a new `*Aggregate` for a new commit or a
`*Historical` for an exact repeat of a committed operation (no capability, no
state). Every error carries the id (`OperationIDOf`); `IsUnsent` is true only
when the request was provably never dispatched. `IK_OPERATION_CONFLICT` maps to
`*OperationConflictError`. The client never retries and never refreshes a
handle. `Replay`/`Verify` do not answer whether an operation committed.

`CreateWith` is the atomic admission. `LookupCreateWith` reconciles a dropped reply and sends no capability. A bare `Create` followed by a required initialize is not atomic. See `docs/patterns.md`.
