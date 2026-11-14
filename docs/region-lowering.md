# Region lowering

Schema 2 is the region-authoring frontend for `.ik.toml` machines. It lowers
into the schema-1 `Definition` that the existing compiler and runtime execute.
There is one canonical machine, not a chart interpreter alongside the kernel.
Hosts still request named commands and args, never destination states
(CORE_LAWS 1–2). PDK describes architecture; it is not the machine language.

The region frontend is additive in the `0.4.1` source candidate after `0.3.0`.
Protocol remains `1.0` and persisted store schema remains `3`; authoring schema
`2` is unrelated to the historical refused store schema `2`. Binary, Python
client/worker, and Go adapter runtime versions must still match exactly.

## Execution contract

Execution is **deterministic, named, command-scoped**. A local command changes
one region and preserves every sibling. An explicit joint command can change
several regions atomically. Equal command names in different regions do not
broadcast: they are refused as ambiguous ownership. There is no SCXML event
broadcast, run-to-completion loop, implicit transition cascade, history,
delayed transition, or invoked actor.

A region has leaf states and optionally one nonrecursive compound parent.
The parent groups existing leaves; it is not another active state. Local
commands use schema-1 command syntax. Parent commands use the parent name in
`from`; lowering expands it to its children.

For a child and parent command with the same name, applicable child outcomes
are evaluated first, in authored order. If all child guards are false, parent
outcomes are the fallback. A guard evaluation error refuses the command; it is
not false and does not trigger fallback. An unconditional last child outcome
suppresses the parent at that child source. Child and parent `args` and
`effect` declarations must have exactly equal serialized values, not merely
compatible types or similar meaning. A command name otherwise has exactly
one owner across all regions and the top-level joint-command map.

## Compound example

Save this complete example as `compound.ik.toml`:

```toml
schema_version = 2
machine_id = "compound"
machine_version = 1
name = "Compound example"

[regions.power.states.grinding]
initial = true
[regions.power.states.brewing]
[regions.power.states.ready]
[regions.power.states.off]
terminal = true

[regions.power.commands.finish_grinding]
from = ["grinding"]
to = "brewing"

[regions.power.commands.finish_brewing]
from = ["brewing"]
to = "ready"

[regions.power.parent]
name = "on"
children = ["grinding", "brewing", "ready"]

[regions.power.parent.commands.power_off]
from = ["on"]
to = "off"
```

`power_off` is available at all three children without three authored copies.
A local command cannot source a terminal leaf. The aggregate terminal state
has no commands.

For conditional child override syntax, a child command can instead contain:

```toml
[regions.power.commands.power_off]
from = ["brewing"]
outcomes = [{ when = 'region_power == "brewing"', to = "ready" }]
```

Here the child handles `power_off` while brewing; the parent still handles it
at grinding and ready. To make fallback conditional on domain data, declare a
normal boolean fact and use it in the child's guard. Parent fallback occurs
only when every applicable child guard evaluates successfully to false.

## Parallel regions, joints, and invariants

Save this complete example as `parallel.ik.toml`:

```toml
schema_version = 2
machine_id = "parallel"
machine_version = 1

[regions.playback.states.playing]
initial = true
[regions.playback.states.paused]

[regions.playback.commands.pause]
from = ["playing"]
to = "paused"

[regions.volume.states.unmuted]
initial = true
[regions.volume.states.muted]

[regions.volume.commands.mute]
from = ["unmuted"]
to = "muted"

[commands.pause_and_mute]
from = { playback = ["playing"], volume = ["unmuted"] }
to = { playback = "paused", volume = "muted" }

[invariants.audible_playback]
assert = 'region_playback != "playing" || region_volume != "muted"'
```

`pause` preserves volume. `pause_and_mute` changes both regions in one
transition. `mute` while playing proposes an invariant violation: the kernel
refuses it rather than committing an unsafe state. The region checker reports
that reachable invariant-refusal witness and gives an overall `REFUSED`.
The invariant is not a claim that the unsafe combination was committed.

Top-level commands are joints. Their `from` maps region names to nonempty
leaf-source lists; `to` maps region names to target leaves. Missing source
regions are wildcards; missing target regions remain unchanged. Source and
target maps must be nonempty, and unknown regions or leaves are refused.
Joint commands accept `when`, `then`, `args`, and `effect`, or ordered outcomes
instead of the single-target sugar, for example:

```toml
[commands.quiet]
from = { playback = ["playing"] }
outcomes = [
  { when = 'region_volume == "unmuted"', to = { playback = "paused", volume = "muted" } },
  { to = { playback = "paused" } },
]
```

As with schema 1, use either `to`/`when`/`then` or `outcomes`, not both.
Each outcome has optional `when` and `then` and a required target map.
`args` declares `int`, `string`, or `bool` argument types; `then` uses the
existing assignment-expression syntax. `effect` uses the existing `kind`,
`id`, optional `payload_digest`, and optional `idempotency_key` fields.
No new effect-execution semantics are introduced by regions.

A joint may not explicitly list a terminal leaf as a source. Even with a
source wildcard, outcomes that target any currently terminal component are
excluded, including same-leaf assignments; omitted terminal siblings remain
unchanged.

## Names, expressions, and resource boundary

- `schema_version = 2`, `machine_id`, and `machine_version` are required;
  `name` is optional. Authored `facts` and `invariants` default to empty maps.
- Region, leaf-state, and parent names are ASCII identifiers
  (`[A-Za-z_][A-Za-z0-9_]*`) without `__`. A parent is distinct from its
  leaves and lists nonempty, unique, known children.
- Each region has exactly one initial leaf, which is nonterminal. A flat
  state is terminal exactly when all its components are terminal.
- Regions are ordered lexically by `BTreeMap`; flat state names join the
  corresponding leaf names with `__`.
- The frontend checks product sizes with checked multiplication and explicitly
  refuses overflow or more than the configured `region_product_bound` product
  states, reporting all region dimensions and the bound. The default is **64**.
  This check precedes allocation of the Cartesian product. Parsing and other
  frontend bookkeeping still allocate. The bound is a frontend resource limit,
  not a CORE_LAW, runtime semantic rule, or CLI tuning flag. A caller that owns
  an appropriate resource policy may use `parse_source_with_limits` and
  `RegionLimits`; increasing the bound also increases eager product allocation.

The default bound remains 64. A caller-selected `RegionLimits` bound of 72 can
accept a 65-product source while retaining the same checked-multiplication
overflow refusal before Cartesian expansion.

After generating outcomes, lowering folds only authored guards contradicted by
the source product's `region_<name>` assignments. Unknown domain facts and
arguments are retained; cross-fact relations and arithmetic are not inferred.
It then performs corpus-independent breadth-first reachability over surviving
product edges from the unique initial product. Unreachable products and their
projection invariants, sources, and outcomes are removed. `RegionMap` retains
corpus-independent diagnostics naming pruned guards, joint source restrictions,
and product assignments; these are rendered separately from corpus-dependent
check evidence.

A false-folded guard removes its edge/outcome but keeps the source product in
the command's source set when that command has another surviving outcome at a
reachable source. Runtime then reports no applicable outcome
(`IK_INVARIANT_FAILED`), not `IK_ILLEGAL_STATE`. If no outcomes survive at any
reachable source, the dead command and its sources are removed before
flat-definition validation. The verification plan (`schema_version` 2) records
those refusal-only sources as discharged command/source impossibilities
instead of demanding a commit. Lowering records an origin map from each
generated copy to the authored coordinate it implements (parent fallback and
child override stay distinct) and keeps authored coordinates even when no
executable copy survives. Generated product-context obligations may be
discharged by a sound impossibility proof with an explicit basis; authored
behavior is satisfied only by a kernel witness of some remaining copy.
Runtime guards, first-match order, and legal sources are unchanged.

Lowering generates string facts `region_<name>` containing the active leaf
and updates them with transitions. Every lowered outcome also guards source
agreement: each `region_*` fact must equal its product component. Generated
hard poststate consistency invariants require those facts to agree with the
canonical flat lifecycle state; decoding refuses mismatches too.
All authored fact and invariant names beginning `region_` are reserved;
authored assignments to generated facts are forbidden.

Use `region_playback == "playing"` in guards and invariants. Schema-2 authored
expressions cannot use the `state` operand: refusal is structural on the
expression AST, not textual substitution of local names. Comparing a region
fact with an unknown leaf string is refused. Cross-region conditions use the
generated facts, not hand-built product names or parent names as active leaves.

## Loader and analysis APIs

`declarative::region::parse_source(&str)` returns
`Result<SourceDefinition, Vec<String>>` and uses `RegionLimits::default()`.
`parse_source_with_limits(&str, RegionLimits)` exposes the eager
`region_product_bound` policy. `SourceDefinition` contains the lowered
`definition: Definition` and `regions: Option<RegionMap>`.
`parse_definition`, `parse_and_compile`, `load_definition`, and `load_def`
dispatch through this parser. Schema 1 bypasses region lowering; its
strict deserialization, semantics, and `spec_hash` are unchanged.

Schema-2 `spec_hash` is computed after guard and product pruning; affected
lowered definitions can receive new identities. The hash does not depend on
corpus data, exploration bounds, or the caller-selected product limit. Store
compatibility for affected identities must be assessed before release.
Schema-2 configurations whose initial state/facts violate an invariant are
refused during parsing, using the same shared `evaluate_invariants` logic as
the runtime. This does not change schema-1 behavior.

`RegionMap::decode(&Value)` returns a borrowed `BTreeMap<String, String>` or a
string error. `RegionMap::mermaid(&Definition)` derives a region chart from
the lowered definition. Neither is an alternate execution authority.
The source maps are private derived metadata accessed through decoding and
projection, not mutable public region-state authority.

`RegionMap::project_path(&CompiledMachine, &[WitnessStep], &CaseCorpus)`
returns `Result<Vec<RegionStep>, String>`. It replays authoritative
`decide`/`apply` and reduction, checks outcome indices, and resolves corpus
cases. Each step includes `command`, optional `case`, `before`, `target`,
`committed`, and optional `diagnostic`. For a refused step, `target` is only
a proposal, not a committed after-state. A refused final step is permitted;
steps after refusal are errors.
`RegionMap::project_invariant_witness` additionally verifies both the final
diagnostic code and invariant rule, rather than accepting an unrelated refusal.

`check_source(&SourceDefinition, Option<&Definition>, &CaseCorpus,
&[String], Bounds)` returns `Result<RegionCheckResult, String>`, including
`verdict`, `text`, `flat: CheckResult`,
`invariant_refusals: Vec<InvariantRefusalWitness>`, and
`exploration_complete`. The shared core traversal captures the first
invariant-refusal witness per rule. The shared check pipeline includes those
refusal reasons in the specialized flat judgment, so `flat`, overall verdict,
and text agree; this is not a CLI-only verdict override. Overall verdict is
`REFUSED` if the flat check refuses or any such witness exists.

## Check output and identity

```sh
ik identity compound.ik.toml
ik check compound.ik.toml
ik check parallel.ik.toml
ik check parallel.ik.toml --corpus cases.json --baseline validation/verification_baseline.json
ik check machines/invariant_probe.ik.toml
```

These are usage examples, not a record of executed validation.
`invariant_probe` intentionally violates an invariant: its expected check
verdict is **REFUSED**, not ACCEPT. Its schema-1 identity hash is unaffected
by introducing schema 2.

`ik check` prints the overall `VERDICT` before derived region paths and the
Mermaid chart. The chart is explanatory output, not an input to the verdict
or a second editable specification. Exit status remains 0 for ACCEPT, 1 for
REFUSED, and 2 for transport/usage failure. Invariant-refusal paths explicitly
distinguish proposed targets from committed states.
The chart is a lossy structural view: guards, invariants, facts, and joint
atomicity are not shown.

For region checks with unresolved obligations, `UNRESOLVED REGION OBLIGATIONS`
lists uncovered and falsified obligations by command when a command
coordinate exists. Command groups include source products decoded from flat
labels as `region=state` assignments; state-only obligations likewise identify
their product. Each row prints the stable obligation ID and status. Invariants
and other coordinate types use stable named groups. This section is omitted
when there are no unresolved obligations; it supplements rather than replaces
`REASONS` and `NEW SEMANTIC DEBT`.

`CounterexampleV1` is unchanged: flat `minimal_steps` and `failure_class`
remain the replay contract. Region paths are derived from the same lowered
machine and authoritative replay. `spec_hash` identifies the lowered
schema-1 definition; the separate authoring hash is metadata, not part of
`MachineIdentity`. Journal replay binds to the executed definition.

An ACCEPT is bounded and corpus-relative. It does not prove safety for all
possible argument values or paths beyond the exploration bounds, and must
not be described as exhaustive merely because the reported verdict accepts.
Use `exploration_complete` together with the supplied bounds and corpus to
interpret coverage. “Complete within the supplied finite corpus” is not
exhaustive: it proves nothing about arguments absent from that corpus.

## Validation scope

The suites and finite examples are evidence for the exercised behaviors,
not an unbounded state-space certification.

## Risk-specific normative evidence map

For the `0.4.1` candidate, release qualification MUST address each risk below.
The map names implemented evidence contracts and their declared finite domains;
it is not a report that the integrated source has passed them. The release owner
MUST record results against the actual integrated source and then against the
packed bytes before claiming qualification. General suite counts, earlier
old-base green runs, and projections derived from the candidate's own lowered
outcomes MUST NOT substitute for the independent or durable evidence named here.

| Risk | Required evidence and finite scope | Evidence location |
|---|---|---|
| **RA01 — wrong region execution** | Independent expected relations MUST drive real CLI requests and read-only SQLite inspection, not candidate-derived expectations. The declared domains contain **183 cells**: **15 parallel** (3 safe playback/volume configurations × 5 local/joint commands), **20 compound** (4 power leaves × 5 command/guard cases, including an unavailable guard fact), **20 terminal-sibling** (4 left/right configurations × 5 commands), and **128 three-region** (4 power × 2 volume × 2 audit configurations × 8 command/argument cases). Each cell MUST check commit/refusal, resulting state, generation/capability, journal/receipt persistence, replay, and verification as applicable. This is coverage of these finite relations only, not arbitrary machines or arguments. | `tests/region_contract.rs`: the four `every_*_matches_the_durable_relation` / terminal-sibling relation tests and their CLI/SQLite cell helpers. |
| **RA02 — unsafe commit or misleading witness** | An invariant-refused real CLI command MUST preserve public state, generation, capability, and authoritative SQLite rows. Authoritative witness replay MUST distinguish the proposed unsafe target from committed state and validate the invariant diagnostic. **Four tamper controls** MUST reject a wrong rule, a missing rule, a wrong code with the correct rule, and both wrong code and rule; successful path replay alone is insufficient. | `tests/region_contract.rs::cli_unsafe_region_command_preserves_durable_state_and_capability`; `tests/region_frontend.rs::reusable_check_reports_invariant_refusal_and_validates_witness_classification`. |
| **RA03 — schema-1 identity drift** | Expected schema-1 identities MUST come from independently validated accepted `v0.2.1` artifact bytes, not regenerated candidate expectations. Direct schema-1 parsing, source parsing, file loading, and real CLI loading MUST match the preserved hashes for `verification_attempt`, `invariant_probe`, and `effect_probe`. Accepted provenance is source `568525461ed4b73cc4874e2735cc0786bc23781c`, darwin-arm64 artifact SHA-256 `81edee455b8d139210508e1f12e525540f043deba165f6bc824969b7f24c79a5`. This is preservation evidence, not a region-semantics oracle. | `tests/region_contract.rs::accepted_schema1_identities_survive_direct_source_file_and_cli_loading` and its `ACCEPTED_SCHEMA1` hashes. |
| **RA04 — malformed authoring or unbounded expansion** | The **161-case causal negative corpus** MUST run through the actual CLI and require the diagnostic caused by each forbidden declaration, not merely any parse failure. Actual CLI checks MUST also accept the **64-state** boundary, refuse **65 states**, and refuse a huge theoretical product (**128 two-leaf regions, `2^128` configurations**) before Cartesian expansion; an unrelated poisoned-identifier refusal cannot satisfy that last obligation. These are declared malformed-input and product-boundary domains, not exhaustive malformed TOML coverage. | `tests/region_frontend.rs::malformed_authoring_is_refused_before_runtime` and `product_bound_accepts_64_and_refuses_65`. |
| **RA05 — invalid initial or corrupted canonical state** | An initially unsafe source submitted to actual CLI `create` MUST leave **zero authoritative writes**: no aggregate/genesis, receipt, or capability may be minted. With **3 source-state corruptions** (each generated region fact and the flat lifecycle label), real CLI commands MUST refuse without healing or writing; verification MUST detect corruption and explicit replay may repair the projection without changing journal/receipts. With **3 poststate corruptions** (the same two generated facts and lifecycle label), the actual CLI MUST refuse before durable commit. | `tests/region_contract.rs::cli_initially_invalid_region_source_cannot_create_any_durable_row`, `cli_corrupted_durable_source_facts_and_labels_refuse_without_healing_or_writing`, and `cli_corrupted_lowered_poststate_facts_and_labels_refuse_before_durable_commit`. |
| **RA06 — source-only success hiding broken artifacts** | Qualification MUST extract the checksummed release `ik`, install the actual sdist-derived wheel offline into a fresh environment, and exercise installed Python and the real Go client against that extracted binary. It MUST bind manifest versions, source revision, and artifact digests to those bytes. The installed Python probe also checks persistent region-session transactions: an unsafe final step rolls back earlier safe steps without durable mutation, and the original capability commits the safe batch. Source tests do not discharge this obligation. | `scripts/release`, `scripts/verify-release.py`, and `python/tests/test_release_artifacts.py`; the release owner records packed-byte results and source acceptance in the corresponding GitHub Release. |

The installed probe takes transactional SQL-query-only snapshots. Its SQLite
connection may create WAL/SHM bookkeeping, but cannot create a missing store
or execute authoritative SQL writes; file-level read-only opening alone fails
before the first query on the exercised macOS SQLite.

## Chart as the authored structure model

`Chart` is the authored structure model: regions, compound groups (parent
handlers counted once), joints, and the edges a human wrote. The compiler still
lowers that structure into one schema-1 `Definition`. The flat definition is the
compile and runtime target; `spec_hash` is the digest of that definition and is
unchanged by attaching a chart. The runtime does not interpret the chart.

After parse, `check_consistent` compares the chart to the lowered definition.
A mismatch refuses with `IK_CHART_INCONSISTENT`. Three invariants hold:

1. every authored edge maps to at least one flat transition;
2. every live flat transition maps to exactly one authored edge;
3. recovered keys round-trip equal the expected set (authored keys on a flat
   machine; lowered keys on a hierarchical one).

`ik diagram` reads the chart, not a reconstructed flattening of parent
handlers. That is the edge-conservation guarantee of the command: parent
commands appear once. A back-edge to an earlier state is drawn as a return
rail in a left gutter: it leaves the row of its annotation and enters the
target state's row with an arrowhead, and the annotation
(`↺ cmd → [ target ]`, `↗ cmd → [ target ]`) stays on the row that names
the command, so each command is still labelled exactly once. At most two
rails are drawn; rails must be disjoint or strictly nested (the inner one
sits next to the river). A back-edge that would be a third rail, cross
another rail or share a row with one keeps its annotation without a rail.
Diagrams without back-edges have no gutter and are unchanged.

## Qualification boundary

Observations on exercised source paths qualify those paths only: not packed artifacts, and not
unbounded language semantics. Artifact qualification is a separate release step.
