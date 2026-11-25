# Patterns

These are canonical uses of the existing runtime. They are not new lifecycle
stores. Helpers do not execute external effects, retry, or substitute a fresh
capability into an old operation.

## Validated admission

`create` writes genesis only. A later initializing command is a second
operation. A reader can observe the pre-accept aggregate, and a crash between
the two steps leaves that aggregate behind. Do not advertise a bare `create`
followed by a required initialize as one atomic admission.

`create_with_command` is the atomic admission. It commits genesis and the
initializing command in one store transaction, or it commits neither. An
invalid initial command or a law violation exposes no aggregate. The same
operation id replays the original result as history (`historical: true`, no
capability). A different command under that id conflicts. Note that unlike
bare `create` (which always refuses an existing aggregate id), an identical
`create_with_command` against an existing aggregate whose journal is still
`[genesis, initial_event]` returns `ExactDuplicate` (`mutation: false`) with
the live handle even under a new `operation_id`; if your application must
refuse duplicate creation explicitly, check existence with `show` first. A dropped reply is
reconciled with `lookup_operation` using the same command body and no
capability: `committed_exact`, `conflict`, or `not_recorded`. A `conflict`
can mean the supplied lookup fingerprint (command body, capability, and
other fingerprinted fields) does not match the original request, not only
that two writers raced. `not_recorded`
is not permission to invent a second id.

Protocol `1.0` gained this op additively. Older binaries reject the unknown
op; they do not partially apply it. See `docs/release-compatibility.md`.

Clients:

- TypeScript: `machine.createWith({ aggregateId, command, operationId })`, then `machine.lookupCreateWith(...)`.
- Python: `machine.create_with(aggregate_id, command, operation_id=...)`, then `machine.lookup_create_with(...)`.
- Go: `Machine.CreateWith`, then `Machine.LookupCreateWith`.

## Invalidating a decision without deleting a fact

Do not delete a fact, and do not write a magic empty string, to mean "forget
the approval." Keep the decision as the revision it applied to.

`examples/approval-revision/` uses two counters:

- `current_revision` advances by one on every edit. Zero is the declared
  initial value: no revision has been admitted yet.
- `approved_revision` is set to the revision the approver named, and only when
  that argument equals `current_revision`.

Authorization applies only while `approved_revision == current_revision` and
`current_revision > 0`. An edit after approval advances `current_revision` and
leaves `approved_revision` where it was, so the old approval no longer
applies. Re-approval writes the new matching revision. The authorized state
still requires the match, so a later reader cannot treat a stale approval as
current authority.

A boolean `approval_applies` would also avoid deletion, but it would hide
which revision was approved. The counters keep that history, which is the
domain meaning of "approved this artifact revision."

The useful path is edit, approve revision 1, authorize. The always-refuse
variant keeps the same commands and makes authorize unreachable
(`current_revision < 0`). `ik check --profile examples/approval-revision/checks.toml`
with the sibling corpus reports `witness_found` for `authorized` on the useful
machine and does not on the variant. Edit can repeat, so the bounded search is
incomplete and the developer verdict can still be `REFUSED`; that is missing
exhaustive evidence, not a failed useful path. `ik scenario run` of
`scenarios/useful.json` is the completed path. A scenario that asks to drop
the reply of a step that did not commit fails; absence of the fault is not a
pass. See `docs/scenarios.md`.

## Telling a retired identifier from an unknown one

When a command names an identifier that a later transition replaced, the
refusal differs from the one for an identifier the machine never held.
"Expired", "superseded", and "not found" are three answers. A machine that
stores only the current identifier cannot give the first two: once a later
transition overwrites it, the old value is indistinguishable from a typo.

Two independent builds of a waitlist change hit this. A reservation was
replaced by a promoted reservation, and a checkout of the replaced id was
answered `not_reserved` where the requirement said `expired`. In each, the
check was "the named id equals the current id and that reservation has
expired", and the promotion had already overwritten the current id.

The cure is at the state that owns the identifier, not at the caller that
formats the refusal:

- keep a bounded record of retired identifiers, each with the reason it was
  retired (`expired`, `superseded`, `cancelled`), in the same aggregate and
  written by the same transition that retires it;
- classify a named identifier as current, retired with a reason, or unknown,
  and map the refusal from that classification;
- retire and replace in one transition, so no committed state holds the new
  identifier without the old one recorded.

This is application state. The kernel's operation identity answers whether an
operation id committed; it does not know what your domain called a retired
reservation. Bound the record by the domain's own retention rule and say what
is forgotten: an identifier beyond the bound becomes unknown again.
