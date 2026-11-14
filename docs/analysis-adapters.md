# Analysis adapters

An analysis adapter is an approved executable plus a manifest. The host reads
the manifest, writes one request, spawns the pinned argv, and validates one
result. There is no shared-library ABI, daemon, registry, or shell invocation.

This document is the host contract. A mock adapter can exercise it. Passing
that suite proves the host's validation and process controls. It does not prove
any analysis engine, mapping, or model checker.

## Commands

```text
ik analysis describe <manifest.json>
ik analysis run <manifest.json> --request <request.json> --out <dir> [--timeout-seconds N]
ik analysis validate-result <request.json> <result.json>
ik analysis conformance <manifest.json> [--fixtures <dir>]
```

`describe` reads the manifest document and does not execute the adapter.
`run` spawns the pinned argv. `conformance` spawns that same path once per
fixture. `validate-result` rechecks a stored result against the request and the
current input bytes; it does not spawn the adapter.

`run` exit codes: `0` the result document was accepted by the host, `1` the
run was rejected or failed, `2` the manifest or request was refused before
execution. Exit `0` is not a property pass and is not a release acceptance.

Existing `ik` commands are unchanged.

## Manifest (`ik.analysis.adapter.v1`)

| Field | Meaning |
|---|---|
| `protocol` | Must be `ik.analysis.adapter.v1`. |
| `adapter.id`, `adapter.version` | Adapter identity. |
| `invocation.argv` | Absolute argv array. The first element is the executable. The host appends `run --request <path> --out <dir>`. |
| `invocation.executable_sha256` | Lowercase SHA-256 of that executable. A relative path, a symlink executable, or a digest mismatch is refused before spawn. |
| `engine.id`, `engine.version_constraint` | Engine identity and the version constraint the adapter claims to probe. |
| `inputs` | Closed set: `ik.runtime-driver.v1`, `finite_runtime_graph`, `native_model`, `observation_trace`, `executable_driver`. |
| `properties` | Closed set: `safety`, `reachability`, `finite_scope_stuck`, `temporal`, `conformance`, `observational`. |
| `evidence` | Closed set: `sampled_execution`, `bounded_paths`, `exhaustive_finite_model`, `observed_trace`, `conformance_test`, `checked_induction_or_certificate`. |
| `requires.production_store_access` | Must be `false`. The host never passes a production store path or credentials. |
| `requires.network` | Recorded. The host does not enforce a network sandbox on this platform. |
| `result_schema` | Must be `ik.analysis.result.v1`. |

A `supports_everything` claim is refused. The manifest is a claim, not certification.

## Request (`ik.analysis.request.v1`)

| Field | Meaning |
|---|---|
| `request_id`, `run_id` | Correlation ids. The result must echo them. |
| `subject.kind`, `subject.identity.machine_id`, `subject.identity.spec_hash` | Subject identity. A different `spec_hash` is a stale subject, not the same question. |
| `goals[]` | `id`, `content_digest`, and `required`. A goal id is not enough; the content digest must match. |
| `profile_digest` | Profile content digest, echoed by the result. |
| `property_class` | One property kind for this request. |
| `inputs[]` | `path` relative to `bundle_dir`, plus lowercase SHA-256. Paths must stay inside the bundle. Symlinks are rejected. |
| `bundle_dir` | Absolute allowlisted input root. |
| `limits.timeout_seconds`, `limits.max_output_bytes` | Host-enforced bounds. `--timeout-seconds` overrides the timeout. |
| `permissions.scratch_root` | Absolute directory that must contain the fresh scratch directory. |
| `permissions.production_store_access` | Must be `false`. |
| `controls` | Adapter-interpreted strings. The host does not copy them into the process environment. |

The host hashes inputs before spawn and again after the process exits. A change during the run invalidates the result.

## Result (`ik.analysis.result.v1`)

Axes stay separate. There is no overall verdict field. A backend `ACCEPT` (or any overall verdict key) is rejected.

### Execution status

| Status | Meaning |
|---|---|
| `completed` | The planned invocation ended. It does not mean the state space was exhausted. |
| `timed_out` | The host timeout fired. The process group was killed. Not an accepted report. |
| `cancelled` | The host cancelled the run and killed the process group. Not an accepted report. |
| `tool_unavailable` | A required tool was missing. A required goal is blocked. |
| `unsupported` | The adapter does not support the question. A required goal is blocked. |
| `failed` | The invocation failed, or the host could not accept a result. |

A non-zero exit with a valid result is allowed: engines exit non-zero on counterexamples. Exit 0 without a valid result is an infrastructure error, never a pass. A killed or partial run keeps partial logs and is not an accepted report, even if `result.json` was already written.

### Finding

| Finding | Meaning |
|---|---|
| `violation` | A violating case was found for the stated scope. |
| `witness_found` | A positive witness was found. |
| `no_violation_found` | The search found no violation. Not a proof. |
| `no_witness_found` | No witness was found in the search. |
| `holds_in_scope` | The property holds in the declared finite scope. |
| `inconclusive` | The run does not answer the question. |

`sampled_execution` cannot support `holds_in_scope`. `holds_in_scope` requires `exhaustive_finite_model` and `coverage.complete: true`. `no_witness_found` with incomplete coverage is inconclusive; the host does not treat it as unreachability.

### Evidence class

`sampled_execution`, `bounded_paths`, `exhaustive_finite_model`, `observed_trace`, `conformance_test`, `checked_induction_or_certificate`.

These are not ranked by a numeric confidence score.

### Coverage

`complete` is a boolean. `reason` states why the search is complete or not. A finite budget is not a completeness claim.

### Reproduction status

`not_attempted`, `model_only`, `concrete_reproduced`, `prefix_cycle_validated`, `not_reproduced`, `mapping_incomplete`, `nondeterministic`.

`concrete_reproduced` and `prefix_cycle_validated` require an artifact inside the output directory whose JSON has `replayable: true` and a non-empty `steps` array. A witness that does not replay is not upgraded to a concrete runtime bug. A non-reproduction does not by itself erase a model finding when the claim is honest (`not_reproduced`); a claim of concrete replay that the artifact does not support is rejected.

## Requirement status

The host maps each request goal to a requirement status. It does not adopt a backend verdict.

| Status | When |
|---|---|
| `met` | Required goal, completed run, and either `holds_in_scope` with exhaustive complete coverage, or `witness_found` for `reachability`. |
| `blocked` | Required goal is missing, the execution is `unsupported` or `tool_unavailable`, or coverage is incomplete / the run did not complete. `reason` is `missing`, `unsupported`, or `incomplete`. |
| `not_met` | A finding is present and does not meet the requirement (`violation`, sampled `no_violation_found` with complete coverage, and similar). |
| `not_required` | The goal was not required. The exclusion stays explicit. |

`meets_requirement` is true only for `met`.

## Invocation and controls

The host spawns the approved argv with `std::process::Command`. It does not use a shell. The child environment is cleared; the allowlist is empty. The working directory is a fresh scratch directory under `--out`. The adapter's `--out` is `<out>/adapter`.

On timeout, cancellation, or an output overrun, the host kills the process group (`kill` of the negative process-group id) and reaps the child. It does not signal a process group after that child has been reaped.

Output paths must stay inside the adapter output directory. `..`, absolute paths, and symlinks are rejected. Stdout and stderr are bounded by `max_output_bytes` (capped by the host). The host writes `host-report.json` by a temp file and rename in `--out`.

Effective controls are recorded on the host report: no shell, cleared environment, no production store path, no credentials, no network restriction enforced, and `hostile_code_sandbox: false`. A subprocess boundary is not a hostile-code sandbox.

The host report protocol is `ik.analysis.host-report.v1`. Validated findings and requirement statuses are separate fields. `result_accepted: true` means the result document correlated and its findings were supported by the declared evidence. It is not a release acceptance.


## Hegel adapter

`examples/ik_hegel_adapter.rs` is a sampled-execution adapter. It is property-based
testing, not a proof. A completed run means the planned sample ended. It does
not mean the state space was exhausted, and it is never evidence that a
property holds in scope.

The adapter drives the real store in a fresh scratch store. Command arguments
are drawn from the machine's declared arg types (`int`, `string`, `bool`) with
the same generator and shrinker the kernel tests use. Sequences include
declared commands, stale-handle submissions, duplicate operation ids, and
reopen. The evidence class is always `sampled_execution`. Coverage `complete`
is false: a finite case and step budget is not a completeness claim.
`holds_in_scope` is not emitted.

Supported goal ids, each named in the request:

| Goal | Class | What a sample can say |
|---|---|---|
| `refusal_is_atomic` | `safety` | A refused command left authoritative state unchanged, or a counterexample was found. |
| `store_verifies` | `safety` | `Store::verify` passed after the sampled steps, or it did not. |
| `stale_capability_refused` | `safety` | A non-current capability was refused with `IK_CAPABILITY_STALE` and did not mutate, or a counterexample was found. |
| `declared_invariants_hold` | `safety` | Committed states satisfied the machine's declared invariants, or a counterexample was found. |
| `reach_terminal` / `reach_state` | `reachability` | A witness was found, or `no_witness_found` with incomplete coverage. The host marks that inconclusive, not unreachable. |

Guards, including `text_in`, are evaluated through the shared `expr::eval`
path, so a sampled command has the same truth value as the runtime. Sampling
still draws from declared argument types; it does not enumerate a character
set.

`temporal`, `finite_scope_stuck`, `conformance`, and `observational` are
`unsupported`. The adapter does not answer them with a weaker safety sample.

On a violation the out directory receives the Hegel reproduce blob under
`native/` and the shrunk command sequence as an `ik scenario` v1 document at
`findings/<goal>/scenario.json`. `concrete_reproduced` is set only after that
scenario is replayed through the scenario driver in the same adapter run.
Otherwise the status is `model_only` or `not_reproduced`.

`controls.law_mutant` selects a test-support law mutant (for example
`SkipInvariantValidation`). That field is compiled only into an adapter built
with `--features test-support`. Without that feature the same field is
`unsupported`; the adapter does not silently run the unmutated driver.
`controls.disable` skips named goal checks so a mutant can be shown not to
fail every property.

An always-refuse machine does not fail `refusal_is_atomic` or
`stale_capability_refused`. Every generated command is refused and the
authoritative projection is unchanged, so refusal atomicity is
`no_violation_found` on exercised refusals, not a vacuous failure. The driver
also submits a non-current generation; the runtime refuses it with
`IK_CAPABILITY_STALE` and commits nothing, so the stale property is exercised
and is not a failure. Nothing commits, so no retained handle becomes stale;
that absence is not itself a violation. `reach_terminal` on the same machine
is `no_witness_found` with incomplete coverage. The host marks that
inconclusive, not unreachable.

Build the adapter as an example so the packaged `ik` binary does not link the
engine. A manifest template is `tests/fixtures/hegel/manifest.json`; pin
`invocation.argv` and `executable_sha256` to the built example before
`ik analysis run`.
## TLC over an exported finite graph
`ik graph export` walks one machine with the existing explorer and writes `ik.graph.v1`. It does not start a second explorer.

```text
ik graph export <machine.ik.toml> [--corpus cases.json] [--max-nodes N] [--max-depth D] --out graph.json
```

Defaults are 4096 nodes and depth 16. The document names machine_id, spec_hash, corpus_digest, bounds, complete, incomplete_reason, the initial node, and:

| Field | Meaning |
|---|---|
| nodes | Stable id (n plus the canonical state digest), state label, canonical facts JSON, terminal flag. |
| edges | from, command, case name, args, outcome, to. |
| frontier | Nodes whose outgoing commands were not fully expanded because of a bound. |
| restrictions | Declared input-domain limits. A command that needs arguments and has no corpus case is no_corpus_case. A command limited to named cases is corpus_cases:names. |

outcome is committed, noop, or refused:CODE. Refusals and no-ops are self-loops. A committed edge is never given those outcomes. A depth or node cut is complete false. An edge that was evaluated is kept. An edge that leaves the declared input domain is a restriction or a frontier node, not a silent drop.

The TLC adapter is adapters/tlc/ik_tlc_adapter.py. It implements describe and run --request R --out D. Register it with an absolute argv whose first element is that script and whose digest is the script SHA-256. Pass --toolchain toolchain.json in that argv when the pin is not the file beside the script. The toolchain file names the Java binary, the TLC jar, the jar SHA-256, and the expected version string. The adapter probes java -cp JAR tlc2.TLC and refuses a digest or version mismatch. It does not download a tool. A missing Java binary or jar is tool_unavailable.

The staged pin is Java /opt/homebrew/opt/openjdk/bin/java and tla2tools-1.7.4.jar (SHA-256 936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88), which probes as TLC2 Version 2.19 of 08 August 2024 (rev: 5a47802). The shipped toolchain.json expects the jar at ~/.ik-toolchain/tla2tools-1.7.4.jar (a leading ~ is expanded); the tests honor IK_TLC_JAR to point elsewhere and fail, rather than pass vacuously, when the toolchain is absent.

The shipped toolchain.json names a macOS Homebrew Java path, so on any other machine tests/tlc_adapter.rs fails until IK_TLC_TOOLCHAIN names a toolchain file for that machine. `sh scripts/ci-tlc-toolchain` writes one: it uses the Java on PATH (or IK_JAVA), downloads the pinned jar into ~/.ik-toolchain (or IK_TOOLCHAIN_DIR) when absent, refuses a jar whose SHA-256 differs from the pin, and prints `IK_TLC_TOOLCHAIN=` and `IK_TLC_JAR=` lines to export. It installs no Java. The CI workflow runs it on macOS and Ubuntu before `scripts/public-verify`.

The adapter reads one ik.graph.v1 input. It writes a TLA+ module whose nodes are an enumerated set, whose Next is the committed-edge relation plus stuttering on the declared TERMINAL set, and runs that module under TLC. Native stdout, stderr, IkGraph.tla, and IkGraph.cfg are kept under native/.

controls selects the question. property_class must match.

| Question | property_class | Meaning |
|---|---|---|
| safety_invariant | safety | controls.predicate holds in every reachable node. |
| reach_terminal | reachability | Some declared terminal is reachable. Optional target_label. |
| reach_state | reachability | target_label is reachable. |
| stuck | finite_scope_stuck | A non-terminal, non-frontier node has no committed outgoing edge. |
| temporal or liveness | temporal | execution_status unsupported. Not checked. |

Predicate language, a conjunction with &&:

```text
true
label == "NAME"
label != "NAME"
fact.NAME == JSON
fact.NAME != JSON
node != ID
```

Safety and stuckness are TLC invariants over the node-id set that violates the question. Reachability asserts that the target is never reached; a TLC violation is the witness. A counterexample is written as ik scenario v1 JSON at findings/goal/scenario.json. When every step has a corpus case and controls.ik_bin plus controls.machine are set, the adapter replays that scenario with ik scenario run. concrete_reproduced is used only when that replay ran and the committed state labels matched. Otherwise the status is mapping_incomplete.

TLC does not re-evaluate guards. A `text_in` refusal appears only as a
refused edge that the explorer already classified with the shared evaluator.

Results are MODEL-level evidence for the exported finite graph, the restrictions, and the exact tool versions. They are not a proof of the executable runtime, and they do not cover inputs omitted from the graph.

| Graph | TLC | Evidence | Finding |
|---|---|---|---|
| complete | exhaustive, property holds | exhaustive_finite_model | holds_in_scope |
| complete | violation or reachability witness | exhaustive_finite_model | violation or witness_found |
| incomplete | no counterexample in the fragment | bounded_paths | inconclusive or no_witness_found, never holds_in_scope |
| incomplete | counterexample inside the fragment | bounded_paths | violation or witness_found, coverage not complete |

The adapter recomputes the committed-edge reachable set from graph.json and compares it with TLC distinct-state count on a completed run. A generated Next that drops an edge fails that check. The result is failed, not holds_in_scope.

Hegel remains sampled execution and never supports holds_in_scope. A completed TLC run is model-level evidence for that finite graph only.



## SMT-LIB2 adapter

`adapters/smt/` is an optional pinned Z3/cvc5 runner. It never downloads a
solver. Digest placeholders in `toolchain.json` and
`manifest.template.json` are not pins; copy the template, install solvers
offline, and fill absolute paths plus SHA-256. A digest mismatch is refused.
Missing binaries are `tool_unavailable`. `ik check` uses this adapter only
when `IK_SMT_ADAPTER` is set; otherwise check output is unchanged. See
`docs/smt-backend.md`.

## Authoring an adapter

An adapter is added by copying a directory and registering a manifest. No
kernel, store, or transaction code changes. `adapters/starter/` is a working
example: a Python 3.9 standard-library checker over one `ik.graph.v1` graph
from `ik graph export`. Its property family is `dead_end_count` (non-terminal
nodes that are not on the export frontier and have no committed outgoing
edge). A closed graph uses `exhaustive_finite_model`. An incomplete graph uses
`bounded_paths`. Any other question is `unsupported`.

Four steps, also in `adapters/starter/README.md`:

1. Copy `adapters/starter/`.
2. Edit the manifest template: absolute argv, SHA-256 of the executable, and
   the adapter id. The template digest is a placeholder, pinned at install.
3. Implement the property function. Keep the `controls.conformance_fault` hook
   so timeout and cancellation fixtures still run. Report `unsupported` for
   every question the function does not answer.
4. Run `ik analysis conformance <manifest.json> --fixtures <dir>`.

The command lists each fixture as pass, fail, or skipped, with a reason. Exit
1 if any fixture fails. Exit 2 if the manifest or the fixture directory is
refused before the battery. Passing proves host-contract conformance, not
engine correctness.

The host builds each request and uses the same spawn and result checker as
`ik analysis run`. Five rejection fixtures cannot be produced by an adapter
that also passes the positive cases, so the host injects them after the
process exits and judges the mutated `result.json` with that same checker.
The fixture field is `fault`:

| `fault` | Host action | Rejection |
|---|---|---|
| `wrong_request_id` | Rewrites `request_id` | `request_id_mismatch` |
| `stale_subject_hash` | Rewrites `subject.identity.spec_hash` | `stale_subject_hash` |
| `malformed_output` | Replaces the result with `{` | `malformed_json` |
| `path_traversal` | Adds an artifact `../escaped.txt` | `path_traversal` |
| `exit_without_result` | Deletes `result.json` and judges exit 0 | `exit_without_result` |

`timeout` and `cancel` are not injected into the file. The host sets
`controls.conformance_fault` and enforces the limit. The starter writes its
result and then sleeps, so a killed run still had a result on disk. The host
must not accept it.

`adapters/starter/mapping.example.json` maps a graph node and a committed edge
onto `dead_end_count`. A source label is not a node id. A refusal is not a
continuation. A frontier node is not a dead end. Zero dead ends on an
incomplete graph is inconclusive, not unreachable.

## P route

STATUS: prepared, not qualified; no real P run exists.

`adapters/p/` is a registration template, a mapping example, and parser
fixtures. It is not a qualified checker. The version probe is `p --version`.
When that command is absent, `adapter.py describe` reports `tool_unavailable`
and does not spawn `p` or `dotnet`. `ik analysis describe` on the manifest
reads the document only; it does not execute the adapter or P. `run` writes
`tool_unavailable` and does not start a checker.

`adapters/p/mapping.example.md` maps one Desmo named command to a P event.
Default P delivery is a reliable FIFO queue. `send` enqueues; it does not run
the handler in the sender's turn. A monitor that is still hot at the end of a
finite schedule is not a proved liveness failure. A schedule bound is
`bounded_paths`, not an exhaustive proof of the Desmo machine. Default P
does not stand in for network loss or reordering unless the P model states
that behavior. Parser fixtures under `adapters/p/fixtures/` cover a bug line,
a bounded pass, and a hot monitor. A bounded pass is not `holds_in_scope`.
These fixtures do not qualify the route.
