#!/usr/bin/env python3
"""Write adapters/smt/toolchain.json entries from real executables, pinning SHA-256.

Usage: pin-smt-toolchain.py OUT_DIR --z3 /path/to/z3 [--cvc5 /path/to/cvc5]

OUT_DIR is a private directory you point IK_SMT_ADAPTER at; the shipped template is
never rewritten. Offline only: nothing is downloaded. The cvc5 pip wheel has no CLI;
pass `adapters/smt/cvc5_stdin.py` run by an interpreter that can import cvc5 as a
wrapper executable you control.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess
import sys


def entry(path):
    p = pathlib.Path(path).resolve()
    if not p.is_file():
        sys.exit(f"not a file: {p}")
    out = subprocess.run([str(p), "--version"], capture_output=True, text=True, timeout=30)
    version = (out.stdout or out.stderr).strip().splitlines()[0] if (out.stdout or out.stderr) else ""
    if out.returncode != 0 or not version:
        sys.exit(f"{p} did not answer --version")
    return str(p), hashlib.sha256(p.read_bytes()).hexdigest(), version


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out_dir")
    ap.add_argument("--z3")
    ap.add_argument("--cvc5")
    args = ap.parse_args()
    if not (args.z3 or args.cvc5):
        sys.exit("give at least one of --z3 / --cvc5")
    doc = {}
    for name in ("z3", "cvc5"):
        path = getattr(args, name)
        if path:
            doc[name], doc[f"{name}_sha256"], doc[f"{name}_version"] = entry(path)
    out = pathlib.Path(args.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    (out / "toolchain.json").write_text(json.dumps(doc, indent=2) + "\n")
    print(f"pinned {', '.join(k for k in ('z3', 'cvc5') if k in doc)} in {out / 'toolchain.json'}")


main()
