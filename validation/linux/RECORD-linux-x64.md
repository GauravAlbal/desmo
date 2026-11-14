# Linux packaged-artifact run (x86_64 container, emulated)

Scope: packaged bytes for the linux-x64 host target, built from the public export of origin commit `35dc9bdd51a2b0aa02d579bc45830ca681f7c689`. The build ran on Linux x86_64 (Debian 12 bookworm, glibc 2.36, rustc 1.97.1) inside an official `rust:1.97-bookworm` container with `--platform linux/amd64`. That is emulated x86_64 on the same Docker Desktop VM as the host, not a separate machine and not native linux-x64 hardware.

The export was git-initialized and committed before the build. Crate sources were the earlier offline vendor tree (112 registry crates, matching `Cargo.lock`); `cargo` used `[source.crates-io] replace-with = "vendored-sources"`.

## Image digests

| Image | Platform | Repo digest |
|---|---|---|
| `rust:1.97-bookworm` | linux/amd64 | `sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97` |
| `python:3.11` | linux/amd64 | `sha256:731670a425deabbc48d39f0ee3115cfa665b2a4002c19939ad3e88b2832b4549` |
| `node:22.23.2-bookworm` | linux/amd64 | `sha256:dd5847a04b0deee391fa145f1f4c6d214196668b6bcc7988ebed67249f226844` |

Confirmed inside the rust image: `uname -m` = `x86_64`; `rustc -vV` host = `x86_64-unknown-linux-gnu`. Confirmed inside the Python image: `platform.machine()` = `x86_64`, Python 3.11.18. Node 22.23.2 (`x64`) was copied out of the node image into the Python container because the rust image has no `ensurepip` and the Python image has no Node.

## Commands

Public export of `HEAD`, then in the rust image:

```
cargo build --release --locked --offline --bin ik
```

Finished `release` profile in 1m 54s. Binary: ELF 64-bit LSB pie executable, x86-64, sha256 `5396cb6c135e206144c0eb077ab533d3bdd7c8482d40b8d1da2b593d88170c6c`. Packed as `ik-linux-x64.tar.gz`.

Cheap source sanity check, same image, same vendor, release profile:

```
cargo test --release --offline --test text_in --test genesis_validity
```

`tests/genesis_validity.rs`: 5 passed, 0 failed. `tests/text_in.rs`: 15 passed, 0 failed. The full Rust suite was not run.

TypeScript `dist` was the already-compiled runtime from the same origin tree. Pack and qualify ran in the Python 3.11 image with `HOME` set to a writable directory and Node 22.23.2 on `PATH`:

```
python3 scripts/pack_npm.py --binary <extracted-ik> --target linux-x64 --out dist/npm
python3 scripts/qualify_artifacts.py --dist dist --out qualify-linux-x64.json --node <node>
```

The Python wheel is `invariant_kernel-0.5.0-py3-none-any.whl` from the darwin-arm64 pack (`py3-none-any`, platform-independent).

Shipped-example `ik check` (and `ik identity`) ran with the extracted linux-x64 binary against `examples/shipped/loan_checkout.ik.toml` and `examples/shipped/tagged_item.ik.toml`. The same commands ran on the darwin-arm64 packed binary. See `spec_hash-linux-x64.md`.

## Results

`scripts/qualify_artifacts.py` on the linux-x64 host target (`qualify-linux-x64.json`): the installed platform package resolves the binary and its digest equals the extracted binary's; the demo and codegen checks pass; the operation probe reports `committed_exact` and `HistoricalResult`; the generated application's tests pass; the Python wheel check passes; the three negative controls fire (no platform package, missing asset, tampered tarball digest). Not run here: previous-release store compatibility (no previous linux-x64 release exists).

| Run | Result |
|---|---|
| `cargo build --release --locked --offline --bin ik` | ELF x86-64, digest prefix `5396cb6c` |
| `cargo test --release --offline --test text_in --test genesis_validity` | 20 passed, 0 failed |
| `pack_npm.py --target linux-x64` | runtime + `@ikaros/linux-x64` tarballs; binary digest `5396cb6c…` |
| `qualify_artifacts.py` linux-x64 | pass; three negative controls fire |
| `ik check` shipped examples vs darwin-arm64 packed `ik` | `spec_hash` identical for both examples |

## What is not covered

- Native linux-x64 hardware. This is emulated x86_64 (`docker run --platform linux/amd64`) on the same Docker Desktop VM.
- The full Rust test suite.
- Contention and execution-envelope measurements (those remain darwin-arm64 only).
- Shipped skill tests (TypeScript and Python lanes) on Linux.
- A Linux release built by the release script itself (the script needs a clean git tree and Python build tooling the images lack).
- Previous-release store compatibility on linux-x64.
- linux-arm64 (already recorded separately).
