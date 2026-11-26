"""Typed adapter over the authoritative `ik` executable."""

from __future__ import annotations

import difflib
import threading
import uuid
import weakref
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Optional


from . import transport
from .errors import EffectCompletionError, TransportError
from .invocation import Invocation
from .reducer import ReducerIdentity, binding_descriptor, canonical_digest, freeze
from .types import (
    Capability,
    CheckResult,
    Effect,
    EffectIntent,
    EventCursorRow,
    EventFeed,
    EventProjection,
    HistoricalResult,
    MachineIdentity,
    OperationLookup,
    Receipt,
)

def _call_context(
    machine: Optional["Machine"],
    op: str,
    fields: Mapping[str, Any],
) -> tuple[Optional[MachineIdentity], Optional[Path], Optional[Mapping[str, Any]]]:
    if machine is None or not machine.identity.machine_id:
        return None, None, None
    identity = machine.identity
    path = machine.path
    if machine.worker_spec is None or op not in {
        "create",
        "create_with_command",
        "command",
        "replay",
        "verify",
        "transaction",
    }:
        return identity, path, None
    for field in ("command", "observation"):
        if fields.get(field) is not None:
            freeze(fields[field])
    for step in fields.get("commands") or ():
        freeze(step["command"])
        if step.get("observation") is not None:
            freeze(step["observation"])
    return identity, path, machine.worker_spec


def _close_sessions_map(sessions: dict[tuple[Optional[str], Optional[str]], transport.Session]) -> None:
    items = list(sessions.values())
    sessions.clear()
    for open_session in items:
        try:
            open_session.close()
        except Exception:
            pass


class Kernel:
    def __init__(
        self,
        store: str | Path,
        *,
        binary: Optional[str] = None,
        timeout: float = 30.0,
        session: bool = False,
    ) -> None:
        self.store = Path(store)
        self.binary = transport.locate_binary(binary)
        self.timeout = timeout
        runtime, major, minor = transport.parse_version(self.binary)
        self._runtime_version = runtime
        self._protocol = {"major": major, "minor": minor}
        self._next_id = 1
        self._id_lock = threading.Lock()
        # Persistent `ik --session` processes, keyed by (machine_path, reducer_spec_hash)
        # so two external reducers never collide on the same session slot.
        self._use_session = session
        self._sessions_lock = threading.Lock()
        self._sessions: dict[tuple[Optional[str], Optional[str]], transport.Session] = {}
        self._uncertain_ops: set[str] = set()
        self._finalizer = weakref.finalize(self, _close_sessions_map, self._sessions)

    def close(self) -> None:
        """Close persistent sessions. One-shot kernels have nothing to close."""
        with self._sessions_lock:
            sessions, self._sessions = self._sessions, {}
        for open_session in sessions.values():
            open_session.close()

    def reset_sessions(self) -> None:
        """Close and evict broken/open sessions so new operations can proceed cleanly.

        Any operation id that was in flight when a session died remains recorded as uncertain
        and still refuses resend until reconciled via ``lookup_operation``.
        """
        with self._sessions_lock:
            sessions, self._sessions = self._sessions, {}
            for open_session in sessions.values():
                self._uncertain_ops.update(open_session._uncertain)
        for open_session in sessions.values():
            open_session.close()

    def __enter__(self) -> "Kernel":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()

    def _session_for(
        self, path: Optional[Path], identity: Optional[MachineIdentity] = None
    ) -> "transport.Session":
        key: tuple[Optional[str], Optional[str]] = (
            (str(path), None)
            if path is not None
            else (None, identity.to_wire().get("spec_hash") if identity is not None else None)
        )
        with self._sessions_lock:
            open_session = self._sessions.get(key)
            if open_session is None:
                open_session = transport.Session(
                    self.binary, self.store, path, timeout=self.timeout
                )
                self._sessions[key] = open_session
            return open_session

    def machine(self, path: str | Path) -> "Machine":
        return Machine.from_file(self, path)

    def reducer(
        self,
        entrypoint: str,
        *,
        python: Optional[str] = None,
        worker_timeout: Optional[float] = None,
        kernel_laws: Optional[list[Mapping[str, Any]]] = None,
        code_source: Optional[str] = None,
    ) -> "Machine":
        """Discover and bind a source-checkout-safe external reducer."""
        inner_timeout = (
            worker_timeout
            if worker_timeout is not None
            else min(5.0, self.timeout - 0.5)
        )
        if inner_timeout <= 0 or inner_timeout >= self.timeout:
            raise ValueError("worker_timeout must be positive and shorter than Kernel timeout")
        discovery_timeout = min(
            self.timeout,
            max(1.0, inner_timeout + 0.5),
        )
        response = transport.invoke_worker(
            entrypoint,
            None,
            op="describe",
            python=python,
            worker_timeout=discovery_timeout,
        )
        result = response.get("result")
        identity_wire = response.get("reducer_identity")
        if not isinstance(result, Mapping) or set(result) != {"initial_state"}:
            raise TransportError(
                "IK_TRANSPORT", "reducer describe result must contain only initial_state"
            )
        if not isinstance(identity_wire, Mapping):
            raise TransportError(
                "IK_TRANSPORT", "reducer describe response missing identity"
            )
        identity = ReducerIdentity.from_wire(identity_wire)
        laws = [dict(law) for law in kernel_laws] if kernel_laws else None
        descriptor = binding_descriptor(
            entrypoint, identity, result["initial_state"], kernel_laws=laws
        )
        command = transport.reducer_worker_command(python)
        spec = dict(descriptor)
        spec.pop("kernel_laws_digest", None)
        spec["command"] = command
        spec["initial_state"] = result["initial_state"]
        spec["timeout_ms"] = max(1, int(inner_timeout * 1000))
        if laws:
            spec["kernel_laws"] = laws
        if code_source:
            spec["code_source"] = code_source
        machine_wire = {
            "machine_id": identity.reducer_name,
            "version": identity.reducer_version,
            "spec_hash": canonical_digest(descriptor),
        }
        return Machine(
            self,
            None,
            MachineIdentity.from_wire(machine_wire),
            worker_spec=spec,
        )

    def check(
        self,
        path: str | Path,
        *,
        prior: Optional[str | Path] = None,
        corpus: Optional[str | Path] = None,
        baseline: Optional[str | Path] = None,
    ) -> CheckResult:
        text, ok = transport.run_check(
            self.binary,
            path,
            prior=prior,
            corpus=corpus,
            baseline=baseline,
            timeout=self.timeout,
        )
        return CheckResult(text, ok)

    def show(self, aggregate_id: str, machine: Optional["Machine"] = None) -> "Aggregate":
        result = self._call("show", machine, aggregate_id=aggregate_id)
        handle = machine or _Detached(self)
        return Aggregate.from_result(handle, result, created=False)

    def replay(self, machine: "Machine", aggregate_id: Optional[str] = None) -> dict[str, Any]:
        return self._call("replay", machine, aggregate_id=aggregate_id)

    def verify(self, machine: "Machine", aggregate_id: Optional[str] = None) -> dict[str, Any]:
        return self._call("verify", machine, aggregate_id=aggregate_id)

    def events(
        self,
        *,
        since: int = 0,
        aggregate_id: Optional[str] = None,
        limit: Optional[int] = None,
    ) -> EventFeed:
        """Stream committed events strictly after ``since`` paired with latest aggregate state."""
        raw = transport.run_events(
            self.binary,
            self.store,
            since=since,
            aggregate_id=aggregate_id,
            limit=limit,
            timeout=self.timeout,
        )
        rows: list[EventCursorRow] = []
        for item in raw["events"]:
            proj = item["current_projection"]
            rows.append(
                EventCursorRow(
                    cursor=int(item["cursor"]),
                    aggregate_id=str(item["aggregate_id"]),
                    sequence=int(item["sequence"]),
                    generation=int(item["generation"]),
                    machine_id=str(item["machine_id"]),
                    machine_version=int(item["machine_version"]),
                    machine_spec_hash=str(item["machine_spec_hash"]),
                    event_kind=str(item["event_kind"]),
                    event_type=str(item["event_type"]),
                    event_version=int(item["event_version"]),
                    command=item["command"],
                    state_digest=str(item["state_digest"]),
                    terminal=bool(item["terminal"]),
                    effect_delta=item["effect_delta"],
                    event_hash=str(item["event_hash"]),
                    prev_event_hash=str(item["prev_event_hash"]),
                    is_latest_for_aggregate=bool(item["is_latest_for_aggregate"]),
                    current_projection=EventProjection(
                        state=proj["state"],
                        sequence=int(proj["sequence"]),
                        generation=int(proj["generation"]),
                        terminal=bool(proj["terminal"]),
                    ),
                    raw=dict(item),
                )
            )
        return EventFeed(
            schema=str(raw["schema"]),
            store=str(raw["store"]),
            since=int(raw["since"]),
            next_cursor=int(raw["next_cursor"]),
            has_more=bool(raw["has_more"]),
            events=rows,
            raw=dict(raw),
        )

    def raw(self) -> "RawKernelClient":
        return RawKernelClient(self)

    def new_operation_id(self) -> str:
        """A fresh caller operation id. Persist it before dispatch to survive your own crash."""
        return f"py-{uuid.uuid4().hex}"

    def _request_id(self) -> str:
        with self._id_lock:
            rid = f"py-{self._next_id}"
            self._next_id += 1
            return rid

    def _envelope(
        self,
        op: str,
        *,
        aggregate_id: Optional[str] = None,
        machine: Optional[MachineIdentity] = None,
        capability: Optional[Capability] = None,
        command: Optional[Mapping[str, Any]] = None,
        observation: Optional[Mapping[str, Any]] = None,
        effect: Optional[Mapping[str, Any]] = None,
        commands: Optional[list[Mapping[str, Any]]] = None,
        operation_id: Optional[str] = None,
    ) -> dict[str, Any]:
        env: dict[str, Any] = {
            "protocol": dict(self._protocol),
            "runtime_version": self._runtime_version,
            "request_id": self._request_id(),
            "op": op,
        }
        if aggregate_id is not None:
            env["aggregate_id"] = aggregate_id
        if machine is not None:
            env["machine"] = machine.to_wire()
        if capability is not None:
            env["capability"] = capability.to_wire()
        if command is not None:
            env["command"] = dict(command)
        if observation is not None:
            env["observation"] = dict(observation)
        if effect is not None:
            env["effect"] = dict(effect)
        if commands is not None:
            env["commands"] = [dict(step) for step in commands]
        if operation_id is not None:
            env["operation_id"] = operation_id
        return env

    def _call_bound(
        self,
        op: str,
        machine: Optional["Machine"],
        operation_id: Optional[str],
        **fields: Any,
    ) -> dict[str, Any]:
        """Dispatch once under ``operation_id`` (generated when omitted); never retries.

        Any failure carries the id as ``error.operation_id`` so the caller can reconcile with
        ``Machine.lookup_operation`` instead of guessing.
        """
        bound = operation_id or self.new_operation_id()
        try:
            return self._call(op, machine, operation_id=bound, **fields)
        except Exception as err:
            err.operation_id = bound  # type: ignore[attr-defined]
            raise

    def _call(
        self,
        op: str,
        machine: Optional["Machine"],
        **fields: Any,
    ) -> dict[str, Any]:
        identity, path, worker_spec = _call_context(machine, op, fields)
        envelope = self._envelope(op, machine=identity, **fields)
        if worker_spec is not None:
            envelope["reducer_worker"] = dict(worker_spec)
        if self._use_session:
            op_id = str(envelope.get("operation_id") or "")
            if op_id and op_id in self._uncertain_ops and op != "lookup_operation":
                raise TransportError(
                    "IK_TRANSPORT",
                    "an earlier attempt under this operation id was in flight when a session died; "
                    "reconcile with lookup_operation instead of resending",
                )
            return self._session_for(path, identity).invoke(envelope, self.timeout)
        return transport.invoke(
            self.binary, self.store, path, envelope, self.timeout
        )


class Machine:
    def __init__(
        self,
        kernel: Kernel,
        path: str | Path | None,
        identity: MachineIdentity,
        *,
        worker_spec: Optional[Mapping[str, Any]] = None,
    ) -> None:
        self.kernel = kernel
        self.path = Path(path) if path is not None else None
        self.identity = identity
        self.worker_spec = dict(worker_spec) if worker_spec is not None else None
        self._declared_commands: Optional[frozenset[str]] = None

    def declared_commands(self) -> Optional[frozenset[str]]:
        if self.path is None:
            return None
        if self._declared_commands is None:
            try:
                schema = transport.run_schema(self.kernel.binary, self.path, self.kernel.timeout)
                cmds = schema.get("commands") or []
                self._declared_commands = frozenset(
                    str(c["name"]) for c in cmds if isinstance(c, Mapping) and "name" in c
                )
            except Exception:
                return None
        return self._declared_commands


    @classmethod
    def from_file(cls, kernel: Kernel, path: str | Path) -> "Machine":
        data = transport.run_identity(kernel.binary, path, kernel.timeout)
        return cls(kernel, path, MachineIdentity.from_wire(data))

    def lookup_operation(
        self,
        aggregate_id: str,
        capability: Capability,
        operation_id: str,
        *,
        command: Optional[Mapping[str, Any]] = None,
        observation: Optional[Mapping[str, Any]] = None,
        steps: Optional[list[Mapping[str, Any]]] = None,
        effect: Optional[Mapping[str, Any]] = None,
    ) -> OperationLookup:
        """Ask whether ``operation_id`` committed, without mutating anything.

        Pass the ORIGINAL request body (the same command/steps/effect and the capability the
        original attempt used). A changed body reports ``conflict``. This is not replay: replay
        and verify audit the journal, they do not answer whether an operation committed.
        """
        result = self.kernel._call(
            "lookup_operation",
            self,
            operation_id=operation_id,
            aggregate_id=aggregate_id,
            capability=capability,
            command=dict(command) if command is not None else None,
            observation=dict(observation) if observation is not None else None,
            commands=[dict(step) for step in steps] if steps is not None else None,
            effect=dict(effect) if effect is not None else None,
        )
        return OperationLookup(aggregate_id, result)

    def create(self, aggregate_id: str) -> "Aggregate":
        result = self.kernel._call("create", self, aggregate_id=aggregate_id)
        return Aggregate.from_result(self, result, created=True)

    def create_with(
        self,
        aggregate_id: str,
        command: Mapping[str, Any],
        *,
        operation_id: Optional[str] = None,
        observation: Optional[Mapping[str, Any]] = None,
    ) -> "Aggregate | HistoricalResult":
        """Create and apply the initializing command in one validated operation.

        An invalid command or a law violation exposes no aggregate. The same operation id
        replays as history. Reconcile a dropped reply with ``lookup_create_with``. A bare
        ``create`` followed by a required initialize is not this operation.
        """
        result = self.kernel._call_bound(
            "create_with_command",
            self,
            operation_id,
            aggregate_id=aggregate_id,
            command=dict(command),
            observation=dict(observation) if observation is not None else None,
        )
        return _settle(self, result)

    def lookup_create_with(
        self,
        aggregate_id: str,
        operation_id: str,
        command: Mapping[str, Any],
        *,
        observation: Optional[Mapping[str, Any]] = None,
    ) -> OperationLookup:
        """Ask whether a create_with operation id committed. Do not send a capability."""
        result = self.kernel._call(
            "lookup_operation",
            self,
            operation_id=operation_id,
            aggregate_id=aggregate_id,
            command=dict(command),
            observation=dict(observation) if observation is not None else None,
        )
        return OperationLookup(aggregate_id, result)

    def dispatch(self, invocation: Invocation) -> "Aggregate | HistoricalResult":
        """Send a prepared invocation exactly once under its own operation id and capability.

        No retry and no handle refresh. If this raises, reconcile with ``reconcile`` using the
        same invocation; do not resend under a new id.
        """
        self._check_invocation(invocation)
        if invocation.kind == "command":
            result = self.kernel._call_bound(
                "command",
                self,
                invocation.operation_id,
                aggregate_id=invocation.aggregate_id,
                capability=invocation.capability,
                command=dict(invocation.command or {}),
                observation=dict(invocation.observation) if invocation.observation is not None else None,
            )
        else:
            result = self.kernel._call_bound(
                "transaction",
                self,
                invocation.operation_id,
                aggregate_id=invocation.aggregate_id,
                capability=invocation.capability,
                commands=[dict(step) for step in invocation.steps or []],
            )
        return _settle(self, result)

    def reconcile(self, invocation: Invocation) -> OperationLookup:
        """Exact lookup of a prepared invocation: did THIS operation id commit under THIS request?

        ``committed_exact`` is history, not a current handle; ``conflict`` means the id was used
        for a different request; ``not_recorded`` is not permission to resend under a refreshed
        handle (it may be running, undelivered, refused or a no-op).
        """
        self._check_invocation(invocation)
        return self.lookup_operation(
            invocation.aggregate_id,
            invocation.capability,
            invocation.operation_id,
            command=invocation.command if invocation.kind == "command" else None,
            observation=invocation.observation,
            steps=invocation.steps if invocation.kind == "transaction" else None,
        )

    def _check_invocation(self, invocation: Invocation) -> None:
        if not isinstance(invocation, Invocation):
            raise TypeError("expected an Invocation")
        if invocation.machine != self.identity:
            raise ValueError(
                "this invocation was prepared for a different machine identity than this Machine"
            )

    def show(self, aggregate_id: str) -> "Aggregate":
        return self.kernel.show(aggregate_id, self)

    def replay(self, aggregate_id: Optional[str] = None) -> dict[str, Any]:
        return self.kernel.replay(self, aggregate_id)

    def verify(self, aggregate_id: Optional[str] = None) -> dict[str, Any]:
        return self.kernel.verify(self, aggregate_id)


def _transaction_wire(
    steps: "list[Mapping[str, Any] | tuple[Mapping[str, Any], Optional[Mapping[str, Any]]]]",
) -> list[dict[str, Any]]:
    wire: list[dict[str, Any]] = []
    for item in steps:
        command, observation = item if isinstance(item, tuple) else (item, None)
        if not isinstance(command, Mapping):
            raise TypeError("each transaction command must be a mapping")
        if observation is not None and not isinstance(observation, Mapping):
            raise TypeError("each transaction observation must be a mapping or None")
        entry: dict[str, Any] = {"command": dict(command)}
        if observation is not None:
            entry["observation"] = dict(observation)
        wire.append(entry)
    return wire


@dataclass(frozen=True)
class EffectRunResult:
    """Result of ``Aggregate.run_effect``: planned handle, recorded result, intent, and digest."""

    planned: "Aggregate"
    recorded: "Aggregate | HistoricalResult"
    intent: EffectIntent
    outcome_digest: str
    result: Any = None


def _settle(machine: "Machine", result: Mapping[str, Any]) -> "Aggregate | HistoricalResult":
    """A mutation answer is a new handle, or history when it was an exact repeat."""
    if result.get("historical") is True:
        return HistoricalResult.from_mutation(result)
    return Aggregate.from_result(machine, result, created=False)


class Aggregate:
    def __init__(
        self,
        machine: Machine,
        aggregate_id: str,
        *,
        capability: Capability,
        state: Any,
        generation: int,
        terminal: bool,
        effect: Effect,
        receipt: Optional[Receipt],
        mutation: Optional[bool],
        raw: Mapping[str, Any],
    ) -> None:
        self._machine = machine
        self.id = aggregate_id
        self._capability = capability
        self._raw_state = state
        self.generation = generation
        self.terminal = terminal
        self.effect = effect
        self.receipt = receipt
        self.mutation = mutation
        self.raw = dict(raw)

    @classmethod
    def from_result(
        cls,
        machine: Machine,
        result: Mapping[str, Any],
        *,
        created: bool,
    ) -> "Aggregate":
        del created
        cap = result.get("capability")
        if not isinstance(cap, Mapping):
            raise TransportError("IK_TRANSPORT", "result missing capability")
        receipt = result.get("receipt") or result.get("last_receipt")
        return cls(
            machine,
            str(result.get("aggregate_id", "")),
            capability=Capability.from_wire(cap),
            state=result.get("state"),
            generation=int(result.get("generation", 0)),
            terminal=bool(result.get("terminal", False)),
            effect=Effect(result.get("effect") if isinstance(result.get("effect"), Mapping) else None),
            receipt=Receipt(receipt) if isinstance(receipt, Mapping) else None,
            mutation=result.get("mutation") if isinstance(result.get("mutation"), bool) else None,
            raw=result,
        )

    @property
    def state(self) -> Any:
        if isinstance(self._raw_state, Mapping) and "state" in self._raw_state:
            return self._raw_state["state"]
        return self._raw_state

    @property
    def facts(self) -> dict[str, Any]:
        if not isinstance(self._raw_state, Mapping):
            return {}
        return {k: v for k, v in self._raw_state.items() if k != "state"}

    @property
    def owner(self) -> MachineIdentity:
        """The identity that owns this aggregate in the store.

        After a machine version change it can differ from ``machine.identity`` on a ``show`` through another
        version. Route an existing aggregate to the retained definition whose ``identity`` equals ``owner``."""
        wire = self.raw.get("machine")
        if isinstance(wire, Mapping):
            return MachineIdentity.from_wire(wire)
        return self._machine.identity

    @property
    def capability(self) -> Capability:
        """The current mutation authority. Persist ``capability.to_wire()`` with your operation id
        before dispatch if you need to recover after your own process dies."""
        return self._capability

    def command(self, name: str, **args: Any) -> "Aggregate | HistoricalResult":
        """Invoke a command under a freshly generated operation id (see ``execute`` to supply one)."""
        payload: dict[str, Any] = {"name": name}
        if args:
            payload["args"] = args
        return self.execute(payload)

    def record(
        self,
        outcome_digest: str,
        *,
        effect_id: Optional[str] = None,
        payload_digest: Optional[str] = None,
        idempotency_key: Optional[str] = None,
        planned_generation: Optional[int] = None,
        operation_id: Optional[str] = None,
    ) -> "Aggregate | HistoricalResult":
        intent = self.effect.intent
        wire = {
            "effect_id": effect_id or (intent.effect_id if intent else ""),
            "aggregate_id": self.id,
            "planned_generation": planned_generation
            if planned_generation is not None
            else (intent.generation if intent else self.generation),
            "payload_digest": payload_digest or (intent.payload_digest if intent else ""),
            "idempotency_key": idempotency_key or (intent.idempotency_key if intent else ""),
            "outcome_digest": outcome_digest,
        }
        result = self._machine.kernel._call_bound(
            "record",
            self._machine,
            operation_id,
            aggregate_id=self.id,
            capability=self._capability,
            effect=wire,
        )
        return _settle(self._machine, result)

    @property
    def pending_intent(self) -> Optional[EffectIntent]:
        """The pending effect intent, or None when nothing is pending."""
        if self.effect.status != "pending":
            return None
        return self.effect.intent

    def record_effect_outcome(
        self,
        outcome_digest: str,
        *,
        intent: Optional[EffectIntent] = None,
        operation_id: Optional[str] = None,
    ) -> "Aggregate | HistoricalResult":
        """Bind ``outcome_digest`` to this aggregate's pending intent.

        Identity fields are copied from that intent, never from the current
        generation. The digest is caller-supplied. Dispatch is exactly once;
        a transport failure stays submission-unknown and is not retried. This
        method does not perform external I/O.
        """
        try:
            pending = _completion_intent(self.effect, outcome_digest, intent)
        except EffectCompletionError as err:
            err.operation_id = operation_id
            raise
        result = self._machine.kernel._call_bound(
            "record",
            self._machine,
            operation_id,
            aggregate_id=self.id,
            capability=self._capability,
            effect=_outcome_wire(pending, outcome_digest),
        )
        return _settle(self._machine, result)

    def run_effect(
        self,
        command_name: str,
        args: Optional[Mapping[str, Any]] = None,
        *,
        executor: Any,
        plan_operation_id: Optional[str] = None,
        record_operation_id: Optional[str] = None,
    ) -> EffectRunResult:
        """Plan an external effect, invoke ``executor(intent)``, and record the outcome digest.

        ``executor`` receives the kernel-bound ``EffectIntent`` (with ``idempotency_key``)
        and must return either a non-empty ``outcome_digest`` string or a
        ``(outcome_digest, result)`` pair. Refuses if planning replays as
        ``HistoricalResult`` or commits without a pending intent. Never retries transport
        failures silently.
        """
        planned = self.execute(
            {"name": command_name, "args": dict(args or {})},
            operation_id=plan_operation_id,
        )
        if isinstance(planned, HistoricalResult):
            err = EffectCompletionError(
                f"planning command `{command_name}` replayed as HistoricalResult; "
                "inspect show() and reconcile before re-running external side effects"
            )
            err.operation_id = plan_operation_id
            raise err
        intent = planned.pending_intent
        if intent is None:
            err = EffectCompletionError(
                f"command `{command_name}` committed without planning a pending effect intent"
            )
            err.operation_id = plan_operation_id
            raise err
        executed = executor(intent)
        if isinstance(executed, tuple) and len(executed) == 2:
            outcome_digest, extra_result = executed
        else:
            outcome_digest, extra_result = executed, None
        recorded = planned.record_effect_outcome(
            outcome_digest,
            intent=intent,
            operation_id=record_operation_id,
        )
        return EffectRunResult(
            planned=planned,
            recorded=recorded,
            intent=intent,
            outcome_digest=outcome_digest,
            result=extra_result,
        )

    def execute(
        self,
        command: Mapping[str, Any],
        observation: Optional[Mapping[str, Any]] = None,
        operation_id: Optional[str] = None,
    ) -> "Aggregate | HistoricalResult":
        """Dispatch one command exactly once under ``operation_id`` (generated when omitted).

        There is no automatic retry or handle refresh. If this raises, ``error.operation_id`` is
        the id to reconcile with ``Machine.lookup_operation``. An exact repeat of a committed
        operation returns a ``HistoricalResult`` (history, not a usable handle).
        """
        if not isinstance(command, Mapping):
            raise TypeError("command must be a mapping")
        if observation is not None and not isinstance(observation, Mapping):
            raise TypeError("observation must be a mapping or None")
        result = self._machine.kernel._call_bound(
            "command",
            self._machine,
            operation_id,
            aggregate_id=self.id,
            capability=self._capability,
            command=dict(command),
            observation=dict(observation) if observation is not None else None,
        )
        return _settle(self._machine, result)

    def prepare(
        self,
        command: Mapping[str, Any],
        observation: Optional[Mapping[str, Any]] = None,
        operation_id: Optional[str] = None,
    ) -> Invocation:
        """Describe one command invocation BEFORE sending it.

        Returns the operation id and the original request, including the capability this handle
        holds. Persist ``invocation.to_wire()`` in your durable job record, then pass it to
        ``Machine.dispatch``. Nothing is sent here.
        """
        if not isinstance(command, Mapping):
            raise TypeError("command must be a mapping")
        if observation is not None and not isinstance(observation, Mapping):
            raise TypeError("observation must be a mapping or None")
        return Invocation(
            kind="command",
            machine=self._machine.identity,
            aggregate_id=self.id,
            operation_id=operation_id or self._machine.kernel.new_operation_id(),
            capability=self._capability,
            command=command,
            observation=observation,
        )

    def prepare_transaction(
        self,
        steps: "list[Mapping[str, Any] | tuple[Mapping[str, Any], Optional[Mapping[str, Any]]]]",
        operation_id: Optional[str] = None,
    ) -> Invocation:
        """Describe an atomic transaction BEFORE sending it; see ``prepare``."""
        return Invocation(
            kind="transaction",
            machine=self._machine.identity,
            aggregate_id=self.id,
            operation_id=operation_id or self._machine.kernel.new_operation_id(),
            capability=self._capability,
            steps=_transaction_wire(steps),
        )

    def transaction(
        self,
        steps: "list[Mapping[str, Any] | tuple[Mapping[str, Any], Optional[Mapping[str, Any]]]]",
        operation_id: Optional[str] = None,
    ) -> "Aggregate | HistoricalResult":
        """Apply ordered commands atomically (all-or-nothing) in ONE store transaction.

        Each item is a command mapping or a ``(command, observation)`` pair. Only the last step may
        create an effect intent; a failure at any step rolls back every step and raises the native
        diagnostic (its message names the failing ``transaction step N``). Per-step receipts are in
        ``result.raw["steps"]``; the returned aggregate is the state after the last step.
        """
        wire = _transaction_wire(steps)
        result = self._machine.kernel._call_bound(
            "transaction",
            self._machine,
            operation_id,
            aggregate_id=self.id,
            capability=self._capability,
            commands=wire,
        )
        return _settle(self._machine, result)

    def show(self) -> "Aggregate":
        return self._machine.show(self.id)

    def replay(self) -> dict[str, Any]:
        return self._machine.replay(self.id)

    def kernel_verify(self) -> dict[str, Any]:
        return self._machine.verify(self.id)

    def __getattr__(self, name: str):
        if name.startswith("_"):
            raise AttributeError(name)
        declared = self._machine.declared_commands()
        if declared is not None and name not in declared:
            close = difflib.get_close_matches(name, sorted(declared), n=1)
            hint = f" (did you mean `{close[0]}`?)" if close else ""
            raise AttributeError(
                f"Aggregate has no attribute or declared command `{name}`{hint}; "
                f"declared commands: {sorted(declared)}"
            )

        def _invoke(**kwargs: Any) -> Aggregate:
            return self.command(name, **kwargs)

        return _invoke


class RawKernelClient:
    """Escape hatch: send a protocol envelope without constructing it by hand in app code."""

    def __init__(self, kernel: Kernel) -> None:
        self.kernel = kernel

    def call(self, op: str, **fields: Any) -> dict[str, Any]:
        machine: Optional[Machine] = fields.pop("bound_machine", None)
        identity = fields.pop("machine", None)
        if isinstance(identity, MachineIdentity):
            fields["machine"] = identity
        elif identity is not None and machine is None:
            raise TransportError("IK_TRANSPORT", "raw machine must be a MachineIdentity")
        if machine is None and "path" in fields:
            raise TransportError("IK_TRANSPORT", "pass bound_machine=Machine, not a raw path")
        return self.kernel._call(op, machine, **fields)


class _Detached(Machine):
    def __init__(self, kernel: Kernel) -> None:
        self.kernel = kernel
        self.path = Path()
        self.identity = MachineIdentity("", 0, "")


def _completion_intent(
    effect: Effect, outcome_digest: str, supplied: Optional[EffectIntent]
) -> EffectIntent:
    if not isinstance(outcome_digest, str) or outcome_digest == "":
        raise EffectCompletionError("outcome digest is required and must be non-empty")
    pending = effect.intent if effect.status == "pending" else None
    if pending is None:
        raise EffectCompletionError("no pending effect intent to bind")
    if supplied is not None and not pending.same_binding(supplied):
        raise EffectCompletionError("intent is not this aggregate's pending effect intent")
    return pending


def _outcome_wire(intent: EffectIntent, outcome_digest: str) -> dict[str, Any]:
    return {
        "effect_id": intent.effect_id,
        "aggregate_id": intent.aggregate_id,
        "planned_generation": intent.generation,
        "payload_digest": intent.payload_digest,
        "idempotency_key": intent.idempotency_key,
        "outcome_digest": outcome_digest,
    }
