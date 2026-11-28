"""RA06: mandatory after packaging; ordinary checkout tests need not stage a release.

Offline prerequisites: Python >=3.9 with venv/ensurepip, the supported Go
compiler and cached modules, and this host's staged ik tarball + real wheel.
Run with IK_DIST=/absolute/dist/v0.7.1 IK_SOURCE_REVISION=<full committed SHA>.
No candidate-produced hashes are accepted as preservation goldens.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys

import pytest


@pytest.mark.skipif("IK_DIST" not in os.environ, reason="release artifacts not staged (IK_DIST unset)")
def test_staged_release_is_consumable_by_installed_python_and_real_go() -> None:
    repo = Path(__file__).resolve().parents[2]
    dist = os.environ["IK_DIST"]
    revision = os.environ.get("IK_SOURCE_REVISION")
    assert dist, "IK_DIST must name an existing staged release directory"
    assert revision, "staged release requires explicit IK_SOURCE_REVISION, never an inferred candidate SHA"
    result = subprocess.run(
        [sys.executable, "-I", str(repo / "scripts" / "verify-release.py"),
         dist, "--source-revision", revision],
        cwd=repo,
        text=True,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, f"packaged release verification failed:\n{result.stdout}\n{result.stderr}"
    observed = json.loads(result.stdout)
    assert observed["runtime"] == "invariant-kernel 0.7.1 protocol 1.0"
    assert observed["python_version"] == "0.7.1"
    assert observed["store_schema"] == "4"
    assert observed["schema1_hash"] == "aae4f03a6e4169bdc86021914bc57db4bc391878651fa61fcd2a9d0d215b4a7f"
    assert observed["refusal_code"] == "IK_INVARIANT_FAILED"
    assert observed["safe_generation"] == 1
    assert observed["source_sha"] == revision
    assert observed["go_client"] == "passed"
