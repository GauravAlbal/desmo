from __future__ import annotations

from pathlib import Path

import pytest

from conftest import REPO
from invariant_kernel import IllegalStateError, Kernel

PUB = REPO / "machines" / "publish_approval.ik.toml"


def naive_edit_after_approval() -> dict[str, object]:
    """Host map: keep approved=True while replacing the artifact, then publish."""
    job = {"artifact": "art-1", "approved": True, "state": "approved"}
    job["artifact"] = "art-2"
    if job["approved"]:
        job["state"] = "published"
    return job


def test_naive_path_publishes_unapproved_artifact() -> None:
    assert naive_edit_after_approval()["state"] == "published"
    assert naive_edit_after_approval()["artifact"] == "art-2"


def test_feature_change_publish_refused_then_corrected(ik_bin: Path, store: Path) -> None:
    machine = Kernel(store, binary=str(ik_bin)).machine(PUB)
    doc = machine.create("doc-1")
    doc = doc.edit(artifact_digest="art-1")
    doc = doc.approve(artifact_digest="art-1")
    assert doc.state == "approved"
    revised = doc.revise(artifact_digest="art-2")
    assert revised.state == "draft"
    with pytest.raises(IllegalStateError) as exc:
        revised.publish()
    assert exc.value.code == "IK_ILLEGAL_STATE"
    fresh = machine.show("doc-1")
    assert fresh.state == "draft"
    approved = fresh.approve(artifact_digest="art-2")
    published = approved.publish()
    assert published.state == "published"
    assert published.facts["artifact_digest"] == "art-2"
    assert published.facts["approved_digest"] == "art-2"
