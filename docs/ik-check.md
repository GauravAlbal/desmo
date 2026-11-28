# ik check

Developer analysis. Not a lifecycle operation.

```bash
ik check machine.ik.toml [--prior old.toml] [--corpus cases.json] [--auto-seed] [--summary] [--compact] [--bound "retries <= 5"] [--simulate] [--seed <u64>] [--steps <n>] [--threads <n>] [--fail-fast] [--emit-scenario repro.json] [--baseline validation/verification_baseline.json]
```

Prints a repair packet and a `VERDICT` of `ACCEPT` or `REFUSED` (or a concise developer summary with `--summary`, or agent-friendly JSON with `--compact`).
Exit 0 on accept, 1 on refuse, 2 on transport/usage failure.

- **Zero-corpus static structural lints:** every `ik check` and `ik laws` run checks for unreachable states, reachable non-terminal sinks (dead ends), duplicate-guard shadowed outcomes, and initial-state invariant violations without requiring a corpus.
- **Guard-guided `--auto-seed` & automatic invariant falsifier:** `--auto-seed` synthesizes concrete boundary cases from guards, `text_in` predicates, and fact-equality links as an explicit `auto_seeded_bounded` advisory check (`--auto-seed` is refused with `--release`; use `ik corpus init --smart` to save and pin an editable corpus). Even without `--corpus` or `--auto-seed`, `ik check` runs a fast bounded auto-seeded falsifier on machines with `[invariants]` to surface concrete runtime invariant-violation witnesses early.
- **Inline `[reach]` (`sometimes`) properties:** declare `[reach]` predicates directly in `.ik.toml` (for example `can_refund = "state == 'refunded'"`). Because `[reach]` is omitted from `spec_hash` serialization, adding `[reach]` preserves live aggregate compatibility; any unreached `[reach]` property refuses the check (`unreached_property`).
- **Domain bounds (`--bound` / `[limits].boundary`), `--threads`, `--fail-fast`:** pass repeatable `--bound "<expr>"` predicates (e.g. `--bound "retries <= 5"`) so counter/unbounded BFS completes cleanly inside that domain instead of truncating at `max_nodes`. Use `--threads <n>` for parallel BFS and `--fail-fast` to halt on the first `IK_INVARIANT_FAILED` witness.
- **Seeded deep simulation (`--simulate --seed <u64> --steps <n> --traces <n>`):** runs a deterministic seeded random walk ($O(\text{depth})$ trace memory) to catch deep invariant bugs beyond BFS depth 16, and automatically falls back when BFS hits node/depth limits in advisory mode.
- **Actionable traces & interactive Explorer (`--emit-scenario`, `ik simulate --check`):** any invariant refusal prints step-by-step pre/post-states, concrete arguments, fact diffs, and an `ik simulate <file> --check --open` command; `--emit-scenario <file.json>` writes a runnable `ik scenario run` reproduction bundle, while `ik simulate` embeds checked properties, 1-click **Replay Bug** traces, a **Delta / Full** fact view toggle, and timeline scrubbing/branching directly inside the offline HTML.

`--json` `judgment.verdict` is `accept` or `refused` (the same two
outcomes as the text `VERDICT`, in lowercase). `judgment.status` is the
typed distinction: `accepted`, `missing_evidence` (no corpus, uncovered
or incomplete obligations, stale evidence, infrastructure failure), or
`witnessed_violation` (a concrete invariant or falsified obligation).
Exit codes are unchanged. There is no `refuse` token.

Python: `kernel.check(path, prior=..., corpus=..., baseline=...)`.
Go: `k.Check(ctx, path, prior, corpus, baseline)`.
Those wrappers expose `verdict` `accept` or `refused` from the process
exit; `judgment.status` is on the `--json` document.

## Developer verdict versus strict release

`ACCEPT` is a developer judgment: no newly introduced mandatory debt and no
falsified obligation. It is corpus-relative and may rest on a truncated
exploration. Add `--release` for the strict release policy (exit 1 unless both
hold), and `--json` for a machine-readable document with `report`, `judgment`
and `release`.

Each obligation carries exactly one status: `satisfied` (executed and
witnessed), `falsified`, `incomplete` (not witnessed and the exploration was
truncated, so absence proves nothing), `not_applicable` (stale evidence),
`not_examined` (outside the standard lane: no verifier looked at it, and it is
never counted as satisfied), `approved_debt` (baseline-listed) and `uncovered`.

Coverage is two-tier. **Authored** goals (each authored outcome, including
parent fallback vs child override as distinct coordinates; each declared
state and terminal) are mandatory and are satisfied only by a committed
kernel witness of some generated copy that maps to that coordinate. A sound
impossibility proof never satisfies an authored goal. Flat schema-1 machines
treat the flat entities as authored, so a shadowed later arm stays uncovered.

**Generated** product-context obligations (command/source rows, lowered
outcome copies, product states and terminals) may be discharged only by a
sound impossibility record with explicit `basis` and `scope`. When the
optional pinned SMT adapter is configured (`IK_SMT_ADAPTER` pointing at
`adapters/smt` or `auto`), a generated row may be discharged only by a
`ProvenImpossible` record whose `trust` is `CheckedInvariant` or
`CheckedExhaustive` and whose certificate re-verifies. `SolverAsserted`
records are reported and never discharge. Absent the adapter, `ik check`
is unchanged: solvers are not probed and the plan digest is identical.
Unknown
feasibility stays retained. Corpus absence is never impossibility. Guard
evaluation errors are not false and do not fall through to later arms.
String ordering over unbounded argument domains is outside the sound
fragment and stays unknown. `text_in` is also outside that fragment: partial
evaluation leaves it unknown, so a generated obligation is never discharged
by `text_in` alone. Concrete corpus cases still evaluate it through the
shared runtime `holds()` path. Any later backend must pass the known-solution
suite in `docs/feasibility-validation.md` before it may discharge a generated
success obligation.

Proofs appear on the plan as `impossibilities` (the plan's `schema_version`
is 2 when records or authored-goal rows exist; a plan with neither stays
version 1 and keeps its digest). Command/source records
(`class=command_source`) are discharged. On a lowered machine, ordered-outcome
and reachability records for generated copies are discharged when proved;
the authored goal remains until witnessed. Schema-1 ordered-outcome proofs
stay recorded and retained. A refusal never satisfies a success goal.
Explicit `--profile` reachability/command goals are never satisfied by a
proof; the proof is attached as explanation and the required goal still fails.

Old-to-new obligation mapping: existing `command/{cmd}/from/{source}/executable`,
`outcome/{cmd}/{index}/executable`, `state/{name}/executable`, and
`terminal/{name}/reachable` ids are unchanged. On schema-2 machines they
name generated copies. New authored ids
`authored/outcome/{cmd}/{layer}/{ordinal}/executable`,
`authored/state/{region}/{leaf}/executable`, and
`authored/terminal/{region}/{leaf}/reachable` are added and are not a
relabel of prior debt as satisfied. Evidence binds to the authored model,
compiled semantics, and coverage-policy identities.

A strict release passes only when the exploration completed within its bounds,
at least one outcome was executed, and no standard-lane obligation is
falsified, incomplete, stale, approved debt, or unwitnessed, with no surviving
challenge and no infrastructure failure. The release section records the corpus
digest, node/depth bounds, completion reason, engine versions and plan digest.
`runtime_artifact_digest` is `null` when the build supplies none; it is never
synthesized. Obligation kinds with no verifier are listed under excluded
assumptions.

`ik laws` links each invariant to its plan obligation and reports the honest
status (`not_examined` today: runtime enforcement is the only evidence).

`ik laws` also lists `unguarded_arguments`: each declared command argument that no
`when` (command or outcome) reads as a whole `$name` token. Such an argument is
checked only by host code, so a direct runtime command may carry any value of its
declared type, for example an empty string. It is advisory inventory, not a
verdict: an identifier that is only stored is legitimately unguarded.

## Trace export (`ikaros.trace.v2`)

Events are tagged: `commit`, `refusal`, `no-op`, `unknown`, `read`,
`historical` (an exact-duplicate response, not a new commit) and `witness` (a
bounded checker path, not a run). Bundles declare `origin`
(`observed`/`replayed`/`bounded-witness`), `reproducible` and `redacted`; a
redacted bundle cannot be reproducible and a replayed one must name its inputs.
`parseTraceBundle` rejects contradictory fields instead of rendering them.
This is a breaking change from `ikaros.trace.v1`, which could label a read as a
commit; the export is advisory and carries no authority.

## Running `ik check` without a corpus

Without `--corpus` and without a sibling `<machine>.ik.cases.json`, there are no cases to explore, so `ik check` reports `REFUSED` (for example "uncovered newly
introduced mandatory obligation": no command outcome was witnessed); this is a statement about missing evidence, not a defect in the machine. The runtime enforces the
machine's laws regardless. The refusal text adds one trailing `NEXT COMMAND` line, and `--json` adds a `next_actions` array, naming the exact command to generate an editable starter corpus. Exit codes and the existing JSON members stay as they were. A check that already has a corpus does not add that hint.

```bash
ik corpus init machine.ik.toml --out machine.ik.cases.json
ik check --corpus-template machine.ik.toml
```

The starter has `"_note": "example cases, not an exhaustive domain"`, one type-valid example per command, and, for each command with an `int` argument, `boundary_min` and `boundary_max` at the shared signed 64-bit ends. Edit it. It is not a domain proof. Supply `--corpus cases.json` (see `examples/publish-approval/machines/*.cases.json`) to
get an accept/refused verdict that is bounded and corpus-relative.

## Check profiles

`--profile <file>` evaluates a version-1 TOML profile after the normal check.
`.ik/checks.toml` is only a conventional path; a profile is loaded only when
the flag is passed. Without `--profile` or `--plan`, text, `--json`, and exit
codes are unchanged.

```toml
profile_version = 1

[limits]
max_nodes = 4096
max_depth = 16

[[goals]]
id = "published"
kind = "reach_terminal"
terminal = "published"
required = true

[[goals]]
id = "not-stuck"
kind = "no_stuck_states"
allow_waiting = ["awaiting_review"]
```

`kind` is `reach_terminal` (field `terminal`), `reach_state` (field `state`),
`command_executes` (field `command`), or `no_stuck_states` (optional
`allow_waiting`). `required` defaults to true. Unknown fields, unknown kinds,
duplicate ids, and references to undeclared states or commands are usage
errors (exit 2).

`[limits]` may set `max_nodes`, `max_depth`, and optional `boundary = "<expr>"` (using the same state-and-fact predicate grammar as `--bound` to prune out-of-domain states cleanly so BFS finishes inside that finite domain). Omitted limits use 4096 and 16.
A profile has its own digest over the canonical profile, so reformatting the
file does not change it, and it does not change the machine spec hash. Each
goal carries a content digest over the canonical JSON of that goal. Changing
the goal content changes both digests even when the id stays the same.

`--plan` prints the goals, kinds, content digests, limits, corpus digest, and
this scope, and does not run exploration:

> search up to N nodes / depth D over the supplied corpus; a witness proves reachability, absence of a witness does not prove unreachability unless the search is complete

It exits 0. With `--json` it prints a plan object and no witness.

After a normal check, each goal has one status:

| Status | Meaning |
|---|---|
| `witness_found` | Shortest witness steps from the exploration. Satisfies a required goal. |
| `no_witness_found_complete` | Search completed and the target is unreachable within the supplied corpus. Does not satisfy a required goal. |
| `no_witness_found_incomplete` | Search was truncated by the node or depth bound. Absence of a witness is not unreachability. Does not satisfy a required goal. |
| `input_coverage_gap` | A command needed on a path has no corpus case, or every supplied case was refused. Not an unreachability result. Does not satisfy a required goal. |
| `stuck_states_found` | An executably reached, non-terminal, non-allowed state had no successful declared outgoing command. Each candidate is `no_corpus_case`, `all_cases_refused`, or `no_outgoing_command`. This is not a proven deadlock when the search is incomplete or the corpus is missing cases. Does not satisfy a required goal. |
| `no_stuck_states_found` | No such candidate among executably reached states. Satisfies a required goal only when the search completed. |

A terminal state with no outgoing command is not stuck. States listed in
`allow_waiting` are not stuck.

With `--profile`, exit 1 if the check would already be non-accept, or if any
required goal is not satisfied. Optional goals are reported and do not change
the exit code. `--json` adds a top-level `profile` object. Text output appends
a `PROFILE` section after the existing report.


## Prior baseline

`--prior` prints a `CHANGE IMPACT` paragraph from the prior/new identity
comparison: the new version number, whether commands or states were added or
removed, and, when the machine version differs, that existing instances stay on
their version and that moving one to the new version in place is not supported.
The paragraph
states only what that comparison contains. It does not say how a live store
would refuse a command on an old instance. `--json` also includes a
`definition` object (`ik.definition.delta.v1`) with that declared command and
state delta. `ik report` projects it as a finding with evidence class
`definition` (declared, not executed).

`--prior` with `--profile` also evaluates the same goals against the prior
machine where the referenced states and commands exist in both. A goal that
cannot be asked of the prior is `not_comparable`, with the reason.
`profile.baseline` shows the candidate status, the prior status, and
`regression` when a goal satisfied on the prior is not satisfied on the
candidate. See `docs/change-evidence.md`.

## Lifecycle diagrams

`ik diagram <machine.ik.toml>` draws the definition as a monospace route map
and a table (`--format mermaid` for a flowchart). Evidence, kinds, and the handoff embedding
are documented in [`docs/workbench-report.md`](workbench-report.md#diagram).
The diagram does not replace `ik check`; it compresses the same graph the
definition already states. Reachability in the map is structural (guards are
not executed).
