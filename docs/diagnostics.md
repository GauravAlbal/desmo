# Diagnostics

Every refusal has a stable `code`. Clients expose it on the exception /
error. Transport failures (`IK_TRANSPORT`, process, timeout, non-JSON)
are not domain diagnostics and must not be retried as if they were.

| Code | Meaning |
|---|---|
| `IK_PROTOCOL_INVALID` | Malformed envelope |
| `IK_PROTOCOL_VERSION_UNKNOWN` | Protocol/runtime mismatch |
| `IK_MACHINE_*` | Unknown / mismatched machine identity |
| `IK_ILLEGAL_STATE` | Command not legal, including terminal |
| `IK_COMMAND_INVALID` | Unknown command or bad args |
| `IK_CAPABILITY_STALE` | Handle is not current |
| `IK_OPERATION_CONFLICT` | `operation_id` already committed with different semantics or capability |
| `IK_NONCE_INVALID` | Generation matches, nonce does not |
| `IK_INVARIANT_FAILED` | Domain invariant, or a command `when` guard that failed (`rule` is that command's `when`) |
| `IK_EFFECT_STALE` / `IK_EFFECT_MISMATCH` / `IK_EFFECT_OUTCOME_CONFLICT` | Effect identity |
| `IK_CONCURRENT_MUTATION` | Write lock |
| `IK_STORE_NOT_FOUND` | Store file does not exist (`rule=store.missing`). Not corruption. Read-only operations do not create it. |
| `IK_STORE_CORRUPT` / `IK_JOURNAL_CORRUPT` / `IK_PROJECTION_CORRUPT` / `IK_REPLAY_MISMATCH` | Present but invalid durability data |
| `IK_COUNTER_OVERFLOW` | Generation/sequence ceiling |
| `IK_INTERNAL` | Runtime contract violation |
| `IK_CHART_INCONSISTENT` | Authored chart disagrees with the lowered definition |

Domain machines may emit their own codes. Treat those as opaque strings.

A missing store file is `IK_STORE_NOT_FOUND`, not `IK_STORE_CORRUPT`. An unknown
aggregate inside an existing store remains `IK_ILLEGAL_STATE` with
`rule=aggregate.not_found`. The domain-adapter vocabulary that maps these (and
transport outcomes) is in [error taxonomy](design/error-taxonomy.md).
