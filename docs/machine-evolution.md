# Machine evolution (first public procedure)

Desmo identity is strict. A legitimate edit that changes commands or laws
changes `spec_hash` and usually `machine_version`. That is not automatic
approval of old aggregates under the new meaning.

## Supported first procedure

**Old aggregates continue under their original machine version. Newly created
aggregates use the new version.**

1. Keep the v1 definition loadable.
2. Author v2 as a separate file (`publish_approval_v2.ik.toml`).
3. Route creates by an explicit ownership/version rule.
4. Inspect `show` / identity on an aggregate before sending commands.
5. Drain v1 (no new creates). Do not insert rows into kernel tables.
6. Application rollback does not move v2 aggregates back to a host `status`
   field.

There is no generic historical-state migrator in this release. Snapshot/import
is a separate, validated procedure if you ever need it.

See `python/tests/test_machine_evolution.py`.

## Routing, draining and rollback (worked example)

`examples/publish-approval/approval_app.py` implements `VersionRouter`. It resolves the
definition for an existing aggregate from the **identity persisted with that aggregate**
(`show` returns `machine_id`, `version`, `spec_hash`) and refuses with `UnroutableAggregate`
if no loaded definition matches. `create_version` only decides where *new* aggregates start.

- Rolling the application back sets `create_version` to 2; existing v3 aggregates stay on v3.
- Draining a version stops new creates on it; its aggregates keep working.
- A router that instead uses the current default after rollback is refused by the kernel with
  `IK_MACHINE_MISMATCH` (`NaiveRollbackRouter`, a deliberate negative control).

See `python/tests/test_publish_approval_app.py`.

## Reading an aggregate's recorded identity

An aggregate keeps the identity it was created under. `show` through a `Machine` for a *different* version
still returns the aggregate's view; the mismatch is enforced when you send a command
(`IK_MACHINE_MISMATCH`). The recorded `(machine_id, version, spec_hash)` is `aggregate.owner` (TypeScript and
Python) or `Owner()` (Go): it names the definition that owns the aggregate, and it can differ from the
`Machine` you loaded. `MachineIdentity` values compare by all three fields (`equals` in TypeScript, `==` in
Python and Go). Route an existing aggregate to the retained definition whose `identity` equals its `owner`
(see `VersionRouter` in `examples/publish-approval/approval_app.py`), never by a "current default" version.
The same identity is on the latest receipt (`raw.last_receipt`).
