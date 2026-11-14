import shutil, pathlib
G = pathlib.Path(__file__).resolve().parent
i = 0
for p in ("G1", "G3"):
    for a in ("A", "B"):
        run = G / "runs" / p / a / "r1"
        dest = run / "change2" / "project"
        shutil.copytree(run / "project", dest, symlinks=True)
        shutil.copy(G / "projects" / p / "change.md", dest / "CHANGE.md")
        b = 43000 + 100 * i
        i += 1
        text = (G / "prompts" / "maintainer.md").read_text()
        text += f"\nIf you start any server, use only TCP ports {b}-{b+9} on 127.0.0.1 (other runs share this machine).\n"
        (run / "change2" / "PROMPT.md").write_text(text)
print("staged")
