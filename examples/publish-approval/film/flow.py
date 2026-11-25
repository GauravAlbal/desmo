#!/usr/bin/env python3
"""Run the film's exact-artifact lifecycle and pin its observed trace bundle."""
import argparse
import json
import sys
import tempfile
from pathlib import Path
from typing import Optional

HERE = Path(__file__).resolve().parent
EXAMPLE = HERE.parent
sys.path.insert(0, str(EXAMPLE))
import scenarios  # noqa: E402
from runtime_gate import verify_runtime


class FilmTrace(scenarios.Trace):
    """Add film-only digest annotations at the refusal event, not after the run."""

    def __init__(self):
        super().__init__()
        self.next_film_annotation: Optional[dict[str, str]] = None

    def refusal(self, attempted, before, err, intended):
        super().refusal(attempted, before, err, intended)
        if self.next_film_annotation is not None:
            self.events[-1]["filmFlowAnnotation"] = json.dumps(self.next_film_annotation, sort_keys=True)
            self.next_film_annotation = None


def validate_pinned_runtime(binary: str, *, package_version=None, version_output=None):
    pinned = json.loads((HERE / "source.trace.json").read_text())
    verify_runtime(
        binary,
        pinned["runtime_version"],
        package_version=package_version,
        version_output=version_output,
    )


def observed_bundle(binary: str) -> dict:
    validate_pinned_runtime(binary)
    with tempfile.TemporaryDirectory(prefix="ik-publish-film-") as tmp:
        env = scenarios.make_env(Path(tmp) / "run", binary)
        trace = FilmTrace()
        env.trace = trace
        env.app.trace = trace

        approved = scenarios.approved_doc(env, "film-publication")
        revised = env.app.revise(approved, scenarios.DOC_B)
        approved_digest = str(approved.facts["approved_digest"])
        proposed_digest = scenarios.sha256_hex(env.drafts.read(approved.id))
        if approved_digest == proposed_digest:
            raise ValueError("real approved and proposed artifact digests unexpectedly match")

        trace.next_film_annotation = {
            "kind": "digest-comparison",
            "approvedDigest": approved_digest,
            "proposedDigest": proposed_digest,
        }
        refusal = scenarios.refusal_code(
            lambda: env.app.authorize_publish(approved, payload_digest=proposed_digest)
        )
        if refusal != "IK_CAPABILITY_STALE":
            raise ValueError(f"stale authorization was not refused as expected: {refusal}")
        canonical = env.app.fresh(approved.id)
        reapproved = env.app.approve(canonical)
        result = scenarios.publish_exact(env, approved.id)
        if reapproved.state != "approved" or result["state"] != "published":
            raise ValueError("renewed approval and publication did not commit")
        if result["published"].digest != proposed_digest:
            raise ValueError("published bytes do not match the real proposed artifact")

        return scenarios.trace_bundle(
            "film-flow-observed",
            trace.events,
            "python3 examples/publish-approval/film/flow.py --out examples/publish-approval/film/source.trace.json",
        )


def main(argv=None) -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--binary", default=scenarios.default_binary())
    args = parser.parse_args(argv)
    data = observed_bundle(args.binary)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(data, indent=2) + "\n")
    print(json.dumps({"output": str(args.out), "events": len(data["events"]), "product": data["product"]}, indent=2))


if __name__ == "__main__":
    main()
