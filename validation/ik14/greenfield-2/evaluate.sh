#!/bin/sh
# Evaluate one staged submission: sh validation/ik14/greenfield-2/evaluate.sh <G4|G5|G6> <A|B> <r1|r2> <initial|change>
# Runs the project's protected evaluator against <IK14_WORK>/<project>/<arm>/<rep>/<stage>/project and stores the
# JSON result next to it. Exit status is the evaluator's (nonzero when any requirement fails).
set -u
[ "$#" -eq 4 ] || { echo "usage: $0 <G4|G5|G6> <A|B> <r1|r2> <initial|change>" >&2; exit 2; }
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ROOT=${IK14_WORK:-/tmp/ik14-runs}
PROJECT_DIR="$ROOT/$1/$2/$3/$4/project"
[ -d "$PROJECT_DIR" ] || { echo "no staged project at $PROJECT_DIR" >&2; exit 2; }
OUT="$ROOT/$1/$2/$3/results"
mkdir -p "$OUT"
PROJECT_DIR="$PROJECT_DIR" sh "$HERE/projects/$1/evaluator/run.sh" "$4" > "$OUT/$4.json"
status=$?
cat "$OUT/$4.json"
exit "$status"
