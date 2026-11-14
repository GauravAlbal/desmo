"""Failure classification against the installed ik binary.

Expectations come from docs/design/error-taxonomy.md, not from the client's
previous output. A missing store is not corruption. Operation ids are
aggregate-scoped; global uniqueness is an application constraint.
"""

from __future__ import annotations

import stat
import subprocess
from pathlib import Path

import pytest

from invariant_kernel import (
    Capability,
    HistoricalResult,
    Kernel,
    KernelError,
    PreDispatchError,
    StaleCapabilityError,
    StoreNotFoundError,
    TransportError,
    classify,
)

from conftest import EFFECT, VA

START = {"name": "start"}
CANCEL = {"name": "cancel"}


def test_missing_store_is_not_found_and_read_only_ops_create_nothing(
    ik_bin: Path, tmp_path: Path
) -> None:
    parent = tmp_path / "absent-parent"
    path = parent / "store.db"
    assert not parent.exists()
    machine = Kernel(path, binary=str(ik_bin)).machine(VA)
    with pytest.raises(StoreNotFoundError) as shown:
        machine.show("missing")
    assert shown.value.code == "IK_STORE_NOT_FOUND"
    assert shown.value.rule == "store.missing"
    assert shown.value.diagnostic["code"] == "IK_STORE_NOT_FOUND"
    assert shown.value.diagnostic["rule"] == "store.missing"
    assert classify(shown.value) == "not_found"
    assert classify(shown.value) != "corrupt"
    with pytest.raises(StoreNotFoundError) as replayed:
        machine.replay("missing")
    assert replayed.value.rule == "store.missing"
    assert classify(replayed.value) == "not_found"
    with pytest.raises(StoreNotFoundError) as verified:
        machine.verify("missing")
    assert classify(verified.value) == "not_found"
    capability = Capability.from_wire({"generation": 0, "nonce": "n"})
    with pytest.raises(StoreNotFoundError) as looked:
        machine.lookup_operation("missing", capability, "op-1", command=START)
    assert looked.value.code == "IK_STORE_NOT_FOUND"
    assert classify(looked.value) == "not_found"
    assert not path.exists()
    assert not parent.exists()


def test_create_creates_the_store_and_a_command_without_one_does_not(
    ik_bin: Path, tmp_path: Path
) -> None:
    missing = tmp_path / "no-parent" / "store.db"
    machine = Kernel(missing, binary=str(ik_bin)).machine(VA)
    capability = Capability.from_wire({"generation": 0, "nonce": "n"})
    with pytest.raises(StoreNotFoundError):
        machine.lookup_operation("agg-1", capability, "op-1", command=START)
    assert not missing.exists()
    assert not missing.parent.exists()
    created = tmp_path / "created.db"
    Kernel(created, binary=str(ik_bin)).machine(VA).create("agg-1")
    assert created.is_file()


def test_garbage_store_file_is_corrupt_not_missing(ik_bin: Path, tmp_path: Path) -> None:
    path = tmp_path / "garbage.db"
    path.write_bytes(b"this is not a sqlite database")
    machine = Kernel(path, binary=str(ik_bin)).machine(VA)
    with pytest.raises(KernelError) as raised:
        machine.show("agg-1")
    assert not isinstance(raised.value, StoreNotFoundError)
    assert raised.value.code == "IK_STORE_CORRUPT"
    assert raised.value.rule != "store.missing"
    assert classify(raised.value) == "corrupt"
    assert path.is_file()
    with pytest.raises(KernelError) as again:
        machine.show("agg-1")
    assert again.value.code == "IK_STORE_CORRUPT"
    assert classify(again.value) == "corrupt"


def test_unknown_aggregate_is_not_found_not_corrupt(ik_bin: Path, store: Path) -> None:
    machine = Kernel(store, binary=str(ik_bin)).machine(VA)
    machine.create("present")
    with pytest.raises(KernelError) as raised:
        machine.show("absent")
    assert raised.value.code == "IK_ILLEGAL_STATE"
    assert raised.value.rule == "aggregate.not_found"
    assert raised.value.diagnostic["rule"] == "aggregate.not_found"
    assert classify(raised.value) == "not_found"
    assert classify(raised.value) != "corrupt"


def test_stale_capability_is_refused(ik_bin: Path, store: Path) -> None:
    machine = Kernel(store, binary=str(ik_bin)).machine(VA)
    created = machine.create("attempt-1")
    created.execute(START, operation_id="first-op")
    with pytest.raises(StaleCapabilityError) as raised:
        created.execute(CANCEL, operation_id="other-op")
    assert raised.value.code == "IK_CAPABILITY_STALE"
    assert raised.value.submission == "refused"
    assert classify(raised.value) == "refused"


def test_exact_repeat_is_historical_not_an_error(ik_bin: Path, store: Path) -> None:
    machine = Kernel(store, binary=str(ik_bin)).machine(VA)
    created = machine.create("attempt-1")
    created.execute(START, operation_id="same-op")
    repeat = created.execute(START, operation_id="same-op")
    assert isinstance(repeat, HistoricalResult)
    assert not isinstance(repeat, KernelError)
    assert classify(repeat) == "historical"


def test_missing_binary_is_not_sent(ik_bin: Path, store: Path, tmp_path: Path) -> None:
    kernel = Kernel(store, binary=str(ik_bin))
    created = kernel.machine(VA).create("attempt-1")
    kernel.binary = str(tmp_path / "no-such-ik")
    with pytest.raises(PreDispatchError) as raised:
        created.execute(START, operation_id="never-left")
    assert raised.value.submission == "not-sent"
    assert raised.value.operation_id == "never-left"
    assert classify(raised.value) == "not_sent"


def test_timeout_after_dispatch_is_uncertain(
    ik_bin: Path, store: Path, tmp_path: Path
) -> None:
    dispatches = tmp_path / "dispatches.log"
    stub = tmp_path / "slow-ik"
    version = subprocess.run(
        [str(ik_bin), "--version"], capture_output=True, text=True, check=True
    ).stdout.strip()
    stub.write_text(
        "#!/bin/sh\n"
        f'if [ "$1" = "--version" ]; then echo "{version}"; exit 0; fi\n'
        f'echo dispatched >> "{dispatches}"\n'
        "sleep 5\n"
    )
    stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
    machine = Kernel(store, binary=str(ik_bin)).machine(VA)
    created = machine.create("attempt-1")
    machine.kernel.binary = str(stub)
    machine.kernel.timeout = 0.4
    with pytest.raises(TransportError) as raised:
        created.execute(START, operation_id="maybe-committed")
    assert not isinstance(raised.value, PreDispatchError)
    assert raised.value.submission == "submission-unknown"
    assert raised.value.operation_id == "maybe-committed"
    assert classify(raised.value) == "uncertain"
    assert dispatches.read_text().splitlines() == ["dispatched"]


def test_legacy_missing_store_code_is_still_classified_not_found() -> None:
    legacy = KernelError(
        "IK_STORE_CORRUPT",
        "store file missing",
        "store.missing",
        {
            "code": "IK_STORE_CORRUPT",
            "rule": "store.missing",
            "message": "store file missing",
        },
    )
    assert legacy.rule == "store.missing"
    assert classify(legacy) == "not_found"


def test_machine_mismatch_is_contract_mismatch(ik_bin: Path, store: Path) -> None:
    home = Kernel(store, binary=str(ik_bin)).machine(VA)
    created = home.create("attempt-1")
    foreign = Kernel(store, binary=str(ik_bin)).machine(EFFECT)
    created._machine = foreign  # present a different identity for the same aggregate
    with pytest.raises(KernelError) as raised:
        created.execute(START, operation_id="wrong-machine")
    assert raised.value.code.startswith("IK_MACHINE_")
    assert classify(raised.value) == "contract_mismatch"


def test_same_operation_id_on_two_aggregates_does_not_collide(
    ik_bin: Path, store: Path
) -> None:
    """Operation ids are per aggregate. Global uniqueness is the application's constraint."""
    machine = Kernel(store, binary=str(ik_bin)).machine(VA)
    first = machine.create("agg-a")
    second = machine.create("agg-b")
    started_a = first.execute(START, operation_id="shared-op")
    started_b = second.execute(START, operation_id="shared-op")
    assert not isinstance(started_a, HistoricalResult)
    assert not isinstance(started_b, HistoricalResult)
    lookup_a = machine.lookup_operation(
        "agg-a", first.capability, "shared-op", command=START
    )
    lookup_b = machine.lookup_operation(
        "agg-b", second.capability, "shared-op", command=START
    )
    assert lookup_a.outcome == "committed_exact"
    assert lookup_b.outcome == "committed_exact"
    with pytest.raises(KernelError) as conflict:
        first.execute(CANCEL, operation_id="shared-op")
    assert conflict.value.code == "IK_OPERATION_CONFLICT"
    assert classify(conflict.value) == "refused"
    still_b = machine.lookup_operation(
        "agg-b", second.capability, "shared-op", command=START
    )
    assert still_b.outcome == "committed_exact"
