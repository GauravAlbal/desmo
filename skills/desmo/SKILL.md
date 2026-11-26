---
name: desmo
description: Audit existing code for implicit state machines and lifecycle bugs, or deliver, change, and diagnose a Desmo application lifecycle with the installed ik and desmo commands. Use when a coding agent must find hidden state bugs in an existing repo, build stateful behavior, change an existing machine without eroding old instances, or recover a lifecycle problem. The human answers at most one behavior or permission question.
---

# Desmo

You engineer the lifecycle. The human supplies behavior and reserved permission. Install this skill with `npx skills add GauravAlbal/desmo` or `desmo skill install [--dir <project>] [--target <agent-skills-dir>] [--force]`, then follow one sibling journey in this skill directory:

- Read `routing.json` first and pick a mode before any write.
- `audit.md` — read-only 60-second brownfield discovery & state-bug audit.
- `build.md` — author and wire a new authoritative `.ik.toml` lifecycle.
- `change.md` — evolve an accepted machine without breaking persisted instances.
- `diagnose.md` — investigate a runtime refusal, race, or lost reply.
- `readout.md` — write and lint the human summary around `ik handoff`.

## Fast onboarding

1. **Check binary availability:** run `ik --version` (and check `$HOME/.local/bin/ik`). If `ik` is missing and the human asked to install or onboard Desmo, run `curl -fsSL https://raw.githubusercontent.com/GauravAlbal/desmo/main/install.sh | sh` and prepend `$HOME/.local/bin` to `PATH`.
2. **Audit read-only first (`audit.md`):** scan the repo for implicit `status`/flag/timestamp lifecycles, extract a scratch `.ik.toml`, run `ik check` (`--auto-seed --summary`) and `ik simulate`, and report findings with `file:line` pointers. If the workspace has no application code yet, run `ik demo` and offer `ik init order --lang ts` (or `--lang py`, `--lang go`).
3. **Ask once before modifying source:** share the findings + simulator path and ask which lifecycle (if any) to enforce with `build.md`.

## Commands & resolution

`ik` is the single kernel binary (`demo`, `init`, `doctor`, `diagram`, `simulate`, `check`, `codegen`, `laws`, `corpus`, `scenario`, `guard`, `identity`, `schema`, `operations`, `events`, `census`, `store`, `report`, `handoff`, `readout`, plus `--store`/`--session` runtime modes and `--help-all` for advanced commands). Resolve it before assuming it is missing:
- **Global install (all languages):** `ik` on `PATH` (or `$HOME/.local/bin/ik`, or `IK_BINARY`).
- **Node/TypeScript:** `node_modules/@desmo/<platform>/bin/ik` (e.g. `darwin-arm64`, `linux-x64`); `desmo` runs as `npx --no-install desmo ...` or `node node_modules/@desmo/runtime/dist/cli.js ...`.
- **Python / Go:** `ik` on `PATH` or `IK_BINARY`.
- **Installing language SDKs (do not use bare npm/PyPI yet):** read `v<VER>` from `ik --version`. Node tarballs (`desmo-runtime-<VER>.tgz` + `desmo-<platform>-<VER>.tgz`) and the Python wheel (`invariant_kernel-<VER>-py3-none-any.whl`) install from `https://github.com/GauravAlbal/desmo/releases/tag/v<VER>`; Go installs via `go get github.com/GauravAlbal/desmo/go@go/v<VER>`.

## Discover

1. **Environment read (no store created):**
   - Any language: `ik doctor --dir . --json` (validates all `*.ik.toml` files, runs static structural lints, and checks SDK version pins).
   - Node projects may additionally run `npx --no-install desmo doctor --dir .`.
2. `ik identity <machine.ik.toml>` prints `machine_id`, version, and `spec_hash` (never hash the spec yourself).
3. `ik laws <machine.ik.toml>` is an advisory inventory; `not_examined` means no verifier looked, not a pass.
4. `ik schema <machine.ik.toml>` is structural metadata for generators (does not evaluate guards).
5. Find writers yourself by searching the project for status fields, machine paths, and command names. Never ask the human to locate them.

*(Deep reference docs in the Desmo repository: `docs/agent.md`, `docs/client-recovery.md`, `docs/operation-identity.md`.)*

## Hard rules

- `investigate_only` mode never writes source and never mutates a store (`verify` is read-only; `replay` can repair projections and belongs in `authorized_repair`).
- A current `show` read is not attribution of a past request.
- Never refresh a capability into an old logical operation.
- Never retry an uncertain effect; reconcile first.
- An absent operation record is not proof of refusal or cancellation.
- Analysis never runs against a production store.
- Specialist checkers (TLC, Hegel) are optional, chosen by the question, run only in scratch, and never block `ik check` or `ik scenario run`. A model counterexample is `concrete_reproduced` only after `ik scenario run` matches it.
- A fit refusal must cite a concrete requirement or topology conflict and name the simpler path.
- Ask the human at most one consequential behavior or permission question. Never ask them to choose a checker, write a corpus, inspect a payload schema, or find writers.

## Human projections

Every trust-bearing statement cites an evidence id or path. List unknown and unrun claims explicitly. Do not invent a pass. Human approval of behavior is not correctness.

### Behavior proposal

```text
Behavior: <user-visible behavior in domain terms>
Defaults assumed: <only consequential defaults, or none>
Not included: <explicit exclusions>
Unknown: <unresolved non-blocking unknowns, or none>
```

### Decision request (at most one open question)

```text
Decision: <one behavior or permission question>
Options:
- A: <consequence>
- B: <consequence>
Recommendation: <option plus evidence id, or none>
Unlocks: <what proceeds after the answer>
Continues meanwhile: <work that does not depend on the answer>
```

### Delivery, change, or recovery handoff

`ik handoff` renders the handoff — do not rewrite it. See `readout.md` for the handoff structure, lint rules (`ik readout`), and quoting rules. `ik diagram <machine.ik.toml> [--prior <old.ik.toml>] [--evidence <path>]... [--kind delivery|change|recovery] [--format markdown|text|mermaid|table|json]` draws the route map and command table without a behavior file (Mermaid is `--format mermaid` only). Scenario-only evidence or a declared delta never establishes readiness.
