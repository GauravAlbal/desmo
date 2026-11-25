# IK-14 greenfield product observations

Verified against the checkout's current `ik` binary (`invariant-kernel 0.5.0 protocol 1.0`), built from source with a dedicated cargo target directory. Reproductions use shipped `machines/effect_probe.ik.toml` or the tiny `/tmp/ik-observe.ik.toml` machine; they do not depend on the absent builder fixtures. No Rust behavior changed.

## Findings by reported observation

### (a) EffectOutcomeV1 missing from TypeScript guide — documentation gap repaired

`src/effect.rs:36-45` defines the complete six-field payload: `effect_id`, `aggregate_id`, `planned_generation`, `payload_digest`, `idempotency_key`, `outcome_digest`. `typescript/src/client.ts:341-350` constructs exactly those fields; `:481-510` shows `record()` accepts the complete object and `recordEffectOutcome()` builds it from the pending intent and caller digest. `docs/typescript.md` now documents the shape, field binding, responsibility boundary, and a `record()` example.

Command run from `typescript/`: `IK_BINARY=<built 0.5.0 binary> npm test -- --test-name-pattern='outcome is bound'`. Observed: `tests 79`, `pass 79`, `fail 0`; existing effect tests exercise intent binding and record operations against the selected `ik` binary. The pattern followed the package test file list, so Node ran the full package test list rather than filtering to one test.

### (b) Corpus-less check — existing guide accurate; Greenfield cross-link added

Ran `ik check examples/publish-approval/machines/publish_approval_v3.ik.toml` using the 0.5.0 binary. Observed `VERDICT / REFUSED`, uncovered mandatory obligations, and `NEXT COMMAND` with corpus initialization; exit status 1. Ran `ik corpus init examples/publish-approval/machines/publish_approval_v3.ik.toml --out <temp>/cases.json`; exit status 0. This is a normal analysis refusal for missing evidence, not inability to run check. `docs/ik-check.md:114-126` already states this; `docs/greenfield.md` now links to it.

### (c) Facts cannot be deleted; empty sentinel — verified and documented

Tiny machine `/tmp/ik-observe.ik.toml` has replace-policy string fact `approval`; `forget` assigns `""` using the documented `then` string-literal encoding. Ran `node /tmp/ik-fact-repro.mjs` using the current binary. Observed create state `{"state":"idle"}`, approve state `{"approval":"yes","state":"ready"}`, and forget state `{"approval":"","state":"ready"}`. Empty string clears the value semantically but the fact remains present with an empty value; it is a sentinel, not deletion. `docs/machine-language.md:58-63` already documents the encoding; `docs/greenfield.md` links to it.

### (d) Create does not set facts — verified and documented

Same tiny-machine run: the create request has `aggregate_id` and `machine` identity, no fact field; returned state was `{"state":"idle"}`. The subsequent `approve` machine command sets the fact. `src/cli.rs:679-690` shows create requires aggregate id and machine and creates initial state; `:693-715` shows `create_with_command` requires a command. `docs/python.md:28` already says bare create plus required initialize is not atomic; Greenfield now links lifecycle guidance and explains that facts are initialized through a machine command, with `create_with` for atomic admission.

### (e) Edited machine identity does not rebind existing aggregates — verified and documented

Created an aggregate in `/tmp/ik-facts.db` under `/tmp/ik-observe.ik.toml`, then ran `node /tmp/ik-mismatch.mjs`, presenting edited definition `/tmp/ik-observe-v2.ik.toml` (same machine id/version, different spec hash) to mutate it. Observed exit 1 and diagnostic `IK_MACHINE_MISMATCH: presented machine identity does not match the aggregate's recorded identity`. This is identity ownership, not store corruption or transparent migration. `docs/machine-evolution.md:3-20,39-48` already documents retained old identity and routing; Greenfield now links it.

### (f) Missing store reported as corruption — report disproved

With the current binary, ran a read-only `show` request against a fresh nonexistent `--store` path, supplying the identity for `machines/effect_probe.ik.toml`. Observed exit 1 and JSON diagnostic code `IK_STORE_NOT_FOUND`, rule `store.missing`; the path remained absent. This matches `src/diagnostic.rs:45-49`, which distinguishes missing from corrupt stores. No product defect and no behavior change.

## Commands and source availability

Build/version command: `CARGO_TARGET_DIR=<held target directory> cargo build --bin ik` → `OK`; resulting binary `--version` → `invariant-kernel 0.5.0 protocol 1.0`.

The submitted builder project directories and `batch-001/kit-B` are not present in this checkout. These small binary reproductions establish the named behavior independently; they do not recreate a builder run or alter its recorded outcomes. `results.jsonl` remains unchanged.
