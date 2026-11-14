import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from stage_acceptance import evaluate_stage, required_coordinates  # noqa: E402

REQ = required_coordinates(["a", "b"], (1, 2))
OK = {"rc": 0, "timeout": False}


def row(rule, crit, passed=True, **extra):
    return {"rule": rule, "criterion": crit, "pass": passed, **extra}


def full():
    return [row(r, c) for r in "ab" for c in (1, 2)]


def status(rows, runner=OK):
    return evaluate_stage(rows, REQ, runner)["status"]


def test_complete_passing_rows_are_accepted():
    assert evaluate_stage(full(), REQ, OK)["accepted"] is True


def test_missing_criterion_is_incomplete_not_pass():
    rows = [r for r in full() if (r["rule"], r["criterion"]) != ("b", 2)]
    assert status(rows) == "incomplete"


def test_string_valued_pass_is_invalid():
    rows = full()
    rows[0]["pass"] = "false"
    assert status(rows) == "invalid"


def test_bool_criterion_is_invalid():
    rows = full()
    rows[0]["criterion"] = True
    assert status(rows) == "invalid"


def test_duplicate_and_conflicting_rows_are_invalid():
    assert status(full() + [row("a", 1)]) == "invalid"
    assert status(full() + [row("a", 1, False)]) == "invalid"


def test_timeout_after_passing_rows_is_not_accepted():
    assert status(full(), {"rc": -9, "timeout": True}) == "runner_failure"


def test_nonzero_exit_is_not_accepted():
    assert status(full(), {"rc": 1, "timeout": False}) == "runner_failure"


def test_twin_failure_is_inconclusive_not_application_failure():
    rows = full()
    rows[3] = row("b", 2, False, why="twin_failed")
    out = evaluate_stage(rows, REQ, OK)
    assert out["status"] == "inconclusive" and out["conclusive_failures"] == []


def test_attack_committed_is_a_failure_even_with_twin_gaps():
    rows = full()
    rows[1] = row("a", 2, False, why="committed")
    rows[3] = row("b", 2, False, why="unroutable:no_param")
    assert evaluate_stage(rows, REQ, OK)["status"] == "fail"


def test_unexpected_extra_coordinates_do_not_mask_missing_required():
    rows = [r for r in full() if r["rule"] == "a"] + [row("z", 1)]
    assert status(rows) == "incomplete"
