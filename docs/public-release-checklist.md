# Public source release: owner clearance checklist (IK-12)

Nothing here publishes anything. The export tool builds a candidate; the owner decides whether and
when it becomes public. Items marked OWNER cannot be inferred or completed by tooling.

## Mechanical (tooling)

- [x] `python3 scripts/public_export.py --rev <commit> --out <empty dir>` exports an allowlisted,
  verbatim subset of one commit and writes `EXPORT_MANIFEST.json` (origin commit, allowlist digest,
  per-file sha256, exclusions, acknowledgements).
- [x] The export fails on private-workflow references, absolute user paths and secret patterns
  unless an exact (path, pattern) acknowledgement with a reason exists. Secrets cannot be
  acknowledged.
- [x] `scripts/public-verify` runs the Rust, Python, Go, TypeScript, mutant-manifest and example
  gates with no private tooling.
- [x] `.github/workflows/ci.yml` runs `scripts/public-verify` (not yet run on a public repository).

## OWNER decisions (recorded 2026-11-13)

- [x] **O-1 Disclosure and licensing authority.** Confirmed by the owner. `LICENSE-MIT` and
  `LICENSE-APACHE` are in the repository (`MIT OR Apache-2.0`, copyright Gaurav Albal).
- [x] **O-2 Branding.** The product name is `Desmo` (renamed from `Ikaros` on 2026-11-14 by the owner).
  The engine keeps the `invariant-kernel` name, the `ik` binary and `.ik.toml` files. The first public
  repository is `GauravAlbal/desmo`, a new repository with a single snapshot commit; the Go module path
  still names the old repository and moves with that step. The alpha is distributed through GitHub
  Releases, so no npm, PyPI or crates.io publication is required. A domain is optional (a landing page
  can be the README or GitHub Pages); `desmo.dev` is not registered. Trademark and name-clearance
  searches have not been done.
- [x] **O-3 Consumer references.** Generalized: no public file names the first internal consumer.
- [x] **O-3a Private-workflow references.** Generalized in 17 files; the export carries no
  acknowledged references (`acknowledged` in `export/allowlist.json` is empty).
- [x] **O-4 Security channel.** `SECURITY.md` names GitHub private vulnerability reporting. It is not
  available while the repository is private (the settings API returned 404), so it is enabled after
  the visibility change. Enabling it is a step of IK-16, not yet done.
- [x] **O-5 Support expectations.** Confirmed: best effort, latest release only.
- [x] **O-6 History.** Snapshot only: publish the single exported commit, with lineage in
  `EXPORT_MANIFEST.json`.
- [ ] **O-7 Release step.** (The owner authorized the full release on 2026-11-14.) Repository creation, visibility, tags, release assets, enabling
  private vulnerability reporting and the `LATEST_RELEASE` update are separate owner-authorized
  actions (IK-16), in that order, each needing its own authorization. Registry publication is not
  part of the alpha: the Node packages and the wheel are release assets.
- [x] **O-8 npm scope.** The `@desmo` scope on npm is not registered. The packed runtime tarball no
  longer lists `@desmo/<platform>` packages as optional dependencies, so installing from the release
  tarballs asks no registry about them. Registering the scope (an owner account action) is needed only
  if the packages are ever published to npm, and the optional dependencies would be restored then.
- [ ] **O-9 Go module tag.** The Go module path is now `github.com/GauravAlbal/desmo/go`, so it
  resolves only once the public repository exists, and a release needs a `go/vX.Y.Z` tag.

Evidence decisions recorded the same day (see `docs/alpha-readiness.md`): the IK-06 independent review
stays an agent review and the IK-13 cold run an agent run plus the owner's read, both stated plainly in
the alpha notes; IK-10 is narrowed to qualification in scratch and on disposable stores; native Linux
x64 is required: source and packaged-artifact qualification pass on native hardware (`validation/linux/RECORD-linux-x64-native.md`); the release script on Linux is still not run.

Any change to the export's files after qualification invalidates earlier byte qualification: rerun
the export and `scripts/public-verify` on the new candidate.
