# batch-001 (DFY cold qualification) frozen inputs

Candidate: Ikaros 0.5.0 host-target (macOS arm64), built from private commit 04ea262e925c263efad6b2b381bac99bb13c7b97
via the public-source export (493 files), stand-in commit e6a3dd6e4f1959909d3757fffb8c0ffc009c2cc6.
Installed-artifact qualification of these files passed (`qualify.log`, `qualify.json`).

| File | sha256 |
|---|---|
| artifacts/ik-darwin-arm64.tar.gz | 7d0fdf3af2d35ab738ef0f9e2bb6c73241c344a467272282c3b2bc1225b5527b |
| artifacts/invariant_kernel-0.5.0-py3-none-any.whl | b15978938c6a647c965ace4197d39b4bf75c2eccc6b17e483b45d592c1e46e87 |
| artifacts/invariant_kernel-0.5.0.tar.gz | 6cdc774e2c4c23bc0649fc0b6b6ed585154db1a0b4a8f24582aa1634885fff39 |
| kit/npm/ikaros-runtime-0.5.0.tgz | d8bb5c0db30f8d0a41a03a49dc126e181d9c677354555fb18a73c4fe8f5b9d51 |
| kit/npm/ikaros-darwin-arm64-0.5.0.tgz | 68e3dfc99ffafe4d6eaba8aeeb981d1f64c8d6eec463db5a45b6e7c98cef4d77 |

Builder kits: TypeScript families receive the two npm tarballs; the Python family receives the wheel and the packed
`ik` binary. All receive the shipped README/docs/examples and the installed skill (`ikaros skill install`, run by
the harness with the candidate's own CLI into `.agents/skills/ikaros/`). Agents: type `reviewer`, one fresh context
per run; the underlying model and reasoning settings are not exposed to the orchestrator (unknown). Wall-clock caps
declared before dispatch: build 45 min, change 30 min, diagnose 25 min. Parallel dispatch within a stage; arm
counterbalancing does not apply (single arm).
Port ranges per run are disjoint (46000-48999).
Known limitations: the Python family's wheel locates the binary only via IK_BINARY.
