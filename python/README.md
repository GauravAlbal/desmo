# Python in 10 minutes

`invariant_kernel` is a typed client for `ik`. The Rust runtime remains the
authority for canonical storage and lifecycle truth. Python can also provide a
**versioned external reducer**: Python owns deterministic domain reduction,
while Rust still owns every durable and security-relevant lifecycle boundary.

## Install

```bash
cargo install --path .
pip install -e python/
```

Tests against the real binary:

```bash
cargo build --bin ik
PYTEST_DISABLE_PLUGIN_AUTOLOAD=1 IK_BINARY=target/debug/ik python3 -m pytest -q python/tests
```

The persistent-session timing qualification is opt-in; ordinary suites skip
this timing check while the correctness, reuse, and lifecycle tests remain
blocking. Run only that qualification with:

```bash
PYTEST_DISABLE_PLUGIN_AUTOLOAD=1 IK_BINARY=target/debug/ik IK_RUN_PERFORMANCE=1 python3 -m pytest -s -q python/tests/test_worker_walls.py::test_persistent_session_is_materially_faster_than_one_shot_for_the_same_workload
```
`Kernel(binary=...)` or `IK_BINARY` or `PATH` locates `ik`. Wheels do not
bundle platform binaries.

The `0.7.1` client uses protocol `1.0` and requires the `0.7.1` runtime and
worker exactly; a shared protocol major does not permit mixing runtime versions.
Install `ik` with the one-line installer (`curl -fsSL https://raw.githubusercontent.com/GauravAlbal/desmo/main/install.sh | sh`) or from the [GitHub Release](https://github.com/GauravAlbal/desmo/releases).

Both schema-1 flat machines and schema-2 compound/parallel sources load through
`kernel.machine` and `kernel.check`; schema-2 identities are those of the single
lowered flat machine, not the authoring document. Persisted store schema is `4`
(authoring schema `2` is unrelated to historical persisted schema `2`, which
remains refused). Checker `ACCEPT` is bounded analysis, not runtime
authorization or a proof over arbitrary inputs. Runtime invariant refusals
raise `InvariantFailedError` and leave the aggregate's public state, generation,
and capability unchanged.

### Transport process lifecycle

Every subprocess call owns a fresh process session and a unique inherited
process token. On success as well as timeout, the transport MUST clean up
tracked descendants, including workers that started another session, before
returning or raising. After the root has been reaped, its PID MUST NOT be used
to signal a process group: a stale PID is not group ownership. Cleanup still
uses descendant and token discovery. While the root is live, process-group
cleanup remains required; permission failures MUST propagate, not masquerade
as successful cleanup. This is cooperative process cleanup, not an OS sandbox.

## Define a machine

Use any `.ik.toml`. The repo fixture is enough to start:

```text
machines/verification_attempt.ik.toml
```

## Check the machine

```python
from invariant_kernel import Kernel

kernel = Kernel(store=".state/kernel.db")
print(kernel.check("machines/verification_attempt.ik.toml").text)
```

Or: `ik check machines/verification_attempt.ik.toml`

## Create and use an aggregate

```python
from invariant_kernel import Kernel

kernel = Kernel(store=".state/kernel.db")
attempts = kernel.machine("machines/verification_attempt.ik.toml")
job = attempts.create("attempt-123")
job = job.start()
job = job.submit(artifact_digest="abc", baseline_digest="def")
print(job.state)
print(job.generation)
```

The returned handle carries the current capability. Do not thread nonces.

## External reducers

Implement a reducer as a stable, importable `module:qualname` and load it with
`Kernel.reducer`. The entrypoint must resolve to a no-argument reducer
class/object; arbitrary in-memory closures and callback transports are not
supported.

```python
from invariant_kernel import Kernel

kernel = Kernel(store=".state/kernel.db")
pricing = kernel.reducer("my_app.reducer:OrderPricingReducer")
order = pricing.create("order-1042")
order = order.execute(
    {"name": "apply_discount", "percent": 10},
    {"tier": "member", "subtotal_cents": 5000},
)
```

`Aggregate.execute(command: Mapping, observation: Optional[Mapping] = None)` is the
generic reducer API. Existing `command(name, **args)` remains unchanged.
Command, observation, state, effect, and event payload trees are recursively
immutable (`MappingProxyType`/tuples) before reducer code sees them.
`observation` is input to semantic reduction; it is not canonical state and
does not bypass Rust checks. The reducer decides whether the observed inputs
permit the command, emits a versioned domain event, and applies that event to
produce the next state.

The public reducer contract consists of frozen
`ReducerIdentity`, `DomainEvent`, `Decision`, `ApplyOutcome`, and
`ReducerDiagnostic` values plus the `Reducer` protocol/duck contract.
`Decision.emit(...)`, `Decision.noop(...)`, and `Decision.with_note(...)`
represent accepted semantic decisions and notes; a `ReducerDiagnostic` is a
domain rejection. Reducer inputs are recursively immutable
(`MappingProxyType`/tuples), so a reducer cannot mutate its inputs.

The identity is a descriptor, not a display label. It uses exactly
`reducer_name`, `reducer_version`, `code_digest`,
`command_schema_digest`, and `state_schema_digest`; each digest is exactly 64
lowercase hexadecimal characters. The binding includes the exact entrypoint
and `initial_state_digest`, which is SHA-256 of canonical initial state.
Rust recomputes that initial-state digest and recomputes `MachineIdentity` as
`machine_id=reducer_name`, `version=reducer_version`, and
`spec_hash=SHA256(canonical full binding descriptor)`.

The worker carries strict JSON protocol `1.0` in newline-delimited JSON-RPC 2.0
over stdio: sorted keys, compact separators, no `NaN`/infinite values, and
exact integers only in the shared serde range $[-2^{63}, 2^{64}-1]$.
Requests and responses reject unknown fields and echo/validate the exact
entrypoint and identity. `describe` discovers and binds initial state once;
`decide` returns `noop` or an emitted versioned event; `apply` returns state,
terminal status, and optional effect or outcome. Workers remain long-lived
and stateless by contract, are recycled after 5000 calls, and are replaced
after failure; tracked descendants are reaped after every call.
`IK_REDUCER_PARANOID=1` compares each call with a fresh worker and fails closed
on divergence (off by default).

An emitted event is the only path to `apply`; optional effect/outcome values
are conditional apply results. For external commands, stale preflight
deliberately wins over native exact-outcome duplicate replay because Rust
cannot identify a hidden reducer outcome without running Python. Thus no
stale external command starts a worker; consumers with an already-known
effect outcome use the storage-only `record` operation. Malformed envelopes,
mismatches, unsupported return values, and non-finite values fail closed
without canonical mutation.

Rust performs capability preflight before starting an external command
worker. A stale capability is rejected before Python runs. Rust then applies
the result inside its transaction and alone commits currentness, journal,
projection, replay, and verification. If the reducer or worker fails, the
transaction has zero mutation. Binding discovery alone invokes `describe`;
after binding, `create`, `record`, and `show` do not launch Python. Semantic
launch is limited to `command`, `replay`, and `verify`.

The reducer is trusted and cooperative, not an OS security sandbox. It
receives no store path, but Python code can still perform its own process or
filesystem IO. The boundary prevents Python from becoming canonical storage;
it does not enforce OS-level IO denial.

## Persistent sessions and transactions

Persistent sessions and transactions are available in `0.5.0`.
`Kernel(..., session=True)` or `transport.Session` uses
`ik --store <path> --session`: one canonical JSON request and response per line,
with the registered machine/reducer and SQLite store held open. One-shot
transport remains the default. A session serializes concurrent callers, never
retries, and becomes unusable after timeout, broken pipe, mismatched request ID,
or garbled response; EOF exits normally.

`Aggregate.transaction([...])` submits 1–64 ordered commands under one
capability. Rust threads successor capabilities and commits all steps or none.
Only the final step may create an effect intent (`IK_TRANSACTION_BOUNDARY`);
a failing step rolls the whole batch back. Session and one-shot modes use the
same authoritative validation, journal, and projection path.

## Stale authority

```python
from invariant_kernel import StaleCapabilityError

old = job
job = job.start()
try:
    old.submit(artifact_digest="abc", baseline_digest="def")
except StaleCapabilityError as exc:
    print(exc.code)  # IK_CAPABILITY_STALE
    print(exc.rule)
```

The Python wrapper does not emulate this rule. The Rust kernel refuses it.

## Check workflow

`kernel.check(path)` is developer analysis. `CheckResult.verdict` is
`accept` or `refused`. Production calls are
`create` / `command` / `record` / `show` / `replay` / `verify`.

A competent agent should not need `protocol.rs`, `runtime.rs`, `store.rs`,
or `machine.rs` to write the consumer above.

## Go

The Go adapter is `github.com/GauravAlbal/desmo/go`. Same
boundary: typed transport over `ik`, no second semantics. Install `ik`
from the GitHub Release; see [docs/go.md](../docs/go.md).

There is no Go reducer API or Go reducer-parity promise; Go remains a typed
transport client.
