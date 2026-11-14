# Client recovery matrix

Which client behavior to expect for each situation. The kernel semantics are in
[operation-identity.md](operation-identity.md); per-client usage is in
[python.md](python.md), [typescript.md](typescript.md) and [go.md](go.md).
No client retries automatically or refreshes a stale handle.

| Situation | Kernel answer | Python | TypeScript | Go |
|---|---|---|---|---|
| New commit | new capability | `Aggregate` | `Aggregate` | `*Aggregate` |
| Exact repeat of a committed operation | historical, no capability/state | `HistoricalResult` | `HistoricalResult` | `*Historical` |
| Same id, changed semantics or refreshed capability | `IK_OPERATION_CONFLICT` | `OperationConflictError` | `OperationConflictError` | `*OperationConflictError` |
| Stale handle, different id | `IK_CAPABILITY_STALE` | `StaleCapabilityError` | `StaleCapabilityError` | `*StaleCapabilityError` |
| Binary missing before dispatch | none sent | `PreDispatchError` (`not-sent`) | `PreDispatchError` (`not-sent`) | `*TransportError{Unsent: true}` |
| Timeout / process death after dispatch | unknown | `TransportError` (`submission-unknown`) | `TransportError` (`submission-unknown`) | `*TransportError{Unsent: false}` |
| Ask whether an id committed | `lookup_operation` | `Machine.lookup_operation` | `Machine.lookupOperation` | `Machine.LookupOperation` |
| Refused or no-op command | no row written | error / no change | error / no change | error / no change |
| Store file absent | `IK_STORE_NOT_FOUND` `rule=store.missing` | `StoreNotFoundError` | `StoreNotFoundError` | `*StoreNotFoundError` |
| Aggregate id absent | `IK_ILLEGAL_STATE` `rule=aggregate.not_found` | `classify` → `not_found` | `classify` → `not_found` | `Classify` → `not_found` |
| Present store unreadable | `IK_STORE_CORRUPT` (no `store.missing` rule) | `classify` → `corrupt` | `classify` → `corrupt` | `Classify` → `corrupt` |

`classify` (Go: `Classify`) is the stable domain-adapter vocabulary:
`not_sent`, `refused`, `contract_mismatch`, `not_found`, `corrupt`,
`historical`, `uncertain`. `historical` is the result type above, not an error.
Code, rule, and the diagnostic object are preserved. The table and the
migration note are in [error taxonomy](design/error-taxonomy.md).

Every error carries the operation id of the logical invocation. `not_recorded`
is not permission to retry under a stale handle, and replay/verify are journal
audits, not operation-outcome queries.

`create_with_command` uses the same operation id and `lookup_operation` rules. The lookup body is the initializing command and no capability. See `docs/patterns.md`.

## Recovery from a lost reply: the `Invocation`

| Step | Python | TypeScript | Go |
|---|---|---|---|
| Describe, send nothing | `aggregate.prepare(command)` / `prepare_transaction(steps)` | `aggregate.prepare(command)` / `prepareTransaction(steps)` | `aggregate.Prepare(id, name, args)` (returns an error for an unserializable request) |
| Persist before sending | `invocation.to_json()` | `invocation.toJson()` | `invocation.JSON()` |
| Send once | `machine.dispatch(invocation)` | `machine.dispatch(invocation)` | `machine.Dispatch(ctx, inv)` |
| After a crash | `Invocation.from_json` then `machine.reconcile` | `Invocation.fromJson` then `machine.reconcile` | `InvocationFromJSON` then `machine.Reconcile` |

The id and the original capability travel together. Recovery never recomputes the id from a
refreshed generation and never resends. See the ADR section in
[operation-identity.md](operation-identity.md).
