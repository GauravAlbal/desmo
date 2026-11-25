# Changelog

All notable consumer-facing changes are documented here. Format:
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning:
[SemVer](https://semver.org/).

Protocol major is independent of package SemVer. A protocol-major bump is
always a SemVer major. See [docs/release-compatibility.md](docs/release-compatibility.md).

## [Unreleased]

## [0.5.1] - 2026-11-24

### Added

- `ik simulate <machine.ik.toml> [--out <file.html>]` (and `desmo simulate`) renders a zero-dependency, self-contained interactive HTML structural simulator directly from `ik schema`, including legal/illegal command refusal (`IK_ILLEGAL_STATE`), terminal locks, and Copy/Fire Stale Capability Handle (`IK_CAPABILITY_STALE`).
- `skills/desmo/audit.md`: read-only brownfield audit workflow (`investigate_only`) compatible with `npx skills add GauravAlbal/desmo` to discover implicit state machines, extract a scratch `.ik.toml`, run `ik diagram` + `ik check`, and generate an interactive simulator with `ik simulate`.
- `install.sh`: POSIX one-line `curl` installer for prebuilt macOS arm64 and Linux x86_64 `ik` binaries with strict `SHA256SUMS` verification.
- `examples/orders/` (`order.ik.toml`, `order.ik.cases.json`, `demo.py`, `README.md`) and `desmo demo orders` CLI command.
- `docs/assurance.md`: transparent verification and assurance boundary table covering tests, deleted-wall mutants, crash-matrix tests, linearizability, SMT/TLC adapters, and open Kani entrypoint proofs.
- TypeScript SDK typed error exports (`IllegalStateError`, `CommandInvalidError`, `InvariantFailedError`, `NonceInvalidError`, `MachineError`).
- Bounded Kani proof harness for capability generation and nonce validation, plus optional entrypoint authority-prefix Kani harnesses (full entrypoint qualification remains open).

### Fixed

- `ik check` now lists uncovered obligations (`UNCOVERED OBLIGATIONS`, up to 20 IDs plus remainder) on flat schema-1 refusals without `--prior`.
- Declarative `then` binding parser/compiler now reports actionable spelling hints on boolean/literal errors and checks `$arg` type mismatches statically.
- `desmo skill install` now copies all 7 skill files including `readout.md` and `audit.md`; unknown `desmo` subcommands now exit with code `1`; `desmo init` generates an idempotent `job.mjs` and documents GitHub Release installation; `desmo doctor` surfaces runtime/package version mismatches.
- `scripts/public-verify` now cleans its temporary scenario output directory so repeat runs succeed cleanly.
- `ik --help` updated to list all envelope ops (`create_with_command`, `transaction`, `lookup_operation`), `ik check --corpus-template`, and `ik simulate`.

## [0.5.0] - 2026-11-14

### Changed

- The product is now named Desmo (it was named Ikaros). The npm package is `@desmo/runtime`, the Node command is `desmo`, the agent skill directory is `skills/desmo` (installed at `.agents/skills/desmo`), and human-readable output such as the handoff first line (`Desmo <kind>: <subject>`) and the claim sentence says Desmo. The `ik` binary, `.ik.toml` files, `IK_*` variables, the crate and Python package names, schema identifiers (`ikaros.*.vN`), the trace `product` value and the `ikaros-readout` stamp marker are unchanged, so existing machines, stores and bundles keep working. A readout stamp written for the old claim wording is stale and must be rewritten.

### Added

- Every handoff carries a sanctioned claim: one sentence (JSON `claim`, and a `Claim:` line under `Scope`) saying what was checked, how many rule acceptances are awaited and how many material definition gaps are listed, and that the criteria's validity, execution fidelity and readiness are not claimed. `ik readout` gains a `claim_not_quoted` vector: a summary must quote it.

### Fixed

- `ik readout` accepts a recovery handoff, which has no `Status:` line (it was refused with a usage error); it now requires only the `Not checked:` paragraph. A machine-level gap (no invariants) counts as named when the summary says `invariant`, not only when it repeats the kind id `no_invariants`. Found by linting six agent-written summaries over the three real handoffs.

### Added

- `ik readout <summary.md> --handoff <handoff.txt> [--stamp] [--json]` lints the prose an agent writes around a handoff. Nine named defect vectors (readiness words, completeness claims, proof words, a `>` quote that is not verbatim handoff text, the `Status:` line or `Not checked:` paragraph not quoted, a material gap not mentioned, a stale or hand-written stamp) each print the line, what is wrong and the fix. Exit 0 clean, 1 defects, 2 usage. `--stamp` writes a hash line (summary and handoff sha256) only when the lint is clean; the hash binds content, not author, and a clean lint does not say the summary is true. Procedure in `skills/desmo/readout.md`.

### Changed

- `ik handoff` now asks the decisions the product can compute instead of printing `Decision needed: none`. Every command with an authored guard is a rule in a ledger (a hierarchical machine is read from its authored regions and joints, not the flat lowering) with a hash of its guard text and the moves it applies to. A delivery handoff (and, for a change, each new, changed or dropped rule) shows `Status: DECISION REQUIRED` until the behavior file lists `accepted_rules` entries (`{rule, guard_hash, examples_hash}`) matching the current hashes. A changed guard, moves or example set makes an acceptance stale and asks again. A change handoff also asks the version policy for existing instances (keep them on the old version until they finish, or drain and recreate on the new one) and requires `why` (`{text, source}`), which the handoff quotes as authored and never writes; a change handoff without `why` is refused with exit 2. `rule_witnesses` (examples with a stated `accepted` or `rejected` outcome) are required for each rule that needs acceptance and have exactly one distinct guard: the handoff refuses (exit 2) unless every stated outcome matches the guard, every atomic clause is true in one example and false in another, and every one-step variant of the guard (flipped comparison, boundary or constant moved by one, `text_in` length bound moved by one, negated existence test, dropped clause) changes some example's outcome; a variant that is the same rule is declared in `equivalent` with a reason and shown to the human. These checks apply to declarative machines; with no declarative definition in the evidence the handoff says rule fitness and version policy are not established. The handoff JSON gains `decisions`, `decision_status` and `why`; `ik.diagram.v1` gains `rules` and the change versions.
- Every markdown table renders through one helper that pads columns to the widest cell by display width, wraps long cells onto continuation rows within 120 columns, and never wraps a header. The rules table keeps its given/stored gloss on a line above the table.
- The recovery view lists only events the evidence supports (request sent, reply lost, committed, asked again, and plan or saved only when the evidence names them), not a fixed checklist; the table and JSON field are `timeline`, replacing `checkpoints`. A recovery handoff states that whether the caller saved the operation id before sending is not established.
- The sentence about existing instances now says they stay on their version and that moving one in place is not supported. It replaces "needs an explicit migration which is not performed", which implied a migration was available.
- The rule decision no longer tells a human to copy hashes into the behavior file. It says to tell the agent accept or reject, that the agent records the decision against the displayed content, and that accepting says the behavior is what the business wants, not that a release is approved. A recovery handoff whose lookup found the earlier commit adds "To change the item again": read the item's current state for a fresh handle; the lookup result is history, not a handle.
- Every handoff states its frame. A `Scope` block says what this kind of handoff asks of the evidence, phrased as a question so it cannot be read as a result (delivery: whether the state transitions behave as the definition says, within the cited evidence; change: that, and only where continuity evidence is cited, whether existing instances keep their version; recovery: whether a recorded operation exists in the store under the given id and request), what it does not check (that the criteria are valid, that the code and effects performing a transition do what is intended, that the definition says what was needed, anything the requirement does not state), that execution is assumed perfect and reliable, and that readiness is the owner's decision and the report does not state it. A `Who attested what` table lists requirement source, rule acceptance, independent review, approver identity and parties in roles as an expected party and what the evidence records; `not recorded` is the only gap wording, and a cleared rule decision reads "a rule acceptance, not a release approval". It records contributions and never judges them. The handoff JSON gains `scope` and `duties`, and `readiness` is now `not_stated` instead of `not_established`; the first block no longer carries a readiness line.
- Criterion origins. Each decision point of a rule under review (a comparison operator, a constant, or the character set and length bounds of a `text_in`) has an origin: `stated` with a span quoted verbatim from a pinned `requirement` `{text, source, provenance}`, `chosen` by the author, `default` with its identity, or `unrecorded`. The behavior file gains `requirement`, `origins` and `acknowledged_origins`; the handoff prints `Where each criterion came from` and a `Criteria origin` row with counts, and JSON `origins`. An owner's acknowledgment never turns a chosen point into a stated one. The handoff refuses an origin that fails a necessary condition (span not verbatim, literal or operator phrase absent, default without identity, unknown point or rule). Ikaros does not judge whether a span means what the author says. An operator is stated only by a phrase that names that operator (whole words, longest phrase first). One current origin per point; a stale one reads unrecorded; an acknowledgment needs a current chosen origin. The requirement reads `author-stated, not verified` and carries a text digest. The enumeration is closed and stated: existence tests, connectives, reducer code, effects, rules with several guards (listed as skipped) and absent rules are not enumerated, and the scope sentence prints even when nothing is enumerated. Origins are not part of the machine identity.
- What is missing. Every delivery and change handoff prints `What is missing` and a `Known gaps` row, and JSON `gaps`. Definition gaps are derived from the definition alone (arguments no guard reads, facts read but never set or stored but never read, dead-end and unreachable states, commands with no guard, no invariants; a change marks a gap the prior version lacked as new). Evidence gaps count only runs observed against this definition (continuity observations on the prior version are not counted): commands with no cited committed run, and guarded commands with no cited `IK_INVARIANT_FAILED` refusal. When `requirement` carries `atoms` (verbatim spans), `traces` (atom to `command:`, `state:`, `fact:`, `invariant:` or `point:` element) and `not_applicable` (atom with a reason), the handoff also lists atoms traced to nothing, atoms marked not applicable (with reasons), sentences with a word no atom covers (a fragment does not cover its sentence), and commands no atom is traced to (for a change, only new or changed commands, and the line says so); a stated origin inside an atom traces it by position and the link is printed. Empty lists read `none found`; a span must occur once in the text. Nonexistent elements, unknown or duplicate atoms, spans not in the requirement, and a not-applicable atom with no reason are refused (exit 2). Every list is closed-world over a scope sentence that is always printed and never claims completeness. `ik laws` `unguarded_arguments` now shares its implementation with the handoff.

### Added

- Recovery API in the Python, TypeScript and Go clients: `Aggregate.prepare` (`prepare_transaction`; Go: `Prepare`, command only) returns an `Invocation` that binds one stable operation id to the original request, including the original capability. `Machine.dispatch` sends it once; `Machine.reconcile` does the exact lookup from a restored copy (`to_json`/`from_json`). A prepared invocation is an owned, immutable snapshot of the request: mutating a caller-owned argument object, a returned wire value or a returned command afterwards cannot change what is dispatched or reconciled, and an unserializable value is refused before anything is sent. The persisted form is versioned and semantically lossless across clients (non-ASCII text, escaping, nesting and integers beyond 2^53 restore exactly); the bytes are not promised identical across languages. Recovery never derives a new id from a refreshed generation, never refreshes, and never resends. A listing from `ik operations` is not attribution. Effect reconciliation stays separate. See `docs/operation-identity.md`.

- `ik operations <store.sqlite> --aggregate <id> [--limit <n>] [--json]` lists a window of the operations recorded for one aggregate, newest first (`ik.operations.v1`), from a read-only open of the store. The aggregate position and the rows are read in one read transaction and reported as a snapshot, with the window size and whether it is complete. A negative stored coordinate is an error, not zero. It is discovery, not attribution: an operation missing from the window has no established outcome (older, still running, never delivered, refused or a no-op); only an exact lookup answers whether it committed. See `docs/operation-identity.md`.
- `ik census <store.sqlite> [--json]` counts instances per machine version, state and lifecycle from a read-only open of the store (`ik.census.v1`); it never writes, upgrades or reads fact values. `ik handoff --kind change --store <store.sqlite>` puts the version-1 count in the version decision (and says no census was taken without `--store`). The version decision now carries a drain-and-recreate compatibility table computed from the two definitions: per version-1 state, each exit's same/guard-changed/target-changed/removed verdict, what the new version adds, a new-version guard or invariant that reads a fact by value which some version-1 instance in that state may never have set, new or changed invariants, and changed or removed effects. It finds differences, does not prove safety, and says that stored values, unresolved effect intents and hierarchical machines are not analysed. `ik.diagram.v1` change gains `compat` and `machine_id`.
- `Chart` is the authored structure model attached at parse (regions, compound groups with parent handlers counted once, joints). The flat `Definition` remains the compile target; `spec_hash` is unchanged. `check_consistent` refuses a mismatch with `IK_CHART_INCONSISTENT` (authored edge maps to ≥1 flat transition; each live flat transition maps to exactly one authored edge; keys round-trip). `ik diagram` conserves authored edges and lays them out in-house: identity-sorted nodes, ancestor feedback arcs, longest-path ranks, barycenter layers, paired 2-cycles. Back-edges to an earlier state are drawn as return rails in a left gutter (at most two, disjoint or strictly nested); the annotation naming the command stays on its row, and any back-edge that would exceed that budget or cross a rail keeps the annotation alone. JSON includes `drawn_edges`.
- `ik diagram <machine.ik.toml> [--prior <old.ik.toml>] [--evidence <path>]... [--kind delivery|change|recovery] [--format markdown|text|mermaid|table|json] [--focus <state>] [--level 0|1|2] [--ascii]` draws a lifecycle diagram from the definition graph. The source of truth is `ik.diagram.v1`. Default markdown is a facts strip, a two-column river map (state once on the left, rules and loops on the right), and a table with one row per edge plus a given/stored rules table. `--format mermaid` emits a `flowchart LR`. Delivery is the shortest-path river with you-are-here; change keeps one river and annotates the changed state, then `WHAT ELSE IS AT RISK`; recovery is a bookended unfolded arrow of the events the evidence supports (`lookup by operation id; never resend`). `ik handoff` embeds the river in the first block (which still ends on the action/decision line) and the command table after it. JSON adds a `diagram` object. There is no SVG.

### Fixed

- `ik report` lookup findings no longer print `committed: not committed` or
  `next: inspect status committed_exact`. A `committed_exact` lookup names the
  earlier commit and that this lookup did not commit again, with next action
  `do not resend; the earlier result stands`. `reply_dropped`, `not_recorded`,
  and `conflict` each state what is established and what is still uncertain.
- `ik handoff` defaults a lookup/reconciliation finding source to claim
  `reconciled`, so a correct lookup is `exercised` for the earlier operation
  being reconciled. It still does not support `new_mutation` or
  `current_handle`. A check-document `witness_found` finding defaults to claim
  `reachable` (the checked scenarios were reached in this search), not a
  runtime outcome. Recovery handoff text adds `established:` / `uncertain:` /
  `safe next action:` derived from evidence statuses.
- `ik check --prior` prints a `CHANGE IMPACT` paragraph from the prior/new
  identity comparison: new version number, added or removed commands and
  states, and, when the version differs, that existing instances keep the
  version they were created under and that moving them needs an explicit
  migration which this check does not perform.
- `ik check --json` `judgment.verdict` is `accept` or `refused`. Missing
  evidence and a witnessed invariant violation both use `refused`;
  `judgment.status` is `accepted`, `missing_evidence`, or
  `witnessed_violation`. Exit codes are unchanged. The previous JSON token
  `refuse` is not emitted and is not aliased. Python and Go `CheckResult.verdict`
  follow the same two-value vocabulary; read `ik check --json` for `status`.
- `ik --help` lists every dispatched subcommand from the dispatch table
  (including `scenario`, `report`, `handoff`, `corpus`, `analysis`, and
  `regression`). There is no `ik explain` command.
- `ik handoff` treats evidence as claim-specific: `committed_exact` and
  `historical` may support `claim: "reconciled"` (and default to it when the
  finding source is a lookup) and do not support `new_mutation` or
  `current_handle`. A check `witness_found` finding may support `reachable`.
- `IK_INVARIANT_FAILED` is also raised when a command `when` guard fails
  (`rule` names that `when`). A `lookup_operation` `conflict` includes a
  lookup fingerprint that does not match the original request.
- `ik check` uses a two-tier coverage contract. Authored outcomes, states, and
  terminals stay mandatory and are satisfied only by a committed kernel witness
  of some generated copy; a proof never makes deleting behavior greener.
  Generated product-context obligations (command/source, outcome copies, product
  states/terminals) may be discharged by a sound impossibility record with
  `basis` and `scope`. Plan `schema_version` is 2 when impossibility records or
  authored-goal rows exist; plans with neither stay version 1 with the same
  digest. Schema-1 ordered-outcome proofs stay recorded and retained. String
  ordering over unbounded argument domains stays unknown. Explicit profile
  goals stay failed when proved impossible, with the proof attached. Unknown or
  feasible goals stay mandatory. A refusal never satisfies a success obligation.
- Counter `then = { count = "+N" }` uses `checked_add` and refuses with
  `integer arithmetic overflow` instead of wrapping at `i64::MAX`. Replay
  consequence: a journal that already stored a wrapped value is not produced
  by new code; replaying such a journal against this runtime refuses the
  increment instead of restoring the wrapped fact.
- `then = { n = "+1" }` on a replace fact is rejected at static check and
  compile: ``+N`` is only for counter facts (the fact is named in the error).
- `*` and `/` in guards report the supported forms instead of
  `expected a comparison operator` or `unexpected token Op("")`.

### Added

- `ik handoff --kind change` renders agent-authored statements that cite a definition finding under the `declared` claim in the `New in this version` block, labelled as declared in the new definition. A declared statement that does not cite a definition finding is not shown there and appears under `Not established` marked incomplete; declared statements never count as demonstrated.

- Scenario `prior_machine` step and per-command `present` observation: create
  an instance under a prior definition, then attempt a prior-supported command
  and a new-only command. `ik report` projects those steps as
  `continuity_observation` findings. `ik handoff --kind change` with claim
  `continuity` states, from that evidence only, that instances created under
  version N keep version N, which named commands still work, which new
  commands are refused and with what typed reason, and that moving them needs
  an explicit migration which is not performed. Unrelated statuses do not
  support the claim. Absent evidence renders `continuity not demonstrated`.
- SMT-LIB2 encoder and pinned Z3/cvc5 adapter prototype for guard evaluation,
  local outcome selectability, and Horn/BMC reachability. See
  `docs/smt-backend.md`. Integers are checked `i64`; string ordering is
  unsupported. String facts and arguments use integer identity over collected
  literals plus an open remainder (a single OTHER bucket is exact only
  against literals). Zero-mismatch differential tests are an admission gate,
  not a proof of universal encoder equivalence. Impossibility results carry a
  trust level: `SolverAsserted`, `CheckedInvariant` (three independent queries
  on an inductive `I`, rechecked by a second solver), or `CheckedExhaustive`
  (kernel-driven finite-model fixpoint with stated domains and a reachable-set
  digest, or a kernel-replayed witness). A wrong invariant is rejected.
  Large inductive disjunctions are encoded as n-ary SMT-LIB `or` (no 64-disjunct
  cutoff). `ik check` consults the adapter only when `IK_SMT_ADAPTER` is set;
  `SolverAsserted` never discharges. Evidence class
  `checked_induction_or_certificate` is declared on the SMT adapter manifest.
- Guard comparisons may add or subtract two integer operands (fact or `$arg`)
  and may multiply or divide an operand by a positive integer constant
  (`checked_mul`, toward-zero `i64` division). Existing `base ± INT` encoding
  is unchanged. See `docs/machine-language.md` and
  `examples/shipped/loan_checkout.ik.toml`.
- Guard predicate `text_in(VALUE, SET, MIN, MAX)`: VALUE is a `$arg` or string
  fact; SET is an ASCII character class (`a-z` ranges, `-` literal first or
  last, `*` any Unicode scalar); MIN/MAX are Unicode-scalar length bounds in
  `0..=4096`. Membership is exact. Runtime, explain, explore, scenario, and
  Hegel share one `holds()` implementation. SMT reports the form unsupported.
  Feasibility treats it as opaque. See `docs/machine-language.md` and
  `examples/shipped/tagged_item.ik.toml`.

### Changed

- `ik handoff` text leads with `Ikaros <kind>: <subject>` and a plain-language
  first block (demonstrated, explored only, not established, decision). Cited
  finding detail, the reading, and the evidence index follow under
  `Exact evidence`. Recovery subject and operation identity come from evidence;
  if they are absent the subject is `subject not identified in evidence`.
  Scenario-only evidence never establishes readiness. A reachable check witness
  is explored only (bounded search, not a runtime commit), not a demonstrated
  runtime outcome. Change handoff leads with `New in this version` from the
  declared prior-vs-new definition delta (`ik check --prior`), labelled
  `declared in the new definition`; exploration and observation follow with
  `explored (bounded search, not a runtime commit)` and `observed (runtime)`.
  That delta is an additive check/report finding with evidence class
  `definition`. It supports only a declared-shape claim. Without a prior
  comparison the first block says `what is new: not stated in evidence`.
- `docs/machine-language.md` states the guard/`then` grammar, including
  `text_in`. `skills/ikaros/build.md` and `change.md`: contract amounts, deadlines, approval predicates, and invalidations are machine guards; identifier grammars of 1 to 64 characters of `A-Za-z0-9._-` are `text_in($arg, 'A-Za-z0-9._-', 1, 64)`; clearing a command must clear facts the next reconcile reads; a terminal reconcile does not require a live provider.
- `skills/ikaros/build.md` step 2: try one aggregate before concluding that a
  transaction splits, and never substitute a cross-store host saga, sidecar
  file, or host lifecycle branch; when no human can answer, deliver what one
  aggregate can honor and list the unmet requirement.
- `skills/ikaros/change.md` step 5b: a frozen definition cannot gain a command
  or rule and there is no migrator; existing instances stay on their original
  rules, refuse the new command with the named code, and the unmet requirement
  is listed in the handoff. A sidecar file or host status is never the
  substitute.
- `ik report`: a scenario step whose expectation failed is `expectation_failed`
  even when the runtime refused the command.
- `ik check`: a refusal caused only by uncovered obligations prints a `NEXT`
  block naming `ik corpus init` and `ik check --corpus`.
- Clients expose the identity that owns an aggregate: `Aggregate.owner`
  (TypeScript, Python) and `Aggregate.Owner()` (Go). After a machine version
  change it can differ from the loaded `Machine` identity; route to the retained
  definition whose identity equals it. `MachineIdentity` compares by
  `(machine_id, version, spec_hash)`. See `docs/machine-evolution.md`.
- `ik handoff <behavior.json> --evidence <path>...` renders delivery, change,
  and recovery summaries from `ik report` evidence with citations. A claim that
  cites nothing, or cites ids that do not resolve, is never `exercised`.

- A missing store file is `IK_STORE_NOT_FOUND` with `rule=store.missing`, not
  `IK_STORE_CORRUPT`. `IK_STORE_CORRUPT` is only for a present file or row that
  is invalid. Protocol stays `1.0`. Callers that matched `IK_STORE_CORRUPT` for
  a missing path must match `IK_STORE_NOT_FOUND` (the rule `store.missing` is
  unchanged). `show`, `replay`, `verify`, and `lookup_operation` do not create
  a store file or its parent directory; `create` remains the only creator.
  Python, TypeScript, and Go map the new code to `StoreNotFoundError` and
  expose `classify`, whose vocabulary is `not_sent`, `refused`,
  `contract_mismatch`, `not_found`, `corrupt`, `historical`, and `uncertain`.
  `historical` is a result, not an error. See `docs/design/error-taxonomy.md`.
- `create_with_command` admits an aggregate and its initializing command in one validated operation. Invalid input exposes no aggregate. The same operation id replays as history and is reconcilable with `lookup_operation`. A bare `create` followed by a required initialize is not atomic. TypeScript `createWith`, Python `create_with`, and Go `CreateWith` expose it.
- `ik corpus init` and `ik check --corpus-template` write an editable starter corpus. A corpus-less `ik check` refusal names that next command in a trailing line and, with `--json`, in `next_actions`. Exit codes are unchanged.
- `ik scenario run` records `seam_fired` and the observable proof on each `faults_applied` entry. A requested fault whose seam did not fire fails the run.

- Go `Command` and `Record` now bind a generated operation id, matching Python and TypeScript; a dead Python
  session keeps an in-flight operation id submission-unknown. The constitutional mutants
  `SkipCommandDuplicateGate`/`OmitCommandCapabilityFromFingerprint` are now
  `SkipDuplicateGate`/`OmitCapabilityFromFingerprint` and cover transactions, records and create-with-command.
  The TypeScript demo prints the refusal code; `pack_npm.py` writes `SHA256SUMS`.

### Added

- Adapter starter (`adapters/starter/`) and `ik analysis conformance <manifest.json> [--fixtures <dir>]`. A third party copies the starter, pins the manifest, and implements one property function without editing kernel code. The shipped starter answers `dead_end_count` on an `ik graph export` graph: `exhaustive_finite_model` when the export is closed, `bounded_paths` otherwise, and `unsupported` for any other question. Conformance runs each fixture through the same host path as `ik analysis run`. Passing proves host-contract conformance, not engine correctness. See `docs/analysis-adapters.md`.
- Prepared P route (`adapters/p/`). STATUS: prepared, not qualified; no real P run exists. The manifest names engine P and the probe `p --version`. When that command is absent, describe reports `tool_unavailable` and nothing is executed. A mapping example states queue, send, and monitor semantics. Parser fixtures do not qualify the route.

- TypeScript and Python expose the pending effect intent and recordEffectOutcome / record_effect_outcome. The helper copies effect identity from that intent, requires a caller-supplied outcome digest, dispatches once, and does not call an external service or substitute the current generation. An empty digest or a non-pending intent is a client error before dispatch. A malformed effect object is IK_PROTOCOL_INVALID and echoes request_id, not a transport error.
- Coding-agent skill `skills/ikaros/` with BUILD, CHANGE, and DIAGNOSE procedures, permission routing, and human handoff templates. `ikaros skill install [--dir <project>] [--target <agent-skills-dir>] [--force]` copies it to `<project>/.agents/skills/ikaros/`. The npm package ships those files. `ikaros doctor` reports whether that project copy is present. A locally modified file is not overwritten unless `--force` is set.

- Hegel executable adapter (`examples/ik_hegel_adapter.rs`): sampled execution of a declarative machine against a fresh store. Evidence is property-based testing (`sampled_execution`), not a proof, and is never reported as `holds_in_scope`. A test-support build can select a law mutant from the request; without that build the field is `unsupported`. See `docs/analysis-adapters.md`.
- `ikaros doctor [--dir project] [--json]` reports the installed `@ikaros/runtime` version against `ik --version`, whether `.ik/checks.toml` exists, discovered `*.ik.toml` machines and `scenarios/`, and adapter manifests under `.ik/adapters`. It reads those JSON files only: it does not execute an adapter or fetch anything. A missing optional tool is an `unavailable` line with the next command and does not fail the command. Exit 1 only when the `ik` runtime cannot be found.
- `ikaros init --template job` also writes `.ik/checks.toml` (reach `verified`, no stuck states), `scenarios/normal.json`, `scenarios/stale-handle.json`, and the sibling corpus `job.ik.cases.json`. The template README states the authority/executor boundary and the `ik check` / `ik scenario run` commands.

- `ik graph export` writes an `ik.graph.v1` finite graph from the existing explorer: stable state ids, committed/refused/noop edges, frontier, and input-domain restrictions. A depth or node cut is incomplete; edges that leave the declared domain are recorded, not dropped.
- TLC adapter (`adapters/tlc/`) checks that graph with the pinned TLC 2.19 jar. A complete graph plus an exhaustive run is model-level `holds_in_scope` for that graph only. An incomplete graph is `bounded_paths` and never `holds_in_scope`. Temporal questions are `unsupported`. A missing Java or jar is `tool_unavailable`. Counterexamples map to an `ik scenario` and are `concrete_reproduced` only when `ik scenario run` matched. See `docs/analysis-adapters.md`.
