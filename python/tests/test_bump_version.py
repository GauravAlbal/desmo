"""IK-11: the version bump moves whole version tokens in explicit targets and nothing else."""

from __future__ import annotations

import sys
from pathlib import Path

from conftest import REPO

sys.path.insert(0, str(REPO / "scripts"))

import bump_version as bv  # noqa: E402


def make_tree(root: Path) -> None:
    (root / "src").mkdir()
    (root / "src" / "protocol.rs").write_text('pub const RUNTIME_VERSION: &str = "0.4.1";\n')
    (root / "CHANGELOG.md").write_text("## [0.4.1] released\n")
    (root / "Cargo.toml").write_text('version = "0.4.1"\ndep = "0.4.14"\n')


def test_only_whole_tokens_in_explicit_targets_are_rewritten(tmp_path: Path) -> None:
    make_tree(tmp_path)
    changed = bv.rewrite(tmp_path, "0.4.1", "0.5.0")
    assert sorted(p.name for p in changed) == ["Cargo.toml", "protocol.rs"]
    assert (tmp_path / "Cargo.toml").read_text() == 'version = "0.5.0"\ndep = "0.4.14"\n'
    assert (tmp_path / "CHANGELOG.md").read_text() == "## [0.4.1] released\n"


def test_stale_reports_what_a_forgotten_target_still_expects(tmp_path: Path) -> None:
    make_tree(tmp_path)
    bv.rewrite(tmp_path, "0.4.1", "0.5.0")
    (tmp_path / "src" / "protocol.rs").write_text('pub const RUNTIME_VERSION: &str = "0.4.1";\n')
    assert [p.name for p in bv.stale(tmp_path, "0.4.1")] == ["protocol.rs"]
