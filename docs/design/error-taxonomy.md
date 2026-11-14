# Decision record: failure classification

Status: accepted for the unreleased runtime. Protocol remains `1.0`.

## Problem

A caller that opens a path where no store file exists, including a read during
recovery (`show`, `replay`, `verify`, `lookup_operation`, and the same operations
through `ik`), was answered `IK_STORE_CORRUPT` with `rule=store.missing`. That
collapses "nothing was ever stored here" into "stored data is invalid". A
recovery procedure that treats corruption as a damaged file will not take the
same next step as one that treats absence as "not created".

Read-only recovery must also not create a store. Creating a file in order to
report that it was missing would hide the absence and invent an empty authority.

## What the protocol already distinguishes

Codes below are the strings the runtime emits today (`src/diagnostic.rs`,
`DIAGNOSTICS`, [diagnostics](../diagnostics.md)). Clients already separate
transport from those codes ([client recovery](../client-recovery.md)).

| Situation | Actual signal | Notes |
|---|---|---|
| Not sent | No envelope. Python/TypeScript `PreDispatchError` (`not-sent`). Go `*TransportError{Unsent: true}`. | Binary missing, or a later call on a session that never dispatched that operation id. |
| Submission uncertain | Python/TypeScript `TransportError` (`submission-unknown`). Go `*TransportError{Unsent: false}`. | Timeout or process death after dispatch. The kernel may have committed. Not a domain diagnostic. |
| Runtime refusal | Kernel diagnostic other than the rows below. Includes `IK_CAPABILITY_STALE`, `IK_NONCE_INVALID`, `IK_ILLEGAL_STATE` (except `aggregate.not_found`), `IK_COMMAND_INVALID`, `IK_INVARIANT_FAILED`, `IK_OPERATION_CONFLICT`, `IK_EFFECT_*`, `IK_CONCURRENT_MUTATION`, `IK_COUNTER_OVERFLOW`, `IK_TRANSACTION_BOUNDARY`, `IK_INTERNAL`, and domain codes. | The kernel answered. Canonical state was not changed by that refusal. |
| Contract or machine mismatch | `IK_PROTOCOL_INVALID`, `IK_PROTOCOL_VERSION_UNKNOWN`, `IK_MACHINE_UNKNOWN`, `IK_MACHINE_MISMATCH`, `IK_MACHINE_VERSION_UNKNOWN`, `IK_MACHINE_HASH_MISMATCH`, `IK_EVENT_SCHEMA_UNKNOWN`, `IK_PERSISTED_SCHEMA_UNKNOWN`. | Presented identity, protocol, or persisted schema does not match the runtime. |
| Store not found | Previously `IK_STORE_CORRUPT` + `rule=store.missing`. Now `IK_STORE_NOT_FOUND` + `rule=store.missing`. | The path does not exist. |
| Aggregate not found | `IK_ILLEGAL_STATE` + `rule=aggregate.not_found`. | The store exists; that aggregate id does not. The code is unchanged. |
| Invalid or corrupt persisted data | `IK_STORE_CORRUPT` (present file, I/O, unreadable row, or missing meta), `IK_JOURNAL_CORRUPT`, `IK_PROJECTION_CORRUPT`, `IK_REPLAY_MISMATCH`. | The file is present and does not satisfy the store contract. |
| Historical response | Not an error. Python/TypeScript `HistoricalResult`. Go `*Historical`. `mutation: false`, no capability. | Exact repeat of a committed operation. See [operation identity](../operation-identity.md). |

`lookup_operation` outcomes `committed_exact`, `conflict`, and `not_recorded`
are results, not these classes. `not_recorded` is not "refused" and not
"not found".

## Decision

1. Add diagnostic code `IK_STORE_NOT_FOUND`. `Store::open` emits it, with the
   existing rule `store.missing`, when the path does not exist. It does not
   create the file or a missing parent directory. `Store::create` remains the
   only constructor that creates a store file. The CLI `create` operation is
   the only protocol operation that calls `open_or_create`; `show`, `replay`,
   `verify`, `lookup_operation`, `command`, `transaction`, and `record` open
   an existing file.
2. `IK_STORE_CORRUPT` is only for a present file that cannot be read as a valid
   store, or for persisted rows that are internally invalid. A missing file is
   not corruption.
3. Do not add a second code for an unknown aggregate. `IK_ILLEGAL_STATE` with
   `rule=aggregate.not_found` already names that case and was never reported as
   corruption. Changing it would be a separate behavior break for callers that
   match `IK_ILLEGAL_STATE`.
4. Protocol stays `1.0`. Diagnostic codes are strings in the existing
   diagnostic object; adding a code does not add a field and does not bump the
   protocol major. Changing the code emitted for a missing store **is** a
   behavior change for any consumer that matches `IK_STORE_CORRUPT` and treats
   every such refusal as damaged data.
5. The rule `store.missing` is unchanged. Consumers that branch on the rule
   keep working. Consumers that branch only on `IK_STORE_CORRUPT` must also
   accept `IK_STORE_NOT_FOUND` for a missing path. Recommended migration:
   treat `IK_STORE_NOT_FOUND`, or `rule` of `store.missing`, as not found; treat
   `IK_STORE_CORRUPT` without that rule as corrupt data.
6. Each client exposes `StoreNotFoundError` mapped from `IK_STORE_NOT_FOUND`,
   and a `classify` helper. The stable domain-adapter vocabulary is exactly:

   `not_sent`, `refused`, `contract_mismatch`, `not_found`, `corrupt`,
   `historical`, `uncertain`.

   `historical` is a result (`HistoricalResult` / `*Historical`), not an error.
   `classify` returns `historical` only when given that result. A caught error
   is one of the other six. Rule, code, and the original diagnostic object stay
   on the error. Transport certainty (`not-sent` / `submission-unknown` /
   Go `Unsent`) is unchanged.
7. `classify` maps `rule=store.missing` to `not_found` even if an older binary
   still emits `IK_STORE_CORRUPT` with that rule. That is adapter compatibility,
   not a promise that the current runtime emits the old pair.
8. Operation ids are scoped to `(aggregate_id, operation_id)`. The same id on
   two aggregates does not collide and does not conflict. The runtime does not
   provide a global operation-id namespace. An application that needs global
   uniqueness must enforce that constraint itself. This restates
   [operation identity](../operation-identity.md) D1 and outcome row 8; it does
   not strengthen or weaken that scope.

## Tests updated

No existing test asserted that a missing store is `IK_STORE_CORRUPT`. Searches
of `tests/`, client tests, and conformance fixtures found no pin of
`store.missing` to that code. Nothing was rewritten to preserve the old
classification. `tests/operation_attribution.rs` already commits the same
operation id on two aggregates; the new client tests state the application
constraint explicitly against the installed binary.

## Compatibility

See [release compatibility](../release-compatibility.md). Package SemVer for
this behavior change is recorded in `CHANGELOG.md` and is not a protocol-major
bump.
