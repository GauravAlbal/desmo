# ADR: durable operation identity and uncertain-outcome semantics

Status: **implemented** in the `0.5.0` candidate (kernel: `tests/operation_attribution.rs`; clients:
`docs/client-recovery.md`). The text below is the design record; read "proposed" as the shipped
behavior unless an item is marked open.

## Problem

A caller sends a mutating request, the reply is lost, and another writer
advances the aggregate. A current-state read cannot say whether the caller's
request committed: the state may equal what the request would have produced, or
something else changed it, or the request may still be running.

Three identities exist and MUST NOT be conflated:

| Identity | Scope | Exists today | Purpose |
|---|---|---|---|
| `request_id` | One transport attempt | Yes (`EnvelopeRequest.request_id`) | Correlate a response with an attempt. Not stored. |
| `operation_id` | One logical caller intent, stable across attempts | **No** | Attribute a committed mutation to the caller's intent. |
| Effect identity (`effect_id`, idempotency key, outcome digest) | One external effect | Yes (effect module) | Bind an outcome to a planned effect. Unrelated to command attribution. |

## Decisions

### D1. `operation_id` is separate from `request_id`

Caller-chosen string, aggregate-scoped, 1..=128 bytes, restricted charset
(`[A-Za-z0-9._:-]`). A generating client creates it once per logical
invocation, before dispatch, and exposes it on every success and error.

### D2. Attribution commits atomically with the mutation, outside the hash chain

A new `operations` table keyed by `(aggregate_id, operation_id)` is written in
the same SQLite transaction as `persist_plan`. It is **not** part of
`JournalEvent`'s hash input: changing the event hash would invalidate replay of
existing stores. Each row stores:

- operation kind (`command`, `transaction`, `record`, `create_with_command`);
- `request_fingerprint` (D3);
- the sequence range `[first_sequence, last_sequence]` and each event hash in it;
- the resulting `generation` and `state_digest` as of that commit;
- the original receipt ids.

It stores **no capability**. A historical result is never a current handle.

`verify` checks that every operation row's events exist in the journal with the
recorded hashes; a binding without its commit is `IK_JOURNAL_CORRUPT`.

### D3. Request fingerprint

SHA-256 over canonical JSON of the semantic request:
`{kind, machine {id, version, spec_hash}, aggregate_id, capability {generation,
nonce}, command, observation, steps[], effect_outcome}` as applicable.
Excluded: `request_id`, runtime/protocol versions, local paths, `reducer_worker`
launch details. The **original capability is included**: a refreshed capability
under the same ID is a different request and conflicts (D5).

Clients never compute the fingerprint; the runtime does (same rule as
`spec_hash`).

### D4. Duplicate check precedes capability validation, inside the write transaction

Under `BEGIN IMMEDIATE`, before currentness is checked:

1. Look up `(aggregate_id, operation_id)`.
2. Found, same fingerprint: return the recorded result as **historical**
   (`mutation: false`, no reducer execution, no generation/nonce/receipt change).
3. Found, different fingerprint: refuse with `IK_OPERATION_CONFLICT`.
4. Not found: continue to the normal path (identity, capability, decide, apply).

Ordering matters: an exact repeat carries the now-stale original capability and
must not be answered `IK_CAPABILITY_STALE`.

### D5. Lookup is read-only and reports "as of this read"

Proposed op `lookup_operation` takes the same request body as the mutation but
executes nothing. Outcomes:

| Outcome | Meaning |
|---|---|
| `committed_exact` | A row exists and the fingerprint matches. Returns the historical record. |
| `conflict` | A row exists and the fingerprint differs. Returns the stored op kind and sequence range, not the conflicting payload. A lookup that uses a different capability, command body, or other fingerprinted field than the original request is a `conflict` even when the `operation_id` exists: the lookup fingerprint did not match, not only that two writers raced. |
| `not_recorded` | No row **as of this read**. |

`not_recorded` is **not** "refused", "cancelled", or "safe to retry with a new
ID". The original request may still be running, may have been refused (refusals
are not recorded, D7), or may never have been delivered.

Every response also carries the aggregate's current generation and sequence so a
caller can see that the historical result is not current.

### D6. Explicit same-ID resubmission

Resubmitting the **same request with the same ID and original capability** is an
explicit caller action, not a recovery default and not a read:

- If the original committed in the meantime: it returns the historical result (D4.2).
- If it did not commit and the original capability is still current: it executes
  the original semantics.
- If it did not commit and state moved: it refuses `IK_CAPABILITY_STALE`. The
  caller must decide anew; the runtime never refreshes authority.

Clients MUST NOT perform this automatically after a transport failure.

### D7. Refusals and no-ops are not recorded in the alpha

Refusal immutability stands: a refused request appends nothing and writes no
operation row. A no-op (`Decision::Noop`) commits no event and likewise writes no
row. Consequence, stated plainly: **alpha attribution covers committed
mutations only.** An absent record cannot distinguish "refused" from "never
arrived" from "still running". A durable negative-result design would need its
own ADR and must not weaken refusal immutability.

### D8. Participation by operation

| Operation | Attribution in alpha |
|---|---|
| `command` | Yes: one row, one event. |
| `transaction` (batch) | Yes: one row mapping to the sequence range; the result is the batch result. A batch is not one journal event. |
| `record` (effect outcome) | Yes: one row. Independent of, and in addition to, the existing exact-outcome deduplication. |
| `create` with first command | Yes: row covers genesis plus the first event **when the command commits**; if the first command is a no-op only genesis commits and the row covers genesis. Existing `ExactDuplicate` behavior remains. |
| `create` (plain genesis) | **No operation row.** `aggregate_id` is caller-chosen and genesis is deterministic; recover with `show`. Narrowed recovery claim. |

### D9. What this does not claim

At-most-one **logical kernel commit** per `(aggregate, operation_id)` does not
imply exactly-once external execution. A planned effect may have been executed
by an external system whose outcome is unknown; repeating that effect is the
executor's reconciliation problem, using effect identity, never this table.

### D10. Retention and compatibility

- Committed operation ids are retained for the store's lifetime. Pruning is
  deferred. A pruned id could be reused to repeat a mutation, so no pruning
  until a separate ADR.
- Adding the `operations` table is a store-schema change; adding `operation_id`
  and `lookup_operation` is a protocol change (the envelope is
  `deny_unknown_fields`). Version consequences follow
  `docs/release-compatibility.md`; **this ADR does not preselect a version
  number.**
- Opening an older store creates an empty `operations` table inside a
  transaction. Legacy events receive no invented ids. A newer store is refused
  by older binaries (existing schema-version check). Binary rollback across this
  change is not promised.
- Requests without `operation_id` behave exactly as today and get no attribution
  guarantee.

## Outcome table (acceptance)

Notation: `C` = original capability, `C'` = refreshed capability.

| # | Situation | Required result |
|---|---|---|
| 1 | Response lost after commit; no other writer; lookup | `committed_exact`, historical; current generation equals recorded |
| 2 | Response lost after commit; another writer advanced state; lookup | `committed_exact`, historical; current generation > recorded; not a current handle |
| 3 | Original still running; lookup before it commits | `not_recorded` (not "refused"); no implication it is cancelled |
| 4 | Concurrent same-ID same-request attempts | Serialized by `BEGIN IMMEDIATE`; exactly one commit; the other returns historical |
| 5 | Same ID, same request, resent after commit, with stale `C` | Historical result, `mutation: false`; **not** `IK_CAPABILITY_STALE` |
| 6 | Same ID, changed command or observation | `IK_OPERATION_CONFLICT` |
| 7 | Same ID, refreshed `C'` | `IK_OPERATION_CONFLICT`; never treated as the original |
| 8 | Same ID against another aggregate | Independent (ids are aggregate-scoped); not found there |
| 9 | Same ID, wrong machine identity | Identity refusal (`IK_MACHINE_*`) before lookup result is revealed |
| 10 | Request refused by invariant or illegal state | No row; later lookup is `not_recorded` |
| 11 | Request is a no-op | No row; later lookup is `not_recorded` |
| 12 | Explicit resubmit after case 3 resolves uncommitted, state unmoved | Executes original semantics once |
| 13 | Explicit resubmit, state moved | `IK_CAPABILITY_STALE`; no refresh |
| 14 | Resubmit of a batch that partly applied | Cannot occur: batches are one transaction; all-or-nothing |
| 15 | Repeated external effect after unknown outcome | Out of scope here; effect identity governs |

## Distinguishing the four things

| Action | Mutates? | Re-runs reducer? | Authority used |
|---|---|---|---|
| Exact repeat (D4.2) | No | No | None; returns recorded evidence |
| `lookup_operation` | No | No | None |
| Refreshed command (new ID, new `C`) | Maybe | Yes | The caller's fresh read |
| Re-executed external effect | N/A to kernel | N/A | Executor, via effect identity |

## Negative controls IK-03 must carry

1. A reconciliation based only on current state fails on case 2 (constructed
   ambiguity: another writer produced a state equal to the original's result).
2. Absent lookup never reports `refused`.
3. `C'` under an old ID is not accepted as an exact repeat.
4. Crash between operation-row insert and journal/projection commit leaves
   neither, and crash after commit leaves both.
5. Duplicate returns old result after newer state exists without mutating.

## Open items for review

- Whether `lookup_operation` should be a distinct op or a `dry_run` flag on the
  mutation ops. Preference: distinct op (`Op` is a closed read/mutate split).
- Exact `IK_OPERATION_CONFLICT` placement in `docs/diagnostics.md`.
- Whether the alpha should also bound the number of retained rows per aggregate.

## Listing recorded operations (discovery, not attribution)

`ik operations <store.sqlite> --aggregate <id> [--limit <n>] [--json]` lists a window of the
operations the store recorded for one aggregate, newest first (`ik.operations.v1`). It opens the
store read-only and never writes. The aggregate's position and the rows are read in one read
transaction, so they describe the same store snapshot; the output names that snapshot
(`snapshot.generation`, `snapshot.sequence`) and the window (`window.limit`, `returned`,
`complete`). A negative stored coordinate is reported as a damaged store, never read as zero.

It answers "what was recorded recently", not "did my request commit". An operation missing from
the list has no established outcome here: it may be older than the window, still running, never
delivered, refused, or a no-op (cases 3, 10 and 11 above). Only `lookup_operation` with the
original operation id and request answers whether that operation committed; if the caller lost
both, the attribution is unknown, and the newest row that looks similar is not an answer. The
list does not make any operation a current handle: take a fresh handle from `show`.

## Recovery contract: one stable id per logical invocation, bound to its original request

Operation identity is the caller's, minted once per logical invocation. The generation (the
capability) is authority for the write, not identity: it is part of the original request, and a
lookup is exact only against that original request.

- Build an `Invocation` before sending (`Aggregate.prepare` / `prepareTransaction` in Python and
  TypeScript, `Aggregate.Prepare` in Go). It holds the operation id, the machine identity, the
  aggregate id, the capability the request was authorized against, and the command (or steps).
  Nothing is sent.
- Persist `invocation.to_wire()` / `toJson()` / `JSON()` in the host's existing durable job or
  request record BEFORE `dispatch`. The form is authority (it carries the capability): store it
  like a secret. The persisted form is versioned and restores to the same request in all three clients (a shared corpus with non-ASCII text, escaping, nesting and integers beyond 2^53 is tested); the bytes are not promised identical across languages, because JSON string escaping differs.
  A prepared invocation owns a snapshot: mutating a caller-owned argument object, a returned wire value or a returned command cannot change the bound request, and a value that cannot be serialized is refused before sending. An intentionally different request needs a new invocation.
- After a crash, restore it and call `reconcile`. `committed_exact` is history; `conflict` means
  the id was used for a different request; `not_recorded` means that exact request has no
  recorded commit, and is not permission to resend under a refreshed handle.
- Never derive a replacement id from a freshly read generation, and never refresh and resubmit.
  A new id with a new capability is a new logical invocation the host must decide to make.
- `ik operations` is discovery, not attribution. If the host lost the invocation and has no
  stable business identity to rebuild it from, the attribution is unknown and stays unknown; do
  not pick a plausible row.
- Not covered: external-effect reconciliation (`record`, effect outcomes, `create_with`) stays
  separate. The Go client restores command invocations only (it has no transaction call).
- A dispatch or reconcile against a `Machine` with a different identity is refused before
  anything is sent.
