# Scenarios

A scenario exercises one machine against a fresh scratch store. It submits
named commands, retains handles, looks up operation ids, reopens the store, and
records synthetic effect outcomes. Nothing in a scenario sends mail, publishes
an artifact, or calls an external executor. The runtime is the authority: a
refused command is not a committed state, and a read is not a mutation.

## Run

```sh
ik scenario run <machine.ik.toml> <scenario.json> --out <dir>
ik scenario reproduce <bundle-dir> --out <dir>
```

There is no `--store` flag. The driver creates `<dir>/scratch/store.sqlite` and
refuses to start if that path, or the output directory, already contains a
store file. It never opens a store that already exists.

Exit codes: `0` when every expectation is met, or when `reproduce` matches;
`1` when an expectation fails or the reproduction differs; `2` for usage
errors and for the store-file refusal.

`reproduce` prints `reproduced` or `not_reproduced`. A mismatch names the first
differing step. A different `binary_sha256` is reported as `runtime difference:
binary_sha256` and is not, by itself, a failure.

## Scenario file

Version 1 is a JSON object:

```json
{
  "scenario_version": 1,
  "machine": "../machines/publish_approval_v3.ik.toml",
  "aggregate": "paper-1",
  "steps": []
}
```

`machine` is a path relative to the scenario file. `aggregate` is the id
created before the first listed step (generation 0). Operation ids come from
the step, or are `op-<label>` when omitted. Labels and operation ids use
`[A-Za-z0-9._:-]`.

Each step has a `kind` and an optional `label`.

| Kind | Fields | Expectation |
|---|---|---|
| `command` | `name`, `args`, optional `operation_id`, `handle`, `use_handle`, `present` | `committed`, `noop`, or `refused:<CODE>` |
| `prior_machine` | `path` (relative prior definition). Must be the first step. | implicit: the instance is created under that definition |
| `drop_reply` | `of` (label) | implicit: the named step committed; the reply is discarded and the commit stays |
| `lookup` | `operation_id` | `committed_exact`, `conflict`, or `not_recorded` |
| `duplicate` | `of` (label) | implicit: the original operation is returned and nothing new is committed |
| `reopen` | none | implicit: state after reopening the scratch store matches state before |
| `effect_outcome` | `effect_id`, `payload_digest`, `idempotency_key`, `planned_generation`, `outcome_digest`, optional `operation_id` | `committed`, `noop`, `committed_exact`, or `refused:<CODE>` |

`handle` retains the capability returned by a committed command. `use_handle`
submits that retained capability instead of the current one. A retained
capability from before a later commit is stale; the runtime refuses it with
`IK_CAPABILITY_STALE` and commits nothing.

`prior_machine` creates the aggregate under a previous definition. Later
commands default to presenting that owner identity. `present` of `new` presents
the scenario machine instead. Each of those steps records a `present`
observation: created version, presented version, command name, and refusal
code when one fired. Report and handoff read that observation; they do not
decode store JSON.

`drop_reply` does not roll back the named commit. A following `lookup` of that
operation id returns `committed_exact` when the request still matches.

`duplicate` resubmits the same operation id, arguments, and capability. The
same id with different arguments is a `command` step and is
`IK_OPERATION_CONFLICT`.

`effect_outcome` calls the runtime record path. An exact repeat of a recorded
outcome commits nothing (`noop`, or `committed_exact` when the same operation
id is replayed). A different outcome for the same effect is
`IK_EFFECT_OUTCOME_CONFLICT`; the first outcome remains.

`<CODE>` is a public diagnostic name such as `IK_CAPABILITY_STALE` or
`IK_OPERATION_CONFLICT`.

## Bundle

`--out` receives:

| File | Contents |
|---|---|
| `scenario.json` | verbatim copy of the scenario |
| `machine.ik.toml` | copy of the machine that ran |
| `result.json` | `ik.scenario.result.v1` |
| `trace.json` | `ikaros.trace.v2` |
| `SHA256SUMS` | digests of the four files above |

`result.json` records the scenario digest, machine id, spec hash, runtime
identity (`ik_version`, `binary_sha256`, `store_schema`), and one row per step:
label, kind, request summary, authoritative outcome, state after (state name,
generation, facts, effect), expectation, and `pass` or `fail`. It also lists
`faults_applied`, `seams_fired`, and `all_expectations_met`.

Each entry in `faults_applied` is an object `{name, seam_fired, proof}`.
`seam_fired` is true only when the runtime treated the requested fault that
way, and `proof` is the observation that establishes it. For a dropped reply
the proof includes the named step's operation lookup (sequence span when the
row exists) and the generation. `seams_fired` remains the list of names whose
`seam_fired` is true.

Fault names are `reply_dropped`, `stale_handle_used`, `duplicate_submission`,
`reopened`, and `conflicting_effect_outcome`. A name is applied when the
schedule asked for that fault. It is listed in `seams_fired` only when the
runtime actually treated it that way: a stale submission was refused as stale,
a duplicate did not create a second commit, a dropped reply belonged to a
commit (the operation lookup is `committed_exact`), a reopen preserved state,
or a conflicting effect outcome was refused and the first outcome still stands.

A requested fault whose seam did not fire fails the run (exit 1). A
`drop_reply` of a step that did not commit is that failure: the proof's
lookup is `not_recorded`, `seam_fired` is false, and the scenario does not
pass. Absence of injection is not a crash or drop pass.

`trace.json` is advisory. Commits, refusals, no-ops, reads, and historical
results are different events. A refusal has no destination state. A lookup or
reopen is a read or a historical result, not a commit.

`reproduce` runs `scenario.json` and `machine.ik.toml` from the bundle in a new
scratch store and compares `result.json` except the runtime identity.

See `examples/publish-approval/scenarios/adverse-ordering.json` for a schedule
that uses the publish-approval machine.
