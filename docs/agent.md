# Agent guide

Desmo is an authoritative lifecycle runtime. You keep TypeScript, Python, or
Go. Critical transitions go through `ik`. Do not reimplement those transitions
in the host. Do not silently refresh stale capabilities or retry uncertain
effects.

No private tooling is required to complete the public examples.

## Skill

The packaged procedure is [skills/desmo/SKILL.md](../skills/desmo/SKILL.md).
Install it into the project you are editing:

```text
desmo skill install --dir .
```

`desmo doctor --dir .` reports whether `.agents/skills/desmo/` is present.
Follow `build.md`, `change.md`, or `diagnose.md`. Do not ask the human to
choose a checker, write a corpus, inspect a payload schema, or find writers.

## Install

Use a released `ik` on `PATH` or `IK_BINARY`. Language packages talk to that
binary. Qualified deployment: a worker with **persistent local storage**, not
a read-only serverless filesystem.

```text
ik identity <machine.ik.toml>
ik laws <machine.ik.toml>
ik check <machine.ik.toml>
ik profile diff <old.toml> <new.toml>
ik evidence verify <evidence.json> --machine <machine.ik.toml> --profile <checks.toml>
```

`ik laws` is **advisory inventory**. The `.ik.toml` (and its `spec_hash`) is
authoritative. Do not invent a parallel `LAWS.md`.

## Bounded task

Protect one lifecycle:

1. Locate writers of the lifecycle fields.
2. Author or reuse one machine. Read `ik laws` output with the user.
3. Route **new** aggregates to Desmo. Leave old aggregates on the previous
   implementation or previous machine version until they drain.
4. Add a positive test and a meaningful refusal test.
5. After a refusal, `show` canonical state. Do not resubmit with a fresh
   capability unless the user explicitly re-authorizes.

If the boundary is a poor fit (stateless functions, dual writers you cannot
find, proofs of algorithms), say so and stop.

## Hero example

`machines/publish_approval.ik.toml`: publication authorization must reference
the artifact that was approved. Editing after approval without a new approval
must not publish. See `python/tests/test_publish_approval.py`.

## Second deployment

Old aggregates keep their original `(machine_id, version, spec_hash)`. New
aggregates may use a new version (`machines/publish_approval_v2.ik.toml`).
Rolling back application code must not silently return ownership to a host-side
status field.

## Lifecycle workbench journey

One definition, then questions about it. Each step has a command. The result
answers that question only.

### Define

```text
desmo init --template job --language typescript
ik identity job.ik.toml
ik laws job.ik.toml
```

`init` writes a job project, including `.ik/checks.toml`, a normal scenario,
and a stale-handle scenario. `identity` is the machine identity. `laws` is an
advisory inventory of that definition.

This does not mean a command can finish, a law was proved, or a commit is
authorized. The runtime still decides each command when it is submitted.

### Check usefulness

```text
ik check job.ik.toml --profile .ik/checks.toml
```

Exit 0 means the developer check accepted and every required profile goal was
satisfied in this search. `witness_found` names a path the explorer executed.
When `--corpus` is omitted, the corpus is `job.ik.cases.json` beside the machine.

This does not mean a state is unreachable when no witness was found and the
search was incomplete or a case was missing. It is not a strict release pass,
and it does not authorize a production commit.

### Exercise adverse scenarios

```text
ik scenario run job.ik.toml scenarios/normal.json --out .ik/runs/normal
ik scenario run job.ik.toml scenarios/stale-handle.json --out .ik/runs/stale-handle
```

Exit 0 means every expectation in that file was met on a fresh scratch store.
The stale-handle scenario expects `refused:IK_CAPABILITY_STALE`. A refused
step committed nothing.

This does not mean a real executor ran, mail was sent, or an artifact was
published. The driver does not open an existing store. A scenario pass is not
a proof about inputs the file did not submit.

### Explain and reproduce

```text
ik scenario reproduce .ik/runs/stale-handle --out .ik/runs/stale-repro
desmo trace .ik/runs/stale-handle/trace.json --out trace.html
```

`reproduced` means a second scratch run matched the bundle result, aside from
runtime identity. The HTML trace is advisory.

This does not mean the trace is authority, a read was a commit, or a refusal
was an unsafe committed state. A specialist finding is not an observed
production execution until a concrete replay says so.

### Integrate and recover

After a lost reply, ask whether that operation id committed. TypeScript:
`Machine.lookupOperation`. The kernel operation is `lookup_operation`. `show`
is a current-state read, not that query.

`committed_exact` is history: not a new commit and not a current handle.
`not_recorded` is what that read saw. It is not a cancellation and not
permission to retry on a stale handle. A checker result does not decide
whether a production command may commit.

### Change

```text
ik check job.ik.toml --prior old.ik.toml --profile .ik/checks.toml
```

This compares the candidate with the prior definition and evaluates the
current profile against the candidate. A goal id is not enough: the profile
has its own digest.

This does not move old aggregates onto the new definition. Evidence from the
old profile does not satisfy a changed goal.

### Optional specialist checker

```text
desmo doctor --dir .
ik analysis describe .ik/adapters/<manifest>.json
ik analysis run .ik/adapters/<manifest>.json --request <request.json> --out <dir>
ik analysis conformance .ik/adapters/<manifest>.json --fixtures <dir>
```

`doctor` and `describe` read files. They do not execute an adapter or fetch a
tool. A missing optional tool is an `unavailable` line with the next command.
Native `ik check` and `ik scenario run` still run.

`ik analysis run` and `ik analysis conformance` spawn a pinned adapter. Conformance proves the host contract, not engine correctness. Exit 0 means
the host accepted a result document. It does not mean a property passed, and
it does not mean the runtime should commit. An unavailable or unsupported
specialist result does not silently become a native pass.
