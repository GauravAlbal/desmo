# Extension exercise

You are a developer who has never seen this project. In `work/` is the shipped starter analysis adapter (README.md there). `docs/analysis-adapters.md` documents the host contract. The `ik` binary is `./ik` in this directory.

Goal: turn the starter into an adapter for a DIFFERENT question: "how many nodes of the exported graph are not reachable from the initial node" (unreachable_node_count; `holds_in_scope` when the count is 0 on a closed graph, `violation` when it is greater than 0 and a witness node can be named, `inconclusive` with `bounded_paths` on an incomplete export, `unsupported` for any other question). Do not edit anything outside `work/` (docs/ and ./ik are read-only). Use only the files shipped in `work/` and `docs/`.

Deliver: an adapter whose `ik analysis conformance <abs manifest> --fixtures <abs fixtures>` exits 0 with fixtures you adapt to the new question (keep the twelve case names and the contract they test; add graphs as needed). Pin the manifest digest correctly. Report: what you changed, the exact conformance command and its observed output, every point where the README/docs were unclear or wrong, and anything you could not do. Use your shell and run things; do not claim results you did not observe.
