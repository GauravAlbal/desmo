import pathlib, shutil, sys
root = pathlib.Path(__file__).resolve().parents[2]
source = pathlib.Path(__file__).resolve().parent
project, arm, rep = sys.argv[1:4]
stage = sys.argv[4] if len(sys.argv) > 4 else 'initial'
run = root / 'runs' / project / arm / rep
if stage == 'change':
    src = run / 'project'
    dest = run / 'change' / 'project'
    if not src.is_dir(): raise SystemExit(f'missing frozen initial workspace: {src}')
    if dest.exists(): raise SystemExit(f'already exists: {dest}')
    shutil.copytree(src, dest, symlinks=True)
    shutil.copy2(source / 'change.md', dest / 'CHANGE.md')
    print(dest)
else:
    if stage != 'initial': raise SystemExit('stage must be initial or change')
    dest = run / 'project'
    if dest.exists(): raise SystemExit(f'already exists: {dest}')
    (dest / 'stub').mkdir(parents=True)
    shutil.copy2(source / 'brief.md', dest / 'BRIEF.md')
    shutil.copy2(source / 'stub' / 'server.mjs', dest / 'stub' / 'server.mjs')
    shutil.copy2(source / 'stub' / 'CONTRACT.md', dest / 'stub' / 'CONTRACT.md')
    print(dest)
