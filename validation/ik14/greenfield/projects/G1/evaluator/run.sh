#!/bin/sh
set -eu
ROOT=$(CDPATH= cd "$(dirname "$0")" && pwd)
if [ -z "${PROJECT_DIR:-}" ]; then
  echo '{"error":"PROJECT_DIR unset"}'
  exit 1
fi
if [ -z "${STUB_DIR:-}" ]; then
  STUB_DIR=$(CDPATH= cd "$ROOT/../stub" && pwd)
fi
EVAL_SCOPE="${EVAL_SCOPE:-brief}"
export PROJECT_DIR STUB_DIR EVAL_SCOPE
exec node --test --test-concurrency=1 --test-reporter="$ROOT/reporter.mjs" --test-force-exit "$ROOT/acceptance.test.mjs"
