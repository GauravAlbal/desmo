from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SRC = Path(__file__).resolve().parents[1] / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))

VA = REPO / "machines" / "verification_attempt.ik.toml"
EFFECT = REPO / "machines" / "effect_probe.ik.toml"


def _resolve_ik() -> Path:
    env = os.environ.get("IK_BINARY") or os.environ.get("IK_BIN")
    if env:
        return Path(env)
    debug = REPO / "target" / "debug" / "ik"
    if debug.exists():
        return debug
    subprocess.run(["cargo", "build", "--bin", "ik", "--offline"], cwd=REPO, check=True)
    return debug


@pytest.fixture(scope="session")
def ik_bin() -> Path:
    path = _resolve_ik()
    assert path.exists(), f"ik binary missing at {path}"
    return path


@pytest.fixture
def store(tmp_path: Path) -> Path:
    return tmp_path / "kernel.db"
