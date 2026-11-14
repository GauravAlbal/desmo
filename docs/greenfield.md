# Greenfield

1. Install a **released** `ik` (see [upgrading.md](upgrading.md)).
2. Write a `.ik.toml` machine. Start from
   `machines/verification_attempt.ik.toml`.
3. `ik check path/to/machine.ik.toml`
4. Create and mutate from Python or Go. Do not send JSON envelopes.
5. Keep the returned handle. A copied handle that mutates after a later
   command is refused with `IK_CAPABILITY_STALE`.

The host owns IO and orchestration. The kernel owns destinations,
guards, capabilities, effects, and replay.
