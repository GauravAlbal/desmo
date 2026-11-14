"""Reducer whose latency and failure are chosen per command, for contention measurement.

`accept` args: `delay_ms` (sleep inside `decide`, i.e. while the store write
transaction is held) and `mode` (`normal`, `hang`, `kill`). Everything else is the
reference ParityReducer behaviour.
"""

from __future__ import annotations

import os
import signal
import sys
import time
from pathlib import Path
from typing import Any, Mapping, Optional

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "python" / "tests"))
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "python" / "src"))

from invariant_kernel.reducer import Decision  # noqa: E402
from reducer_fixtures import ParityReducer  # noqa: E402


class ControlledReducer(ParityReducer):
    def decide(
        self,
        state: Mapping[str, Any],
        observation: Optional[Mapping[str, Any]],
        effect: Any,
        command: Mapping[str, Any],
    ) -> Decision:
        args = command.get("args") or {}
        mode = args.get("mode", "normal")
        if mode == "kill":
            os.kill(os.getpid(), signal.SIGKILL)
        delay_ms = float(args.get("delay_ms", 0))
        if mode == "hang":
            delay_ms = max(delay_ms, 600_000)
        if delay_ms:
            time.sleep(delay_ms / 1000.0)
        return super().decide(state, observation, effect, command)
