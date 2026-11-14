#!/bin/sh
set -u
cd "$(dirname "$0")"
NODE="${NODE:-$HOME/.nvm/versions/node/v22.17.1/bin/node}"
if [ ! -x "$NODE" ]; then
  NODE="$(command -v node)"
fi
export PATH="$(dirname "$NODE"):$PATH"
if [ -z "${PROJECT_DIR:-}" ] || [ -z "${STUB_DIR:-}" ]; then
  echo "PROJECT_DIR and STUB_DIR are required" >&2
  exit 2
fi
TAP="$(mktemp)"
ERR="$(mktemp)"
"$NODE" --test --test-concurrency=1 --test-timeout=20000 --test-reporter=tap --test-force-exit eval.test.mjs >"$TAP" 2>"$ERR"
"$NODE" summarize.mjs "$TAP"
status=$?
if [ "$status" -ne 0 ]; then
  cat "$ERR" >&2
fi
rm -f "$TAP" "$ERR"
exit "$status"
