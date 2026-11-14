import re, shutil, sys, pathlib
G = pathlib.Path(__file__).resolve().parent
proj, arm, rep = sys.argv[1:4]
dest = G / "runs" / proj / arm / rep / "project"
assert not dest.exists(), dest
(dest / "stub").mkdir(parents=True)
src = G / "projects" / proj
shutil.copy(src / "brief.md", dest / "BRIEF.md")
shutil.copy(src / "stub" / "server.mjs", dest / "stub" / "server.mjs")
text = (src / "stub" / "CONTRACT.md").read_text()
cut = re.search(r"^## (Evaluator|Control API).*$", text, re.M)
(dest / "stub" / "CONTRACT.md").write_text(text[: cut.start()] if cut else text)
if arm == "B":
    shutil.copytree(G / "batch-001" / "kit-B", dest / "KIT")
prompt = (G / "prompts" / "common.md").read_text() + "\n" + (G / "prompts" / f"arm{arm}.md").read_text()
(dest.parent / "PROMPT.md").write_text(prompt)
print(dest)
