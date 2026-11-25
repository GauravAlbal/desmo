#!/bin/sh
set -eu
ROOT=$(CDPATH= cd "$(dirname "$0")" && pwd)
: "${PROJECT_DIR:?PROJECT_DIR is required}"
: "${STUB_DIR:=$(CDPATH= cd "$ROOT/../stub" && pwd)}"
export PROJECT_DIR STUB_DIR
SCOPE=${1:-${EVAL_SCOPE:-initial}}
case "$SCOPE" in
  initial|change) ;;
  *) echo '{"error":"argument must be initial or change"}'; exit 2 ;;
esac
exec node "$ROOT/evaluate.mjs" "$SCOPE"
