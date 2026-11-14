from __future__ import annotations

from pathlib import Path

from conftest import VA
from invariant_kernel import Kernel, StaleCapabilityError


class LegacyJobs:
    def __init__(self) -> None:
        self.jobs: dict[str, dict[str, object]] = {}

    def create(self, job_id: str) -> dict[str, object]:
        self.jobs[job_id] = {"owner": "legacy", "state": "created"}
        return self.jobs[job_id]

    def finish(self, job_id: str) -> None:
        self.jobs[job_id]["state"] = "done"


def test_new_jobs_desmo_old_jobs_legacy(ik_bin: Path, store: Path) -> None:
    legacy = LegacyJobs()
    legacy.create("old-1")
    legacy.finish("old-1")
    assert legacy.jobs["old-1"]["state"] == "done"

    k = Kernel(store, binary=str(ik_bin)).machine(VA)
    new = k.create("new-1")
    running = new.start()
    cancelled = running.cancel()
    assert cancelled.state == "cancelled"
    try:
        running.submit(artifact_digest="a", baseline_digest="b")
        raise AssertionError("stale submit committed")
    except StaleCapabilityError:
        pass
    fresh = k.show("new-1")
    assert fresh.state == "cancelled"
    assert "new-1" not in legacy.jobs
