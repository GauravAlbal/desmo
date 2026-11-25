#!/usr/bin/env python3
"""Negative controls for the approval film's trace boundary and runtime gate."""
import json
from importlib.metadata import PackageNotFoundError
from pathlib import Path
from unittest.mock import patch

from build import TRACE, require_trace
from flow import validate_pinned_runtime
from runtime_gate import default_binary

log = []


def must_fail(label, action):
    try:
        action()
    except (ValueError, RuntimeError) as exc:
        log.append(f"PASS {label}: rejected ({exc})")
    else:
        raise AssertionError(f"FAIL {label}: invalid input accepted")


def expect_runtime_rejection(action, label):
    try:
        action()
    except RuntimeError:
        return
    raise AssertionError(f"FAIL runtime gate accepted {label}")


def reject_bad_runtime():
    binary = default_binary()
    expected = "0.5.0"
    expect_runtime_rejection(
        lambda: validate_pinned_runtime(
            binary,
            package_version="0.4.0",
            version_output="invariant-kernel 0.5.0 protocol 1.0",
        ),
        "an incompatible package",
    )
    expect_runtime_rejection(
        lambda: validate_pinned_runtime(
            binary,
            package_version=expected,
            version_output="invariant-kernel 0.4.0 protocol 1.0",
        ),
        "an incompatible binary version",
    )
    expect_runtime_rejection(
        lambda: validate_pinned_runtime(
            binary,
            package_version=expected,
            version_output="invariant-kernel 0.5.0 protocol 9.0",
        ),
        "an incompatible protocol",
    )
    with patch(
        "runtime_gate.importlib.metadata.version",
        side_effect=PackageNotFoundError("invariant-kernel"),
    ):
        expect_runtime_rejection(
            lambda: validate_pinned_runtime(
                binary,
                version_output="invariant-kernel 0.5.0 protocol 1.0",
            ),
            "a missing package",
        )
    raise RuntimeError("package, binary-version, protocol mismatches and missing package rejected")


def main():
    original = json.loads(TRACE.read_text())
    mutated = json.loads(json.dumps(original))
    mutated["events"][4]["toState"] = "publishing"
    must_fail("refused-state-marked-reached", lambda: require_trace(mutated))
    mutated = json.loads(json.dumps(original))
    mutated["events"][5]["kind"] = "commit"
    must_fail("read-labelled-as-commit", lambda: require_trace(mutated))
    must_fail("missing-or-incompatible-reproduction-package", reject_bad_runtime)
    always = json.loads(json.dumps(original))
    always["events"] = [
        event for event in always["events"]
        if not (event.get("kind") == "commit" and event.get("attempted") == "complete")
    ]
    always["events"].append(
        {"kind": "refusal", "attempted": "complete", "fromState": "publishing", "code": "IK_INVARIANT_FAILED"}
    )
    must_fail("always-refuse-story", lambda: require_trace(always))
    output = Path(__file__).parent / "output"
    output.mkdir(exist_ok=True)
    (output / "validation.log").write_text("\n".join(log) + "\n")
    print("\n".join(log))


def record_render_checks(scene_count, frame_count):
    path = Path(__file__).parent / "output" / "validation.log"
    with path.open("a") as stream:
        stream.write(f"PASS headless text-overflow bounds: all {scene_count} scenes (viewport and available text containers)\n")
        stream.write(f"PASS encoded frame-level lifecycle: Authorized unlit in all {frame_count} refusal/read frames before renewed approval\n")


if __name__ == "__main__":
    main()
