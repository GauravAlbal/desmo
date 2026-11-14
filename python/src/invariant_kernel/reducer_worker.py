"""Stdio worker for the versioned Python reducer protocol.

This cooperative process receives values only; it is not given a canonical-store
path and is not an OS security sandbox.
"""
from __future__ import annotations

import contextlib
import json
import os
import signal
import sys
from pathlib import Path
from typing import Any, Mapping, Optional

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from invariant_kernel.reducer import (  # type: ignore[import-not-found]
    PROTOCOL_MAJOR,
    PROTOCOL_MINOR,
    ApplyOutcome,
    Decision,
    DomainEvent,
    ReducerDiagnostic,
    ReducerIdentity,
    freeze,
    load_reducer,
    reducer_identity,
    thaw,
)

_BASE = {"protocol", "op", "entrypoint"}
_IDENTITY = {
    "reducer_name",
    "reducer_version",
    "code_digest",
    "command_schema_digest",
    "state_schema_digest",
}
_DECIDE = _BASE | {"identity", "state", "observation", "effect", "command"}
_APPLY = _BASE | {"identity", "state", "event"}


def _object(value: Any, fields: set[str], required: set[str]) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError("expected object")
    unknown = set(value) - fields
    missing = required - set(value)
    if unknown:
        raise ValueError(f"unknown fields: {sorted(unknown)!r}")
    if missing:
        raise ValueError(f"missing fields: {sorted(missing)!r}")
    return value


def _failure(
    message: str,
    *,
    code: str,
    rule: Optional[str],
    entrypoint: Any = None,
    identity: Optional[ReducerIdentity] = None,
) -> dict[str, Any]:
    return {
        "protocol": {"major": PROTOCOL_MAJOR, "minor": PROTOCOL_MINOR},
        "ok": False,
        "entrypoint": entrypoint,
        "reducer_identity": identity.to_wire() if identity is not None else None,
        "diagnostic": {"code": code, "rule": rule, "message": message},
    }


def _response(
    entrypoint: str, identity: ReducerIdentity, result: Mapping[str, Any]
) -> dict[str, Any]:
    return {
        "protocol": {"major": PROTOCOL_MAJOR, "minor": PROTOCOL_MINOR},
        "ok": True,
        "entrypoint": entrypoint,
        "reducer_identity": identity.to_wire(),
        "result": dict(result),
    }


def _parse_request(request: Any) -> tuple[dict[str, Any], str, str]:
    if not isinstance(request, dict):
        raise ValueError("expected object")
    missing = _BASE - set(request)
    if missing:
        raise ValueError(f"missing fields: {sorted(missing)!r}")
    protocol = _object(request["protocol"], {"major", "minor"}, {"major", "minor"})
    if protocol != {"major": PROTOCOL_MAJOR, "minor": PROTOCOL_MINOR}:
        raise ValueError("unsupported reducer protocol")
    op = request["op"]
    if op not in {"describe", "decide", "apply"}:
        raise ValueError("unknown reducer operation")
    entrypoint = request["entrypoint"]
    if not isinstance(entrypoint, str) or ":" not in entrypoint or "<" in entrypoint:
        raise ValueError("entrypoint must be a stable module:qualname")
    if op == "describe":
        req = _object(request, _BASE, _BASE)
    elif op == "decide":
        req = _object(request, _DECIDE, _DECIDE - {"observation"})
    else:
        req = _object(request, _APPLY, _APPLY)
    return req, op, entrypoint


def _requested_identity(
    request: Mapping[str, Any], operation: str
) -> Optional[ReducerIdentity]:
    if operation == "describe":
        return None
    return ReducerIdentity.from_wire(request["identity"])


_SERVE = False
_CALL_ACTIVE = False
_REDUCER_CACHE: dict[str, tuple[Any, ReducerIdentity]] = {}


def _reap_children() -> None:
    while True:
        try:
            pid, _ = os.waitpid(-1, os.WNOHANG)
        except ChildProcessError:
            return
        if pid == 0:
            return


def _on_child_exit(_signum: int, _frame: Any) -> None:
    # Rust terminates stray descendants after receiving the reply. Reap while
    # idle, but never steal exit statuses from subprocess waits inside callbacks.
    if not _CALL_ACTIVE:
        _reap_children()


def _load_checked_reducer(
    request: Mapping[str, Any], operation: str, entrypoint: str
) -> tuple[Any, ReducerIdentity, Optional[ReducerIdentity]]:
    requested = _requested_identity(request, operation)
    cached = _REDUCER_CACHE.get(entrypoint) if _SERVE else None
    if cached is not None:
        reducer, actual = cached
    else:
        reducer = load_reducer(entrypoint)
        actual = reducer_identity(reducer)
        if _SERVE:
            _REDUCER_CACHE[entrypoint] = (reducer, actual)
    return reducer, actual, requested


def _diagnostic_failure(
    diagnostic: ReducerDiagnostic, entrypoint: str, identity: ReducerIdentity
) -> dict[str, Any]:
    return _failure(
        diagnostic.message,
        code=diagnostic.code,
        rule=diagnostic.rule,
        entrypoint=entrypoint,
        identity=identity,
    )


def _handle_describe(
    reducer: Any, entrypoint: str, identity: ReducerIdentity
) -> dict[str, Any]:
    initial = freeze(reducer.initial_state())
    return _response(entrypoint, identity, {"initial_state": thaw(initial)})


def _decide_observation(request: Mapping[str, Any]) -> Optional[Any]:
    observation = request.get("observation")
    if observation is not None and not isinstance(observation, Mapping):
        raise ValueError("observation must be an object or null")
    return None if observation is None else freeze(observation)


def _format_decision(
    result: Any, entrypoint: str, identity: ReducerIdentity
) -> dict[str, Any]:
    if type(result) is ReducerDiagnostic:
        return _diagnostic_failure(result, entrypoint, identity)
    if type(result) is not Decision:
        raise TypeError("decide must return exact Decision or ReducerDiagnostic")
    outcome: dict[str, Any] = {"decision": result.kind}
    if result.event is not None:
        outcome["event"] = result.event.to_wire()
    if result.note is not None:
        outcome["note"] = thaw(result.note)
    return _response(entrypoint, identity, outcome)


def _handle_decide(
    reducer: Any, request: Mapping[str, Any], entrypoint: str, identity: ReducerIdentity
) -> dict[str, Any]:
    observation = _decide_observation(request)
    result = reducer.decide(
        freeze(request["state"]),
        observation,
        freeze(request["effect"]),
        freeze(request["command"]),
    )
    return _format_decision(result, entrypoint, identity)


def _format_apply(
    result: Any, entrypoint: str, identity: ReducerIdentity
) -> dict[str, Any]:
    if type(result) is ReducerDiagnostic:
        return _diagnostic_failure(result, entrypoint, identity)
    if type(result) is not ApplyOutcome:
        raise TypeError("apply must return exact ApplyOutcome or ReducerDiagnostic")
    return _response(entrypoint, identity, result.to_wire())


def _handle_apply(
    reducer: Any, request: Mapping[str, Any], entrypoint: str, identity: ReducerIdentity
) -> dict[str, Any]:
    event = DomainEvent.from_wire(request["event"])
    result = reducer.apply(freeze(request["state"]), event)
    return _format_apply(result, entrypoint, identity)


def _dispatch(
    reducer: Any,
    request: Mapping[str, Any],
    operation: str,
    entrypoint: str,
    identity: ReducerIdentity,
) -> dict[str, Any]:
    if operation == "describe":
        return _handle_describe(reducer, entrypoint, identity)
    if operation == "decide":
        return _handle_decide(reducer, request, entrypoint, identity)
    return _handle_apply(reducer, request, entrypoint, identity)


def handle(request: Any) -> dict[str, Any]:
    entrypoint: Any = request.get("entrypoint") if isinstance(request, dict) else None
    try:
        req, operation, entrypoint = _parse_request(request)
    except Exception as exc:
        return _failure(
            str(exc),
            code="IK_PROTOCOL_INVALID",
            rule="reducer.protocol",
            entrypoint=entrypoint,
        )

    identity: Optional[ReducerIdentity] = None
    try:
        reducer, actual_identity, requested_identity = _load_checked_reducer(
            req, operation, entrypoint
        )
        if requested_identity is not None and actual_identity != requested_identity:
            return _failure(
                "worker identity does not match requested identity",
                code="IK_MACHINE_MISMATCH",
                rule="reducer.identity",
                entrypoint=entrypoint,
                identity=actual_identity,
            )
        identity = actual_identity
        return _dispatch(reducer, req, operation, entrypoint, identity)
    except Exception as exc:
        return _failure(
            str(exc),
            code="IK_INTERNAL",
            rule="reducer.exception",
            entrypoint=entrypoint,
            identity=identity,
        )

def _configure_python_path(arguments: list[str]) -> None:
    if not arguments:
        return
    if len(arguments) != 2 or arguments[0] != "--python-path-json":
        raise ValueError("worker accepts only --python-path-json <json-array>")
    paths = json.loads(arguments[1])
    if not isinstance(paths, list) or any(
        not isinstance(path, str) or not path for path in paths
    ):
        raise ValueError("python path must be a JSON array of non-empty strings")
    for path in reversed(paths):
        if path not in sys.path:
            sys.path.insert(0, path)


_JSONRPC = "2.0"
_METHODS = {"describe", "decide", "apply"}


def _dump(value: Any) -> str:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    )


def _rpc_error(request_id: Any, code: int, message: str) -> dict[str, Any]:
    return {
        "jsonrpc": _JSONRPC,
        "id": request_id,
        "error": {"code": code, "message": message},
    }


def _reject_constant(value: str) -> Any:
    raise ValueError(f"non-finite JSON value {value}")


def _rpc_reply(line: str) -> dict[str, Any]:
    """One JSON-RPC 2.0 request line -> one reply. Domain refusals stay in `result`."""
    try:
        message = json.loads(line, parse_constant=_reject_constant)
    except ValueError as exc:
        return _rpc_error(None, -32700, str(exc))
    request_id = message.get("id") if isinstance(message, dict) else None
    if (
        not isinstance(message, dict)
        or message.get("jsonrpc") != _JSONRPC
        or "id" not in message
        or not isinstance(message.get("method"), str)
    ):
        return _rpc_error(request_id, -32600, "invalid JSON-RPC 2.0 request")
    method = message["method"]
    if method not in _METHODS:
        return _rpc_error(request_id, -32601, f"method not found: {method}")
    params = message.get("params")
    if not isinstance(params, dict) or params.get("op") != method:
        return _rpc_error(request_id, -32602, "params.op must equal method")
    try:
        return {"jsonrpc": _JSONRPC, "id": request_id, "result": handle(params)}
    except Exception as exc:  # handle() contains its own guards; this is the last resort
        return _rpc_error(request_id, -32603, str(exc))


_MAX_OUTPUT = 1024 * 1024


class _BoundedSink:
    """Per-call bound on reducer diagnostics so a noisy reducer cannot wedge or flood the host."""

    def __init__(self, real: Any, limit: int) -> None:
        self._real = real
        self._limit = limit
        self.written = 0
        self.exceeded = False

    def reset(self) -> None:
        self.written = 0
        self.exceeded = False

    def write(self, text: str) -> int:
        size = len(text.encode("utf-8", "replace"))
        self.written += size
        if self.written > self._limit:
            self.exceeded = True
            return size
        return self._real.write(text)

    def flush(self) -> None:
        self._real.flush()

    def isatty(self) -> bool:
        return False


def _output_limit_reply(reply: dict[str, Any]) -> dict[str, Any]:
    result = reply.get("result")
    if not isinstance(result, dict):
        return reply
    reply["result"] = {
        "protocol": result.get("protocol"),
        "ok": False,
        "entrypoint": result.get("entrypoint"),
        "reducer_identity": result.get("reducer_identity"),
        "diagnostic": {
            "code": "IK_INTERNAL",
            "rule": "reducer.output_limit",
            "message": "reducer output exceeded the capture limit",
        },
    }
    return reply


def serve() -> int:
    """Long-lived worker: newline-delimited JSON-RPC 2.0 over stdio (language-server pattern)."""
    global _SERVE, _CALL_ACTIVE
    _SERVE = True
    if os.name == "posix":
        signal.signal(signal.SIGCHLD, _on_child_exit)
    protocol_out = sys.stdout
    sink = _BoundedSink(sys.stderr, _MAX_OUTPUT)
    sys.stdout = sink  # reducer prints must never corrupt the protocol stream
    sys.stderr = sink
    for line in sys.stdin:
        if not line.strip():
            continue
        sink.reset()
        _CALL_ACTIVE = True
        try:
            reply = _rpc_reply(line)
            if sink.exceeded:
                reply = _output_limit_reply(reply)
            try:
                payload = _dump(reply)
            except (TypeError, ValueError) as exc:
                payload = _dump(_rpc_error(reply.get("id"), -32603, str(exc)))
        finally:
            _CALL_ACTIVE = False
            if os.name == "posix":
                _reap_children()
        protocol_out.write(payload + "\n")
        protocol_out.flush()
    return 0


def main() -> int:
    arguments = sys.argv[1:]
    if "--serve" in arguments:
        arguments = [argument for argument in arguments if argument != "--serve"]
        try:
            _configure_python_path(arguments)
        except Exception as exc:
            sys.stderr.write(f"ik reducer worker: {exc}\n")
            return 2
        return serve()
    # Reducer prints are diagnostics and must never corrupt protocol stdout.
    with contextlib.redirect_stdout(sys.stderr):
        try:
            _configure_python_path(sys.argv[1:])
            request = json.loads(
                sys.stdin.read(),
                parse_constant=lambda value: (_ for _ in ()).throw(
                    ValueError(f"non-finite JSON value {value}")
                ),
            )
            response = handle(request)
        except Exception as exc:
            response = _failure(
                str(exc), code="IK_PROTOCOL_INVALID", rule="reducer.protocol"
            )
        try:
            payload = json.dumps(
                response,
                sort_keys=True,
                separators=(",", ":"),
                ensure_ascii=False,
                allow_nan=False,
            )
        except (TypeError, ValueError) as exc:
            payload = json.dumps(
                _failure(
                    str(exc), code="IK_INTERNAL", rule="reducer.exception"
                ),
                sort_keys=True,
                separators=(",", ":"),
            )
    sys.stdout.write(payload + "\n")
    sys.stdout.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
