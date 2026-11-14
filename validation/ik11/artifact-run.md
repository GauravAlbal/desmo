# IK-11 artifact run (executed, macOS arm64 only)

## Lineage

- Origin (accepted private commit): `adf071afc11bee405ee376a6426051d64bc55229`
- Public-source candidate: `python3 scripts/public_export.py --rev adf071afc11b... --out /tmp/ik-cand` (270 files, allowlisted, verbatim).
- Local stand-in for the owner-created public commit: `3ca83dd807e01fd467d2a09c966577aadea59d2b` (`git init` of that export in an empty
  directory). The release manifest's `source_sha` is this stand-in. **It is not a published commit**; the owner's real public commit will have a
  different SHA and everything below must be re-run on it.

## Release build and verification (`sh scripts/release v0.5.0`, `env -i`, no checkout of the origin repo, no private tooling)

Exit 0. It built `ik` in release mode, ran the Python suite, packed the archive/wheel/sdist, wrote the manifest, and ran
`python/tests/test_release_artifacts.py`, which installs the wheel offline into a fresh venv, extracts `ik` from the archive and runs the Python and
real Go clients against that extracted binary.

| Artifact | sha256 |
|---|---|
| `ik-darwin-arm64.tar.gz` | `120906e0bec5e6a03c566486b3930fe90c14dec7c86f7a30f99ee2f325a26916` |
| `invariant_kernel-0.5.0-py3-none-any.whl` | `2442240916734a22d33d9e73d40a5041e0962f6efe8fc12995badff8a6790625` |
| `invariant_kernel-0.5.0.tar.gz` | `0d5646d02636c8a1c201d5e4a7bb875fbcf8e2fa31b2dacb41f49cacf58e0a37` |
| `ikaros-runtime-0.5.0.tgz` (npm) | `ef37a1f52a1db8e5e1e8c8b97cf9ccfb0f77f215a1c761b0b6e054ad6515e710` |
| `ikaros-darwin-arm64-0.5.0.tgz` (npm) | `df9acb510321f16d231de8ce9a1332663845cf60b7d5a6c88592716677a8571e` |

Manifest: version `0.5.0`, protocol major 1, store schema 4. The npm tarball digests were identical across the two Node runs below.

## Installed-artifact qualification (`scripts/qualify_artifacts.py`)

Empty directories outside any checkout, scrubbed environment (no `IK_BINARY`, no `ik` on `PATH`, no Rust toolchain). Run under Node
**25.2.1** and **22.17.1** (both: all checks true):

- the npm runtime package resolved its binary from the installed platform package, and that binary's sha256 equals the archive's `ik`;
- `ikaros demo stale-worker` printed the stale refusal and "Canonical state was not changed";
- `ikaros codegen` on the bundled machine emitted types; a generated job app (`ikaros init`) installed from the tarballs and its `npm test` passed;
- operation lookup through the installed package: committed command, `lookupOperation` returned `committed_exact`, exact repeat returned a `HistoricalResult`;
- the wheel, installed offline, reported `not_recorded` for an unsent operation through `lookup_operation`.

Negative controls (all failed loudly as intended): runtime package without a platform package and without a binary (exit 1, names the remedy);
runtime package with its `assets/` removed (`ENOENT`); a tampered tarball has a different digest.

## Previous-release store compatibility (real `v0.4.1` artifacts)

A store written by the published `v0.4.1` binary and client (schema 3) was opened by the `0.5.0` binary: it read the old aggregate and `verify`
passed, and the store's schema became `4`. The `v0.4.1` binary then refused the migrated store
(`IK_PERSISTED_SCHEMA_UNKNOWN ... unknown persisted schema version 4`), and a pre-migration backup was still readable by `v0.4.1`.
Binary rollback across the schema change therefore requires restoring a backup; the docs say so.

## Not done / not claimed

- **Linux x64 (and every other target) was not qualified.** No runner was available. The `darwin-x64`, `linux-arm64` and `linux-x64` npm platform
  packages exist only as skeletons.
- Node 20 and 21 were not tested; `@ikaros/runtime` declares `node >=22` because the code uses `Promise.withResolvers`.
- Nothing was published: no registry, repository, tag, or `LATEST_RELEASE` change.
- No LICENSE files exist in the candidate (owner decision O-1); the packages declare `MIT OR Apache-2.0` without bundled license text.
- Not run on a hosted CI runner. The Python wheel intentionally does not bundle `ik`, so the Python path needs `IK_BINARY`; only the Node path
  resolves the binary itself.
- The Go client was exercised against the extracted binary by the release verification, not through a published module version.
