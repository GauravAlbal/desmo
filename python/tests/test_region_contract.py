"""Normative source/client contracts; IK_BINARY is the only runtime authority."""
from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path
from typing import Iterator

import pytest

from conftest import REPO
from invariant_kernel import InvariantFailedError, Kernel


# Independently authored equivalent schema-1 machine, not emitted by the frontend.
# The pinned hash is SHA-256 of its canonical typed Definition (including default
# initial/terminal booleans and replace policy), calculated without invoking ik.
IDENTITY_HASH = "73bb9c4b1fbcf9671b8d3236c276e85aacc226c09477ccd959ccad0da14592cb"
REGION_IDENTITY = '''schema_version = 2
machine_id = "client_identity"
machine_version = 7

[regions.power.states]
off = { initial = true }
on = { terminal = true }

[regions.power.commands.turn_on]
from = ["off"]
to = "on"
'''
FLAT_IDENTITY = '''schema_version = 1
machine_id = "client_identity"
machine_version = 7

[facts.region_power]
type = "string"
policy = "replace"
initial = "off"

[states]
off = { initial = true }
on = { terminal = true }

[commands.turn_on]
from = ["off"]
[[commands.turn_on.outcomes]]
when = 'state == "off" && (region_power == "off")'
to = "on"
then = { region_power = '\"on\"' }

[invariants.region_projection_off]
when = 'state == "off"'
assert = 'region_power == "off"'
[invariants.region_projection_on]
when = 'state == "on"'
assert = 'region_power == "on"'
'''


@pytest.fixture
def region_binary() -> str:
    binary = os.environ.get("IK_BINARY")
    assert binary, "region contracts require explicit IK_BINARY; no build/download fallback"
    path = Path(binary).resolve()
    assert path.is_file() and os.access(path, os.X_OK), f"not executable: {path}"
    return str(path)


@pytest.fixture(params=[False, True], ids=["one-shot", "session"])
def kernel(
    region_binary: str, tmp_path: Path, request: pytest.FixtureRequest
) -> Iterator[Kernel]:
    with Kernel(
        tmp_path / "region.db", binary=region_binary, session=request.param
    ) as client:
        yield client


def test_source_aware_checker_accepts_compound_and_binds_parallel_refusal(
    region_binary: str, tmp_path: Path
) -> None:
    kernel = Kernel(tmp_path / "check.db", binary=region_binary)
    corpus = tmp_path / "compound.cases.json"
    corpus.write_text(json.dumps({"commands": {"power_off": [
        {"name": "child-enabled", "args": {"override_enabled": True}},
        {"name": "parent-fallback", "args": {"override_enabled": False}},
    ]}}), encoding="utf-8")
    compound = kernel.check(REPO / "machines/region_compound.ik.toml", corpus=corpus)
    assert compound.ok is True, compound.text
    assert compound.verdict == "accept"
    assert "VERDICT\n  ACCEPT\n" in compound.text
    assert "invariant-refusal witness" not in compound.text

    parallel_path = REPO / "machines/region_parallel.ik.toml"
    parallel = kernel.check(parallel_path)
    assert parallel.ok is False, parallel.text
    assert parallel.verdict == "refused"
    assert "VERDICT\n  REFUSED\n" in parallel.text
    assert "invariant-refusal witness IK_INVARIANT_FAILED rule=no_playing_muted\n" in parallel.text
    identity = kernel.machine(parallel_path).identity.to_wire()
    assert f"  spec_hash {identity['spec_hash']}\n" in parallel.text


def test_source_and_explicit_flat_machine_share_pinned_identity_and_store(
    kernel: Kernel, tmp_path: Path
) -> None:
    source_path = tmp_path / "source.ik.toml"
    flat_path = tmp_path / "flat.ik.toml"
    source_path.write_text(REGION_IDENTITY, encoding="utf-8")
    flat_path.write_text(FLAT_IDENTITY, encoding="utf-8")
    source = kernel.machine(source_path)
    flat = kernel.machine(flat_path)
    expected = {"machine_id": "client_identity", "version": 7, "spec_hash": IDENTITY_HASH}
    assert source.identity.to_wire() == expected
    assert flat.identity.to_wire() == expected
    created = source.create("cross-source")
    assert created.state == "off"
    assert created.facts == {"region_power": "off"}
    # Rebinding the same durable aggregate through explicit schema 1 must not
    # cause a machine mismatch or weaken source projection checks.
    completed = flat.show("cross-source").command("turn_on")
    assert completed.raw["state"] == {"state": "on", "region_power": "on"}
    assert completed.generation == 1
    assert completed.terminal is True
    replayed = source.replay("cross-source")
    assert replayed["aggregates"][0]["state"] == {"state": "on", "region_power": "on"}
    assert source.verify("cross-source")["verified"] is True


def test_parallel_refusal_preserves_authority_then_joint_commits_and_replays(
    kernel: Kernel, region_binary: str
) -> None:
    version = subprocess.check_output([region_binary, "--version"], text=True).split()
    assert version == ["invariant-kernel", "0.5.0", "protocol", "1.0"]
    machine = kernel.machine(REPO / "machines/region_parallel.ik.toml")
    original = machine.create("playback")
    before = machine.show("playback")
    before_replay = machine.replay("playback")
    assert before.raw["state"] == {
        "state": "playing__unmuted", "region_playback": "playing", "region_volume": "unmuted"
    }
    assert before.generation == 0
    with pytest.raises(InvariantFailedError) as refusal:
        original.command("mute")
    assert refusal.value.code == "IK_INVARIANT_FAILED"
    assert refusal.value.rule == "no_playing_muted"
    after = machine.show("playback")
    assert after.raw == before.raw
    assert after.raw["capability"] == original.raw["capability"]
    assert machine.replay("playback") == before_replay
    refused_verify = machine.verify("playback")
    assert refused_verify["verified"] is True
    assert refused_verify["journal_ok"] is True
    assert refused_verify["projection_ok"] is True

    # The original authority remains usable; this joint must not pass through
    # a forbidden playing+muted intermediate state.
    committed = original.command("pause_and_mute")
    expected = {"state": "paused__muted", "region_playback": "paused", "region_volume": "muted"}
    assert committed.raw["state"] == expected
    assert committed.facts == {"region_playback": "paused", "region_volume": "muted"}
    assert committed.generation == 1
    assert committed.terminal is False
    assert committed.raw["capability"] != original.raw["capability"]
    replayed = machine.replay("playback")
    assert replayed["aggregates"][0]["state"] == expected
    assert replayed["aggregates"][0]["generation"] == 1
    assert machine.replay("playback") == replayed
    verified = machine.verify("playback")
    assert verified["verified"] is True
    assert verified["journal_ok"] is True
    assert verified["projection_ok"] is True
