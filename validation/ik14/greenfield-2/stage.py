#!/usr/bin/env python3
"""Stage one builder or maintainer workspace for IK-14 tranche 2, OUTSIDE the repository tree.

  python3 validation/ik14/greenfield-2/stage.py <G4|G5|G6> <A|B> <r1|r2> initial
  python3 validation/ik14/greenfield-2/stage.py <G4|G5|G6> <A|B> <r1|r2> change

Workspace root: $IK14_WORK (default /tmp/ik14-runs). Layout:

  <root>/<project>/<arm>/<rep>/initial/{PROMPT.md, project/}
  <root>/<project>/<arm>/<rep>/change/{PROMPT.md, project/}      (copy of the frozen initial project)

A builder sees only project/ and PROMPT.md. The staged project holds BRIEF.md (+ CHANGE.md in the change
stage), stub/CONTRACT.md, stub/server.mjs and, for arm B only, KIT/. Evaluators, controls, sidecars and
reference implementations are never staged. The script refuses to finish if a leak scan finds evaluator or
control material in the staged tree.
"""
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
ROOT = Path(os.environ.get("IK14_WORK", "/tmp/ik14-runs"))
KIT_DOCS = [
    "README.md",
    "docs/typescript.md",
    "docs/greenfield.md",
    "docs/ik-check.md",
    "docs/machine-language.md",
    "docs/client-recovery.md",
    "docs/operation-identity.md",
    "docs/machine-evolution.md",
]
KIT_TARBALLS = ["desmo-runtime-0.5.0.tgz", "desmo-darwin-arm64-0.5.0.tgz"]
LEAK = re.compile(
    r"evaluator|control-sidecar|/control/|controlPort|EVAL_|_DEFECT|defect[-_ ]control|reference/[AB]",
    re.IGNORECASE,
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def build_kit() -> Path:
    """Frozen arm-B kit: the released 0.5.0 npm tarballs plus public guides. Built once per IK14_WORK."""
    kit = ROOT / "KIT-B"
    if (kit / "MANIFEST.json").exists():
        return kit
    (kit / "npm").mkdir(parents=True, exist_ok=True)
    dl = ROOT / "release-dl"
    dl.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["gh", "release", "download", "v0.5.0", "-R", "GauravAlbal/desmo", "-D", str(dl), "--clobber",
         *[a for t in KIT_TARBALLS + ["SHA256SUMS"] for a in ("-p", t)]],
        check=True,
    )
    sums = dict(reversed(l.split(None, 1)) for l in (dl / "SHA256SUMS").read_text().splitlines())
    manifest = {"git_head": subprocess.run(["git", "-C", str(REPO), "rev-parse", "HEAD"], capture_output=True,
                                           text=True, check=True).stdout.strip(), "files": {}}
    for name in KIT_TARBALLS:
        if sums[name].strip() != sha256(dl / name):
            sys.exit(f"stage: checksum mismatch for {name}")
        shutil.copy(dl / name, kit / "npm" / name)
        manifest["files"][f"npm/{name}"] = sha256(kit / "npm" / name)
    for rel in KIT_DOCS:
        dest = kit / ("README.md" if rel == "README.md" else rel)
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(REPO / rel, dest)
        manifest["files"][str(dest.relative_to(kit))] = sha256(dest)
    ex = kit / "examples" / "shipped"
    shutil.copytree(REPO / "examples" / "shipped", ex, dirs_exist_ok=True)
    (kit / "MANIFEST.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    return kit


def leak_scan(project: Path) -> None:
    hits = []
    for path in sorted(project.rglob("*")):
        if not path.is_file() or "KIT" in path.relative_to(project).parts or "node_modules" in path.parts:
            continue
        text = path.read_text(errors="replace")
        for m in LEAK.finditer(text):
            hits.append(f"{path.relative_to(project)}: {m.group(0)}")
    if hits:
        sys.exit("stage: leak scan failed:\n  " + "\n  ".join(hits))


def prompt_for(arm: str, stage: str) -> str:
    base = HERE / "prompts"
    parts = [(base / ("common.md" if stage == "initial" else "maintainer.md")).read_text()]
    parts.append((base / f"arm{arm}.md").read_text())
    return "\n\n".join(parts)


def main() -> None:
    if len(sys.argv) != 5 or sys.argv[2] not in ("A", "B") or sys.argv[4] not in ("initial", "change"):
        sys.exit(__doc__)
    project, arm, rep, stage = sys.argv[1:]
    src = HERE / "projects" / project
    if not (src / "brief.md").exists():
        sys.exit(f"stage: unknown project {project}")
    run = ROOT / project / arm / rep
    out = run / stage
    if out.exists():
        sys.exit(f"stage: {out} already exists (runs are not restaged; use a new IK14_WORK or rep)")
    work = out / "project"
    if stage == "initial":
        (work / "stub").mkdir(parents=True)
        shutil.copy(src / "brief.md", work / "BRIEF.md")
        shutil.copy(src / "stub" / "CONTRACT.md", work / "stub" / "CONTRACT.md")
        shutil.copy(src / "stub" / "server.mjs", work / "stub" / "server.mjs")
        if arm == "B":
            shutil.copytree(build_kit(), work / "KIT")
    else:
        frozen = run / "initial" / "project"
        if not frozen.exists():
            sys.exit("stage: no initial project to maintain; stage and complete the initial stage first")
        shutil.copytree(frozen, work, symlinks=True)  # keep the builder's installed dependencies (node_modules)
        shutil.copy(src / "change.md", work / "CHANGE.md")
    leak_scan(work)
    (out / "PROMPT.md").write_text(prompt_for(arm, stage))
    print(work)


if __name__ == "__main__":
    main()
