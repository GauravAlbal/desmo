"""Subprocess transport over the canonical JSON protocol. No retries."""

from __future__ import annotations

import json
import os
import queue
import shutil
import signal
import subprocess
import sys
import threading
import uuid
from pathlib import Path
from typing import Any, Mapping, Optional

from .errors import PreDispatchError, TransportError, raise_diagnostic

PROTOCOL_MAJOR = 1
PROTOCOL_MINOR = 0
RUNTIME_VERSION = "0.7.0"
_MAX_OUTPUT = 1024 * 1024
_PROCESS_MARKER_ENV = "IK_PROCESS_TREE_TOKEN"


def _run(
    argv: list[str],
    *,
    input_text: Optional[str] = None,
    timeout: float,
) -> subprocess.CompletedProcess[str]:
    """Run a process with bounded output, killing its tracked tree on exit."""
    child_env = os.environ.copy()
    process_token = uuid.uuid4().hex
    child_env[_PROCESS_MARKER_ENV] = process_token
    proc = subprocess.Popen(
        argv,
        stdin=subprocess.PIPE if input_text is not None else None,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
        env=child_env,
    )
    return _communicate_bounded(proc, argv, input_text, timeout, process_token)


def _kill_process_group(proc: subprocess.Popen[bytes]) -> None:
    # poll()/wait() reap exited roots. A released PID cannot authorize a group
    # signal; remaining descendants are identified independently by their token.
    if proc.returncode is not None or proc.poll() is not None:
        return
    try:
        if os.name == "posix":
            os.killpg(proc.pid, signal.SIGKILL)
        else:
            proc.kill()
    except ProcessLookupError:
        pass



def _process_table(
    process_token: str,
) -> tuple[dict[int, list[int]], list[int]]:
    if os.name != "posix":
        return {}, []
    try:
        ps_env = os.environ.copy()
        ps_env.pop(_PROCESS_MARKER_ENV, None)
        # procps (Linux) rejects the dashed `-ax` in BSD form; BSD ps (macOS) rejects bare `ax`.
        scope = "ax" if sys.platform.startswith("linux") else "-ax"
        listing = subprocess.run(
            ["ps", "eww", scope, "-o", "pid=,ppid=,command="],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=1.0,
            check=True,
            env=ps_env,
        ).stdout
        children: dict[int, list[int]] = {}
        token_matches: list[int] = []
        marker = f"{_PROCESS_MARKER_ENV}={process_token}"
        for row in listing.splitlines():
            fields = row.split(maxsplit=2)
            if len(fields) < 2:
                continue
            pid, parent = int(fields[0]), int(fields[1])
            children.setdefault(parent, []).append(pid)
            if len(fields) == 3 and marker in fields[2]:
                token_matches.append(pid)
        return children, token_matches
    except (OSError, ValueError, subprocess.SubprocessError):
        return {}, []


def _descendant_pids(children: Mapping[int, list[int]], root: int) -> list[int]:
    descendants: list[int] = []
    frontier = [root]
    while frontier:
        parent = frontier.pop()
        direct = children.get(parent, [])
        descendants.extend(direct)
        frontier.extend(direct)
    return descendants


def _kill_process_tree(proc: subprocess.Popen[bytes], process_token: str) -> None:
    """Kill descendants even when they created another POSIX session."""
    children, token_matches = _process_table(process_token)
    descendants = (
        _descendant_pids(children, proc.pid) if proc.poll() is None else []
    )
    targets = list(dict.fromkeys([*reversed(descendants), *token_matches]))
    _kill_process_group(proc)
    for pid in targets:
        # The owned group operation handles the root; a later poll may already
        # have reaped it, so a snapshot must never authorize signalling its PID.
        if pid in (os.getpid(), proc.pid):
            continue
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass

def _communicate_bounded(
    proc: subprocess.Popen[bytes],
    argv: list[str],
    input_text: Optional[str],
    timeout: float,
    process_token: str,
    *,
    reap_on_success: bool = False,
) -> subprocess.CompletedProcess[str]:
    captures: dict[str, tuple[bytes, bool]] = {}
    failures: list[BaseException] = []

    def drain(name: str, stream: Any) -> None:
        kept = bytearray()
        exceeded = False
        try:
            while True:
                chunk = stream.read(8192)
                if not chunk:
                    break
                remaining = max(0, _MAX_OUTPUT - len(kept))
                kept.extend(chunk[:remaining])
                exceeded = exceeded or len(chunk) > remaining
        except BaseException as exc:
            failures.append(exc)
        finally:
            stream.close()
            captures[name] = (bytes(kept), exceeded)

    def write_input() -> None:
        assert proc.stdin is not None
        try:
            proc.stdin.write((input_text or "").encode("utf-8"))
            proc.stdin.flush()
        except BrokenPipeError:
            pass
        except BaseException as exc:
            failures.append(exc)
        finally:
            proc.stdin.close()

    assert proc.stdout is not None
    assert proc.stderr is not None
    threads = [
        threading.Thread(target=drain, args=("stdout", proc.stdout)),
        threading.Thread(target=drain, args=("stderr", proc.stderr)),
    ]
    if proc.stdin is not None:
        threads.append(threading.Thread(target=write_input))
    for thread in threads:
        thread.start()

    timed_out = False
    try:
        proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        _kill_process_tree(proc, process_token)
        proc.wait()
    else:
        if reap_on_success or proc.returncode != 0:
            _kill_process_tree(proc, process_token)
        else:
            for thread in threads:
                thread.join(timeout=0.02)
            # Only run the expensive `ps eww` process-tree scan if a stray descendant
            # is still holding an inherited pipe open after the child exited cleanly.
            if any(thread.is_alive() for thread in threads):
                _kill_process_tree(proc, process_token)
    for thread in threads:
        thread.join()

    stdout_raw, stdout_exceeded = captures.get("stdout", (b"", False))
    stderr_raw, stderr_exceeded = captures.get("stderr", (b"", False))
    stdout = stdout_raw.decode("utf-8", errors="replace")
    stderr = stderr_raw.decode("utf-8", errors="replace")
    if timed_out:
        raise subprocess.TimeoutExpired(
            argv, timeout, output=stdout, stderr=stderr
        )
    if failures:
        raise TransportError(
            "IK_TRANSPORT", f"process pipe failed: {failures[0]}"
        )
    if stdout_exceeded or stderr_exceeded:
        raise TransportError(
            "IK_TRANSPORT",
            f"process output exceeded the {_MAX_OUTPUT}-byte capture limit",
        )
    return subprocess.CompletedProcess(argv, proc.returncode, stdout, stderr)


def locate_binary(explicit: Optional[str] = None) -> str:
    if explicit:
        return explicit
    env = os.environ.get("IK_BINARY") or os.environ.get("IK_BIN")
    if env:
        return env
    found = shutil.which("ik")
    if found:
        return found
    raise TransportError(
        "IK_TRANSPORT",
        "ik binary not found; pass Kernel(binary=...) or set IK_BINARY",
    )


def canonical_dumps(value: Any) -> str:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    )


def reducer_worker_command(python: Optional[str] = None) -> list[str]:
    """Build direct worker argv with the caller's import search path frozen."""
    import sys

    worker = Path(__file__).with_name("reducer_worker.py").resolve()
    paths: list[str] = []
    for entry in sys.path:
        resolved = str(Path(entry or os.getcwd()).resolve())
        if resolved not in paths:
            paths.append(resolved)
    return [
        python or sys.executable,
        "-I",
        str(worker),
        "--python-path-json",
        canonical_dumps(paths),
    ]

def _worker_request(
    entrypoint: str,
    identity: Optional[Mapping[str, Any]],
    op: str,
    fields: Mapping[str, Any],
) -> dict[str, Any]:
    request: dict[str, Any] = {
        "protocol": {"major": PROTOCOL_MAJOR, "minor": PROTOCOL_MINOR},
        "op": op,
        "entrypoint": entrypoint,
    }
    if identity is not None:
        request["identity"] = dict(identity)
    request.update(fields)
    return request


def _worker_timeout(worker_timeout: Optional[float]) -> float:
    timeout = worker_timeout if worker_timeout is not None else 10.0
    if timeout <= 0:
        raise ValueError("worker_timeout must be positive")
    return timeout


def _run_worker_checked(
    argv: list[str],
    payload: str,
    timeout: float,
    op: str,
) -> subprocess.CompletedProcess[str]:
    try:
        proc = _run_worker(argv, payload, timeout)
    except FileNotFoundError as exc:
        raise TransportError(
            "IK_TRANSPORT", f"python worker not found: {argv[0]}"
        ) from exc
    except subprocess.TimeoutExpired as exc:
        raise TransportError(
            "IK_TRANSPORT", f"reducer worker {op} timed out after {timeout}s"
        ) from exc
    if proc.returncode != 0:
        error = (proc.stderr or proc.stdout or "").strip()
        raise TransportError(
            "IK_TRANSPORT", error or f"reducer worker exited {proc.returncode}"
        )
    return proc


def _parse_worker_response(output: str) -> dict[str, Any]:
    try:
        response = json.loads(
            output,
            parse_constant=lambda value: (_ for _ in ()).throw(
                ValueError(f"non-finite JSON value {value}")
            ),
        )
    except (json.JSONDecodeError, ValueError) as exc:
        raise TransportError(
            "IK_TRANSPORT",
            f"reducer worker returned invalid JSON: {output!r}",
        ) from exc
    if not isinstance(response, dict):
        raise TransportError("IK_TRANSPORT", "reducer worker returned a non-object")
    return response


def _validate_worker_success(
    response: dict[str, Any],
    identity: Optional[Mapping[str, Any]],
) -> dict[str, Any]:
    if set(response) != {
        "protocol",
        "ok",
        "entrypoint",
        "reducer_identity",
        "result",
    }:
        raise TransportError(
            "IK_TRANSPORT", "reducer worker success envelope is malformed"
        )
    if identity is not None and response.get("reducer_identity") != dict(identity):
        raise TransportError("IK_TRANSPORT", "reducer worker identity mismatch")
    result = response.get("result")
    if not isinstance(result, dict):
        raise TransportError(
            "IK_TRANSPORT", "reducer worker success missing result"
        )
    return response


def _validate_worker_refusal(
    response: dict[str, Any],
    identity: Optional[Mapping[str, Any]],
) -> dict[str, Any]:
    if set(response) != {
        "protocol",
        "ok",
        "entrypoint",
        "reducer_identity",
        "diagnostic",
    }:
        raise TransportError(
            "IK_TRANSPORT", "reducer worker refusal envelope is malformed"
        )
    diagnostic = response.get("diagnostic")
    if not isinstance(diagnostic, dict):
        raise TransportError("IK_TRANSPORT", "reducer worker refused without diagnostic")
    identity_mismatch = diagnostic.get("code") == "IK_MACHINE_MISMATCH"
    if (
        identity is not None
        and response.get("reducer_identity") != dict(identity)
        and not identity_mismatch
    ):
        raise TransportError("IK_TRANSPORT", "reducer worker identity mismatch")
    raise_diagnostic(diagnostic)
    raise AssertionError("raise_diagnostic must raise")


def _validate_worker_response(
    response: dict[str, Any],
    entrypoint: str,
    identity: Optional[Mapping[str, Any]],
) -> dict[str, Any]:
    if response.get("protocol") != {
        "major": PROTOCOL_MAJOR,
        "minor": PROTOCOL_MINOR,
    }:
        raise TransportError("IK_TRANSPORT", "reducer worker protocol mismatch")
    if response.get("entrypoint") != entrypoint:
        raise TransportError("IK_TRANSPORT", "reducer worker entrypoint mismatch")
    if response.get("ok") is True:
        return _validate_worker_success(response, identity)
    return _validate_worker_refusal(response, identity)


def invoke_worker(
    entrypoint: str,
    identity: Optional[Mapping[str, Any]],
    *,
    op: str,
    python: Optional[str] = None,
    worker_timeout: Optional[float] = None,
    **fields: Any,
) -> dict[str, Any]:
    """Invoke the source-checkout-safe reducer worker directly."""
    request = _worker_request(entrypoint, identity, op, fields)
    timeout = _worker_timeout(worker_timeout)
    argv = reducer_worker_command(python)
    proc = _run_worker_checked(argv, canonical_dumps(request), timeout, op)
    response = _parse_worker_response(proc.stdout)
    return _validate_worker_response(response, entrypoint, identity)


def _run_worker(
    argv: list[str], payload: str, timeout: float
) -> subprocess.CompletedProcess[str]:
    child_env = os.environ.copy()
    process_token = uuid.uuid4().hex
    child_env[_PROCESS_MARKER_ENV] = process_token
    proc = subprocess.Popen(
        argv,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
        env=child_env,
    )
    return _communicate_bounded(
        proc, argv, payload, timeout, process_token, reap_on_success=True
    )


def parse_version(binary: str) -> tuple[str, int, int]:
    try:
        proc = _run([binary, "--version"], timeout=10)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise TransportError("IK_TRANSPORT", f"failed to run `{binary} --version`: {exc}") from exc
    text = (proc.stdout or "").strip()
    parts = text.split()
    runtime = RUNTIME_VERSION
    major, minor = PROTOCOL_MAJOR, PROTOCOL_MINOR
    if len(parts) >= 2:
        runtime = parts[1]
    if len(parts) >= 4 and "." in parts[3]:
        left, _, right = parts[3].partition(".")
        try:
            major, minor = int(left), int(right)
        except ValueError:
            pass
    return runtime, major, minor


def run_identity(binary: str, machine: str | Path, timeout: float) -> dict[str, Any]:
    try:
        proc = _run([binary, "identity", str(machine)], timeout=timeout)
    except FileNotFoundError as exc:
        raise TransportError("IK_TRANSPORT", f"ik binary not found: {binary}") from exc
    except subprocess.TimeoutExpired as exc:
        raise TransportError("IK_TRANSPORT", f"ik identity timed out after {timeout}s") from exc
    if proc.returncode != 0:
        err = (proc.stderr or proc.stdout or "").strip()
        raise TransportError("IK_TRANSPORT", err or f"ik identity exited {proc.returncode}")
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise TransportError("IK_TRANSPORT", f"ik identity returned non-JSON: {proc.stdout!r}") from exc
    if not isinstance(data, dict):
        raise TransportError("IK_TRANSPORT", "ik identity returned a non-object")
    return data


def run_schema(binary: str, machine: str | Path, timeout: float) -> dict[str, Any]:
    proc = _run([binary, "schema", str(machine)], timeout=timeout)
    if proc.returncode != 0:
        err = (proc.stderr or proc.stdout).strip()
        raise TransportError("IK_TRANSPORT", err or f"ik schema exited {proc.returncode}")
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise TransportError("IK_TRANSPORT", "ik schema returned non-JSON") from exc
    if not isinstance(data, dict):
        raise TransportError("IK_TRANSPORT", "ik schema returned non-object")
    return data


def run_check(
    binary: str,
    machine: str | Path,
    *,
    prior: Optional[str | Path] = None,
    corpus: Optional[str | Path] = None,
    baseline: Optional[str | Path] = None,
    timeout: float,
) -> tuple[str, bool]:
    argv = [binary, "check", str(machine)]
    if prior is not None:
        argv.extend(["--prior", str(prior)])
    if corpus is not None:
        argv.extend(["--corpus", str(corpus)])
    if baseline is not None:
        argv.extend(["--baseline", str(baseline)])
    try:
        proc = _run(argv, timeout=timeout)
    except FileNotFoundError as exc:
        raise TransportError("IK_TRANSPORT", f"ik binary not found: {binary}") from exc
    except subprocess.TimeoutExpired as exc:
        raise TransportError("IK_TRANSPORT", f"ik check timed out after {timeout}s") from exc
    if proc.returncode not in (0, 1):
        err = (proc.stderr or proc.stdout or "").strip()
        raise TransportError("IK_TRANSPORT", err or f"ik check exited {proc.returncode}")
    return proc.stdout, proc.returncode == 0


def run_events(
    binary: str,
    store: str | Path,
    *,
    since: int = 0,
    aggregate_id: Optional[str] = None,
    limit: Optional[int] = None,
    timeout: float,
) -> dict[str, Any]:
    argv = [binary, "events", str(store), "--since", str(since)]
    if aggregate_id is not None:
        argv.extend(["--aggregate", aggregate_id])
    if limit is not None:
        argv.extend(["--limit", str(limit)])
    argv.append("--json")
    try:
        proc = _run(argv, timeout=timeout)
    except FileNotFoundError as exc:
        raise PreDispatchError("IK_TRANSPORT", f"ik binary not found: {binary}") from exc
    except subprocess.TimeoutExpired as exc:
        raise TransportError("IK_TRANSPORT", f"ik events timed out after {timeout}s") from exc
    if proc.returncode != 0:
        err = (proc.stderr or proc.stdout or "").strip()
        raise TransportError("IK_TRANSPORT", err or f"ik events exited {proc.returncode}")
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise TransportError("IK_TRANSPORT", f"ik events returned invalid JSON: {exc}") from exc
    if not isinstance(data, dict) or data.get("schema") != "ik.events.v1":
        raise TransportError("IK_TRANSPORT", "ik events returned an unexpected payload schema")
    return data


def run_list(
    binary: str,
    store: str | Path,
    *,
    machine_id: Optional[str] = None,
    state: Optional[str] = None,
    effect: Optional[str] = None,
    terminal: Optional[bool] = None,
    after: Optional[str] = None,
    limit: Optional[int] = None,
    timeout: float,
) -> dict[str, Any]:
    argv = [binary, "list", str(store)]
    if machine_id is not None:
        argv.extend(["--machine", machine_id])
    if state is not None:
        argv.extend(["--state", state])
    if effect is not None:
        argv.extend(["--effect", effect])
    if terminal is not None:
        argv.extend(["--terminal", "true" if terminal else "false"])
    if after is not None:
        argv.extend(["--after", after])
    if limit is not None:
        argv.extend(["--limit", str(limit)])
    argv.append("--json")
    try:
        proc = _run(argv, timeout=timeout)
    except FileNotFoundError as exc:
        raise PreDispatchError("IK_TRANSPORT", f"ik binary not found: {binary}") from exc
    except subprocess.TimeoutExpired as exc:
        raise TransportError("IK_TRANSPORT", f"ik list timed out after {timeout}s") from exc
    if proc.returncode != 0:
        err = (proc.stderr or proc.stdout or "").strip()
        raise TransportError("IK_TRANSPORT", err or f"ik list exited {proc.returncode}")
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise TransportError("IK_TRANSPORT", f"ik list returned invalid JSON: {exc}") from exc
    if not isinstance(data, dict) or data.get("schema") != "ik.list.v1":
        raise TransportError("IK_TRANSPORT", "ik list returned an unexpected payload schema")
    return data


def _unwrap(envelope: Any, op: str) -> dict[str, Any]:
    if not isinstance(envelope, dict):
        raise TransportError("IK_TRANSPORT", "ik returned a non-object envelope")
    if envelope.get("ok") is True:
        result = envelope.get("result")
        if not isinstance(result, dict):
            raise TransportError("IK_TRANSPORT", "successful envelope missing object result")
        return result
    diagnostic = envelope.get("diagnostic")
    if isinstance(diagnostic, dict):
        raise_diagnostic(diagnostic)
    raise TransportError("IK_TRANSPORT", f"refused envelope without diagnostic: {envelope}")


def invoke(
    binary: str,
    store: str | Path,
    machine: Optional[str | Path],
    request: Mapping[str, Any],
    timeout: float,
) -> dict[str, Any]:
    argv = [binary, "--store", str(store)]
    if machine is not None:
        argv.extend(["--machine", str(machine)])
    argv.append(str(request["op"]))
    payload = canonical_dumps(request)
    try:
        proc = _run(argv, input_text=payload, timeout=timeout)
    except FileNotFoundError as exc:
        raise PreDispatchError("IK_TRANSPORT", f"ik binary not found: {binary}") from exc
    except subprocess.TimeoutExpired as exc:
        raise TransportError("IK_TRANSPORT", f"ik {request['op']} timed out after {timeout}s") from exc
    stdout = proc.stdout or ""
    if proc.returncode == 2 and not stdout.strip():
        err = (proc.stderr or "").strip()
        raise TransportError("IK_TRANSPORT", err or f"ik exited 2 for op {request['op']}")
    try:
        envelope = json.loads(stdout)
    except json.JSONDecodeError as exc:
        err = (proc.stderr or stdout).strip()
        raise TransportError(
            "IK_TRANSPORT",
            err or f"ik returned non-JSON stdout for op {request['op']}",
        ) from exc
    return _unwrap(envelope, str(request["op"]))


class Session:
    """A persistent `ik --session` process: one canonical JSON request line in, one response line out.

    The session keeps the store and reducer worker open across commands; every request is still
    validated and dispatched independently by the same kernel path as one-shot mode. There is no
    implicit retry: after a timeout or broken pipe the session is unusable and the caller must
    reconcile (read state) before deciding what to do.
    """

    def __init__(
        self, binary: str, store: str | Path, machine: Optional[str | Path], *, timeout: float
    ) -> None:
        argv = [binary, "--store", str(store)]
        if machine is not None:
            argv.extend(["--machine", str(machine)])
        argv.append("--session")
        self._timeout = timeout
        self._lock = threading.Lock()
        self._broken: Optional[str] = None
        self._uncertain: set[str] = set()
        self._responses: "queue.Queue[Optional[str]]" = queue.Queue()
        self._stderr: list[str] = []
        self._stderr_len = 0
        self._process_token = uuid.uuid4().hex
        child_env = os.environ.copy()
        child_env[_PROCESS_MARKER_ENV] = self._process_token
        try:
            self._proc = subprocess.Popen(
                argv,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                encoding="utf-8",
                start_new_session=True,
                env=child_env,
            )
        except FileNotFoundError as exc:
            raise PreDispatchError("IK_TRANSPORT", f"ik binary not found: {binary}") from exc
        threading.Thread(target=self._read_stdout, daemon=True).start()
        threading.Thread(target=self._read_stderr, daemon=True).start()

    def _read_stdout(self) -> None:
        assert self._proc.stdout is not None
        for line in self._proc.stdout:
            self._responses.put(line)
        self._responses.put(None)

    def _read_stderr(self) -> None:
        assert self._proc.stderr is not None
        for line in self._proc.stderr:
            if self._stderr_len < _MAX_OUTPUT:
                self._stderr.append(line)
                self._stderr_len += len(line)

    def _fail(self, reason: str, request: Optional[Mapping[str, Any]] = None) -> TransportError:
        self._broken = reason
        operation_id = request.get("operation_id") if request is not None else None
        if operation_id:
            self._uncertain.add(str(operation_id))
        _kill_process_tree(self._proc, self._process_token)
        self._proc.wait()
        return TransportError("IK_TRANSPORT", reason)

    def _refuse_when_broken(self, request: Mapping[str, Any]) -> None:
        """A broken session never dispatches. For an id that was in flight when it broke, the earlier
        attempt's outcome is still unknown, so the answer is submission-unknown, not not-sent."""
        if self._broken is None:
            return
        if str(request.get("operation_id") or "") in self._uncertain:
            raise TransportError(
                "IK_TRANSPORT",
                f"an earlier attempt under this operation id was in flight when the session died "
                f"({self._broken}); this session cannot answer: reconcile with lookup_operation on a new "
                f"client, do not resend under a new id",
            )
        raise PreDispatchError("IK_TRANSPORT", f"ik session unusable: {self._broken}")

    def invoke(self, request: Mapping[str, Any], timeout: Optional[float] = None) -> dict[str, Any]:
        op = str(request["op"])
        wait = self._timeout if timeout is None else timeout
        with self._lock:
            self._refuse_when_broken(request)
            assert self._proc.stdin is not None
            try:
                self._proc.stdin.write(canonical_dumps(request) + "\n")
                self._proc.stdin.flush()
            except (BrokenPipeError, OSError, ValueError) as exc:
                raise self._fail(f"ik session pipe closed during {op}: {exc}", request) from exc
            try:
                line = self._responses.get(timeout=wait)
            except queue.Empty:
                raise self._fail(f"ik session {op} timed out after {wait}s", request) from None
            if line is None:
                err = "".join(self._stderr).strip()
                raise self._fail(err or f"ik session exited during {op}", request)
            try:
                envelope = json.loads(line)
            except json.JSONDecodeError as exc:
                raise self._fail(f"ik session returned non-JSON for {op}", request) from exc
            if isinstance(envelope, dict) and envelope.get("request_id") not in (
                request.get("request_id"),
                "",
            ):
                raise self._fail(f"ik session response id mismatch for {op}", request)
            return _unwrap(envelope, op)

    def close(self) -> None:
        with self._lock:
            if self._proc.stdin is not None and not self._proc.stdin.closed:
                try:
                    self._proc.stdin.close()
                except OSError:
                    pass
            try:
                self._proc.wait(timeout=5.0)
            except subprocess.TimeoutExpired:
                _kill_process_tree(self._proc, self._process_token)
                self._proc.wait()
            else:
                _kill_process_tree(self._proc, self._process_token)

    def __enter__(self) -> "Session":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()
