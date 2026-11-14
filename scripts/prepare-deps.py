#!/usr/bin/env python3
"""Populate the npm cache from typescript/package-lock.json with verified tarballs.

Run on a networked machine before a restricted-network `sh scripts/accept`.
The TypeScript lane of that gate runs `npm ci --omit=optional` and cannot
fetch when the npm cache (~/.npm/_cacache) is empty.

TLS verification stays on. Bytes are refused unless they match the lockfile
sha512 integrity.

  python3 scripts/prepare-deps.py
  python3 scripts/prepare-deps.py --verify
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import shutil
import ssl
import subprocess
import sys
import tempfile
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TS = os.path.join(ROOT, "typescript")
LOCK = os.path.join(TS, "package-lock.json")


def fail(message):
    sys.exit(message)


def load_lock():
    with open(LOCK) as handle:
        return json.load(handle)


def package_row(name, meta):
    if not name:
        return None
    resolved = meta.get("resolved")
    integrity = meta.get("integrity")
    if not resolved or not integrity:
        return None
    if not str(resolved).startswith("https://"):
        fail("refusing non-https resolved URL for %s" % name)
    if not str(integrity).startswith("sha512-"):
        fail("unsupported integrity for %s: %s" % (name, integrity))
    return (name, resolved, integrity)


def packages_from(lock):
    rows = []
    for name, meta in (lock.get("packages") or {}).items():
        row = package_row(name, meta)
        if row:
            rows.append(row)
    if not rows:
        fail("no packages with resolved+integrity in %s" % LOCK)
    return rows


def sha512_integrity(path):
    digest = hashlib.sha512()
    with open(path, "rb") as handle:
        chunk = handle.read(1024 * 1024)
        while chunk:
            digest.update(chunk)
            chunk = handle.read(1024 * 1024)
    return "sha512-" + base64.b64encode(digest.digest()).decode("ascii")


def download(url, dest):
    ctx = ssl.create_default_context()
    request = urllib.request.Request(url)
    with urllib.request.urlopen(request, context=ctx, timeout=120) as resp:
        data = resp.read()
    with open(dest, "wb") as handle:
        handle.write(data)


def add_to_cache(tarball):
    subprocess.check_call(["npm", "cache", "add", tarball])


def populate():
    rows = packages_from(load_lock())
    tmp = tempfile.mkdtemp(prefix="ik-npm-")
    try:
        for name, url, integrity in rows:
            dest = os.path.join(tmp, "pkg.tgz")
            download(url, dest)
            got = sha512_integrity(dest)
            if got != integrity:
                fail("integrity mismatch for %s" % name)
            add_to_cache(dest)
            print("cached %s" % name)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def verify_offline():
    scratch = tempfile.mkdtemp(prefix="ik-npm-verify-")
    try:
        shutil.copy(os.path.join(TS, "package.json"), os.path.join(scratch, "package.json"))
        shutil.copy(LOCK, os.path.join(scratch, "package-lock.json"))
        subprocess.check_call(["npm", "ci", "--omit=optional", "--offline"], cwd=scratch)
        print("offline npm ci ok")
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--verify",
        action="store_true",
        help="run npm ci --offline in a scratch copy of typescript manifests",
    )
    args = parser.parse_args()
    if args.verify:
        verify_offline()
    else:
        populate()


if __name__ == "__main__":
    main()
