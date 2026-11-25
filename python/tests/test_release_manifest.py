from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

from conftest import REPO


def test_release_manifest_schema_three(tmp_path: Path) -> None:
    (tmp_path / "ik-darwin-arm64.tar.gz").write_bytes(b"ik")
    (tmp_path / "invariant_kernel-0.5.1-py3-none-any.whl").write_bytes(b"whl")
    env = {k: v for k, v in os.environ.items() if k not in ("GITHUB_REF_NAME", "GITHUB_SHA")}
    env["IK_VERSION"] = "v0.5.1"
    env["IK_SOURCE_SHA"] = "abc123"
    subprocess.check_call(
        ["python3", str(REPO / "scripts" / "write-release-manifest.py"), str(tmp_path)],
        env=env,
    )
    manifest = json.loads((tmp_path / "release-manifest.json").read_text())
    assert manifest["schema_version"] == 4
    assert manifest["product"] == "Ikaros"
    assert manifest["version"] == "0.5.1"
    assert manifest["tag"] == "v0.5.1"
    assert manifest["source_sha"] == "abc123"
    assert manifest["protocol_major"] == 1
    assert manifest["runtime_version"] == "0.5.1"
    assert manifest["python_client_version"] == "0.5.1"
    by_file = {a["file"]: a for a in manifest["artifacts"]}
    assert by_file["ik-darwin-arm64.tar.gz"]["target"] == "aarch64-apple-darwin"
    assert "sha256" in by_file["ik-darwin-arm64.tar.gz"]
    assert "target" not in by_file["invariant_kernel-0.5.1-py3-none-any.whl"]
    sums = (tmp_path / "SHA256SUMS").read_text()
    assert "ik-darwin-arm64.tar.gz" in sums
