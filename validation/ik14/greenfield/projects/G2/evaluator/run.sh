#!/bin/sh
set -u
HERE=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
TAP=$(mktemp)
node --test --test-reporter=tap "$HERE/accept.test.mjs" >"$TAP" 2>"$TAP.err" || true
node "$HERE/summarize.mjs" "$TAP"
rc=$?
rm -f "$TAP" "$TAP.err"
exit "$rc"
