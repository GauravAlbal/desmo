# Workbench report and regressions

`ik report` folds scenario runs, analysis results, and check profiles into one
finding list. `ik regression` copies a reviewed scenario bundle and re-runs it.
Promotion is explicit. A failing bundle is copied as it is; expectations are
not rewritten to make it pass.

## Report

```sh
ik report <path>... [--json] [--html <file>]
```

Each path is a run directory or a JSON document of a known kind:

| Kind | How it is recognized |
|---|---|
| `ik.scenario.result.v1` | `result.json` in a scenario bundle, or that document itself |
| `ik.analysis.host-report.v1` | `host-report.json` in an analysis run directory, or that document |
| `ik.analysis.result.v1` | an analysis result document |
| `ik.check.profile.v1` | an `ik check --json` document that has a `profile` object, or the profile object itself |

A directory is read as `host-report.json` when that file exists, otherwise as
`result.json`. Mixing kinds in one invocation is normal: every path contributes
findings to one `ik.report.v1` document. Text is the default. `--json` prints
the document. `--html` writes a self-contained page with no network scripts
and no external resources.

Unknown, missing, or malformed input is an `unreadable` finding. It is not
skipped. Exit `0` when every path was read, `1` when any path was unreadable,
`2` on usage.

### Finding

Each finding has a source kind, a stable id, a status, a scope sentence, an
evidence class, a completeness word, a source coordinate when the source has a
machine command or state, before/after facts when the source has them,
attempted versus committed behavior, the reproduction status, engine-native
artifact paths, and a next action.

A refused command is `committed: refused`. The state that remains is not a
destination of that command. A lookup is not a new commit. `committed_exact`
prints that the earlier operation committed and that this lookup did not
commit again. `reply_dropped`, `not_recorded`, and `conflict` likewise name
what is established and what is still uncertain. A model finding stays
`model-level` until `reproduction_status` is `concrete_reproduced`.
Reproduction status is copied verbatim from an analysis result. A
prefix/cycle validation is not an observed infinite execution.

Evidence classes stay distinct:

| Class | How the report reads it |
|---|---|
| `sampled_execution` | a sample; not a proof. `holds_in_scope` from a sample is not adopted |
| `bounded_paths` | a bounded search, including native `ik check` exploration |
| `exhaustive_finite_model` | exhaustive only inside the declared finite model |
| `observed_trace` | a scenario run against the runtime |
| `continuity_observation` | a scenario step with a `present` observation: an instance created under one definition, then a command presented under the prior or the new identity |

A witness is the smallest found within the enumerated search. The report does
not call it minimal.

### Next action

The next action comes from a fixed table keyed by status. A command name is
filled in only for an input coverage gap. A detail is appended only when the
status says what to configure.

| Status | Next action |
|---|---|
| `witness_found` | promote as regression or continue |
| `input_coverage_gap` | add corpus cases for the named command |
| `no_witness_found_incomplete` | raise bounds or add cases |
| `no_witness_found` with incomplete coverage | raise bounds or add cases |
| `no_witness_found_complete` | unreachable within the supplied corpus; change the corpus or the goal |
| `violation`, `expectation_failed` | reproduce with `ik scenario reproduce` |
| `unsupported` | configure the unsupported capability |
| `tool_unavailable` | configure or install the missing tool |
| `refused` | refused, not a committed state |
| `committed_exact` | do not resend; the earlier result stands (the recovery handoff adds how to continue: read current state for a fresh handle) |
| `historical` | do not treat this as a new commit or a current handle |
| `reply_dropped` | look up the earlier operation; do not resend from the dropped reply |
| `not_recorded` | do not treat the earlier attempt as committed from this lookup |
| `conflict` | do not treat this as the same operation |
| `unreadable` | replace or repair the input; it was not skipped |

`no_violation_found` is not a proof. `holds_in_scope` holds only in the
declared finite model.

## Handoff

```sh
ik handoff <behavior.json> --evidence <path>... [--kind delivery|change|recovery] [--store <store.sqlite>] [--json]
```

`behavior.json` (`behavior_version` 1, unknown fields refused) holds a `title`,
`outcomes` and `unchanged` items (`id`, `text`, optional `claim`, `evidence`: finding ids from
`ik report`), `assumptions`, an optional authored `decision` (`question`, `options`
with `label` and `consequence`, optional `recommended`), `accepted_rules`
(each `{rule, guard_hash, examples_hash}`), `rule_witnesses`, `why` (`{text, source}`), `requirement`
(`{text, source, provenance}`), `origins` and `acknowledged_origins` (see Criterion origins). The command reads the
evidence as `ik report` does and prints one status per claim. Admissibility is
claim-specific: a finding status that supports one claim does not support
another.

### Instance census and new-lifecycle compatibility (drain, then create new)

`ik census <store.sqlite> [--json]` counts instances per machine, version, state and
lifecycle (`ik.census.v1`). It opens the store read-only and never creates, upgrades or
writes it; it reads no fact values. `ik handoff --kind change --store <store.sqlite>` adds the
version-1 count (unfinished and finished, unfinished by state) to the version decision. Without
`--store` the handoff says no census was taken.

The version decision's options are "keep on version N" (existing instances stay owned by N and
complete under its rules) and "drain, then create new version N+1 instances" (old lifecycles
finish; later lifecycles begin under the new version). Draining is not a way to move an
unfinished instance to the new version, and recreating a business operation can repeat external
work unless the domain contract allows it. The fresh-store policy is already selected for the
launch; the cutover authorization is a separate pending decision. The version decision also
prints a compatibility table from a comparison of the two
definitions: for each version-1 state, whether each version-1 exit is the same, has a changed
guard or target, or is removed, what the new version adds, and why the state needs review. It
flags a new-version guard or invariant that reads a fact by value which some version-1
instance in that state may never have set, a new or changed invariant, and a changed or removed
effect. It finds differences; it does not prove safety. Stored fact values, unresolved effect
intents and hierarchical machines are not analysed, and the handoff says so. The census and the
table apply to declarative machines; the census reads the store whatever the machine kind.

### Rule ledger, examples and acceptance

Three separate judgments: the runtime **enforces** the rule; the examples **distinguish** it
from the enumerated one-step variants; the intent owner **accepts** that the behavior is what
the business wants. The first two are mechanical. The third is a human decision that is
presented once, as a single behavior-review packet (a table of the pending rules with their
examples), not as one interruption per guard. Unchanged accepted rules stay resolved. The
agent records an authorized decision in `accepted_rules`; a human never computes a hash or
edits that field. Examples are not the oracle: the original requirements and independent checks
still apply. Acceptance hashes bind content, not the approver's identity (same-account
cooperative boundary).

Every command with an authored guard is a rule. A hierarchical machine is read from its
authored regions and joints, not from the flat lowering. Each rule carries a hash of its guard
text and the moves the guard applies to. A delivery handoff asks the human to accept every rule,
and a change handoff every rule that is new, changed or whose guard was dropped.

`rule_witnesses` is a list of `{rule, examples, equivalent}`. An example is `{args, facts,
state, outcome}` with `outcome` `accepted` or `rejected`: a unit-level input to the guard. It does
not show the state is reachable and does not show the rule matches the business intent.
`ik handoff` checks, for each rule that needs acceptance and has one distinct guard, that

1. every example's stated outcome is what the guard evaluates to;
2. every atomic clause is true in some example and false in another;
3. every one-step variant of the guard changes the outcome of some example. Variants are a
   flipped comparison, a boundary moved by one (including the length bounds of `text_in`), an
   integer constant moved by one, a negated existence test, and a dropped clause.

A missing or inadequate set is refused with exit 2 and the variants it could not tell apart. A
variant that is genuinely the same rule is declared with `equivalent: [{variant, reason}]`; the
human sees it. A command with several guarded outcomes is not checked and says so.

The agent records an acceptance as an `accepted_rules` entry `{rule, guard_hash, examples_hash}` after the
owner decides; the hashes come from the JSON (`decisions[].rules[]`) and a human never copies them. Changing
the guard, its moves or the examples makes the acceptance stale and the rule is asked again. These checks
apply to declarative machines. When no declarative definition is in the evidence the handoff says rule fitness
is not established.

### Criterion origins

Every decision point of a rule under review gets an origin. A decision point is a comparison operator, a
constant, or the character set and length bounds of a `text_in`; each has a stable key such as
`operator:>`, `constant:1209600000`, `charset:A-Za-z0-9._-`, `min_length:1`, `max_length:64` (a repeat is
`#2`, `#3`). Existence tests, `and`/`or`, reducer code, effects and absent rules are not enumerated, and the
output says so.

The author records origins in the behavior file: `requirement` `{text, source, provenance}` (the text spans
are quoted from; `provenance` is `owner-authored`, `agent-restated` or `unknown`, the default) and `origins`
entries `{rule, point, guard_hash, origin, ...}` with `origin`:

| origin | needs | status shown |
|---|---|---|
| `stated` | `span`, a verbatim part of `requirement.text`; optional `equivalent` for a value stated in other units (`14 days`) | stated, with the quote and the requirement's provenance |
| `chosen` | nothing | chosen by the author, not acknowledged |
| `default` | `default_id`, the identity and version of the default | default |
| none recorded, or a stale guard hash | | unrecorded |

After the owner has seen a chosen point and acknowledged it, the agent adds `acknowledged_origins`
`{rule, point, guard_hash}`. Only a chosen point can be acknowledged, and an acknowledgment never turns a
chosen point into a stated one. The handoff refuses (exit 2) a `stated` origin with no span, a span that is
not verbatim in the requirement, a constant whose literal (or `equivalent`) is not in the span, an operator
the span does not name (by whole words, longest phrase first, so `not equal` does not state `==`) with a phrase from a small vocabulary, two current origins for one point, an acknowledgment with no current chosen origin, a `default` without `default_id`, an unknown
point or rule, and an acknowledgment of a point that is not chosen. These are necessary conditions only:
Desmo does not judge whether a quoted span means what the author says. A guard that changed since an
origin was recorded reads `unrecorded`, as a stale acceptance asks again.

The handoff prints `Where each criterion came from`, a `Criteria origin` row in `Who attested what` with
counts, and JSON `origins` (`scope`, `requirement` with a `text_digest`, `counts`, `skipped`, `rules[].points[]`). The requirement row reads `author-stated, not verified`: provenance is the author's label and nothing checks the text against an artifact. A rule under review with several guards is listed in `skipped` and named in the scope sentence, which is printed even when no point is enumerated.

### What is missing

The handoff prints `What is missing` and a `Known gaps` row, and JSON `gaps` (`scope`, `available`,
`definition`, `evidence`, `requirement`, `counts`). Each list is closed-world over a stated scope and is
never a completeness claim: the scope sentence is always printed, and it says that requirement statements
nobody wrote down, rules neither document mentions, whether a state can be occupied forever, and reducer
code and effects are not enumerated, and that a short list does not mean the definition is good.

Gaps in the definition (derived from the definition alone; a hierarchical machine, a recovery handoff and a
handoff with no declarative definition say they were not enumerated). Material gaps are about what the
machine accepts or does and print in a table; the others print as one line per kind.

| kind | material | meaning |
|---|---|---|
| `unguarded_argument` | yes | no guard reads the argument; any value of its type is accepted (the same list as `ik laws`) |
| `fact_never_set` | yes | a guard or invariant reads a fact that has no initial value and no assignment |
| `dead_end` | yes | a state that is not an end state and has no outgoing command |
| `unreachable_state` | yes | no command sequence reaches it (structural) |
| `no_invariants` | yes | no invariant is declared |
| `unguarded_command` | no | a command with no guard |
| `fact_never_read` | no | a stored fact no guard or invariant reads (host code may) |

In a change handoff a gap that the prior definition did not have is marked `(new)`.

Gaps in the evidence: only runs observed against this definition count; continuity observations ran against
the prior version and are not counted. With run evidence cited, each command with no committed run cited and
each guarded command with no run refused with `IK_INVARIANT_FAILED` cited (a refusal for another reason, such as
a stale capability or a machine mismatch, does not exercise a guard); with none, `no cited run of this
definition names a command`.

Gaps against the requirement, only when `requirement` carries `atoms` (`{id, span}`, each a verbatim part of
`requirement.text`). `traces` (`{atom, element}`) map an atom to `command:NAME`, `state:NAME`, `fact:NAME`,
`invariant:NAME` or `point:RULE/KEY`; an element that does not exist is refused. `not_applicable`
(`{atom, reason}`) marks an atom that is not behavior and needs a reason. Each atom span must occur exactly once in the text. A `stated` origin whose span lies
inside an atom (by position, and only when both occur once) traces that atom to the rule's command and point
without a separate entry; the handoff prints those links (`traced_by_stated_origin`). The handoff lists
atoms traced to nothing and not marked not applicable, atoms marked not applicable with their reasons, sentences
with a word no atom covers (a fragment does not cover its sentence; abbreviations such as `e.g.` over-report),
and commands no atom is traced to (`commands_checked`: `all`, or `new_or_changed` for a change, and the line
says so). An empty list reads `none found`. Without atoms it says the requirement was not given as atoms.
Atomization is the author's; Desmo checks verbatim spans, real elements and uncovered sentences, not that the
split is a good one.

Claim admissibility:

| `claim` | Statuses that can mark the item `exercised` |
|---|---|
| omitted | `committed`, `refused`, `holds_in_scope`. A lookup/reconciliation finding source (`committed_exact` or `historical`) defaults to `reconciled`. A check-profile `witness_found` finding source defaults to `reachable`. |
| `reconciled` | `committed_exact`, `historical` (the earlier operation was found; not a new mutation and not a current handle) |
| `new_mutation` | `committed` |
| `current_handle` | none of `committed_exact`, `historical`, or `committed` — a lookup or duplicate is not a live handle |
| `reachable` | `witness_found` from a check document (the checked scenarios were reached in this search; not a runtime outcome) |
| `continuity` | `committed` or `refused` findings whose evidence class is `continuity_observation` (a `present` observation from a scenario). Unrelated statuses do not support it. Absent evidence renders `continuity not demonstrated`. |

| Status | Meaning |
|---|---|
| `exercised` | every cited finding resolves and is admissible for that item's `claim` |
| `incomplete` | a cited finding is partial, a sample, a search without a witness, a status that does not support this claim, or some cited id is missing |
| `contradicted` | a cited finding is `expectation_failed`, `violation`, or `failed` |
| `cited_evidence_not_found` | none of the cited ids resolve |
| `no_evidence_cited` | the claim cites nothing |
| `continuity not demonstrated` | a `continuity` claim with no supporting `present` observation |

Text leads with `Desmo <kind>: <subject>` and a plain-language first block
(declared definition delta, demonstrated runtime outcomes, explored-only check
witnesses, not established, decision). Change handoff prints `New in this
version` from the prior-vs-new comparison before exploration and observation,
with labels `declared in the new definition`, `explored (bounded search, not a
runtime commit)`, and `observed (runtime)`. A `definition` finding is declared
shape only; it does not support execution or readiness. Without a prior
comparison the first block says `what is new: not stated in evidence`.
Recovery takes subject and operation identity from the evidence
(aggregate and `operation_id`); if they are absent the subject is
`subject not identified in evidence`. Cited finding detail, the reading, and
the evidence index follow under `Exact evidence`. Scenario-only evidence
never establishes readiness. Every handoff prints a `Scope` block and a `Who attested what` table. Scope states what this kind of handoff asks of the evidence, as a question (`Checked: whether ...`) so it cannot be read as a result (delivery: whether the state transitions behave as the definition says, within the cited evidence; change: that, and only where continuity evidence is cited, whether existing instances keep their version; recovery: whether a recorded operation exists in the store under the given id and request), what it does not check (that the criteria are valid, that the code and effects performing a transition do what is intended, that the definition says what was needed, anything the requirement does not state; execution is assumed perfect and reliable) and that readiness is the owner's decision and the report does not state it. The table lists requirement source, rule acceptance, independent review, approver identity and parties in roles: an expected party and what the evidence records, `not recorded` where there is no input. It never judges a contribution. JSON: `scope`, `duties`, and `readiness: not_stated`. See `docs/design/desmo-lane.md`.
A reachable check witness is explored only (bounded search, not a runtime
commit). Each cited finding's scope sentence is printed beside the claim, so a
refusal reads as "not committed". Recovery handoff adds a `established:` /
`uncertain:` / `safe next action:` reading derived from the cited evidence
statuses; it does not invent facts those statuses do not carry. Change handoff
adds the same shape from `continuity_observation` findings: instances created
under version N keep version N, which named commands still work on them, which
new commands are refused and with what typed reason, and that existing
instances stay on their version (moving one in place is not supported). Without those findings
the reading is `continuity not demonstrated`. The evidence
index lists every finding, including ones no claim cites. Exit `1` when a
claim is contradicted or any evidence path was unreadable, `2` on a malformed
behavior file or usage. An authored decision is rendered as written; `ik handoff` never
chooses an option. An omitted claim that defaulted still does not support a
different typed claim: a correct lookup stays incomplete for
`new_mutation` and `current_handle`, a check witness stays incomplete for
those runtime claims, and none of those statuses support `continuity`.
Delivery, change, and recovery handoffs embed a lifecycle diagram: a one-line
facts strip, a monospace route map in a `text` fence, one legend line, then
(after the first block) the adjacent command table. Mermaid is
`ik diagram --format mermaid` only. JSON adds a `diagram`
object (`schema` `ik.diagram.v1`, `compressed`, `mermaid`, `table` rows). See
[Diagram](#diagram).

## Claim

Every handoff carries one sentence Desmo stands behind: JSON `claim`, and a `Claim:` line under `Scope`. Shape: `Desmo checked <what this kind of handoff checks>; [N of M rule acceptances are awaited; ][K material definition gaps are listed over a stated scope; ]it does not claim that the criteria are valid, that execution did what was intended, or readiness, which is the owner's decision`. The bracketed clauses appear only when there are rules to accept or gaps were analysed (`definition gaps were not enumerated` when they were not); a recovery handoff has neither. It is built from the handoff's own facts and never says more than the handoff does.

## Readout lint

`ik readout <summary.md> --handoff <handoff.txt> [--stamp] [--json]` checks the prose around a handoff for wording and quoting defects. It is mechanical: it cannot tell whether the summary is true. Vectors: `readiness_word`, `completeness_claim`, `proof_word`, `quote_not_in_handoff` (a `>` block must be verbatim handoff text, `…` lines separate parts), `status_not_quoted`, `not_checked_not_quoted`, `claim_not_quoted` (the `Claim:` paragraph must be quoted), `gap_not_mentioned` (every material gap row), `stamp_stale`, `stamp_malformed`. Each defect prints its line, what is wrong and the fix. `--stamp` appends `<!-- ikaros-readout: lint=clean summary=<sha256> handoff=<sha256> -->` only when clean; a later lint verifies it. The hash binds content, not who wrote it. Procedure: `skills/desmo/readout.md`.

## Diagram

```sh
ik diagram <machine.ik.toml> [--prior <old.ik.toml>] [--evidence <path>]... [--kind delivery|change|recovery] [--format markdown|text|mermaid|table|json] [--focus <state>] [--level 0|1|2] [--ascii]
```

The definition is an exact graph (states, commands, guards, initial/terminal).
`--evidence` is the same path list `ik handoff` and `ik report` read. The
source of truth is `ik.diagram.v1` (`--format json`). Default `--format markdown`
is a facts strip (at most five chunks), a fenced monospace route map, one
legend line (only glyphs present, at most five), and
a table with one row per edge (command, from, to, guard verbatim, effect
marker). `--format mermaid` emits a `flowchart LR`. The diagram is a compression of that table. Reachability and finish
distances are structural: they ignore guard values and never claim a path
will happen. `--ascii` replaces box glyphs.

`--kind delivery` is a transit line: stations left-to-right with
distance-to-end, the observed path numbered, you-are-here at the last
committed state (or the initial state when no trace is present). Stuck or
unreachable states outrank the pin. `--kind change` draws version N and N+1
as two aligned lines, tags added commands `✚`, prints `Reachability:`
(structural) and a per-state cohort table (Existing / New). Continuity
evidence fills refused cells; absent evidence prints `continuity not
demonstrated`. `--kind recovery` is an effect-lifecycle swimlane
(plan / saved / request / accepted / recorded / complete) with a crash mark,
you-are-here, and a three-outcome fork whose resolution is `lookup by saved
id; never resend`. Steps without evidence stay `not observed`.

More than twelve states collapse a trailing run into a `... n states`
super-node (`--level 0` caps at eight); the table still lists the names.
`--focus <state>` keeps the local neighbourhood. Maps are linted for width
(<= 80), line count (<= 24), legend size, and one emphasized glyph class.
A glance-question exercise is pending; this page does not claim the map
reads at a glance. Default output has no mermaid fence and no SVG. Mermaid
is `--format mermaid` only; labels carry the encoding if styling is ignored.

## Regression

```sh
ik regression promote <bundle-dir> --name <name> --reason <reason> [--into <dir>]
ik regression run <dir>
```

`promote` checks `SHA256SUMS` against the bundle files. A mismatch, a missing
required file (`scenario.json`, `machine.ik.toml`, `result.json`), or an
existing `<into>/<name>/` is refused (exit `2`). The default directory is
`regressions/`. The copy includes every file named by `SHA256SUMS`, plus
`SHA256SUMS`, and writes `REGRESSION.json`:

| Field | Meaning |
|---|---|
| `source_bundle_digest` | SHA-256 of the source `SHA256SUMS` bytes |
| `reason` | the `--reason` text, stored unchanged |
| `spec_hash` | machine spec hash from the bundle `result.json` |
| `runtime_binary_sha256` | SHA-256 of the `ik` binary that performed the promotion |

`run` calls the scenario reproduction library on every promoted bundle under
`<dir>` (or on `<dir>` itself when it contains `REGRESSION.json`). It prints
`<name>: reproduced` or `<name>: not_reproduced`. Exit `0` when every case
reproduces, `1` when any does not, `2` on usage. A runtime binary difference
is reported and is not, by itself, a failure. Re-running does not edit the
promoted files.
