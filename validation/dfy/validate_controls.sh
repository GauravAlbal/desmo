#!/bin/sh
D="$(cd "$(dirname "$0")" && pwd)"
for p in N1 N2 N3; do
  for c in $(ls $D/projects/$p/controls); do
    [ -x $D/projects/$p/controls/$c/run ] || continue
    printf "%s %s: " $p $c
    PROJECT_DIR=$D/projects/$p/controls/$c STUB_DIR=$D/projects/$p/stub sh $D/projects/$p/evaluator/run.sh 2>/dev/null | /usr/bin/python3 -c "import sys,json
s=sys.stdin.read()
d=json.loads(s[s.index('{'):s.rindex('}')+1]); r=d.get('requirements',d)
print('FAIL:',[k for k,v in r.items() if v!='pass'] or 'none')" 2>&1
  done
done
echo VALDONE
