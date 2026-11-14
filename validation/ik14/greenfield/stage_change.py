import shutil, sys, pathlib
G = pathlib.Path(__file__).resolve().parent
proj, arm, rep = sys.argv[1:4]
run = G / "runs" / proj / arm / rep
dest = run / "change" / "project"
assert not dest.exists()
shutil.copytree(run / "project", dest, symlinks=True)
shutil.copy(G / "projects" / proj / "change.md", dest / "CHANGE.md")
(run / "change" / "PROMPT.md").write_text((G / "prompts" / "maintainer.md").read_text())
print(dest)
