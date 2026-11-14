---
name: desmo
description: Deliver, change, or diagnose a Desmo application lifecycle with the installed ik and desmo commands. Use when a coding agent must build stateful behavior, change an existing machine without eroding old instances, or recover a lifecycle problem. The human answers at most one behavior or permission question.
---

# Desmo

You engineer the lifecycle. The human supplies behavior and reserved permission. Install this router with `desmo skill install [--dir <project>] [--target <agent-skills-dir>] [--force]`, then follow one journey.

Read `skills/desmo/routing.json` and pick a mode before any write. Procedures: `skills/desmo/build.md`, `skills/desmo/change.md`, `skills/desmo/diagnose.md`. Writing the summary around a handoff: `skills/desmo/readout.md`.

## Commands

Top-level `ik` usage is derived from the dispatch table, including
`scenario`, `report`, `handoff`, `corpus`, `analysis`, and `regression`.
Explanation is the library function in `src/declarative/explain.rs` plus
`ik check --json` witnesses, not a packed subcommand.

`ik` is the kernel binary shipped with the runtime package. In a Node project it is `node_modules/@desmo/<platform>/bin/ik` (for example `darwin-arm64`); in a Python project it is the path in `IK_BINARY`, or the packed binary the project documents. `desmo` is the Node package's command. It is on `PATH` only when `node_modules/.bin` is, so run it as `npx --no-install desmo ...`, or, when `npx` is not on `PATH`, as `node node_modules/@desmo/runtime/dist/cli.js ...` with the Node binary your task names. Every `desmo ...` and `ik ...` command in these journeys means that resolved invocation. A "command not found" is a resolution problem, not a missing capability: resolve it as above before concluding a step cannot run.

## Discover

1. Environment read. In a Node project: `npx --no-install desmo doctor --dir .` reports package version, the `ik` version line, `.ik/checks.toml`, `*.ik.toml` machines, `scenarios/`, adapter manifests, and whether `.agents/skills/desmo/` is present. It does not execute an adapter, fetch a tool, or authorize a commit. `unavailable` is a missing optional tool, not a failed native path. In a Python project there is no `desmo` command: run `ik --version`, list `*.ik.toml` machines, and look for `.ik/checks.toml`, `scenarios/`, and `.agents/skills/desmo/` yourself. Neither read creates a store.
2. `ik identity <machine.ik.toml>` prints `machine_id`, version, and `spec_hash`. Do not hash the spec yourself. This is not a law proof and not permission to commit.
3. `ik laws <machine.ik.toml>` is an advisory inventory. The `.ik.toml` is authoritative. `not_examined` means no verifier looked; it is not a satisfied obligation.
4. `ik schema <machine.ik.toml>` is machine-derived metadata for generators. It does not evaluate guards.
5. Find writers yourself: search the project for the machine path and command names. Do not ask the human to find them.

Read `docs/agent.md`, `docs/client-recovery.md`, and `docs/operation-identity.md` before any recovery claim. Hero machine: `machines/publish_approval.ik.toml`.

## Hard rules

- Inspect-only mode never writes source and never writes a store. Scratch output is not a production store.
- A current read is not attribution of a past request.
- Never refresh a capability into an old logical operation.
- Never retry an uncertain effect. Reconcile first.
- An absent operation record is not proof of refusal or cancellation.
- Analysis never runs against a production store.
- Specialist checkers are optional, chosen by the question, and never block `ik check` or `ik scenario run`.
- A fit refusal must cite a concrete requirement or topology conflict and name the simpler path.
- Ask the human at most one consequential behavior or permission question. Never ask them to choose a checker, write a corpus, inspect a payload schema, or find writers.

## Specialist checkers

Choose by the question, not by preference. Run only in scratch, never against a production store, and cite the result by `ik report`.

- "Can the lifecycle reach a state with no way out?" Use `ik graph export` then the TLC adapter (`adapters/tlc/`). A closed graph plus an exhaustive run is model-level `holds_in_scope` for that graph only; an incomplete graph is `bounded_paths`; a missing Java or jar is `tool_unavailable`, not a pass.
- "Does this property hold over many generated inputs?" Use the Hegel example adapter. The result is `sampled_execution`, never a proof.
- A model counterexample is `concrete_reproduced` only after `ik scenario run` matches it. Otherwise it stays model-level.
- Neither checker is required for a delivery. State in the handoff which were not run.

## Human projections

Fill these. Every trust-bearing statement cites an evidence id or path. List unknown and unrun claims explicitly. Do not invent a pass. Human approval of behavior is not correctness.

### Behavior proposal

```text
Behavior: <user-visible behavior in domain terms>
Defaults assumed: <only consequential defaults, or none>
Not included: <explicit exclusions>
Unknown: <unresolved non-blocking unknowns, or none>
```

### Decision request

Use only when intent, authority, or an irreversible external effect is still open. One question.

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

`ik handoff` renders this. Do not rewrite it.

```text
Desmo <kind>: <subject>
operation: <operation_id, recovery only, when evidence names one>

New in this version
<declared prior-vs-new delta, or what is new: not stated in evidence>
declared in the new definition

Demonstrated
<runtime outcomes actually observed>
observed (runtime)

Explored only
<check witnesses>
explored (bounded search, not a runtime commit)

Not established
<uncited, incomplete, continuity not demonstrated, in-place move not supported, whether the caller saved the operation id>

Scope
Checked: whether <what this kind of handoff asks of the evidence>
Not checked: <criteria validity; execution of the code and effects performing a transition (assumed perfect and reliable); whether the definition says what was needed; anything the requirement does not state>
Readiness: the owner's decision; this report does not state it.

Who attested what
<table: duty, expected party, recorded; "not recorded" is the only gap wording>

Where each criterion came from
<table: rule, decision point, origin, detail; the scope sentence is always printed>

What is missing
<definition gaps, commands with no cited committed run or guard refusal observed against this definition, and requirement statements nothing implements (only if the requirement was given as atoms); the scope sentence is always printed and says this is not a completeness claim>

facts strip
```text
<route map>
```
legend: <only glyphs present>

Decision needed
<none, or computed: each declared rule to accept (with its hash), the version policy for existing instances, and any authored decision>
Safe next action (recovery; last line of this block)

<table of every command: from, to, guard, effect>

Exact evidence
<cited finding ids, scopes, reading, and evidence index>
```

The route map and table compress the first block; they do not
add facts. Reachability is structural (guards are not executed). Recovery is
a bookended unfolded arrow of the events the evidence supports, not a state graph. `ik diagram <machine.ik.toml>
[--prior <old.ik.toml>] [--evidence <path>]... [--kind delivery|change|recovery]
[--format markdown|text|mermaid|table|json]` draws the same diagram without a
behavior file. Default markdown is the route map and table. Mermaid is `--format mermaid` only.

Recovery subject and operation identity come from evidence. If they are absent
the first line is `Desmo recovery: subject not identified in evidence`.
Scenario-only evidence never establishes readiness. A declared definition delta
does not establish execution or readiness.
