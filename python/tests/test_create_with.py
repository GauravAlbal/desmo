"""Atomic admission through the Python client."""

from pathlib import Path

import pytest

from invariant_kernel import HistoricalResult, Kernel, KernelError

ROOT = Path(__file__).resolve().parents[2]
MACHINE = ROOT / "examples" / "approval-revision" / "approval_revision.ik.toml"
EDIT = {"name": "edit", "args": {"artifact_digest": "doc-1"}}
BAD = {"name": "approve", "args": {"revision": 1}}


def test_invalid_initial_command_exposes_no_aggregate(ik_bin, tmp_path):
    machine = Kernel(tmp_path / "store.db", binary=str(ik_bin)).machine(MACHINE)
    with pytest.raises(KernelError):
        machine.create_with("doc-bad", BAD, operation_id="op-bad")
    with pytest.raises(KernelError):
        machine.show("doc-bad")
    lookup = machine.lookup_create_with("doc-bad", "op-bad", BAD)
    assert lookup.outcome == "not_recorded"


def test_same_operation_id_replays_history(ik_bin, tmp_path):
    machine = Kernel(tmp_path / "store.db", binary=str(ik_bin)).machine(MACHINE)
    created = machine.create_with("doc-good", EDIT, operation_id="op-good")
    assert not isinstance(created, HistoricalResult)
    assert created.state == "draft"
    assert created.generation == 1
    again = machine.create_with("doc-good", EDIT, operation_id="op-good")
    assert isinstance(again, HistoricalResult)
    assert again.kind == "create_with_command"
    lookup = machine.lookup_create_with("doc-good", "op-good", EDIT)
    assert lookup.outcome == "committed_exact"
    assert lookup.history.kind == "create_with_command"
