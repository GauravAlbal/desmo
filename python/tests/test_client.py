from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

from conftest import EFFECT, REPO, VA
from invariant_kernel import (
    CommandInvalidError,
    EffectConflictError,
    IllegalStateError,
    InvariantFailedError,
    Kernel,
    ProtocolError,
    StaleCapabilityError,
    TransportError,
)

TAGGED = REPO / "examples" / "shipped" / "tagged_item.ik.toml"


def kernel(ik_bin: Path, store: Path) -> Kernel:
    return Kernel(store, binary=str(ik_bin))


def test_create_and_happy_lifecycle(ik_bin: Path, store: Path) -> None:
    attempts = kernel(ik_bin, store).machine(VA)
    job = attempts.create("attempt-123")
    assert job.state == "created"
    assert job.generation == 0
    job = job.start()
    assert job.state == "running"
    job = job.submit(artifact_digest="abc", baseline_digest="def")
    assert job.state == "submitted"
    job = job.verify(
        receipt_digest="r1",
        receipt_artifact_digest="abc",
        receipt_baseline_digest="def",
    )
    assert job.state == "verified"
    assert job.terminal is True
    assert job.generation == 3


def test_stale_copied_handle_refuses(ik_bin: Path, store: Path) -> None:
    job = kernel(ik_bin, store).machine(VA).create("job")
    old = job
    current = old.start()
    assert current.state == "running"
    with pytest.raises(StaleCapabilityError) as exc:
        old.submit(artifact_digest="a", baseline_digest="b")
    assert exc.value.code == "IK_CAPABILITY_STALE"
    assert exc.value.rule is not None


def test_malformed_command_refuses(ik_bin: Path, store: Path) -> None:
    job = kernel(ik_bin, store).machine(VA).create("job").start()
    with pytest.raises(CommandInvalidError) as exc:
        job.submit()
    assert exc.value.code == "IK_COMMAND_INVALID"


def test_terminal_mutation_refuses(ik_bin: Path, store: Path) -> None:
    job = (
        kernel(ik_bin, store)
        .machine(VA)
        .create("job")
        .start()
        .submit(artifact_digest="a", baseline_digest="b")
        .verify(
            receipt_digest="r",
            receipt_artifact_digest="a",
            receipt_baseline_digest="b",
        )
    )
    assert job.terminal is True
    with pytest.raises(IllegalStateError) as exc:
        job.cancel()
    assert exc.value.code == "IK_ILLEGAL_STATE"


def test_effect_plan_record_duplicate_conflict(ik_bin: Path, store: Path) -> None:
    probe = kernel(ik_bin, store).machine(EFFECT)
    agg = probe.create("fx").plan(
        effect_id="e1", payload_digest="p1", idempotency_key="k1"
    )
    assert agg.effect.status == "pending"
    recorded = agg.record(outcome_digest="o1")
    assert recorded.effect.status == "recorded"
    again = recorded.record(outcome_digest="o1")
    assert again.mutation is False
    with pytest.raises(EffectConflictError) as exc:
        again.record(outcome_digest="other")
    assert exc.value.code == "IK_EFFECT_OUTCOME_CONFLICT"


def test_show_replay_verify(ik_bin: Path, store: Path) -> None:
    machine = kernel(ik_bin, store).machine(VA)
    created = machine.create("shown")
    created.start()
    shown = machine.show("shown")
    assert shown.state == "running"
    assert shown.generation == 1
    replayed = machine.replay("shown")
    assert replayed["aggregates"]
    verified = machine.verify("shown")
    assert verified["verified"] is True


def test_diagnostic_mapping_preserves_kernel_fields(ik_bin: Path, store: Path) -> None:
    job = kernel(ik_bin, store).machine(VA).create("job")
    old = job
    job.start()
    with pytest.raises(StaleCapabilityError) as exc:
        old.start()
    assert exc.value.diagnostic["code"] == "IK_CAPABILITY_STALE"
    assert "message" in exc.value.diagnostic


def test_protocol_mismatch(ik_bin: Path, store: Path) -> None:
    k = kernel(ik_bin, store)
    machine = k.machine(VA)
    k._protocol = {"major": 2, "minor": 0}
    with pytest.raises(ProtocolError) as exc:
        machine.create("job")
    assert exc.value.code == "IK_PROTOCOL_VERSION_UNKNOWN"


def test_transport_failure_is_not_domain_diagnostic(ik_bin: Path, store: Path) -> None:
    with pytest.raises(TransportError):
        Kernel(store, binary="/no/such/ik-binary").machine(VA)


def test_python_matches_direct_cli(ik_bin: Path, tmp_path: Path) -> None:
    py_store = tmp_path / "py.db"
    cli_store = tmp_path / "cli.db"
    py = kernel(ik_bin, py_store).machine(VA).create("agg-1").start()

    identity = json.loads(
        subprocess.check_output([str(ik_bin), "identity", str(VA)], text=True)
    )
    create = {
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": "0.5.0",
        "request_id": "c",
        "op": "create",
        "aggregate_id": "agg-1",
        "machine": identity,
    }
    created = json.loads(
        subprocess.check_output(
            [str(ik_bin), "--store", str(cli_store), "--machine", str(VA), "create"],
            input=json.dumps(create, separators=(",", ":"), sort_keys=True),
            text=True,
        )
    )
    command = {
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": "0.5.0",
        "request_id": "s",
        "op": "command",
        "aggregate_id": "agg-1",
        "machine": identity,
        "capability": created["result"]["capability"],
        "command": {"name": "start"},
    }
    started = json.loads(
        subprocess.check_output(
            [str(ik_bin), "--store", str(cli_store), "--machine", str(VA), "command"],
            input=json.dumps(command, separators=(",", ":"), sort_keys=True),
            text=True,
        )
    )
    cli = started["result"]
    assert py.state == cli["state"]["state"]
    assert py.generation == cli["generation"]
    assert py.terminal == cli["terminal"]
    assert py.effect.status == cli["effect"]["status"]
    assert py.raw["capability"] == cli["capability"]


def test_text_in_identifier_commits_and_refuses(ik_bin: Path, tmp_path: Path) -> None:
    items = kernel(ik_bin, tmp_path / "tag.db").machine(TAGGED)
    tagged = items.create("item-1").command("tag", id="item_1", label="AMBER")
    assert tagged.state == "tagged"
    assert tagged.terminal is True
    assert tagged.facts["item_id"] == "item_1"
    empty = items.create("item-empty")
    with pytest.raises(InvariantFailedError) as exc:
        empty.command("tag", id="", label="AMBER")
    assert exc.value.code == "IK_INVARIANT_FAILED"
    assert exc.value.rule == "tag.when"
    shown = items.show("item-empty")
    assert shown.state == "idle"


def test_check_wrapper_is_developer_surface(ik_bin: Path, store: Path) -> None:
    result = kernel(ik_bin, store).check(
        VA, baseline=REPO / "validation" / "verification_baseline.json"
    )
    assert "VERDICT" in result.text
    assert result.verdict in {"accept", "refused"}
