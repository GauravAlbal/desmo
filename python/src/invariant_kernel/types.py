"""Opaque typed values. Users do not invent these."""

from __future__ import annotations

from typing import Any, Mapping


class MachineIdentity:
    __slots__ = ("machine_id", "version", "_spec_hash")

    def __init__(self, machine_id: str, version: int, spec_hash: str) -> None:
        self.machine_id = machine_id
        self.version = int(version)
        self._spec_hash = spec_hash

    @classmethod
    def from_wire(cls, data: Mapping[str, Any]) -> "MachineIdentity":
        return cls(str(data["machine_id"]), int(data["version"]), str(data["spec_hash"]))

    def to_wire(self) -> dict[str, Any]:
        return {
            "machine_id": self.machine_id,
            "version": self.version,
            "spec_hash": self._spec_hash,
        }

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, MachineIdentity):
            return NotImplemented
        return (self.machine_id, self.version, self._spec_hash) == (other.machine_id, other.version, other._spec_hash)

    def __hash__(self) -> int:
        return hash((self.machine_id, self.version, self._spec_hash))

    def __repr__(self) -> str:
        return f"MachineIdentity({self.machine_id!r}, version={self.version})"


class Capability:
    """Opaque mutation authority. Do not construct or thread this by hand."""

    __slots__ = ("_generation", "_nonce")

    def __init__(self, generation: int, nonce: str) -> None:
        self._generation = int(generation)
        self._nonce = str(nonce)

    @classmethod
    def from_wire(cls, data: Mapping[str, Any]) -> "Capability":
        return cls(int(data["generation"]), str(data["nonce"]))

    def to_wire(self) -> dict[str, Any]:
        return {"generation": self._generation, "nonce": self._nonce}

    def __repr__(self) -> str:
        return "Capability(<opaque>)"


class Receipt:
    __slots__ = ("receipt_id", "aggregate_id", "sequence", "generation", "raw")

    def __init__(self, raw: Mapping[str, Any]) -> None:
        self.raw = dict(raw)
        self.receipt_id = str(raw.get("receipt_id", ""))
        self.aggregate_id = str(raw.get("aggregate_id", ""))
        self.sequence = int(raw.get("sequence", 0))
        self.generation = int(raw.get("generation", 0))

    def __repr__(self) -> str:
        return f"Receipt({self.receipt_id!r})"


class EffectIntent:
    __slots__ = (
        "effect_id",
        "aggregate_id",
        "generation",
        "effect_kind",
        "payload_digest",
        "idempotency_key",
        "payload",
    )

    def __init__(self, raw: Mapping[str, Any]) -> None:
        self.effect_id = str(raw["effect_id"])
        self.aggregate_id = str(raw["aggregate_id"])
        self.generation = int(raw["generation"])
        self.effect_kind = str(raw.get("effect_kind", ""))
        self.payload_digest = str(raw["payload_digest"])
        self.idempotency_key = str(raw["idempotency_key"])
        self.payload = raw.get("payload")

    def same_binding(self, other: "EffectIntent") -> bool:
        return (
            self.effect_id == other.effect_id
            and self.aggregate_id == other.aggregate_id
            and self.generation == other.generation
            and self.effect_kind == other.effect_kind
            and self.payload_digest == other.payload_digest
            and self.idempotency_key == other.idempotency_key
        )


class Effect:
    __slots__ = ("status", "intent", "outcome_digest", "receipt_id")

    def __init__(self, raw: Mapping[str, Any] | None) -> None:
        data = dict(raw or {"status": "none"})
        self.status = str(data.get("status", "none"))
        intent = data.get("intent")
        self.intent = EffectIntent(intent) if isinstance(intent, Mapping) else None
        self.outcome_digest = data.get("outcome_digest")
        self.receipt_id = data.get("receipt_id")

    def __repr__(self) -> str:
        return f"Effect(status={self.status!r})"


class CheckResult:
    """Developer ``ik check`` result.

    ``verdict`` is ``accept`` or ``refused`` (exit 0 or 1). The JSON
    document's ``judgment.status`` is ``accepted``, ``missing_evidence``,
    or ``witnessed_violation``; this wrapper is text-mode and exposes the
    verdict only.
    """

    __slots__ = ("text", "ok", "verdict")

    def __init__(self, text: str, ok: bool) -> None:
        self.text = text
        self.ok = ok
        self.verdict = "accept" if ok else "refused"

    def __repr__(self) -> str:
        return f"CheckResult(verdict={self.verdict!r})"


class HistoricalResult:
    """An exact repeat of an already committed operation.

    This is history, not authority: it carries no capability and no state. ``current_*`` are the
    aggregate's coordinates when the repeat was answered and may be well past the commit.
    Re-read with ``Machine.show`` to obtain a current handle.
    """

    __slots__ = (
        "aggregate_id",
        "operation_id",
        "kind",
        "first_sequence",
        "last_sequence",
        "generation",
        "event_hashes",
        "receipt_ids",
        "current_generation",
        "current_sequence",
        "raw",
    )

    def __init__(self, aggregate_id: str, history: Mapping[str, Any], raw: Mapping[str, Any]) -> None:
        record = history["record"]
        self.aggregate_id = aggregate_id
        self.operation_id = str(record["operation_id"])
        self.kind = str(record["kind"])
        self.first_sequence = int(record["first_sequence"])
        self.last_sequence = int(record["last_sequence"])
        self.generation = int(record["generation"])
        self.event_hashes = list(record["event_hashes"])
        self.receipt_ids = list(record["receipt_ids"])
        self.current_generation = int(history["current_generation"])
        self.current_sequence = int(history["current_sequence"])
        self.raw = dict(raw)

    @classmethod
    def from_mutation(cls, result: Mapping[str, Any]) -> "HistoricalResult":
        current = result["current"]
        history = {
            "record": result["operation"],
            "current_generation": current["generation"],
            "current_sequence": current["sequence"],
        }
        return cls(str(result.get("aggregate_id", "")), history, result)

    def __repr__(self) -> str:
        return (
            f"HistoricalResult({self.operation_id!r}, sequences "
            f"{self.first_sequence}..{self.last_sequence}, current generation {self.current_generation})"
        )


class OperationLookup:
    """Outcome of ``lookup_operation``. ``outcome`` is ``committed_exact``, ``conflict`` or
    ``not_recorded``. ``not_recorded`` is not permission to retry under a stale handle."""

    __slots__ = ("outcome", "history", "current_generation", "current_sequence", "raw")

    def __init__(self, aggregate_id: str, result: Mapping[str, Any]) -> None:
        self.outcome = str(result["outcome"])
        self.history = (
            HistoricalResult(aggregate_id, result, result) if self.outcome == "committed_exact" else None
        )
        generation = result.get("current_generation")
        sequence = result.get("current_sequence")
        self.current_generation = int(generation) if generation is not None else None
        self.current_sequence = int(sequence) if sequence is not None else None
        self.raw = dict(result)

    def __repr__(self) -> str:
        return f"OperationLookup({self.outcome!r})"
