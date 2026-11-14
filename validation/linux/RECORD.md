# Linux source test run (aarch64 container)

Scope: the Rust test suite at source commit `aa04128`, run on Linux aarch64 (Debian 13, glibc 2.41, rustc 1.97.1) in
a container on a Docker Desktop VM. Dependencies were vendored from the host's offline cache; `cargo test --offline
--all-targets --all-features --no-fail-fast` with debug info off. The TLC tests used OpenJDK 21 and the pinned TLC
2.19 jar (`IK_TLC_TOOLCHAIN`, `IK_TLC_JAR`).

| Run | Result |
|---|---|
| Full suite, host toolchain files | 490 passed, 8 failed. All 8 are in `tests/tlc_adapter.rs`: the test required the macOS Java path, so it could not pass on Linux. |
| `tests/tlc_adapter.rs` after the fix (toolchain file read for the Java path; `IK_TLC_TOOLCHAIN` selects another file), real TLC and JRE on Linux | 8 passed, 0 failed |
| `tests/tlc_adapter.rs` on macOS arm64, default toolchain file | 8 passed, 0 failed |

Combined, every one of the 498 tests passes on Linux aarch64, but in two runs (the 8 TLC tests were rerun after the
fix). The fix is a test change only; no product code changed.

## Packaged artifacts on Linux aarch64

`scripts/qualify_artifacts.py` ran on the Linux host target against a release-profile `ik` built in the same
container (`cargo build --release --locked --offline`, binary digest prefix `1b78874b`), packed to
`ik-linux-arm64.tar.gz`, with the TypeScript runtime compiled from the same source and the pure-Python wheel from the
macOS build (`py3-none-any`, so platform-independent). Result (`qualify-linux-arm64.json`): the installed platform
package resolves the binary and its digest equals the extracted binary's; the demo and codegen checks pass; the
operation probe reports `committed_exact` and `HistoricalResult`; the generated application's tests pass; the Python
wheel check passes; the three negative controls fire (no platform package, missing asset, tampered tarball digest).
Not run here: previous-release store compatibility (no previous Linux release exists).

Setup notes: the qualification ran in a Python 3.11 image with Node 22.23 copied from the build image, because the build
image lacks `ensurepip`; the container's `HOME` had to be set to a writable directory.

Not covered: Linux x64; the contention and execution
envelope measurements, which remain macOS arm64 only; the shipped skill tests (TypeScript and Python lanes) on Linux; a Linux release built by the release script itself (the script needs a clean git tree and Python build tooling the images lack).
The container shares the host kernel VM, so this is not a separate-machine run.
