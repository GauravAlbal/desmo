"""IK-09: the exact-artifact approval app, driven against the real `ik` binary."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

from conftest import REPO

sys.path.insert(0, str(REPO / "examples" / "publish-approval"))

import scenarios as sc  # noqa: E402
from invariant_kernel import Kernel  # noqa: E402


@pytest.fixture
def env(tmp_path: Path, ik_bin: Path) -> sc.Env:
    return sc.make_env(tmp_path / "app", str(ik_bin))


def test_approve_a_then_mutate_to_b_is_refused_until_b_is_reapproved(env: sc.Env) -> None:
    result = sc.scenario_approve_then_mutate(env)
    assert result["approved_but_b"] == "IK_INVARIANT_FAILED"
    assert result["authorize_in_draft"] == "IK_ILLEGAL_STATE"
    assert result["reapproved_state"] == "approved"
    assert result["final_state"] == "published"
    assert result["published_digest"] == result["digest_b"] != result["digest_a"]
    assert result["sink_bytes"] == sc.DOC_B


def test_executor_that_reads_mutable_latest_bytes_is_stopped_before_any_write(env: sc.Env) -> None:
    result = sc.scenario_latest_content(env)
    assert result["substituted"] is not None and "nothing written" in result["substituted"]
    assert result["effect_after_refusal"] == "pending"
    assert result["wrong_complete"] == "IK_INVARIANT_FAILED"
    assert result["good_digest"] == result["authorized_digest"]
    assert not list(env.sink.glob("*.tmp"))
    assert (env.sink / "publish-doc-latest-95d16619e8db.bin").read_bytes() == sc.DOC_A


def test_effect_outcome_is_idempotent_and_a_conflicting_outcome_is_refused(env: sc.Env) -> None:
    result = sc.scenario_effect_outcomes(env)
    assert result["recorded_mutation"] is True
    assert result["duplicate_mutation"] is False
    assert result["conflict_code"] == "IK_EFFECT_OUTCOME_CONFLICT"
    assert result["conflict_type"] == "EffectConflictError"
    assert result["outcome"] == result["first_outcome"]


def test_executor_death_after_external_write_recovers_at_least_once(env: sc.Env) -> None:
    result = sc.scenario_executor_uncertainty(env)
    assert result["died"] is True
    assert result["status_while_unknown"] == "pending"
    assert result["sink_duplicate_on_retry"] is True  # the write ran twice: not exactly-once
    assert result["final_state"] == "published"
    assert result["published_digest"] == result["authorized"]


def test_lost_reply_followed_by_another_write_resolves_by_operation_id(env: sc.Env) -> None:
    result = sc.scenario_lost_reply(env)
    assert result["outcome"] == "committed_exact"
    assert result["history_is_historical"] is True
    assert result["current_generation"] > 3
    assert result["stale_retry_code"] == "IK_CAPABILITY_STALE"


def test_client_timeout_is_unknown_and_lookup_gives_the_only_answer(env: sc.Env) -> None:
    result = sc.scenario_timeout_unknown(env)
    assert result["submission"] == "submission-unknown"
    assert result["lookup"] in {"committed_exact", "not_recorded"}
    kinds = [event["kind"] for event in env.trace.events]
    assert kinds.count("unknown") == 1


def test_process_death_after_commit_reopens_verifies_and_replays(tmp_path: Path, ik_bin: Path) -> None:
    result = sc.scenario_process_death(tmp_path / "death", str(ik_bin))
    assert result["exit_code"] == 17
    assert result["recovered_state"] == "published"
    assert result["verify"]["verified"] is True
    assert result["replay"]["aggregates"][0]["state"]["state"] == "published"


def test_stale_projection_is_repaired_from_canonical_state_not_trusted(env: sc.Env) -> None:
    result = sc.scenario_projection(env)
    assert result["projection_claimed"] == "published"
    assert result["canonical_state"] == "approved"
    assert result["projection_after"] == "approved"


def test_rollback_keeps_existing_aggregates_on_their_own_definition(env: sc.Env) -> None:
    result = sc.scenario_versions(env)
    assert result["routed_versions"] == {1: 1, 2: 2, 3: 3}
    assert result["v3_after_rollback"] == 3
    assert result["naive_rollback_code"] == "IK_MACHINE_MISMATCH"
    for audit in result["audits"].values():
        assert audit["verify"]["verified"] is True
        assert audit["replay"]["aggregates"][0]["rebuilt"] is False


def test_v1_definition_cannot_drive_a_v2_aggregate(env: sc.Env) -> None:
    env.app.router.create_version = 2
    env.app.create("doc-wrong-binding")
    v1 = env.app.router.machines[1]
    code = sc.refusal_code(lambda: v1.show("doc-wrong-binding").edit(artifact_digest="x"))
    assert code == "IK_MACHINE_MISMATCH"
    assert env.app.fresh("doc-wrong-binding").state == "draft"


def test_defective_revise_is_refused_at_runtime_and_by_a_bounded_witness(tmp_path: Path, ik_bin: Path) -> None:
    result = sc.scenario_defective(tmp_path, str(ik_bin))
    assert result["state_before_publish"] == "approved"  # the defect: revise kept approval
    assert result["runtime_code"] == "IK_INVARIANT_FAILED"
    assert result["check_verdict"] == "refused"
    witness = result["witnesses"][0]
    assert witness["diagnostic"]["rule"] == "pub_inv_001"
    assert [step["command"] for step in witness["steps"]] == ["edit", "approve", "revise", "publish"]


def test_all_refuse_machine_fails_required_progress_while_the_correct_one_passes(tmp_path: Path, ik_bin: Path) -> None:
    binary = str(ik_bin)
    broken = sc.scenario_all_refuse(binary)
    good = sc.scenario_correct_check(binary)
    assert broken == {"verdict": "refused", "release": "release_refused"}
    assert good == {"verdict": "accept", "release": "release_pass"}
    doc = Kernel(tmp_path / "refuse.db", binary=binary).machine(sc.ALL_REFUSE).create("d")
    doc = doc.edit(artifact_digest="aaaa")
    assert sc.refusal_code(lambda: doc.approve(artifact_digest="aaaa")) is not None


def test_emitted_traces_record_distinct_kinds_and_a_labelled_witness(tmp_path: Path, ik_bin: Path) -> None:
    results = sc.run_all(tmp_path / "run", str(ik_bin))
    observed = results["traces"]["observed"]
    kinds = {event["kind"] for event in observed["events"]}
    assert {"commit", "refusal", "read", "historical", "unknown"} <= kinds
    assert results["traces"]["witness"]["origin"] == "bounded-witness"
    refusals = [e for e in observed["events"] if e["kind"] == "refusal"]
    assert {e["code"] for e in refusals} >= {"IK_ILLEGAL_STATE"}
    witness = json.loads((tmp_path / "run" / "witness.trace.json").read_text())
    assert [e["kind"] for e in witness["events"]] == ["witness"] * 4
    assert witness["events"][-1]["toState"] == "blocked by IK_INVARIANT_FAILED"
