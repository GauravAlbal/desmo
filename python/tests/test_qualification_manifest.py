"""IK-06: the claim -> enforcement -> evidence manifest is complete and the validator is not blind."""

from __future__ import annotations

import copy
import sys

from conftest import REPO

sys.path.insert(0, str(REPO / "validation" / "ik06"))

import qualify  # noqa: E402


def test_manifest_covers_every_mutant_and_every_row_exists() -> None:
    manifest = qualify.load_manifest()
    variants = qualify.enum_variants((REPO / "src" / "law_mutant.rs").read_text())
    assert len(variants) >= 15
    assert qualify.validate(manifest) == []
    declared = {m["name"] for c in manifest["claims"] for m in c["mutants"]}
    assert declared == set(variants)


def test_every_claim_has_enforcement_and_positive_evidence() -> None:
    for claim in qualify.load_manifest()["claims"]:
        assert claim["enforcement"], claim["id"]
        assert claim["evidence"], claim["id"]


def test_a_removed_evidence_row_or_mutant_is_rejected() -> None:
    manifest = qualify.load_manifest()
    no_evidence = copy.deepcopy(manifest)
    no_evidence["claims"][0]["evidence"] = []
    assert any("no evidence rows" in p for p in qualify.validate(no_evidence))
    no_mutant = copy.deepcopy(manifest)
    removed = no_mutant["claims"][0]["mutants"].pop()["name"]
    assert any(removed in p and "not covered" in p for p in qualify.validate(no_mutant))


def test_a_missing_or_ignored_test_is_not_evidence() -> None:
    missing = {"runner": "rust", "file": "tests/genesis_validity.rs", "test": "no_such_test"}
    assert any("not found" in p for p in qualify.row_problems(missing, REPO))
    ignored_text = "#[ignore]\n#[test]\nfn flaky() {}\n"
    index = qualify.find_test(ignored_text, "rust", "flaky")
    assert index == 2
    assert qualify.is_ignored(ignored_text, "rust", index)


def test_a_run_that_executes_zero_tests_does_not_pass() -> None:
    counts = qualify.parse_counts("rust", "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out")
    assert counts["passed"] == 0
    real = qualify.parse_counts("rust", "test result: ok. 1 passed; 0 failed; 0 ignored")
    assert real == {"passed": 1, "failed": 0, "skipped": 0}


def test_the_built_in_negative_controls_are_all_detected() -> None:
    assert qualify.self_test() == []
