# Change-aware evidence

Analysis does not upcast state. Old aggregates keep the machine identity they
were created under. A new definition does not rebind stored state, and evidence
recorded for one identity cannot satisfy a claim recorded for another. Requirement
diffs are visible on their own; they are not folded into the machine spec hash.

## Profile diff

```sh
ik profile diff <old.toml> <new.toml> [--json]
```

Compares two version-1 profiles with the same loader as `ik check --profile`.
Goals are matched by id. The diff reports goals added, removed, or changed
(content digest), a `required` toggle, an `allow_waiting` change, and a limit
change. Reformatting a file does not change the digests. A goal id is not the
requirement: the same id with different content is a change.

`weakened` is set when assurance was relaxed:

- a required goal was deleted
- `required` changed from true to false
- `max_nodes` or `max_depth` decreased (the search bound got smaller)
- `allow_waiting` gained a state (more states are excused from the stuck-state goal)

Exit 0 when the diff is printed, 2 on usage. A weakened diff still exits 0.
Callers that must fail the job check for `weakened`.

## Evidence verify

```sh
ik evidence verify <evidence.json> --machine <m.ik.toml> [--profile <p.toml>] [--corpus <c.json>] [--runtime-binary <path>] [--json]
```

`evidence.json` is one of:

- `ik check --json` output (including a `profile` object when one was requested)
- an `ik.scenario.result.v1` result
- an `ik.analysis.result.v1` result

The command recomputes current identities and compares them with what the
evidence recorded. The machine spec hash is the same value `ik identity` prints.
The profile digest and each goal content digest come from the profile loader.
The corpus digest is the same digest `ik check` records, including the default
case inserted for an argument-less command. The scenario digest is the SHA-256
of `scenario.json` beside the evidence file, which is how a scenario bundle
records it. The runtime digest is the SHA-256 of `--runtime-binary`, compared
only when the evidence recorded a binary digest and the flag is present.

Each axis is `current`, `stale` (naming the mismatching field), or
`not_recorded`. Overall is `current` only when every recorded axis that was
recomputed matches. Otherwise it is `stale_profile`, `stale_machine`,
`stale_corpus`, `stale_runtime`, `stale_scenario`, or `different_machine_version`.

`different_machine_version` means `spec_hash` or `machine_version` differs.
No rebind or migration occurs. Evidence for a new definition can never satisfy
a persisted old-identity claim, and a persisted old identity cannot satisfy a
new definition. Old aggregates keep their machine identity.

A goal id is never enough. The same id with a different content digest is
`stale_profile`.

Exit 0 only when overall is `current`, 1 when the evidence is stale or for a
different machine version, 2 on usage. Omitting `--runtime-binary` leaves a
recorded runtime digest `not_recorded` and does not by itself fail the check.
Pass `--profile` when the evidence records a profile digest.

## Prior baseline

```sh
ik check <machine.ik.toml> --profile <checks.toml> --prior <old.toml>
```

When both flags are set, the same profile goals are evaluated against the prior
machine as well as the candidate. A goal whose referenced state or command
exists in only one of them is `not_comparable`, with the reason. It is not
reported as unreachable.

`profile.baseline` lists each goal's candidate status and prior status. The
marker `regression` is present only when a goal that was satisfied on the prior
is not satisfied on the candidate. A feature change that blocks a normal path
is a regression. A benign addition that leaves the goal satisfied is not.

`ik check --prior` without `--profile` still prints the identity
`CHANGE IMPACT` paragraph and does not add `profile.baseline`.
`--json` includes a `definition` object for the declared command/state delta;
`ik report` projects it as evidence class `definition`. It does not establish
execution or readiness.

## Existing-instance continuity

A scenario whose first step is `prior_machine` creates the aggregate under
that prior definition, then later commands record a `present` observation.
A prior-supported command presented as `prior` commits on the existing
instance. A new-only command presented as `new` is refused with the runtime's
typed reason (`IK_MACHINE_MISMATCH` when the presented identity is not the
stored owner). `ik report` projects those steps as `continuity_observation`
findings. `ik handoff --kind change` with claim `continuity` is `exercised`
only from those findings. Unrelated statuses do not support the claim.
Without that evidence the handoff renders `continuity not demonstrated`.
Existing instances stay on the version they were created under; moving a
running instance to the new version in place is not supported. The choice is
to keep them on the old version until they finish, or to drain and recreate
them on the new one; `ik handoff --kind change` asks it as a decision.

## CI

```sh
set -eu
BASE_MACHINE="${BASE_MACHINE:-machines/job.base.ik.toml}"
BASE_PROFILE="${BASE_PROFILE:-.ik/checks.base.toml}"
MACHINE="${MACHINE:-machines/job.ik.toml}"
PROFILE="${PROFILE:-.ik/checks.toml}"
CORPUS="${CORPUS:-cases.json}"

ik check "$MACHINE" --profile "$PROFILE" --corpus "$CORPUS" --prior "$BASE_MACHINE" --json > evidence.json
ik profile diff "$BASE_PROFILE" "$PROFILE" > profile.diff
if grep -q weakened profile.diff; then
  echo "requirement diff weakened the profile" >&2
  exit 1
fi
ik evidence verify evidence.json --machine "$MACHINE" --profile "$PROFILE" --corpus "$CORPUS"
```

Analysis does not upcast state. Old aggregates keep their machine identity.
Requirement diffs stay visible in `ik profile diff` and are not hidden inside a
machine spec hash.
