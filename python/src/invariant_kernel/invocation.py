"""A recoverable description of one logical invocation.

``Aggregate.prepare`` / ``prepare_transaction`` build an ``Invocation`` BEFORE anything is sent. It
holds the operation id and the original request (machine identity, aggregate, the capability the
request was authorized against, and the command or steps). A host stores ``to_wire()`` in the
durable job or request record it already keeps, then calls ``Machine.dispatch``. After a crash it
loads that record with ``from_wire`` and calls ``Machine.reconcile``: an exact lookup of the
original operation under the original request, which answers ``committed_exact``, ``conflict`` or
``not_recorded``.

Nothing here refreshes a capability, derives an operation id from the current generation, or
resends. If the description was lost, the invocation cannot be attributed from the store alone:
do not pick a plausible row from ``ik operations``.
"""

from __future__ import annotations

import json
from typing import Any, Mapping, Optional

from .types import Capability, MachineIdentity

WIRE_VERSION = 1
KINDS = ("command", "transaction")


def _check_shape(kind: str, operation_id: Any, command: Any, steps: Any) -> None:
    if kind not in KINDS:
        raise ValueError(f"invocation kind must be one of {KINDS}")
    if not isinstance(operation_id, str) or operation_id == "":
        raise ValueError("an invocation needs a non-empty operation id")
    if kind == "command" and (command is None or steps is not None):
        raise ValueError("a command invocation carries a command and no steps")
    if kind == "transaction" and (steps is None or command is not None):
        raise ValueError("a transaction invocation carries steps and no command")


class Invocation:
    """An owned, immutable snapshot of one request.

    The request is serialized once, at construction, into a private string. Every read
    (``command``, ``steps``, ``to_wire``, ``dispatch``, ``reconcile``) parses a fresh copy, so
    mutating an argument object after ``prepare``, or a value returned from this object, cannot
    change the bound request. A value that is not JSON-representable is rejected here, before
    anything is sent. A different request needs a new invocation.
    """

    __slots__ = ("_json",)

    def __init__(
        self,
        *,
        kind: str,
        machine: MachineIdentity,
        aggregate_id: str,
        operation_id: str,
        capability: Capability,
        command: Optional[Mapping[str, Any]] = None,
        observation: Optional[Mapping[str, Any]] = None,
        steps: Optional[list[Mapping[str, Any]]] = None,
    ) -> None:
        _check_shape(kind, operation_id, command, steps)
        wire: dict[str, Any] = {
            "invocation_version": WIRE_VERSION,
            "kind": kind,
            "machine": machine.to_wire(),
            "aggregate_id": aggregate_id,
            "operation_id": operation_id,
            "capability": capability.to_wire(),
        }
        if command is not None:
            wire["command"] = command
        if observation is not None:
            wire["observation"] = observation
        if steps is not None:
            wire["steps"] = list(steps)
        object.__setattr__(self, "_json", json.dumps(wire, sort_keys=True, separators=(",", ":"), allow_nan=False))

    def __setattr__(self, name: str, value: Any) -> None:
        raise AttributeError("an Invocation is immutable; prepare a new one")

    def _wire(self) -> dict[str, Any]:
        return json.loads(self._json)

    @property
    def kind(self) -> str:
        return self._wire()["kind"]

    @property
    def machine(self) -> MachineIdentity:
        return MachineIdentity.from_wire(self._wire()["machine"])

    @property
    def aggregate_id(self) -> str:
        return self._wire()["aggregate_id"]

    @property
    def operation_id(self) -> str:
        return self._wire()["operation_id"]

    @property
    def capability(self) -> Capability:
        return Capability.from_wire(self._wire()["capability"])

    @property
    def command(self) -> Optional[dict[str, Any]]:
        return self._wire().get("command")

    @property
    def observation(self) -> Optional[dict[str, Any]]:
        return self._wire().get("observation")

    @property
    def steps(self) -> Optional[list[dict[str, Any]]]:
        return self._wire().get("steps")

    def to_wire(self) -> dict[str, Any]:
        """A fresh JSON-safe copy for the host's durable record; mutating it changes nothing."""
        return self._wire()

    def to_json(self) -> str:
        return self._json

    @classmethod
    def from_wire(cls, data: Mapping[str, Any]) -> "Invocation":
        if data.get("invocation_version") != WIRE_VERSION:
            raise ValueError("unknown invocation_version")
        known = {
            "invocation_version", "kind", "machine", "aggregate_id", "operation_id",
            "capability", "command", "observation", "steps",
        }
        extra = set(data) - known
        if extra:
            raise ValueError(f"unknown invocation fields: {sorted(extra)}")
        return cls(
            kind=str(data["kind"]),
            machine=MachineIdentity.from_wire(data["machine"]),
            aggregate_id=str(data["aggregate_id"]),
            operation_id=str(data["operation_id"]),
            capability=Capability.from_wire(data["capability"]),
            command=data.get("command"),
            observation=data.get("observation"),
            steps=data.get("steps"),
        )

    @classmethod
    def from_json(cls, text: str) -> "Invocation":
        return cls.from_wire(json.loads(text))

    def __repr__(self) -> str:
        return f"Invocation({self.kind} {self.operation_id} on {self.aggregate_id})"
