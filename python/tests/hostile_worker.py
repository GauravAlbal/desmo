"""Hostile reducer worker for protocol-wall tests. TEST DOUBLE ONLY.

Runs the REAL worker serve loop, then tampers with the reply to ``decide`` according to
the mode in the file named by ``IK_HOSTILE_MODE_FILE``. ``none`` is the negative control: it must behave exactly like the real worker,
which proves the harness itself does not fail everything.

Argv is identical to the real worker's (``--python-path-json <json> --serve``), so the Rust host
cannot tell it apart except by its replies.
"""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path

def current_mode() -> str:
    """Read per call so a long-lived (session) worker can be flipped hostile mid-life."""
    path = os.environ.get("IK_HOSTILE_MODE_FILE")
    return Path(path).read_text(encoding="utf-8").strip() if path else "none"


def _prepare_path() -> None:
    args = sys.argv[1:]
    for index, arg in enumerate(args):
        if arg == "--python-path-json":
            for path in reversed(json.loads(args[index + 1])):
                if path not in sys.path:
                    sys.path.insert(0, path)


def _record_pid() -> None:
    marker = os.environ.get("IK_WORKER_PID_FILE")
    if marker:
        with open(marker, "a", encoding="utf-8") as handle:
            handle.write(f"{os.getpid()}\n")


def main() -> int:
    _prepare_path()
    _record_pid()
    from invariant_kernel import reducer_worker as w

    real_reply = w._rpc_reply
    real_dump = w._dump

    def tampered(line: str):
        reply = real_reply(line)
        try:
            method = json.loads(line).get("method")
        except ValueError:
            return reply
        mode = current_mode()
        if method != "decide" or mode == "none":
            return reply
        if mode == "wrong_id":
            reply["id"] = 10_000_000
        elif mode == "bad_version":
            reply["jsonrpc"] = "1.0"
        elif mode == "error_object":
            return {"jsonrpc": "2.0", "id": reply["id"], "error": {"code": -32603, "message": "boom"}}
        elif mode == "neither":
            return {"jsonrpc": "2.0", "id": reply["id"]}
        elif mode == "both":
            reply["error"] = {"code": -32603, "message": "boom"}
        elif mode == "non_object_result":
            reply["result"] = 5
        elif mode == "garbage":
            return {"__raw__": "this is { not json"}
        elif mode == "oversize":
            return {"__raw__": "x" * (2 * 1024 * 1024)}
        elif mode == "eof":
            os._exit(0)
        elif mode == "hang":
            read_fd, write_fd = os.pipe()
            os.read(read_fd, 1)
        return reply

    def dump(value):
        if isinstance(value, dict) and "__raw__" in value:
            return value["__raw__"]
        return real_dump(value)

    w._rpc_reply = tampered
    w._dump = dump
    return w.main()


if __name__ == "__main__":
    raise SystemExit(main())
