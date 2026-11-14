#!/bin/sh
# usage: evaluate.sh <proj> <arm> <rep> <stage: project|change/project>
G=$(cd "$(dirname "$0")" && pwd)
PROJECT_DIR=$G/runs/$1/$2/$3/$4 STUB_DIR=$G/projects/$1/stub sh $G/projects/$1/evaluator/run.sh 2>&1
