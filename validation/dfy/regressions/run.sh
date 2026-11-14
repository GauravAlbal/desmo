#!/bin/sh
# Run all seams against IK_BINARY. Exit non-zero on mismatch.
set -eu
cd "$(dirname "$0")"
exec python3 ./run.py
