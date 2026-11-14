# Python

Package: `invariant_kernel`. The package is a typed transport client; the
`ik` Rust runtime is the canonical lifecycle authority.

```python
from invariant_kernel import Kernel, StaleCapabilityError

kernel = Kernel(store=".state/kernel.db")  # or Kernel(binary="ik")
machine = kernel.machine("machine.ik.toml")
job = machine.create("job-1")
old = job
job = job.start()
job = job.submit(artifact_digest="a", baseline_digest="b")
print(job.state, job.generation)

try:
    old.submit(artifact_digest="a", baseline_digest="b")
except StaleCapabilityError as exc:
    print(exc.code)  # IK_CAPABILITY_STALE
```

`kernel.check(path)` is developer analysis. `CheckResult.verdict` is
`accept` or `refused`. The JSON document's `judgment.status` is
`accepted`, `missing_evidence`, or `witnessed_violation`. Lifecycle operations are
`create` / `command` / `record` / `show` / `replay` / `verify`.

`machine.create_with(aggregate_id, command, operation_id=...)` is the atomic admission. A bare `create` followed by a required initialize is not. Reconcile a dropped reply with `lookup_create_with`. See `docs/patterns.md`.

## External reducer API

An external reducer is a deterministic semantic component, not a second
kernel. Construct one from a stable importable entrypoint:

```python
from invariant_kernel import Kernel

kernel = Kernel(store=".state/kernel.db")
game = kernel.reducer(
    "my_game.reducer:WeddingWeekendReducer",
    python=None,             # optional Python executable
    worker_timeout=None,     # optional per-worker limit
)
scene = game.create("wedding-weekend-01")
scene = scene.execute(
    {"name": "kiss_noah"},
    {"location": "dance_floor", "consent": True},
)
```

`Kernel.reducer(entrypoint, *, python=None, worker_timeout=None)` discovers
and validates the reducer with `describe`, then returns a worker-bound
`Machine`. `entrypoint` is a stable `module:qualname`, not a lambda,
callback, arbitrary in-memory closure, or callback transport. The resolved
object must be loadable with no constructor arguments.

The public frozen Python values are:

- `ReducerIdentity`: `reducer_name`, `reducer_version`,
  `code_digest`, `command_schema_digest`, and `state_schema_digest`.
- `DomainEvent`: a versioned semantic event.
- `Decision`: `emit(...)`, `noop(...)`, and `with_note(...)`.
- `ApplyOutcome`: resulting state, terminal status, and optional effect/outcome.
- `ReducerDiagnostic`: a domain rejection diagnostic.
- `Reducer`: the protocol/duck contract implemented by the reducer.

The reducer exposes these operations:

```python
class Reducer(Protocol):
    identity: ReducerIdentity
    def initial_state(self) -> Any: ...
    def decide(
        self, state: Any, observation: Optional[Any], effect: Any, command: Any
    ) -> Decision | ReducerDiagnostic: ...
    def apply(
        self, state: Any, event: DomainEvent
    ) -> ApplyOutcome | ReducerDiagnostic: ...
```

`Decision.emit(event)`, `Decision.noop()`, and
`decision.with_note(note)` represent semantic decisions; the optional note is
an annotation. `ApplyOutcome.new` constructs a result; `with_terminal`,
`with_effect`, and `with_outcome` return updated immutable results. Applying
an event (the versioned state delta) produces the next state, terminal flag,
and optional effect/outcome. These values must be the exact public types above;
lookalike dictionaries or ad hoc return values are not accepted by the
official worker.

`Aggregate.execute(command: Mapping, observation: Optional[Mapping] = None)` is the
generic reducer call. Command, observation, state, effect, and event payload
trees are recursively frozen before reducer code sees them
(`MappingProxyType` and tuples); reducers cannot mutate those input trees.
`observation` is semantic input only. It is not canonical state, a replacement
for a capability, or a way around Rust currentness checks. Existing
`Aggregate.command(name, **args)` behavior is unchanged.

## Descriptor and identity

The descriptor schema is `invariant-kernel.python-reducer/1`. Protocol
major/minor is `1/0`. Canonical JSON is serialized with sorted keys and
compact separators, rejects `NaN` and infinite numbers, and restricts
integers to the exact shared serde range $[-2^{63}, 2^{64}-1]$. Input or
output outside that range is rejected before it can be rounded at the
Python/Rust boundary.

Every identity digest is exactly 64 lowercase hexadecimal characters. The
component identity has exactly these fields:

```text
reducer_name
reducer_version
code_digest
command_schema_digest
state_schema_digest
```

The binding additionally carries the exact `entrypoint` and
`initial_state_digest`, where the latter is
`SHA256(canonical initial state)`. Rust recomputes that digest from the
described initial state and does not trust a Python-computed machine identity:
it recomputes `MachineIdentity` as `machine_id=reducer_name`,
`version=reducer_version`, and
`spec_hash=SHA256(canonical full binding descriptor)`.

The launch spec also carries the discovered `initial_state` value, direct
worker `command` argv, and `timeout_ms`. Rust hashes the carried state and
requires it to match `initial_state_digest` before registration. The argv and
timeout are routing, not semantics, and are excluded from the descriptor hash.
Keeping the bound initial value in the spec lets `create` register state
without launching Python and lets stale commands reach capability preflight
before any worker starts.

Optional, versioned:

- `kernel_laws`: `{name, when?, assert}` evaluated by Rust on initial state and
  every `apply` post-state. Violations are `IK_INVARIANT_FAILED`. Omitted
  bindings keep the legacy identity. A nonempty set adds `kernel_laws_digest`
  to `spec_hash`, so weakening laws cannot reuse the prior identity.
- `code_source`: bind-time SHA-256 of a file compared to declared `code_digest`.
  Not proof the worker executed those bytes. Absent, `code_digest` is author-declared.



## Worker protocol and lifecycle

Transport is newline-delimited JSON-RPC 2.0 over the worker's stdio
(`--serve`). `method` is one of `describe`, `decide`, or `apply`; `params`
carries the strict request object (`protocol`, `op`, `entrypoint`, `identity`,
and the operation fields, with `op` equal to `method`) and `result` carries
the strict response object (`protocol`, `ok`, `entrypoint`, `reducer_identity`,
and either `result` or `diagnostic`). Unknown fields are rejected, and every
response must echo and validate the exact bound entrypoint and reducer
identity. Rust refuses, fails closed on, and discards the worker for a reply
with a wrong `id`, a wrong `jsonrpc` version, an `error` member, neither or
both of `result`/`error`, a non-object result, a non-JSON line, a line over
the output bound, or EOF/timeout mid-call.

`describe` returns and binds `initial_state` once. `decide` returns `noop` or
an emitted versioned event plus an optional note. `apply` returns `state`,
`terminal`, and optional `effect` and `outcome`. A worker is **long-lived**
and stateless by contract: every request carries the full state, so any
worker can serve any request. Rust recycles a worker after 5000 calls to
bound reducer-side memory growth and discards it after any failure. After
each successful call Rust kills descendants, including descendants that
detached, but preserves the worker itself for the next call. On Linux it
also asks the kernel to kill the worker if `ik` dies. Replay invokes `apply`
rather than trusting Python to retain mutable process state.

Set `IK_REDUCER_PARANOID=1` to replay every call on a fresh worker and fail
closed on any divergence. It detects reducers that violate the purity
contract (module-level counters, clocks, randomness) at the cost of doubling
worker calls; it is off by default because the contract is cooperative.

### Process ownership states and exits

Process cleanup does not decide commands, commit transactions, or retry a
request. Rust remains the semantic and canonical lifecycle authority.
The process owners and cleanup scopes are:

| Owner | Root and lifetime | Inherited ownership marker |
|---|---|---|
| Python one-shot transport | One `ik` invocation, or the discovery worker | Fresh `IK_PROCESS_TREE_TOKEN` per launch |
| Python session transport | One persistent `ik --session` process | Fresh `IK_PROCESS_TREE_TOKEN` retained until close/failure |
| Rust external machine | One persistent reducer worker, reused across calls | Fresh `IK_REDUCER_PROCESS_TOKEN` per worker launch |
| Reducer call | One `describe`, `decide`, or `apply` request | No independent call token; calls on one worker are serialized |

A direct child belongs to its parent even if it calls `setsid` and leaves
the original process group. An inherited launch marker identifies a
detached descendant after its parent exits; a process-group ID alone does
not. The Rust worker also inherits the outer transport marker, so outer
cleanup must cover the nested worker group and its descendants.

The root ownership states are **live**, **exited but not reaped**, and
**reaped**. A live or unreaped owned root retains its PID; after `poll`,
`wait`, or `try_wait` reaps it, that PID may be reused and no longer authorizes a
process-group signal or ancestry traversal. Token matching remains
independent of that root PID. A successful reducer reply ends the call,
not the worker lifetime; a successful session response ends the request,
not the session lifetime.

| Exit | Required process transition |
|---|---|
| Successful reducer call, including no-op/domain refusal | Rust terminates the call's descendants; the idle worker reaps its terminated direct children while preserving its request loop |
| Inner worker timeout/protocol failure | Discard the worker; terminate descendants and reap the owned root before returning the failure |
| Worker recycling or normal `ik` exit | Close worker stdin, clean descendants, and reap the worker |
| Successful one-shot invocation/discovery | Reap the root and clean remaining marker-bearing descendants without using the reaped PID as group authority |
| Outer invocation/session timeout or broken session | Clean the nested worker tree as well as `ik`, reap `ik`, and keep the session unusable; never retry |
| Normal session close | EOF terminates `ik` and its worker; cleanup retains the session marker until the owned root is reaped and residual descendants are addressed |

Sending a signal is not evidence that a descendant PID has disappeared.
Rust chooses which descendants to terminate. The persistent Python worker,
as their direct parent, reaps terminated children while idle. Its `SIGCHLD`
handler does not reap during a reducer callback; a post-callback drain
handles child exits deferred during that callback. Callback-owned
`subprocess` waits therefore retain their actual exit statuses. This is
mechanical child reaping, not another command or transaction authority.
If the worker itself exits, reaping responsibility for remaining descendants
passes to the operating system.

Both supervisors use `ps eww -ax -o pid=,ppid=,command=` on macOS and
`ps eww ax -o pid=,ppid=,command=` on Linux to discover descendants: BSD
`ps` rejects bare `ax` after `eww`, and procps rejects the dashed form.
Cleanup snapshots descendants only while the root
remains owned, checks for root exit before group signalling, and uses the
launch token independently after reaping. Session failure and close share
this tree-cleanup path with one-shot invocations. Python does not suppress
`PermissionError` from either group or descendant signalling.


## Session and transaction

`ik --store <path> --session` serves one canonical JSON request per line and
emits one response per line, holding the registered machine/reducer and the
SQLite store open. Every request still goes through the same validation,
capability preflight, decide/apply, journal and projection path as one-shot
mode, so committed outcomes are identical (IK-PROP-010); a malformed request
is refused without ending the session; EOF exits 0. Python: `Kernel(...,
session=True)` (off by default; one-shot stays the compatibility mode) or `transport.Session`.
A session never retries: a timeout, broken pipe, mismatched `request_id` or
garbled line marks it unusable and kills the child. Concurrent callers on one
`Session` are serialized.

`op: "transaction"` applies an ordered list of 1..=64 `commands` under one
capability as one all-or-nothing SQLite transaction. The kernel threads each
step's successor capability; callers supply none per step. A failing step
rolls back every step (`transaction step <i>: ...`), only the last step may
create an effect intent (`IK_TRANSACTION_BOUNDARY`), and a crash inside the
transaction leaves exactly the previous committed state. Python:
`Aggregate.transaction([...])`.

The optional external reducer launch spec travels as `reducer_worker` in the
request envelope, not in process argv. It is mutually exclusive with the CLI
`--machine` source and is accepted only for
`create` / `command` / `replay` / `verify`; `record` and `show` remain
storage-only, reject the field, and never start a reducer.

Reducer output is deterministic for the same bound identity, state, command,
and observation. An emitted event is the only path to `apply`; a `noop` does
not apply an event. `effect` and `outcome` are optional conditional results
of `apply`: omitted fields remain absent, while present values are preserved
by Rust under native effect/outcome rules. External commands have one
intentional precedence difference: stale capability preflight wins even when
running the hidden reducer would have reproduced an already-recorded effect
outcome. This preserves the stronger guarantee that no Python worker runs for
any stale external command. Consumers that already possess an effect outcome
use Rust's storage-only `record` operation. A malformed request, unknown
field, identity mismatch, malformed response, non-finite number, or
unsupported return value fails closed and does not commit canonical state.

Binding discovery invokes `describe` once. After binding, `create`, `record`,
and `show` do not launch Python; semantic launch is limited to `command`,
`replay`, and `verify`. Before starting an external command worker Rust
performs capability preflight, so a stale capability is rejected as
`IK_CAPABILITY_STALE` before Python runs. Rust then executes the semantic
result inside its transaction. Rust alone commits canonical currentness,
journal, and projection state, and remains the authority for replay and
verification. A reducer, worker, or transport failure causes zero canonical
mutation.

Rust launches workers directly, never through a shell. It bounds stdout and
stderr and tags each worker tree with a launch-only process token. Cleanup
kills both process-group members and token-bearing descendants, including a
descendant that created another process group, after success or timeout. The
outer Python transport applies the same output and process-tree boundary to
the `ik` process.
A reducer exception is reported as `IK_INTERNAL` with diagnostic code
`reducer.exception`; returning `ReducerDiagnostic` is a domain rejection,
not an internal exception.

## Trust boundary

Python owns deterministic semantic reduction: interpreting command and
observation inputs, choosing `noop` or an event, and applying an event to
produce semantic state/effect/outcome values. Python does **not** own
canonical storage, commits, currentness, capabilities, journal entries,
projections, replay authority, or verification authority. It receives no store
path and cannot mutate canonical storage through this API.

This is a trusted, cooperative process boundary, not an OS security sandbox.
The worker protocol does not prevent Python code from opening files, creating
processes, or performing other IO on its own. “No store path” and failure
atomicity are lifecycle guarantees, not OS-enforced IO denial.

## Consumer shape (not a migration claim)

A WEDDING-WEEKEND-01 consumer with a `D_KISS_NOAH` command can use the generic
API without moving lifecycle authority into the game code:

1. The consumer calls `scene.execute({"name": "kiss_noah"}, observation)`.
2. The reducer examines immutable observation/state inputs and emits a
   versioned event only when the scene's domain rules permit the kiss; a
   disallowed attempt returns a `ReducerDiagnostic` (or an explicit no-op,
   according to the domain contract).
3. Rust validates the capability, invokes `apply` for the event, and commits
   the resulting current state and journal/projection records.
4. A later command uses the new Rust-bound handle; an old handle is stale.

This documents the intended consumer shape only. It does not claim that the
interactive-fiction WEDDING-WEEKEND-01 repository or `D_KISS_NOAH` is already
migrated.

Install the wheel from the private GitHub Release. The wheel does not bundle
`ik`; set `IK_BINARY` or `PATH`. Do not adopt from a source checkout of
`main`.

## Recovering an uncertain command

Every mutation (`command`, `execute`, `transaction`, `record`) is dispatched
once under an operation id. Pass `operation_id=` to `execute`/`transaction`/
`record`, or let the client generate one; `Kernel.new_operation_id()` makes one
before dispatch so you can persist it (with `aggregate.capability.to_wire()` and
the request body) before your process can die. The client never retries and
never refreshes a handle.

Every failure carries `error.operation_id` and `error.submission`:

| `error.submission` | Meaning | Example | Next step |
|---|---|---|---|
| `not-sent` | Provably never dispatched | `PreDispatchError` (missing `ik`) | Fix and send again with the same id |
| `refused` | Kernel answered with a diagnostic | `StaleCapabilityError`, `OperationConflictError` | Act on the diagnostic |
| `submission-unknown` | May have committed | `TransportError` after timeout | `Machine.lookup_operation(...)` |

Persist the description before sending and reconcile from it after a crash (the restart-safe form;
the runnable test is `python/tests/test_invocation_recovery.py`):

```python
invocation = aggregate.prepare({"name": "start"})
job_record["invocation"] = invocation.to_json()   # in your durable job record, before dispatch
machine.dispatch(invocation)                       # exactly once; no retry
# after a restart:
found = machine.reconcile(Invocation.from_json(job_record["invocation"]))
```

```python
operation_id = kernel.new_operation_id()
wire = aggregate.capability.to_wire()            # persist both before dispatch
try:
    aggregate.execute({"name": "start"}, operation_id=operation_id)
except TransportError as err:                    # submission-unknown
    fresh = Kernel(store).machine(path)
    found = fresh.lookup_operation(
        "attempt-1", Capability.from_wire(wire), err.operation_id,
        command={"name": "start"},
    )
    # found.outcome: committed_exact | conflict | not_recorded
    # conflict: the lookup fingerprint does not match the original request.
```

`classify(error_or_history)` returns the stable domain-adapter class:
`not_sent`, `refused`, `contract_mismatch`, `not_found`, `corrupt`,
`historical`, or `uncertain`. `historical` is a `HistoricalResult`, not an
exception. `StoreNotFoundError` is `IK_STORE_NOT_FOUND`. Rule and the diagnostic
mapping stay on the exception. See
[error taxonomy](design/error-taxonomy.md).

An exact repeat of a committed operation returns a `HistoricalResult`: history
with current coordinates, no capability and no state. Call `machine.show(id)`
for a current handle. `not_recorded` is not permission to retry under a stale
handle. `replay`/`verify` audit the journal; they do not answer whether an
operation committed.

## Recording an effect outcome

`aggregate.pending_intent` is the exact pending effect identity. It is `None` when nothing is pending. `record_effect_outcome` copies effect id, aggregate id, planned generation, payload digest, and idempotency key from that intent. It does not invent a digest, call an external service, replace the planned generation with the current generation, or retry. A transport failure after dispatch stays submission-unknown. An empty digest, or an intent that is not the pending one, raises `EffectCompletionError` (`not-sent`) before dispatch. The existing `record` helper is unchanged.

A partial effect object on the wire is `ProtocolError` (`IK_PROTOCOL_INVALID`) and echoes the request id. A wrong planned generation or payload digest is `EffectConflictError`. Another aggregate intent is `EffectMismatchError`. Nothing is recorded. The same outcome again is a no-op. A different digest conflicts and the first outcome stands.

A crash after `plan` commits leaves the pending intent: reopen and `show`. A crash after the external system answered, before record, is that same intent plus the digest the host saved. A dropped record reply is `lookup_operation` with the original capability, operation id, and effect body, not a second record. `finish` is a later command. A crash between record and finish leaves the outcome recorded and the domain state still `planned`.

The runnable recipe below uses `machines/effect_probe.ik.toml`. Set `STORE`, `MACHINE_PATH`, and `IK_BINARY`.

```python
# effect-completion-recipe
import hashlib
import json
import os
import threading
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from invariant_kernel import Capability, Kernel

STORE = os.environ["STORE"]
MACHINE_PATH = os.environ["MACHINE_PATH"]
RESPONSE_BODY = '{"ok":true}'


def digest_of(body):
    return hashlib.sha256(body.encode("utf-8")).hexdigest()


def effect_body(intent, outcome_digest):
    return {
        "effect_id": intent.effect_id,
        "aggregate_id": intent.aggregate_id,
        "planned_generation": intent.generation,
        "payload_digest": intent.payload_digest,
        "idempotency_key": intent.idempotency_key,
        "outcome_digest": outcome_digest,
    }


def start_service(body, hits):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            length = int(self.headers.get("Content-Length", "0"))
            self.rfile.read(length)
            hits.append(1)
            payload = body.encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, fmt, *args):
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    host, port = server.server_address
    return server, "http://%s:%s/work" % (host, port)


def post_work(url, payload_digest):
    data = json.dumps({"payload_digest": payload_digest}).encode("utf-8")
    request = urllib.request.Request(url, data=data, method="POST")
    with urllib.request.urlopen(request) as response:
        return response.read().decode("utf-8")


def open_handle(aggregate_id=None):
    kernel = Kernel(STORE)
    machine = kernel.machine(MACHINE_PATH)
    aggregate = machine.show(aggregate_id) if aggregate_id else None
    return kernel, machine, aggregate


def plan():
    _kernel, machine, _aggregate = open_handle()
    created = machine.create("paper-1")
    planned = created.plan(
        effect_id="publish-1",
        payload_digest="artifact-digest",
        idempotency_key="publish-1",
    )
    if planned.pending_intent is None:
        raise RuntimeError("plan did not leave a pending intent")
    return planned.id, planned.pending_intent


def main():
    aggregate_id, intent = plan()
    _kernel, _machine, after_commit = open_handle(aggregate_id)
    pending = after_commit.pending_intent
    if pending is None or pending.effect_id != intent.effect_id:
        raise RuntimeError("crash after commit lost the pending intent")
    hits = []
    server, url = start_service(RESPONSE_BODY, hits)
    try:
        response_body = post_work(url, pending.payload_digest)
        outcome_digest = digest_of(response_body)
        kernel, _machine, after_host = open_handle(aggregate_id)
        if after_host.pending_intent is None:
            raise RuntimeError("crash after the external call lost the pending intent")
        before_record = len(hits)
        operation_id = kernel.new_operation_id()
        capability = after_host.capability.to_wire()
        effect = effect_body(after_host.pending_intent, outcome_digest)
        after_host.record_effect_outcome(
            outcome_digest,
            intent=after_host.pending_intent,
            operation_id=operation_id,
        )
        if len(hits) != before_record:
            raise RuntimeError("record_effect_outcome called the fake service")
    finally:
        server.shutdown()
    _fresh_kernel, fresh, current = open_handle(aggregate_id)
    found = fresh.lookup_operation(
        aggregate_id,
        Capability.from_wire(capability),
        operation_id,
        effect=effect,
    )
    if found.outcome != "committed_exact":
        raise RuntimeError("dropped reply was not committed_exact")
    finished = current.finish(result_digest=outcome_digest)
    print(json.dumps({
        "aggregateId": aggregate_id,
        "responseBody": response_body,
        "serviceCalls": len(hits),
        "operationId": operation_id,
        "lookup": found.outcome,
        "plannedGeneration": pending.generation,
        "recordedGeneration": current.effect.intent.generation,
        "recordedOutcome": current.effect.outcome_digest,
        "state": finished.state,
    }))


if __name__ == "__main__":
    main()
```
