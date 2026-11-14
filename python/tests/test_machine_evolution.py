from __future__ import annotations

from pathlib import Path

import pytest

from conftest import REPO
from invariant_kernel import Kernel, MachineError

V1 = REPO / "machines" / "publish_approval.ik.toml"
V2 = REPO / "machines" / "publish_approval_v2.ik.toml"


def test_old_aggregates_stay_on_v1_new_use_v2(ik_bin: Path, store: Path) -> None:
    k = Kernel(store, binary=str(ik_bin))
    v1 = k.machine(V1)
    v2 = k.machine(V2)
    assert v1.identity.version == 1
    assert v2.identity.version == 2
    old = v1.create("legacy-doc")
    old = old.edit(artifact_digest="a")
    old = old.approve(artifact_digest="a")
    new = v2.create("fresh-doc")
    new = new.edit(artifact_digest="b")
    new = new.approve(artifact_digest="b")
    withdrawn = new.withdraw()
    assert withdrawn.state == "draft"
    peeked = v2.show("legacy-doc")
    assert peeked.state == "approved"
    with pytest.raises(MachineError) as exc:
        peeked.withdraw()
    assert exc.value.code == "IK_MACHINE_MISMATCH"
    still = v1.show("legacy-doc")
    assert still.state == "approved"
    assert still.generation == old.generation


def test_owner_names_the_definition_that_owns_an_old_aggregate(ik_bin: Path, store: Path) -> None:
    k = Kernel(store, binary=str(ik_bin))
    v1 = k.machine(V1)
    v2 = k.machine(V2)
    created = v1.create("legacy-doc")
    v2.create("fresh-doc")
    legacy = v2.show("legacy-doc")
    assert legacy.owner == v1.identity
    assert legacy.owner != v2.identity
    assert v2.show("fresh-doc").owner == v2.identity
    routed = next(m for m in (v1, v2) if m.identity == legacy.owner)
    assert routed.show("legacy-doc").state == created.state
