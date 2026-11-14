"""A small exact-artifact approval/publication worker around Desmo (IK-09).

The application owns immutable artifact bytes and a mutable working copy; the kernel owns
the approval lifecycle. Publication is an *effect* bound to the exact approved digest: the
executor reads the immutable bytes for that digest, never "latest", and the outcome is
recorded against the kernel's pending intent. External execution is at-least-once with an
idempotent sink; nothing here claims exactly-once.
"""

from __future__ import annotations

import hashlib
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Optional, Sequence

from invariant_kernel import (
    Aggregate,
    HistoricalResult,
    Kernel,
    KernelError,
    Machine,
    TransportError,
)


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class ArtifactIntegrityError(Exception):
    """Stored bytes do not hash to the digest they are filed under."""


class ExecutorSubstitutionError(Exception):
    """The executor produced bytes other than the authorized immutable artifact."""


class UnroutableAggregate(Exception):
    """No loaded definition matches the aggregate's persisted machine identity."""


class ArtifactStore:
    """Content-addressed, write-once object store."""

    def __init__(self, root: Path) -> None:
        self.objects = root / "objects"
        self.objects.mkdir(parents=True, exist_ok=True)

    def put(self, data: bytes) -> str:
        digest = sha256_hex(data)
        path = self.objects / digest
        if path.exists():
            if path.read_bytes() != data:
                raise ArtifactIntegrityError(f"stored bytes differ for {digest}")
            return digest
        tmp = self.objects / f".{digest}.tmp"
        tmp.write_bytes(data)
        os.chmod(tmp, 0o444)
        os.replace(tmp, path)
        return digest

    def get(self, digest: str) -> bytes:
        data = (self.objects / digest).read_bytes()
        if sha256_hex(data) != digest:
            raise ArtifactIntegrityError(f"stored bytes do not hash to {digest}")
        return data


class DraftStore:
    """The mutable working copy. The executor must never read it."""

    def __init__(self, root: Path) -> None:
        self.root = root / "drafts"
        self.root.mkdir(parents=True, exist_ok=True)

    def write(self, doc_id: str, data: bytes) -> None:
        (self.root / f"{doc_id}.bin").write_bytes(data)

    def read(self, doc_id: str) -> bytes:
        return (self.root / f"{doc_id}.bin").read_bytes()


@dataclass(frozen=True)
class Published:
    digest: str
    location: str
    outcome_digest: str
    duplicate: bool


class PublishExecutor:
    """Publishes the exact authorized immutable bytes to an idempotent sink."""

    def __init__(self, artifacts: ArtifactStore, sink: Path) -> None:
        self.artifacts = artifacts
        self.sink = sink
        self.sink.mkdir(parents=True, exist_ok=True)

    def source_bytes(self, effect_id: str, payload_digest: str) -> bytes:
        del effect_id
        return self.artifacts.get(payload_digest)

    def execute(self, effect_id: str, payload_digest: str) -> Published:
        data = self.source_bytes(effect_id, payload_digest)
        if sha256_hex(data) != payload_digest:
            raise ExecutorSubstitutionError(
                f"executor holds {sha256_hex(data)}, authorized {payload_digest}: nothing written"
            )
        target = self.sink / f"{effect_id}.bin"
        duplicate = target.exists()
        if duplicate and target.read_bytes() != data:
            raise ArtifactIntegrityError(f"sink already holds different bytes for {effect_id}")
        if not duplicate:
            target.write_bytes(data)
        digest = sha256_hex(data)
        return Published(digest, str(target), sha256_hex(f"{digest}:{effect_id}".encode()), duplicate)


class LatestContentExecutor(PublishExecutor):
    """DELIBERATELY WRONG: fetches the mutable working copy instead of the authorized bytes."""

    def __init__(self, artifacts: ArtifactStore, sink: Path, drafts: DraftStore, doc_id: str) -> None:
        super().__init__(artifacts, sink)
        self.drafts = drafts
        self.doc_id = doc_id

    def source_bytes(self, effect_id: str, payload_digest: str) -> bytes:
        del effect_id, payload_digest
        return self.drafts.read(self.doc_id)


class Trace:
    """Events in the `ikaros.trace.v2` vocabulary, recorded from what actually happened."""

    def __init__(self) -> None:
        self.events: list[dict[str, str]] = []

    def commit(self, attempted: str, before: str, after: str, generation: int) -> None:
        self.events.append(
            {"kind": "commit", "attempted": attempted, "fromState": before, "toState": after, "generation": str(generation)}
        )

    def noop(self, attempted: str, state: str, generation: int) -> None:
        self.events.append({"kind": "no-op", "attempted": attempted, "state": state, "generation": str(generation)})

    def refusal(self, attempted: str, before: str, err: KernelError, intended: Optional[str]) -> None:
        event = {
            "kind": "refusal",
            "attempted": attempted,
            "fromState": before,
            "code": err.code,
            "message": err.message,
        }
        if intended:
            event["intendedState"] = intended
        self.events.append(event)

    def unknown(self, attempted: str, before: str, err: TransportError) -> None:
        self.events.append(
            {
                "kind": "unknown",
                "attempted": attempted,
                "fromState": before,
                "message": err.message,
                "continuation": "reconcile with lookup_operation using the operation id",
            }
        )

    def read(self, attempted: str, state: str, generation: int, message: str = "") -> None:
        event = {"kind": "read", "attempted": attempted, "state": state, "generation": str(generation)}
        if message:
            event["message"] = message
        self.events.append(event)

    def historical(self, attempted: str, history: HistoricalResult) -> None:
        self.events.append(
            {
                "kind": "historical",
                "attempted": attempted,
                "operationId": history.operation_id,
                "firstSequence": str(history.first_sequence),
                "lastSequence": str(history.last_sequence),
            }
        )


class VersionRouter:
    """Routes by each aggregate's *persisted* machine identity, never by a global default.

    New aggregates use `create_version`; existing aggregates stay on the definition they were
    created under. Rolling the application back changes only `create_version`.
    """

    def __init__(self, kernel: Kernel, definitions: Mapping[int, Path], create_version: int) -> None:
        self.kernel = kernel
        self.machines: dict[int, Machine] = {v: kernel.machine(p) for v, p in definitions.items()}
        self.create_version = create_version
        self.draining: set[int] = set()

    def new_machine(self) -> Machine:
        if self.create_version in self.draining:
            raise UnroutableAggregate(f"version {self.create_version} is draining: no new aggregates")
        return self.machines[self.create_version]

    def machine_for(self, doc_id: str) -> Machine:
        identity = self.kernel.show(doc_id).raw["machine"]
        machine = self.machines.get(int(identity["version"]))
        if machine is None or machine.identity.to_wire() != dict(identity):
            raise UnroutableAggregate(
                f"{doc_id} was created under {identity['machine_id']} v{identity['version']} "
                f"({identity['spec_hash']}), which is not loaded"
            )
        return machine


class NaiveRollbackRouter(VersionRouter):
    """DELIBERATELY WRONG: routes everything by the current default after a rollback."""

    def machine_for(self, doc_id: str) -> Machine:
        del doc_id
        return self.machines[self.create_version]


class Projection:
    """A host-side read model. It is never consulted to learn whether a command committed."""

    def __init__(self, path: Path) -> None:
        self.path = path
        self.rows: dict[str, dict[str, Any]] = {}

    def apply(self, aggregate: Aggregate) -> None:
        self.rows[aggregate.id] = {"state": aggregate.state, "generation": aggregate.generation}
        self.save()

    def save(self) -> None:
        import json

        self.path.write_text(json.dumps(self.rows, sort_keys=True))


class ApprovalApp:
    def __init__(
        self,
        router: VersionRouter,
        artifacts: ArtifactStore,
        drafts: DraftStore,
        sink: Path,
        trace: Optional[Trace] = None,
        projection: Optional[Projection] = None,
    ) -> None:
        self.router = router
        self.artifacts = artifacts
        self.drafts = drafts
        self.sink = sink
        self.trace = trace or Trace()
        self.projection = projection

    # -- kernel access -------------------------------------------------------------------

    def create(self, doc_id: str) -> Aggregate:
        doc = self.router.new_machine().create(doc_id)
        self.trace.read("create", doc.state, doc.generation, "new aggregate")
        return doc

    def fresh(self, doc_id: str) -> Aggregate:
        doc = self.router.machine_for(doc_id).show(doc_id)
        self.trace.read("show", doc.state, doc.generation, "canonical read")
        return doc

    def _send(
        self,
        doc: Aggregate,
        name: str,
        args: Optional[Mapping[str, Any]] = None,
        *,
        intends: Optional[str] = None,
        operation_id: Optional[str] = None,
    ) -> Aggregate:
        command: dict[str, Any] = {"name": name}
        if args:
            command["args"] = dict(args)
        before = doc.state
        try:
            result = doc.execute(command, operation_id=operation_id)
        except TransportError as err:
            self.trace.unknown(name, before, err)
            raise
        except KernelError as err:
            self.trace.refusal(name, before, err, intends)
            raise
        if isinstance(result, HistoricalResult):
            self.trace.historical(name, result)
            raise RuntimeError("an exact repeat is history, not a handle; read with fresh()")
        if result.mutation:
            self.trace.commit(name, before, result.state, result.generation)
        else:
            self.trace.noop(name, result.state, result.generation)
        if self.projection is not None:
            self.projection.apply(result)
        return result

    # -- lifecycle -----------------------------------------------------------------------

    def edit(self, doc: Aggregate, data: bytes) -> Aggregate:
        digest = self.artifacts.put(data)
        self.drafts.write(doc.id, data)
        return self._send(doc, "edit", {"artifact_digest": digest})

    def approve(self, doc: Aggregate) -> Aggregate:
        return self._send(doc, "approve", {"artifact_digest": sha256_hex(self.drafts.read(doc.id))}, intends="approved")

    def revise(self, doc: Aggregate, data: bytes) -> Aggregate:
        digest = self.artifacts.put(data)
        self.drafts.write(doc.id, data)
        return self._send(doc, "revise", {"artifact_digest": digest}, intends="draft")

    def authorize_publish(
        self,
        doc: Aggregate,
        *,
        payload_digest: Optional[str] = None,
        operation_id: Optional[str] = None,
    ) -> Aggregate:
        digest = payload_digest or str(doc.facts["approved_digest"])
        effect_id = f"publish-{doc.id}-{digest[:12]}"
        args = {"effect_id": effect_id, "payload_digest": digest, "idempotency_key": effect_id}
        return self._send(doc, "authorize_publish", args, intends="publishing", operation_id=operation_id)

    def complete(self, doc: Aggregate, published_digest: str) -> Aggregate:
        return self._send(doc, "complete", {"published_digest": published_digest}, intends="published")

    # -- effect --------------------------------------------------------------------------

    def run_executor(self, doc_id: str, executor: Optional[PublishExecutor] = None) -> Published:
        """Execute the kernel's pending intent. Bytes come from the authorized digest only."""
        intent = self.fresh(doc_id).effect.intent
        if intent is None:
            raise RuntimeError(f"{doc_id} has no pending effect intent")
        executor = executor or PublishExecutor(self.artifacts, self.sink)
        return executor.execute(intent.effect_id, intent.payload_digest)

    def record_outcome(self, doc: Aggregate, published: Published) -> Aggregate:
        before = doc.state
        result = doc.record(published.outcome_digest)
        if isinstance(result, HistoricalResult):
            raise RuntimeError("unexpected historical record result")
        label = "record (duplicate)" if not result.mutation else "record"
        if result.mutation:
            self.trace.commit(label, before, result.state, result.generation)
        else:
            self.trace.noop(label, result.state, result.generation)
        return result

    def reconcile_projection(self, doc_id: str) -> Aggregate:
        """Repair the host read model from the canonical aggregate (never the reverse)."""
        doc = self.fresh(doc_id)
        if self.projection is not None:
            self.projection.apply(doc)
        return doc


def effect_summary(aggregate: Aggregate) -> Sequence[str]:
    intent = aggregate.effect.intent
    return [aggregate.effect.status, intent.payload_digest if intent else ""]
