#!/bin/sh
set -eu
HERE=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
NODE=${NODE:-$HOME/.nvm/versions/node/v22.17.1/bin/node}
[ -x "$NODE" ] || NODE=$(command -v node)
STAGE=${1:-initial}
case "$STAGE" in initial|change) ;; *) printf 'stage must be initial or change\n' >&2; exit 2 ;; esac
: "${PROJECT_DIR:?PROJECT_DIR required}"
export PATH="$(dirname "$NODE"):$PATH"
exec "$NODE" "$HERE/eval.mjs" "$PROJECT_DIR" "$HERE/fixture" all "$STAGE"
