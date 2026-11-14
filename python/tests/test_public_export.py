"""IK-12: the public export is a verbatim allowlisted subset and blocks private references."""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest

from conftest import REPO

sys.path.insert(0, str(REPO / "scripts"))

import public_export as pe  # noqa: E402

WORD = "mo" + "at"  # the scanned-for word, spelled so this file does not contain it

CONFIG: dict[str, Any] = {
    "exclude": [{"glob": "private/*", "reason": "private state"}],
    "patterns": [
        {"id": "private-gate", "kind": "private", "regex": "(?i)\\bm[o]at\\b"},
        {"id": "github-token", "kind": "secret", "regex": "ghp_[A-Za-z0-9]{20,}"},
    ],
    "acknowledged": [],
}


def make_repo(tmp_path: Path, files: dict[str, str]) -> Path:
    repo = tmp_path / "origin"
    repo.mkdir()
    for name, text in files.items():
        (repo / name).parent.mkdir(parents=True, exist_ok=True)
        (repo / name).write_text(text)
    for argv in (["init", "-q"], ["add", "."], ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "x"]):
        subprocess.run(["git", *argv], cwd=repo, check=True)
    return repo


def write_config(tmp_path: Path, config: dict[str, Any]) -> Path:
    path = tmp_path / "allowlist.json"
    path.write_text(json.dumps(config))
    return path


def test_export_is_a_verbatim_subset_with_a_digest_for_every_file(tmp_path: Path) -> None:
    repo = make_repo(tmp_path, {"src/a.rs": "fn a() {}\n", "private/x.txt": "secret plans\n", "README.md": "hi\n"})
    out = tmp_path / "out"
    manifest, blocking = pe.export(repo, "HEAD", out, write_config(tmp_path, CONFIG))
    assert blocking == []
    assert sorted(manifest["files"]) == ["README.md", "src/a.rs"]
    assert not (out / "private").exists()
    assert (out / "src" / "a.rs").read_text() == "fn a() {}\n"
    assert manifest["files"]["README.md"] == hashlib.sha256(b"hi\n").hexdigest()
    assert [e["path"] for e in manifest["excluded"]] == ["private/x.txt"]
    assert json.loads((out / "EXPORT_MANIFEST.json").read_text())["origin_commit"] == manifest["origin_commit"]


def test_a_private_reference_blocks_the_export_and_writes_nothing(tmp_path: Path) -> None:
    repo = make_repo(tmp_path, {"docs/x.md": f"ship it through {WORD} acceptance\n"})
    out = tmp_path / "out"
    manifest, blocking = pe.export(repo, "HEAD", out, write_config(tmp_path, CONFIG))
    assert [(b["path"], b["pattern"]) for b in blocking] == [("docs/x.md", "private-gate")]
    assert not out.exists()
    assert manifest["files"]["docs/x.md"]


def test_an_exact_acknowledgement_with_a_reason_unblocks_only_that_pair(tmp_path: Path) -> None:
    repo = make_repo(tmp_path, {"docs/x.md": f"{WORD}\n", "docs/y.md": f"{WORD}\n"})
    config = {**CONFIG, "acknowledged": [{"path": "docs/x.md", "pattern": "private-gate", "reason": "negative statement"}]}
    _, blocking = pe.export(repo, "HEAD", tmp_path / "out", write_config(tmp_path, config))
    assert [b["path"] for b in blocking] == ["docs/y.md"]


def test_a_secret_can_never_be_acknowledged(tmp_path: Path) -> None:
    token = "ghp_" + "a" * 24
    repo = make_repo(tmp_path, {"cfg.txt": f"token={token}\n"})
    config = {**CONFIG, "acknowledged": [{"path": "cfg.txt", "pattern": "github-token", "reason": "trust me"}]}
    _, blocking = pe.export(repo, "HEAD", tmp_path / "out", write_config(tmp_path, config))
    assert [(b["path"], b["kind"]) for b in blocking] == [("cfg.txt", "secret")]


def test_an_acknowledgement_or_exclusion_without_a_reason_is_refused(tmp_path: Path) -> None:
    repo = make_repo(tmp_path, {"a.txt": "x\n"})
    bad = {**CONFIG, "acknowledged": [{"path": "a.txt", "pattern": "private-gate", "reason": ""}]}
    with pytest.raises(SystemExit, match="has no reason"):
        pe.export(repo, "HEAD", tmp_path / "out", write_config(tmp_path, bad))


def test_the_export_refuses_a_non_empty_destination(tmp_path: Path) -> None:
    repo = make_repo(tmp_path, {"a.txt": "x\n"})
    out = tmp_path / "out"
    out.mkdir()
    (out / "stale").write_text("old")
    with pytest.raises(SystemExit, match="not empty"):
        pe.export(repo, "HEAD", out, write_config(tmp_path, CONFIG))
