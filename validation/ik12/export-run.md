# IK-12 export run (executed, macOS arm64)

Candidate: `python3 scripts/public_export.py --rev 835ed478ca8b6683729e68aa20b9df05f662517e --out /tmp/ik-export`

| Fact | Value |
|---|---|
| Origin commit | `835ed478ca8b6683729e68aa20b9df05f662517e` |
| `export/allowlist.json` sha256 | `77dacafe8013bfd8b9339a93ebbbe5ca447e14170776580839dc988516edf229` |
| `EXPORT_MANIFEST.json` sha256 | `eb0a5070aaf7ec78d1b0b97840fc290e181e531c2ce47a8f36d72940aa1f2fe1` |
| Files exported / excluded | 257 / 509 (490 of the exclusions are generated `mutants.out*` files) |
| Acknowledged private-term references | 24 (all `private-consumer`, host-neutrality statements, or ignore rules; each with a reason) |

Verification of the exported tree, in an empty directory with no `.git`, with `env -i` and a PATH
that contains no private tools (cargo, go, node, python3 only):

- `sh scripts/public-verify` exited **0**: Rust 51 test binaries ok (`cargo test --all-targets
  --all-features`), `qualify.py --check` and `--self-test`, pytest **124 passed, 2 skipped**
  (skips are pre-existing platform-conditional tests, not counted as evidence), Go ok, TypeScript
  **38 pass / 0 fail**, and the approval scenario runner.
- Negative control: removing `machines/effect_probe.ik.toml` from the exported tree makes
  `cargo test --test vfy_adopt` fail with `cannot read .../machines/effect_probe.ik.toml`, i.e. an
  export that drops a required fixture is caught by the public gate.
- Negative control: planted private words and secrets are blocked by `python/tests/test_public_export.py`.

Not done / not claimed:

- Not run on Linux or on a hosted CI runner; `.github/workflows/ci.yml` is unexecuted.
- No owner decision is made here (`docs/public-release-checklist.md`, O-1..O-7 are open). The tree has
  no LICENSE text files.
- This verifies the exported source bytes, not packed artifacts (IK-11).
- Any later change to a file in the export invalidates this qualification for the new bytes.
