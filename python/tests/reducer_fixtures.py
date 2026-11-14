"""Stable reducer entrypoints used by the cross-language conformance tests.

The classes in this module deliberately contain all behavior needed by the worker
protocol tests. They are importable by ``module:qualname`` and take no arguments.
"""

from __future__ import annotations

import os
import subprocess
import sys
import time
from typing import Any, Mapping, Optional

from invariant_kernel.reducer import ApplyOutcome, Decision, DomainEvent, ReducerDiagnostic, ReducerIdentity


_DIGEST_A = "a" * 64
_DIGEST_B = "b" * 64
_DIGEST_C = "c" * 64


def _identity(name: str = "native-parity") -> ReducerIdentity:
    return ReducerIdentity(
        reducer_name=name,
        reducer_version=1,
        code_digest=_DIGEST_A,
        command_schema_digest=_DIGEST_B,
        state_schema_digest=_DIGEST_C,
    )


def _initial_state() -> dict[str, Any]:
    return {
        "clock": 0,
        "facts": {"kiss_noah": False},
        "obligation": "none",
        "observed": None,
        "relationship": {"appraisal": 0, "stage": "strangers", "trust": 0},
        "route": "main",
        "score": 0,
    }


def _arg(command: Mapping[str, Any], name: str, default: Any = None) -> Any:
    args = command.get("args", {})
    return args.get(name, default) if isinstance(args, Mapping) else default


class ParityReducer:
    """Reference reducer mirrored by ``tests/reducer_conformance.rs``."""

    identity = _identity()

    def initial_state(self) -> Mapping[str, Any]:
        return _initial_state()
    def decide(
        self,
        state: Mapping[str, Any],
        observation: Optional[Mapping[str, Any]],
        effect: Any,
        command: Mapping[str, Any],
    ) -> Decision:
        del state, effect
        name = command.get("name")
        if name == "accept":
            amount = _arg(command, "amount", 0)
            event = DomainEvent(
                "parity.accepted",
                1,
                {"amount": amount, "observed": observation},
            )
            return Decision.emit(event).with_note({"kind": "accepted", "amount": amount})
        if name == "reject":
            return ReducerDiagnostic("IK_INVARIANT_FAILED", "command was rejected", "parity.reject")
        if name == "noop":
            return Decision.noop().with_note({"kind": "noop", "reason": "nothing_to_do"})
        if name == "plan_effect":
            enabled = bool(_arg(command, "enabled", False))
            if not enabled:
                return Decision.noop().with_note({"kind": "noop", "reason": "effect_disabled"})
            return Decision.emit(
                DomainEvent("parity.effect_planned", 1, {"enabled": True})
            )
        if name == "resolve_effect":
            return Decision.emit(DomainEvent("parity.effect_resolved", 1, {}))
        if name == "finish":
            return Decision.emit(DomainEvent("parity.finished", 1, {}))
        if name == "kiss_noah":
            event = DomainEvent(
                "wedding.kiss_noah",
                1,
                {
                    "appraisal_delta": _arg(command, "appraisal_delta", 0),
                    "clock": _arg(command, "clock", 0),
                    "observation": observation,
                    "obligation": "tell_noah_the_truth",
                    "route": _arg(command, "route", "noah"),
                    "stage": _arg(command, "stage", "chosen"),
                    "trust_delta": _arg(command, "trust_delta", 0),
                },
            )
            return Decision.emit(event).with_note(
                {"scene": "WEDDING-WEEKEND-01", "transition": "D_KISS_NOAH"}
            )
        return ReducerDiagnostic("IK_COMMAND_INVALID", "unknown command", "parity.command")

    def apply(self, state: Mapping[str, Any], event: DomainEvent) -> ApplyOutcome:
        result = {**state}
        result["facts"] = dict(state["facts"])
        result["relationship"] = dict(state["relationship"])
        payload = event.payload
        if event.event_type == "parity.accepted" and event.event_version == 1:
            result["score"] = state["score"] + payload["amount"]
            result["observed"] = payload["observed"]
            return ApplyOutcome.new(result)
        if event.event_type == "parity.effect_planned" and event.event_version == 1:
            result["effect_state"] = "planned"
            effect = {
                "effect_id": "parity-effect-1",
                "aggregate_id": "parity-aggregate",
                "generation": 1,
                "effect_kind": "send.invitation",
                "payload_digest": _DIGEST_B,
                "idempotency_key": "parity-effect-1",
                "payload": {"to": "mira"},
            }
            return ApplyOutcome.new(result).with_effect(effect)
        if event.event_type == "parity.effect_resolved" and event.event_version == 1:
            result["effect_state"] = "resolved"
            outcome = {
                "effect_id": "parity-effect-1",
                "aggregate_id": "parity-aggregate",
                "planned_generation": 1,
                "payload_digest": _DIGEST_B,
                "idempotency_key": "parity-effect-1",
                "outcome_digest": _DIGEST_C,
            }
            return ApplyOutcome.new(result).with_outcome(outcome)
        if event.event_type == "parity.finished" and event.event_version == 1:
            result["finished"] = True
            return ApplyOutcome.new(result).with_terminal(True)
        if event.event_type == "wedding.kiss_noah" and event.event_version == 1:
            result["clock"] = payload["clock"]
            result["facts"]["kiss_noah"] = True
            result["obligation"] = payload["obligation"]
            result["observed"] = payload["observation"]
            result["relationship"]["appraisal"] = state["relationship"]["appraisal"] + payload["appraisal_delta"]
            result["relationship"]["stage"] = payload["stage"]
            result["relationship"]["trust"] = state["relationship"]["trust"] + payload["trust_delta"]
            result["route"] = payload["route"]
            return ApplyOutcome.new(result)
        return ReducerDiagnostic("IK_EVENT_SCHEMA_UNKNOWN", "unknown event", "parity.event")


class IdentityMismatchReducer(ParityReducer):
    identity = _identity("identity-mismatch")


class ParityReducerAlias(ParityReducer):
    identity = ParityReducer.identity


class AlternateInitialStateReducer(ParityReducer):
    def initial_state(self) -> Mapping[str, Any]:
        state = _initial_state()
        state["score"] = 1
        state["float_boundaries"] = [1e-5, 1e-6, 1e-7, 1e16, 1e20, -0.0]
        return state


class LargeInitialStateReducer(ParityReducer):
    def initial_state(self) -> Mapping[str, Any]:
        state = _initial_state()
        state["blob"] = "x" * (300 * 1024)
        return state


class OutOfRangeInitialStateReducer(ParityReducer):
    def initial_state(self) -> Mapping[str, Any]:
        return {"value": 1 << 64}


class InvocationMarkerReducer(ParityReducer):
    @staticmethod
    def _mark() -> None:
        marker = os.environ.get("IK_REDUCER_INVOCATION_FILE")
        if marker:
            with open(marker, "w", encoding="utf-8") as handle:
                handle.write("invoked")

    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Decision:
        self._mark()
        return super().decide(state, observation, effect, command)

    def apply(self, state: Mapping[str, Any], event: DomainEvent) -> ApplyOutcome:
        self._mark()
        return super().apply(state, event)


class RaisingReducer(ParityReducer):
    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Decision:
        del state, observation, effect, command
        raise RuntimeError("fixture exception")


class MalformedReducer(ParityReducer):
    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Any:
        del state, observation, effect, command
        return {"not": "a Decision"}



class RaisingApplyReducer(ParityReducer):
    def apply(self, state: Mapping[str, Any], event: DomainEvent) -> ApplyOutcome:
        del state, event
        raise RuntimeError("fixture apply exception")


class OutOfRangeApplyReducer(ParityReducer):
    def apply(self, state: Mapping[str, Any], event: DomainEvent) -> ApplyOutcome:
        del state, event
        return ApplyOutcome.new({"value": 1 << 64})

class MutationReducer(ParityReducer):
    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Decision:
        del observation, effect, command
        state["facts"]["kiss_noah"] = True
        return Decision.noop()



class NoisyReducer(ParityReducer):
    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Decision:
        del state, observation, effect, command
        print("x" * (1024 * 1024 + 1))
        return Decision.noop()

def _spawn_sleeping_descendant(*, detached: bool = False) -> subprocess.Popen[Any]:
    child = subprocess.Popen(
        [sys.executable, "-c", "import time; time.sleep(60)"],
        start_new_session=detached,
    )
    marker = os.environ.get("IK_DESCENDANT_PID_FILE")
    if marker:
        with open(marker, "w", encoding="utf-8") as handle:
            handle.write(str(child.pid))
    return child


class SleepingReducer(ParityReducer):
    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Decision:
        del state, observation, effect, command
        time.sleep(60)
        return Decision.noop()


class SpawnSleepingReducer(SleepingReducer):
    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Decision:
        del state, observation, effect, command
        _spawn_sleeping_descendant()
        time.sleep(60)
        return Decision.noop()


class DetachedSpawnSleepingReducer(SleepingReducer):
    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Decision:
        del state, observation, effect, command
        _spawn_sleeping_descendant(detached=True)
        time.sleep(60)
        return Decision.noop()


class SpawnOnDescribeReducer(ParityReducer):
    def initial_state(self) -> Mapping[str, Any]:
        _spawn_sleeping_descendant()
        return super().initial_state()


class DetachedOnDescribeReducer(ParityReducer):
    def initial_state(self) -> Mapping[str, Any]:
        _spawn_sleeping_descendant(detached=True)
        return super().initial_state()


class DetachedOnDecideReducer(ParityReducer):
    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Decision:
        _spawn_sleeping_descendant(detached=True)
        return super().decide(state, observation, effect, command)


class SlowTwoPhaseSpawnReducer(ParityReducer):
    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Decision:
        time.sleep(0.45)
        return super().decide(state, observation, effect, command)

    def apply(self, state: Mapping[str, Any], event: DomainEvent) -> ApplyOutcome:
        _spawn_sleeping_descendant()
        time.sleep(60)
        return super().apply(state, event)


def _record_worker_pid() -> None:
    """Test double: observe worker lifetime. Deliberately impure; never a real reducer pattern."""
    marker = os.environ.get("IK_WORKER_PID_FILE")
    if marker:
        with open(marker, "a", encoding="utf-8") as handle:
            handle.write(f"{os.getpid()}\n")


class PidRecordingReducer(ParityReducer):
    """Records the pid of the process serving every decide/apply; ``crash`` kills that process."""

    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Decision:
        _record_worker_pid()
        if command.get("name") == "crash":
            os._exit(7)
        return super().decide(state, observation, effect, command)

    def apply(self, state: Mapping[str, Any], event: DomainEvent) -> ApplyOutcome:
        _record_worker_pid()
        return super().apply(state, event)


class DetachedPidRecordingReducer(PidRecordingReducer):
    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Decision:
        _spawn_sleeping_descendant(detached=True)
        return super().decide(state, observation, effect, command)


class SubprocessExitStatusReducer(ParityReducer):
    def __init__(self) -> None:
        self.startup_exit = subprocess.run(
            [sys.executable, "-c", "raise SystemExit(5)"], check=False
        ).returncode

    def initial_state(self) -> Mapping[str, Any]:
        state = dict(super().initial_state())
        state["score"] = subprocess.run(
            [sys.executable, "-c", "raise SystemExit(3)"], check=False
        ).returncode
        return state

    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Decision:
        status = subprocess.run(
            [sys.executable, "-c", "raise SystemExit(7)"], check=False
        ).returncode
        return Decision.emit(DomainEvent(
            "parity.accepted", 1,
            {"amount": self.startup_exit + status, "observed": observation},
        ))


class LeakyReducer(ParityReducer):
    """Violates the purity contract: a module-level counter leaks between calls (paranoid-mode bait)."""

    calls = 0

    def decide(self, state: Mapping[str, Any], observation: Optional[Mapping[str, Any]], effect: Any, command: Mapping[str, Any]) -> Decision:
        LeakyReducer.calls += 1
        if command.get("name") == "accept":
            event = DomainEvent("parity.accepted", 1, {"amount": LeakyReducer.calls, "observed": observation})
            return Decision.emit(event)
        return super().decide(state, observation, effect, command)
