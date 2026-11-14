"""Versioned Python reducer contract and canonical binding helpers."""
from __future__ import annotations

import hashlib
import importlib
import json
import math
import inspect
import re
from dataclasses import dataclass
from types import MappingProxyType
from typing import Any, Mapping, Optional, Protocol, runtime_checkable

DESCRIPTOR_SCHEMA = "invariant-kernel.python-reducer/1"
PROTOCOL_MAJOR = 1
PROTOCOL_MINOR = 0
_HEX64 = re.compile(r"^[0-9a-f]{64}$")
_MIN_JSON_INTEGER = -(1 << 63)
_MAX_JSON_INTEGER = (1 << 64) - 1


def _validate_integer(value: int) -> int:
    if not _MIN_JSON_INTEGER <= value <= _MAX_JSON_INTEGER:
        raise ValueError(
            "reducer integers must fit the shared JSON range "
            "[-2^63, 2^64-1]"
        )
    return value


def _freeze_mapping(value: Mapping[Any, Any]) -> Any:
    if any(not isinstance(k, str) for k in value):
        raise TypeError("reducer object keys must be strings")
    return MappingProxyType({k: freeze(v) for k, v in value.items()})


def _freeze_sequence(value: list[Any] | tuple[Any, ...]) -> tuple[Any, ...]:
    return tuple(freeze(v) for v in value)


def freeze(value: Any) -> Any:
    """Recursively turn JSON-like input into immutable values."""
    if isinstance(value, Mapping):
        return _freeze_mapping(value)
    if isinstance(value, (list, tuple)):
        return _freeze_sequence(value)
    if type(value) is int:
        return _validate_integer(value)
    if isinstance(value, (str, float, bool)) or value is None:
        return value
    raise TypeError(f"reducer input is not JSON-compatible: {type(value).__name__}")


def thaw(value: Any) -> Any:
    if isinstance(value, Mapping):
        return {str(k): thaw(v) for k, v in value.items()}
    if isinstance(value, tuple):
        return [thaw(v) for v in value]
    return value


def _float_parts(text: str) -> tuple[str, str, int]:
    sign = ""
    if text.startswith("-"):
        sign, text = "-", text[1:]
    if "e" in text:
        mantissa, exponent_text = text.split("e", 1)
        exponent = int(exponent_text)
    else:
        mantissa, exponent = text, 0
    return sign, mantissa, exponent


def _normalized_float_parts(mantissa: str, exponent: int) -> tuple[str, int]:
    if "." in mantissa:
        whole, fraction = mantissa.split(".", 1)
        digits = whole + fraction
        exponent -= len(fraction)
    else:
        digits = mantissa
    digits = digits.lstrip("0") or "0"
    while len(digits) > 1 and digits.endswith("0"):
        digits = digits[:-1]
        exponent += 1
    return digits, exponent


def _format_float(sign: str, digits: str, exponent: int) -> str:
    length = len(digits)
    decimal_position = length + exponent
    if 0 <= exponent and decimal_position <= 16:
        return sign + digits + ("0" * exponent) + ".0"
    if 0 < decimal_position <= 16:
        return sign + digits[:decimal_position] + "." + digits[decimal_position:]
    if -5 < decimal_position <= 0:
        return sign + "0." + ("0" * -decimal_position) + digits
    scientific = digits if length == 1 else digits[0] + "." + digits[1:]
    scientific_exponent = decimal_position - 1
    exponent_sign = "+" if scientific_exponent >= 0 else "-"
    exponent_digits = str(abs(scientific_exponent))
    return sign + scientific + "e" + exponent_sign + exponent_digits


def _serde_float(value: float) -> str:
    if not math.isfinite(value):
        raise ValueError("non-finite floats are not valid reducer JSON")
    sign, mantissa, exponent = _float_parts(repr(value))
    digits, exponent = _normalized_float_parts(mantissa, exponent)
    return _format_float(sign, digits, exponent)


def _canonical_object(value: dict[str, Any]) -> str:
    if any(not isinstance(key, str) for key in value):
        raise TypeError("reducer object keys must be strings")
    return "{" + ",".join(
        json.dumps(key, ensure_ascii=False) + ":" + _canonical_value(value[key])
        for key in sorted(value)
    ) + "}"


def _canonical_value(value: Any) -> str:
    if value is None:
        return "null"
    if value is True:
        return "true"
    if value is False:
        return "false"
    if type(value) is int:
        return str(_validate_integer(value))
    if type(value) is float:
        return _serde_float(value)
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, list):
        return "[" + ",".join(_canonical_value(item) for item in value) + "]"
    if isinstance(value, dict):
        return _canonical_object(value)
    raise TypeError(f"value is not JSON-compatible: {type(value).__name__}")


def canonical_json(value: Any) -> str:
    """Emit the same sorted, compact JSON representation as serde_json."""
    return _canonical_value(thaw(value))


def canonical_digest(value: Any) -> str:
    return hashlib.sha256(canonical_json(value).encode("utf-8")).hexdigest()


def _digest(name: str, value: Any) -> str:
    if not isinstance(value, str) or not _HEX64.fullmatch(value):
        raise ValueError(f"{name} must be exactly 64 lowercase hexadecimal characters")
    return value


@dataclass(frozen=True)
class ReducerIdentity:
    reducer_name: str
    reducer_version: int
    code_digest: str
    command_schema_digest: str
    state_schema_digest: str

    def __post_init__(self) -> None:
        if not isinstance(self.reducer_name, str) or not self.reducer_name:
            raise ValueError("reducer_name must be a non-empty string")
        if type(self.reducer_version) is not int or self.reducer_version <= 0:
            raise ValueError("reducer_version must be a positive integer")
        for field_name in (
            "code_digest",
            "command_schema_digest",
            "state_schema_digest",
        ):
            _digest(field_name, getattr(self, field_name))

    def to_wire(self) -> dict[str, Any]:
        return {
            "reducer_name": self.reducer_name,
            "reducer_version": self.reducer_version,
            "code_digest": self.code_digest,
            "command_schema_digest": self.command_schema_digest,
            "state_schema_digest": self.state_schema_digest,
        }

    @classmethod
    def from_wire(cls, value: Mapping[str, Any]) -> "ReducerIdentity":
        _strict_object(value, set(cls.__dataclass_fields__))
        return cls(**dict(value))


@dataclass(frozen=True)
class DomainEvent:
    event_type: str
    event_version: int
    payload: Any

    def __post_init__(self) -> None:
        if not isinstance(self.event_type, str) or not self.event_type:
            raise ValueError("event_type must be a non-empty string")
        if type(self.event_version) is not int or self.event_version < 1:
            raise ValueError("event_version must be a positive integer")
        object.__setattr__(self, "payload", freeze(self.payload))

    def to_wire(self) -> dict[str, Any]:
        return {"event_type": self.event_type, "event_version": self.event_version, "payload": thaw(self.payload)}

    @classmethod
    def from_wire(cls, value: Mapping[str, Any]) -> "DomainEvent":
        _strict_object(value, {"event_type", "event_version", "payload"})
        return cls(value["event_type"], value["event_version"], value["payload"])


@dataclass(frozen=True, init=False)
class Decision:
    kind: str
    event: Optional[DomainEvent]
    note: Any

    def __init__(self, kind: str, event: Optional[DomainEvent] = None, note: Any = None) -> None:
        if kind not in {"emit", "noop"} or (kind == "emit") != (event is not None):
            raise ValueError("Decision must be emit with an event or noop")
        if event is not None and type(event) is not DomainEvent:
            raise TypeError("Decision event must be an exact DomainEvent")
        object.__setattr__(self, "kind", kind)
        object.__setattr__(self, "event", event)
        object.__setattr__(self, "note", None if note is None else freeze(note))

    @classmethod
    def emit(cls, event: DomainEvent) -> "Decision":
        return cls("emit", event)

    @classmethod
    def noop(cls) -> "Decision":
        return cls("noop")

    def with_note(self, note: Any) -> "Decision":
        return type(self)(self.kind, self.event, note)

    def to_wire(self) -> dict[str, Any]:
        out: dict[str, Any] = {"kind": self.kind}
        if self.event is not None:
            out["event"] = self.event.to_wire()
        if self.note is not None:
            out["note"] = thaw(self.note)
        return out


@dataclass(frozen=True, init=False)
class ApplyOutcome:
    state: Any
    terminal: bool
    effect: Any
    outcome: Any

    def __init__(self, state: Any, terminal: bool = False, effect: Any = None, outcome: Any = None) -> None:
        if not isinstance(terminal, bool):
            raise TypeError("terminal must be bool")
        object.__setattr__(self, "state", freeze(state))
        object.__setattr__(self, "terminal", terminal)
        object.__setattr__(self, "effect", None if effect is None else freeze(effect))
        object.__setattr__(self, "outcome", None if outcome is None else freeze(outcome))

    @classmethod
    def new(cls, state: Any) -> "ApplyOutcome":
        return cls(state)

    def with_terminal(self, terminal: bool) -> "ApplyOutcome":
        return type(self)(self.state, terminal, self.effect, self.outcome)

    def with_effect(self, effect: Any) -> "ApplyOutcome":
        return type(self)(self.state, self.terminal, effect, self.outcome)

    def with_outcome(self, outcome: Any) -> "ApplyOutcome":
        return type(self)(self.state, self.terminal, self.effect, outcome)

    def to_wire(self) -> dict[str, Any]:
        out: dict[str, Any] = {"state": thaw(self.state), "terminal": self.terminal}
        if self.effect is not None:
            out["effect"] = thaw(self.effect)
        if self.outcome is not None:
            out["outcome"] = thaw(self.outcome)
        return out


@dataclass(frozen=True)
class ReducerDiagnostic:
    code: str
    message: str
    rule: Optional[str] = None

    def __post_init__(self) -> None:
        if not isinstance(self.code, str) or not self.code:
            raise ValueError("diagnostic code must be a non-empty string")
        if not isinstance(self.message, str):
            raise TypeError("diagnostic message must be a string")
        if self.rule is not None and not isinstance(self.rule, str):
            raise TypeError("diagnostic rule must be a string or None")
    def to_wire(self) -> dict[str, Any]:
        out: dict[str, Any] = {"code": self.code, "message": self.message}
        if self.rule is not None:
            out["rule"] = self.rule
        return out


@runtime_checkable
class Reducer(Protocol):
    identity: ReducerIdentity
    def initial_state(self) -> Any:
        raise TypeError("Reducer protocol methods have no implementation")

    def decide(
        self,
        state: Any,
        observation: Optional[Any],
        effect: Any,
        command: Any,
    ) -> Decision | ReducerDiagnostic:
        raise TypeError("Reducer protocol methods have no implementation")

    def apply(
        self, state: Any, event: DomainEvent
    ) -> ApplyOutcome | ReducerDiagnostic:
        raise TypeError("Reducer protocol methods have no implementation")


def _strict_object(value: Any, fields: set[str]) -> None:
    if not isinstance(value, Mapping):
        raise ValueError("expected JSON object")
    unknown = set(value) - fields
    missing = fields - set(value)
    if unknown or missing:
        raise ValueError(f"invalid fields (unknown={sorted(unknown)!r}, missing={sorted(missing)!r})")


def _entrypoint_parts(entrypoint: str) -> tuple[str, str]:
    if not isinstance(entrypoint, str) or not entrypoint or ":" not in entrypoint:
        raise ValueError("entrypoint must be module:qualname")
    module_name, qualname = entrypoint.split(":", 1)
    if not module_name or not qualname or "<" in qualname or any(
        not part for part in qualname.split(".")
    ):
        raise ValueError("entrypoint must be a stable module:qualname")
    return module_name, qualname


def _resolve_entrypoint(module_name: str, qualname: str) -> Any:
    obj: Any = importlib.import_module(module_name)
    for part in qualname.split("."):
        obj = getattr(obj, part)
    return obj() if inspect.isclass(obj) else obj


def _validate_reducer_methods(obj: Any) -> None:
    if any(not callable(getattr(obj, method, None)) for method in ("initial_state", "decide", "apply")):
        raise TypeError("entrypoint does not provide reducer methods")


def load_reducer(entrypoint: str) -> Any:
    """Load a no-argument reducer class/object from an exact module:qualname."""
    module_name, qualname = _entrypoint_parts(entrypoint)
    obj = _resolve_entrypoint(module_name, qualname)
    _validate_reducer_methods(obj)
    reducer_identity(obj)
    return obj


def reducer_identity(obj: Any) -> ReducerIdentity:
    value = getattr(obj, "identity", None)
    if callable(value):
        value = value()
    if value is None:
        value = getattr(obj, "reducer_identity", None)
        if callable(value):
            value = value()
    if type(value) is not ReducerIdentity:
        raise TypeError("reducer must expose identity as ReducerIdentity")
    return value


def binding_descriptor(
    entrypoint: str,
    identity: ReducerIdentity,
    initial_state: Any,
    kernel_laws: Any = None,
) -> dict[str, Any]:
    """Build the exact descriptor Rust uses for machine identity."""
    descriptor = {
        "schema": DESCRIPTOR_SCHEMA,
        "entrypoint": entrypoint,
        **identity.to_wire(),
        "initial_state_digest": canonical_digest(freeze(initial_state)),
    }
    if kernel_laws:
        descriptor["kernel_laws_digest"] = canonical_digest(freeze(kernel_laws))
    return descriptor
