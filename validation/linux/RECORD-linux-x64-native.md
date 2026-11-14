# Linux x64 public-verify run (native x86_64 hardware, WSL2 guest)

Scope: the full `scripts/public-verify` run and a packaged-artifact qualification on a Linux x86_64 machine, both from the public export of origin commit `034700c9c8098c02fbed7880cde27b5ded850be6`.

## Machine

| Item | Value |
|---|---|
| CPU | Intel Core i7-9700K, 6 cores, native x86_64 (not emulated) |
| Environment | WSL2 guest, kernel 6.18.33.2-microsoft-standard-WSL2 |
| Userland | Ubuntu 24.04.4 LTS, x86_64 |
| Rust | cargo and rustc 1.100.0 (user-local install) |
| Go | go1.27.4 linux/amd64 |
| Node | v22.23.5 |
| Python | 3.12.3, pytest 9.2.0 (virtualenv) |
| Java (TLC adapter tests) | OpenJDK 21.0.13 with the pinned TLC jar, sha256 `936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88` |

Run date: 2026-11-14 (UTC). The export had 1156 files and 0 acknowledged references.

## Command

```
IK_TLC_TOOLCHAIN=<linux toolchain file> sh scripts/public-verify
```

`adapters/tlc/toolchain.json` pins a macOS Homebrew Java path. On Linux the TLC adapter tests fail unless `IK_TLC_TOOLCHAIN` names a toolchain file with a Linux Java path and the same jar digest and version string. The run used such a file.

## Result

`scripts/public-verify` exited 0.

| Step | Result |
|---|---|
| `cargo test --all-targets --all-features` | 770 passed, 0 failed, 1 ignored |
| `validation/ik06/qualify.py --check` and `--self-test` | passed |
| pytest `python/tests` | 162 passed, 1 skipped |
| `go test ./...` | passed |
| TypeScript `npm ci`, `tsc`, `npm test` | 79 of 79 passed |
| `examples/publish-approval/scenarios.py` | passed |

For comparison, the same export on macOS arm64 gave 770 passed, 0 failed, 1 ignored (Rust), 161 passed, 2 skipped (Python) and 79 of 79 (TypeScript); Go passed.

## Packaged-artifact qualification

Same machine and export. Steps, in order:

```
cargo build --release --locked --bin ik
tar -C target/release -czf dist-linux/ik-linux-x64.tar.gz ik
(cd python && python3 -m build --no-isolation --outdir dist-linux)
(cd typescript && npm ci --omit=optional && npx tsc -p .)
python3 scripts/qualify_artifacts.py --dist dist-linux --out qualify-linux-x64-native.json --node <node 22>
```

`scripts/release` itself was not run: it requires a clean git tree, which an export directory is not, so the same build and archive steps were run by hand. The release manifest and `verify-release.py` were therefore not exercised.

| Item | Value |
|---|---|
| `ik` release binary | ELF 64-bit x86-64, sha256 `c5432567d631d02fff385c87e5a5d5c09ff721355d70b06de6f6339ca9f0d8e0` |
| `ik-linux-x64.tar.gz` | sha256 `70d4c89afc175ef99345373fdf011c1da34e5f978c15ca37e81a37691c20ab90` |
| `invariant_kernel-0.5.0-py3-none-any.whl` | sha256 `bc04e18d983968899165922b4d01c1a170693a5bcce34d1c95b8239678a3c7c6` |
| `invariant_kernel-0.5.0.tar.gz` | sha256 `54bc3596360fad12851ffc800c5bd017d7507420aa2b43a4c98316cb4d637a98` |
| `ikaros-runtime-0.5.0.tgz` | sha256 `94cef87774e6263c977a3ea3eed2acbef0c196f6511b4b4c3e5d5fb4458a94e4` |
| `ikaros-linux-x64-0.5.0.tgz` | sha256 `fb356ee0d3056c82c5e7334c7cdf2c0335f1628cf3aabe3912b3d40a52ed45f2` |

`qualify_artifacts.py` exited 0 (`qualify-linux-x64-native.json` in this directory). The installed platform package resolved the binary and its digest equals the extracted binary's; the demo names a stale refusal; codegen emits types; the operation probe returned `running`, `committed_exact` and `HistoricalResult`; the generated app's tests pass; the wheel installs offline into a fresh virtual environment and a lookup of an unsent operation returns `not_recorded`; and the three negative controls fire (no platform package and no binary exits 1 with a remedy, a missing asset exits 1 with ENOENT, a tampered tarball has a different digest).

## Re-run for the Desmo-named candidate

The product was renamed (Ikaros to Desmo) and the packed runtime package stopped listing platform packages as optional dependencies after the run above, so the packaged bytes changed. The numbers below are for public export of origin commit `bbf5886f2bffcd9cc8a0c6894ed2aec4959149ba` (1161 files, 0 acknowledged references), run on macOS arm64 and on the same Linux machine. The same checks also passed at the earlier rename commit `2ad8932`, before the dependency change.

| Step | macOS arm64 | Linux x64 |
|---|---|---|
| `scripts/public-verify` | exit 0 | exit 0 on a repeat run (see below) |
| Rust | 770 passed, 0 failed, 1 ignored | 770 passed, 0 failed, 1 ignored |
| Python | 161 passed, 2 skipped | 162 passed, 1 skipped |
| Go | passed | passed |
| TypeScript | 79 of 79 | 79 of 79 |
| `qualify_artifacts.py` | exit 0, three negative controls fire (`validation/macos/qualify-darwin-arm64-desmo.json`) | exit 0, three negative controls fire (`qualify-linux-x64-desmo.json`) |

**A failed first Linux run, twice.** On a freshly extracted export, the first Linux `public-verify` run stopped with exit 101 at `hegel_kernel_model_runs`: `FailedHealthCheck: TooSlow` (1 valid input after 46 s; threshold 30 s). The same happened at `2ad8932`, and did not happen on the first run at `034700c`. After the failure at `bbf5886`, four immediate standalone runs of that test passed in 3 to 5 seconds each, and a full `public-verify` repeat passed every step. The cause was not found. It is a timing health check in the property-test engine, it may be tied to the first run after a fresh build on this machine, and it can recur, including in a Linux CI job. The packaged-artifact qualification in the first run executed after the failure (the wrapper script did not stop) and is from the same export.

Artifact digests (sha256):

| Item | Value |
|---|---|
| Linux `ik` release binary | `64900f7ea36d09bb5990b82a4f9b4b1f3db8ab660a6a5493d3629d6b5daf479d` |
| macOS `ik` release binary | `ec8d52b4c0b895178e412b3586fcdb711c4667e0fa4ec256fd423ab6d6aa6344` |
| `desmo-runtime-0.5.0.tgz` (identical on both hosts) | `d8c470c4b681da72a92e1c8b28c5e40fa094a3a9b613fe87bdd830b301cc9efb` |
| `desmo-linux-x64-0.5.0.tgz` | `7fe2a1721516e3da7bacace5d76bf611955cdb00cbca8853810d017ae3df45cc` |
| `desmo-darwin-arm64-0.5.0.tgz` | `abe8b8177b310302b7bda41ab84626820b0ff070d6596620e1f51aa071230520` |

The release `ik` binary and the npm tarballs reproduced with the same digests in the `2ad8932` and `bbf5886` runs wherever their inputs were unchanged (the runtime tarball changed, as described above). The `ik-<target>.tar.gz` archive, the Python wheel and the sdist differ from build to build (different digests for the same source), so a release must ship those files from one named build. `scripts/release`, release-manifest verification and previous-release store compatibility were again not run.

## Defect found and fixed by this run

A first run at `12b7e28` failed four Python tests (three in `test_reducer.py`, one in `test_worker_walls.py`). Linux `ps` rejects `ps eww -ax`, so the descendant listing was empty and detached descendants were never reaped. The scope argument is now `ax` on Linux and `-ax` elsewhere, in both `src/external.rs` and `python/src/invariant_kernel/transport.py`. The run above is at the fixed commit.

## Not covered

- The release script and release manifest verification on Linux (see above), and previous-release store compatibility (`--previous-dist` was not supplied).
- Publication: nothing was published; the tarballs exist only on the test machine.
- Contention and timing numbers (`validation/ik07/`) were not measured on Linux.
- One machine, one distribution, one WSL2 kernel. Other glibc or musl systems, bare-metal kernels and other `ps` implementations are untested.
- Rust, Go and Node toolchains were newer than any earlier recorded run; no older toolchain was tried.
