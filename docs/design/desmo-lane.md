# ADR: Desmo's lane, the verdict frame, and the duty ledger

Status: steps 1 and 2 of the rollout are implemented: the scope statement, scope-coverage wording in
place of "readiness", the duty ledger, and origins for the decision points of declarative guards
(stated, chosen, acknowledged, default, unrecorded). Of step 3, the known-gaps register (5a) and
the sanctioned claim sentence (6.3) are implemented; the rest is proposed. Everything else is a
decision about what to build and what not to.

## The boundary in one paragraph

Desmo owns the state transitions: which moves a lifecycle may make, in which states, under which
declared guards, and that each committed move is recorded atomically. It does not own the validity
of the criteria those guards express, and it does not own the execution that performs a transition
in the world (reducer code, external effects, the host that calls it). Like every formal checking
tool, Desmo assumes that execution is perfect and reliable: the model is checked, and the
implementation is taken to carry it out faithfully. Nothing Desmo reports is a claim about
criteria validity or execution fidelity.

## Problem

A verification result is a statement about a relation between artifacts: a requirement, a
definition of the application, and observed behavior. Four failures recur when that relation is
reported loosely:

1. A report claims more than was checked ("correct", "none", "ready") because the scope of the
   check is not part of the claim.
2. A criterion the author chose (a character set, a length limit) is accepted together with the
   rule around it and thereafter reads as if the requirement had stated it.
3. A duty that nobody performed (independent review, owner acknowledgment) leaves no trace, so
   its absence reads as success.
4. The tool is asked to judge things that only a person can (whether a criterion is right,
   whether a requirement is complete, whether to release).

These are one disease: a verdict is a bare status, and its frame (scope, assumptions, who
asserted what) is kept elsewhere or not at all.

## Decision

### 1. Desmo is an instrument and a ledger, not a judge

Desmo does four things:

1. Enforces the stated definition at runtime: atomic commit, refusals, identity, operation ids.
2. Produces evidence with an honest status (enforced, exercised, explored, declared).
3. Binds judgments made by other parties to content hashes (an acceptance, an origin link) and
   reports when they go stale.
4. Renders only claims that sit inside its frame, with the frame attached.

Desmo does not decide whether a criterion is right, whether a requirement is complete, whether
a quoted requirement span entails a constant, whether the work is ready, or who an approver
really is.

### 2. RACI

R does it, A answers for it, C is consulted, I is informed.

| Judgment or artifact | Owner | Author | Desmo | Host | Reviewer | Platform |
|---|---|---|---|---|---|---|
| Requirement text and its correctness | A/R | C | | | C | |
| Definition of the application | A | R | I | | C | |
| Definition conforms to the requirement | A | R (asserts) | binds to hash | | C | |
| Criteria are right for the business | A/R | | | | | |
| Runtime enforces the definition | I | I | A/R | | | |
| Evidence was produced honestly | | R (runs) | A (instruments) | | C | |
| Which cases to test | A | R | | | C | |
| Report states only what was established | | | A/R | I | | |
| Summary given to a human | A | R | supplies the sentence | | | |
| Persisting the invocation, versions, retirement | | C | supplies the API | A/R | | |
| Who an approver is | | | discloses | | | A/R |
| Readiness, cutover, publication | A | R (after permission) | I | R | C | R |
| Independent assurance | I | | supplies reproducible bundles | | A/R | |

Roles collapse in small teams. When one party holds several roles, the report says so.

### 3. A verdict carries its frame

A verdict has the form: under assumptions A, a stated relation holds between artifact X and
artifact Y, with evidence class E. A relation cannot be rendered without its assumptions.

- Global assumptions (platform envelope, cooperative same-account boundary, pinned binary) are
  stated once per handoff with a stable identity and shown when they change. Claim-specific
  assumptions are shown inline.
- Global assumption, always stated: execution is perfect and reliable. The code that performs a
  transition, the effects it triggers and the host that invokes it are assumed to do what the
  definition says they do. Violations of that assumption are outside the verdict's frame.
- An empty list is never printed bare. It carries what was enumerated and what was not.
- Desmo reports coverage of its own frame, for example "evidence for the stated scope". It does
  not print "ready" or "correct". Readiness belongs to the owner.
- The scope statement is fixed except for the first line, which is what this kind of handoff
  asks of the evidence. It is phrased as a question ("Checked: whether ...") so it cannot be read
  as a result; outcomes live in the Demonstrated, Explored and Existing blocks. For delivery:
  whether the state transitions behave as the definition says, within the cited evidence. For
  change: that, and only where continuity evidence is cited, whether existing instances keep
  their version. For recovery: whether a recorded operation exists in the store under the given
  id and request (a recovery handoff reads the store and does not check transitions). Not
  checked, always: that the criteria are valid, that the code and effects performing a
  transition do what is intended (execution is assumed perfect and reliable), that the definition
  says what was needed, and anything the requirement does not state. A closing line says
  readiness is the owner's decision and the report does not state it. The JSON `readiness` field
  is `not_stated`.

### 4. Origin is a recorded fact, set once and never upgraded by approval

Each decision point in a declarative definition (a constant, boundary, character set,
operator) has an origin:

| Origin | Meaning |
|---|---|
| stated | A span quoted from the requirement text names it. The span is a verbatim substring of that text. |
| chosen, acknowledged | The author chose it and the owner has seen and acknowledged it. |
| chosen, not acknowledged | The author chose it and no one has acknowledged it. |
| default | A library or product default, carrying its own identity and version. |
| unrecorded | An existing definition with no origin data. |

- An owner's acceptance of a rule never converts a chosen point into a stated one.
- For constants and bounds the quoted span must contain the literal (or a stated equivalent for a
  value given in other units). For operators the span must name the operator with a phrase from a
  small vocabulary, matched by whole words and longest phrase first, so `not equal` does not
  state `==` and `no later than` does not state `>`. These are necessary conditions, and they are
  labelled as such: Desmo does not judge whether a span means what the author says.
- The requirement is recorded with its source, an author-set provenance label (`owner-authored`,
  `agent-restated`, `unknown`) and a digest of its text. Desmo does not verify it against an
  artifact, so the handoff says `author-stated, not verified`. Pinning to a checked artifact hash
  is not implemented.
- Exactly one current origin may be recorded for a point; two are refused. An origin recorded
  against an older guard hash reads `unrecorded` and says it is stale. An acknowledgment needs a
  current `chosen` origin and is refused otherwise.
- Origins live in the behavior file, bound to each decision point's guard hash. They are not part
  of the machine identity, so editing provenance does not force a version change.
- Existing definitions show `unrecorded`. Refusing a missing origin is not implemented; a missing
  origin is a visible gap, not a refusal.
- Only declarative guards are enumerable, and only for rules with exactly one guard. Rules with
  several guards are listed as skipped, and the scope sentence is printed even when no point is
  enumerated, so an empty table cannot read as "nothing was left out".

### 5. The duty ledger

Every handoff reports who attested what, in a closed-world way. It records contributions and
never judges them.

| Duty | Expected party | Recorded |
|---|---|---|
| Requirement source | Owner | author-stated, not verified: the source text; or not recorded |
| Rule acceptance | Owner | N of M awaiting, or M of M accepted (a rule acceptance, not a release approval) |
| Independent review | Reviewer | not recorded |
| Approver identity | Platform | not recorded: an acceptance binds content, not who accepted |
| Parties in roles | | not recorded |

The party column names who is expected to perform the duty, not who did. The recorded column says
what the evidence contains. "Not recorded" is the only gap wording, so it cannot be mistaken for a
review that found nothing.

A duty that was not performed appears as a gap. A single-party attestation is flagged as such.
The ledger is what turns a stated limit into a visible one.

### 5a. What is missing

Completeness cannot be proved, but the known gaps can be listed. Each list is a closed-world count
over a stated scope and never reads as "complete".

- Gaps in the definition, derived from the definition alone: arguments no guard reads, facts read but
  never set or stored but never read, dead-end and unreachable states, commands with no guard, no
  invariants.
- Gaps in the evidence: commands with no cited committed run observed against this definition, and
  guarded commands with no cited guard refusal (`IK_INVARIANT_FAILED`).
- Gaps against the requirement, only when the author gives it as verbatim atoms and traces them to
  definition elements: atoms traced to nothing, sentences with a word no atom covers, commands no atom is traced
  to.

Desmo does not enumerate statements nobody wrote down, rules neither document mentions, whether a
state can be occupied forever, or anything in reducer code and effects, and prints that. The list is
not a score: a short list does not mean the definition is good. A third source, domain checklists of
rule categories the requirement never mentions, is not implemented and is not planned: it needs
domain knowledge Desmo does not have.

### 6. Interfaces, not features

Three things stay outside Desmo and enter through narrow interfaces:

1. Requirement traceability from requirements tooling. The handoff already accepts a requirement
   split into verbatim atoms with traces (section 5a). Importing links from an external tool, with
   hashes, is not implemented. Desmo does not own a traceability matrix.
2. Attestations. Desmo accepts an externally produced attestation (a signature, a recorded
   identity) and binds it to content. It does not build identity.
3. Claim text. Desmo emits a sanctioned one-sentence claim with its frame (the `Claim:` line and
   the JSON `claim` field): what was checked, how many rule acceptances are awaited and how many
   material definition gaps are listed, and what is not claimed. It is built from the same facts as
   the rest of the frame, so it cannot say more than the handoff does. `ik readout` checks that a
   summary quotes it. Enforcing its use in chat or pull requests beyond that is a harness concern
   for the author's environment.

## Limits that remain

- Requirement completeness is not proved. Known gaps are listed (section 5a), but a missing rule
  (for example, tag uniqueness) that neither the requirement nor the definition mentions is
  invisible to Desmo.
- Evidence selection is the author's. Desmo checks that examples distinguish a rule from its
  one-step variants, not that they cover the requirement.
- Separation of duties is cooperative unless the platform enforces it.
- Judgments are point-in-time and bound to content. Who owns a requirement later is out of scope.
- Free-form messages outside the handoff can still over-claim.

## What this removes and what it only moves

Removed inside Desmo's lane: bare verdicts, silent "none", staleness, assumption bloat, default
drift, unreachable readiness. Moved to a named party and made visible by the ledger: laundering
by the author, evidence-selection bias, rubber-stamped acceptance, requirement completeness,
approver identity, summaries outside the handoff.

## Rollout

1. Wording and frame (small, no new authoring surface): scope statement, scope-coverage language
   instead of "readiness", the duty ledger populated from data the handoff already has.
2. Origin sidecar and statuses for declarative machines, `unrecorded` for existing ones.
3. The three interfaces, only when a consumer asks.

## Non-goals

A requirements-management system, an identity system, a general policy engine, a prover of
business correctness, and any promise of "formal correctness" for criteria.
