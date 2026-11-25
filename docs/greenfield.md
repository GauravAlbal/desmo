# Greenfield

1. Install a **released** `ik` (see [README.md](../README.md) or [upgrading.md](upgrading.md)).
2. Write a `.ik.toml` machine. Start from
   `examples/orders/order.ik.toml` or `machines/verification_attempt.ik.toml`.
3. Run `ik check path/to/machine.ik.toml`.
4. Create and mutate from TypeScript, Python, or Go. Do not send raw JSON envelopes.
5. Keep the returned handle. A copied handle that mutates after a later
   command is refused with `IK_CAPABILITY_STALE`.

`ik check` runs without a corpus, but the result is normally `REFUSED` because
there is no execution evidence; see [running without a corpus](ik-check.md#running-ik-check-without-a-corpus)
for the starter-corpus workflow.

Creating an aggregate does not initialize its facts. Use a machine command to
set them (or `createWith` / `create_with` / `CreateWith` when admission and the
initializing command should be atomic); see the
[TypeScript lifecycle guide](typescript.md#validated-admission),
[Python lifecycle guide](python.md), and [patterns](patterns.md). String facts
can be cleared by assigning the empty string; see
[fact assignment](machine-language.md#guard-and-then-grammar). This is a
sentinel value, not deletion of the fact.

Editing a machine definition changes its identity. Existing aggregates retain
the identity they were created under; route them to that retained definition
instead of sending commands through the edited one. See
[machine evolution](machine-evolution.md).

The host owns IO and orchestration. The kernel owns destinations,
guards, capabilities, effects, and replay.
