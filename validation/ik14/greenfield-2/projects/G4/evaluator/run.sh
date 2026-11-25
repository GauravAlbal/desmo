#!/bin/sh
set -eu
if [ "$#" -ne 1 ]; then
    echo "usage: sh $0 <initial|change>" >&2
    exit 2
fi
case "$1" in
    initial|change) export EVAL_STAGE=$1 ;;
    *) echo "stage must be initial or change" >&2; exit 2 ;;
esac
HERE=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
NODE=${NODE:-$HOME/.nvm/versions/node/v22.17.1/bin/node}
[ -x "$NODE" ] || NODE=$(command -v node)
: "${PROJECT_DIR:?PROJECT_DIR required}"
export STUB_DIR=${STUB_DIR:-$HERE/fixture}
exec "$NODE" "$HERE/run.mjs"
