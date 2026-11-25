"""Validate the pinned runtime and discover local film tools."""
import importlib.metadata
import os
import shutil
import subprocess
from pathlib import Path

_EXPECTED_PROTOCOL = "1.0"


def default_binary():
    return os.environ.get("IK_BINARY") or str(
        Path(__file__).resolve().parents[3] / "target" / "debug" / "ik"
    )


def parse_binary_identity(output):
    fields = output.strip().split()
    if len(fields) != 4 or fields[0] != "invariant-kernel" or fields[2] != "protocol":
        raise RuntimeError("IK_BINARY --version must report 'invariant-kernel VERSION protocol PROTOCOL'")
    return fields[1], fields[3]


def verify_runtime(binary, expected_version, *, package_version=None, version_output=None):
    if package_version is None:
        try:
            package_version = importlib.metadata.version("invariant-kernel")
        except importlib.metadata.PackageNotFoundError as exc:
            raise RuntimeError("required reproduction package invariant-kernel is not installed") from exc
    if package_version != expected_version:
        raise RuntimeError(
            f"reproduction package invariant-kernel=={package_version}; "
            f"trace requires invariant-kernel=={expected_version}"
        )
    if version_output is None:
        try:
            result = subprocess.run([str(binary), "--version"], check=True, capture_output=True, text=True)
        except (OSError, subprocess.CalledProcessError) as exc:
            stderr = getattr(exc, "stderr", "") or str(exc)
            raise RuntimeError(f"cannot verify IK_BINARY {binary!s} --version: {stderr.strip()}") from exc
        version_output = result.stdout + result.stderr
    binary_version, protocol = parse_binary_identity(version_output)
    if binary_version != expected_version:
        raise RuntimeError(f"IK_BINARY version {binary_version}; trace requires {expected_version}")
    if protocol != _EXPECTED_PROTOCOL:
        raise RuntimeError(f"IK_BINARY protocol {protocol}; required protocol {_EXPECTED_PROTOCOL}")


def resolve_tool(env_name, program, default_paths):
    override = os.environ.get(env_name)
    if override:
        candidate = shutil.which(override) or override
        candidates = [candidate]
    else:
        candidates = [shutil.which(program), *default_paths]
    for candidate in candidates:
        if candidate and Path(candidate).is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    raise RuntimeError(f"required {program} executable not found; set {env_name} to its path")
