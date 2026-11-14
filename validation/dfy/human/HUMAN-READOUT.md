# Human readout

One unfamiliar reader, at most 20 minutes. The blocks below are the product's own rendered handoff text for three journeys. This file does not contain the answers.

Packed binary version line:

~~~~text
invariant-kernel 0.5.0 protocol 1.0
~~~~

## Journey 1 — BUILD

Application under delivery: tagged item.

### Delivery handoff

`ik handoff behavior-build.json --evidence out/sc-build --evidence out/check-build.json --kind delivery`

~~~~text
Ikaros delivery: tagged item delivery
Status: DECISION REQUIRED

Demonstrated
a valid tag on a new item commits

Explored only
the required terminal tagged was witnessed in the supplied cases
explored (bounded search, not a runtime commit)

Scope
Checked: whether the state transitions behave as the definition says, within the cited evidence.
Not checked: that the criteria are valid; that the code and effects performing a
transition do what is intended (execution is assumed perfect and reliable); that the
definition says what was needed; anything the requirement does not state.
Readiness: the owner's decision; this report does not state it.
Claim: Ikaros checked whether the state transitions behave as the definition says, within the cited
evidence; 1 of 1 rule acceptances are awaited; 2 material definition gaps are listed over a stated
scope; it does not claim that the criteria are valid, that execution did what was intended, or
readiness, which is the owner's decision.

Who attested what

| duty               | expected party                     | recorded                                                   |
| ------------------ | ---------------------------------- | ---------------------------------------------------------- |
| Requirement source | Owner                              | author-stated, not verified: exercise brief for the tagged |
|                    |                                    | item (a fixture requirement, not a real PRD)               |
|                    |                                    | (agent-restated)                                           |
| Rule acceptance    | Owner                              | 1 of 1 awaiting acceptance                                 |
| Criteria origin    | Author records, Owner acknowledges | 3 decision points: 0 stated, 0 chosen and acknowledged, 3  |
|                    |                                    | chosen not acknowledged, 0 default, 0 unrecorded           |
| Known gaps         | Author lists, Owner decides        | 4 in the definition (2 about what the machine accepts or   |
|                    |                                    | does), 1 in the evidence; 0 unmapped atoms, 0 sentences    |
|                    |                                    | not fully covered, 0 untraced commands (atomization is the |
|                    |                                    | author's)                                                  |
| Independent review | Reviewer                           | not recorded                                               |
| Approver identity  | Platform                           | not recorded: an acceptance binds content, not who         |
|                    |                                    | accepted                                                   |
| Parties in roles   |                                    | not recorded                                               |

Where each criterion came from

| rule | decision point       | origin                                 | detail                                        |
| ---- | -------------------- | -------------------------------------- | --------------------------------------------- |
| tag  | charset A-Za-z0-9._- | chosen by the author, not acknowledged | the author's choice; no requirement text is   |
|      |                      |                                        | cited for it                                  |
| tag  | min_length 1         | chosen by the author, not acknowledged | the author's choice; no requirement text is   |
|      |                      |                                        | cited for it                                  |
| tag  | max_length 64        | chosen by the author, not acknowledged | the author's choice; no requirement text is   |
|      |                      |                                        | cited for it                                  |
enumerated: comparison operators, constants and text_in sets and bounds in declarative guards, for
rules with one guard; not enumerated: existence tests, and/or connectives, reducer code, effects,
rules with several guards, and rules the definition does not have.

What is missing

| definition gap     | subject    | detail                                                                             |
| ------------------ | ---------- | ---------------------------------------------------------------------------------- |
| unguarded_argument | tag $label | no guard reads this argument, so any value of its type is accepted by the machine; |
|                    |            | a rule on it is enforced only by host code                                         |
| no_invariants      | machine    | no invariant is declared, so nothing is asserted to hold in every state            |
Facts stored but read by no guard or invariant (host code may read them): item_id, label.
No run refused with IK_INVARIANT_FAILED (a guard refusal) is cited for: tag.
Requirement statements traced to no definition element: none found.
Requirement sentences with a word no atom covers: none found.
Commands no requirement statement is traced to (all commands): none found.
enumerated: gaps derived from a flat declarative definition; commands with no cited committed run
and guarded commands with no cited guard refusal, counting only runs observed against this
definition; and, when the requirement is given as atoms, atoms with no traced element, sentences an
atom does not fully cover, and commands no atom is traced to. not enumerated: requirement statements
nobody wrote down, rules neither document mentions, whether a state can be occupied forever, reducer
code and effects. A short list does not mean the definition is good.

2 states, 1 command · 1 end · every state reaches an end (structural) · shortest finish: 1 step
```text
  ● [ idle ]
      │
      │  tag                  if: id in A-Za-z0-9._- length 1..64
      ▼
  ✔ [ tagged ]  END           ◀── YOU ARE HERE (0 steps to end)
```
legend: ◀── YOU ARE HERE · ✔ END · ● initial · ○ other

Decision needed
Accept or reject each rule below as the business rule.
Enforced: the definition declares each rule and the kernel checks it before the move.
Distinguished: the examples show how the rule treats those inputs and differ from the one-step
variants listed. This is not a proof the rule is right.
Accepted: only the person who owns the intent can say the behavior is what the business wants.
Nothing here establishes that.

| rule | moves         | meaning                                 | status                |
| ---- | ------------- | --------------------------------------- | --------------------- |
| tag  | idle → tagged | id (given) in A-Za-z0-9._- length 1..64 | needs your acceptance |

tag
  accepted: $id="a"; $id="xxxxxxxxxxxxxxxxxx…" (64 chars); $id="a.b-9_Z"
  rejected: $id=""; $id="xxxxxxxxxxxxxxxxxx…" (65 chars); $id="item/1"
  checked by ik: every stated outcome, every clause toggled, 6 variants of the rule told apart

To decide: tell the agent accept or reject for each rule above. The agent records your decision
against exactly this displayed content; you do not copy hashes or edit files. Accepting says the
behavior is what the business wants. It does not approve a release.

| command | from | to     | guard                               | effect | class    |
| ------- | ---- | ------ | ----------------------------------- | ------ | -------- |
| tag     | idle | tagged | text_in($id, 'A-Za-z0-9._-', 1, 64) |        | observed |

Rules: given = supplied by the caller, stored = kept by the machine.

| name | rule                                    | verbatim                            |
| ---- | --------------------------------------- | ----------------------------------- |
| tag  | id (given) in A-Za-z0-9._- length 1..64 | text_in($id, 'A-Za-z0-9._-', 1, 64) |

Exact evidence

## Behavior and what the evidence shows

- B1 [exercised claim new_mutation]: a valid tag on a new item commits
    evidence out/sc-build#item-1/tag: committed (observed_trace); observed runtime trace; the command committed
- B2 [exercised claim reachable]: the required terminal tagged was witnessed in the supplied cases
    evidence out/check-build.json#tagged: witness_found (bounded_paths); bounded search over the supplied corpus; smallest found within the enumerated search

## Evidence index

- out/sc-build#item-1/genesis | ik.scenario.result.v1 | committed | observed_trace | observed
- out/sc-build#item-1/tag | ik.scenario.result.v1 | committed | observed_trace | observed
- out/check-build.json#tagged | ik.check.profile.v1 | witness_found | bounded_paths | complete
~~~~


### Questions

1. What will the application do?
2. What is not yet established?
3. Does anything require your decision?

## Journey 2 — CHANGE

Change under review: loan checkout version 2.

### Change handoff

`ik handoff behavior-change.json --evidence out/check-change.json --evidence out/sc-change --kind change --store out/sc-change/scratch/store.sqlite`

~~~~text
Ikaros change: loan checkout version 2
Status: DECISION REQUIRED

New in this version
new in version 2: command extend; removed: none
Version 2 adds Extend for loans created under the new rules. While a loan is on loan, Extend can set a later deadline; an equal or earlier deadline is refused.
declared in the new definition

Why
Borrowers need to extend a loan that is already out, without returning it first.
stated by the author, source exercise brief for the loan change (a fixture requirement, not a real PRD); not verified by evidence

For new instances
the new definition still reaches returned
explored (bounded search, not a runtime commit)

Existing instances
instances created under version 1 keep version 1
checkout still works on them
recall still works on them
extend is refused on them with IK_MACHINE_MISMATCH
observed (runtime)

Not established
existing instances stay on their version; moving one to version 2 in place is not supported

Scope
Checked: whether the state transitions behave as the definition says, and, only where continuity evidence is cited, whether existing instances keep their version.
Not checked: that the criteria are valid; that the code and effects performing a
transition do what is intended (execution is assumed perfect and reliable); that the
definition says what was needed; anything the requirement does not state.
Readiness: the owner's decision; this report does not state it.
Claim: Ikaros checked whether the state transitions behave as the definition says, and, only where
continuity evidence is cited, whether existing instances keep their version; 1 of 1 rule acceptances
are awaited; 2 material definition gaps are listed over a stated scope; it does not claim that the
criteria are valid, that execution did what was intended, or readiness, which is the owner's
decision.

Who attested what

| duty               | expected party                     | recorded                                                   |
| ------------------ | ---------------------------------- | ---------------------------------------------------------- |
| Requirement source | Owner                              | author-stated, not verified: exercise brief for the loan   |
|                    |                                    | change (a fixture requirement, not a real PRD)             |
|                    |                                    | (agent-restated)                                           |
| Rule acceptance    | Owner                              | 1 of 1 awaiting acceptance                                 |
| Criteria origin    | Author records, Owner acknowledges | 1 decision points: 1 stated, 0 chosen and acknowledged, 0  |
|                    |                                    | chosen not acknowledged, 0 default, 0 unrecorded           |
| Known gaps         | Author lists, Owner decides        | 6 in the definition (2 about what the machine accepts or   |
|                    |                                    | does), 1 in the evidence; 0 unmapped atoms, 0 sentences    |
|                    |                                    | not fully covered, 0 untraced commands (atomization is the |
|                    |                                    | author's)                                                  |
| Independent review | Reviewer                           | not recorded                                               |
| Approver identity  | Platform                           | not recorded: an acceptance binds content, not who         |
|                    |                                    | accepted                                                   |
| Parties in roles   |                                    | not recorded                                               |

Where each criterion came from

| rule   | decision point | origin | detail                                                                            |
| ------ | -------------- | ------ | --------------------------------------------------------------------------------- |
| extend | operator >     | stated | exercise brief for the loan change (a fixture requirement, not a real PRD): "a    |
|        |                |        | later deadline" (requirement is agent-restated, not verified)                     |
enumerated: comparison operators, constants and text_in sets and bounds in declarative guards, for
rules with one guard; not enumerated: existence tests, and/or connectives, reducer code, effects,
rules with several guards, and rules the definition does not have.

What is missing

| definition gap     | subject              | detail                                                                   |
| ------------------ | -------------------- | ------------------------------------------------------------------------ |
| unguarded_argument | checkout $deposit_id | no guard reads this argument, so any value of its type is accepted by    |
|                    |                      | the machine; a rule on it is enforced only by host code                  |
| no_invariants      | machine              | no invariant is declared, so nothing is asserted to hold in every state  |
Commands with no guard (accepted whenever the instance is in a source state): recall.
Facts stored but read by no guard or invariant (host code may read them): checkout_ms, deposit_id,
overdue.
Evidence: no cited run of this definition names a command (continuity observations ran against the
prior version and are not counted).
Requirement statements traced to no definition element: none found.
Requirement sentences with a word no atom covers: none found.
Commands no requirement statement is traced to (new or changed commands only): none found.
Traced by a stated origin, not by a trace entry: R2 -> command:extend, R2 ->
point:extend/operator:>.
enumerated: gaps derived from a flat declarative definition; commands with no cited committed run
and guarded commands with no cited guard refusal, counting only runs observed against this
definition; and, when the requirement is given as atoms, atoms with no traced element, sentences an
atom does not fully cover, and commands no atom is traced to. not enumerated: requirement statements
nobody wrote down, rules neither document mentions, whether a state can be occupied forever, reducer
code and effects. A short list does not mean the definition is good.

3 states, 4 commands · 1 end · every state reaches an end (structural) · 1 loop · shortest finish: 2 steps
```text
  ● [ idle ]                  (unchanged)
      │
      │  checkout             (unchanged)
      ▼
  ○ [ on_loan ]               ↺ can also: recall
      │  ✚ ↺ extend           NEW — if: new_until > loan_until
      │    • New loans (v2): can extend
      │    • Old loans (v1): cannot; extend REFUSED (IK_MACHINE_MISMATCH), observed
      │
      │  checkin              (unchanged)
      ▼
  ✔ [ returned ]  END         (unchanged)

CHANGE: +extend

WHAT ELSE IS AT RISK (basis / status)
  • Instances in changed states : on_loan (v1: checkin, recall | v2: +extend)                     (structural / checked)
  • Shared facts touched        : loan_until_ms (written & read by extend)                        (structural / checked)
  • Dead ends / unreachable     : none
  • Shortest finish changed     : none
  • Existing checks / cases     : none reference modified or removed commands
  • Effects & invariants        : no changes
  • Live v1 count & callers     : unknown  (not scanned / NOT ESTABLISHED)
```
legend: ✔ END · ↺ loop · ✚ new · ● initial · ○ other

Decision needed
Accept or reject each rule that is new or changed in this version as the business rule.
Enforced: the definition declares each rule and the kernel checks it before the move.
Distinguished: the examples show how the rule treats those inputs and differ from the one-step
variants listed. This is not a proof the rule is right.
Accepted: only the person who owns the intent can say the behavior is what the business wants.
Nothing here establishes that.

| rule   | moves             | meaning                                       | status                |
| ------ | ----------------- | --------------------------------------------- | --------------------- |
| extend | on_loan → on_loan | new_until_ms (given) > loan_until_ms (stored) | needs your acceptance |

extend
  accepted: $new_until_ms=200, loan_until_ms=100
  rejected: $new_until_ms=100, loan_until_ms=100; $new_until_ms=50, loan_until_ms=100
  checked by ik: every stated outcome, every clause toggled, 4 variants of the rule told apart

To decide: tell the agent accept or reject for each rule above. The agent records your decision
against exactly this displayed content; you do not copy hashes or edit files. Accepting says the
behavior is what the business wants. It does not approve a release.

Keep existing instances on version 1, or let them finish and create new instances on version 2?

- Keep existing instances on version 1 until they finish
    existing instances keep the version 1 rules and cannot use: extend. The host must register both
    versions until no instance is left on the old one.
- Drain, then create new version 2 instances
    version 1 instances finish under version 1; instances created after that begin under version 2. This
    does not move an unfinished instance to version 2: stored state stays bound to the version it was
    created under. Recreating a business operation can repeat external work unless the domain contract
    allows it.
Found: version 1 instances in the store: 1 unfinished, 0 finished
Found: unfinished by state: on_loan 1
Found: version 1 can be retired when its unfinished count is 0 (the host keeps both versions
       registered until then)

Compatibility of new lifecycles after old work finishes (definition-level only). The checks compare the two definitions; they find differences and do not prove safety.

| state   | version 1 exits | in the new version     | review              |
| ------- | --------------- | ---------------------- | ------------------- |
| idle    | checkout        | unchanged              | no difference found |
| on_loan | checkin, recall | unchanged; adds extend | no difference found |

Not established: stored fact values were not checked against the new rules (the checks read
definitions only)
Not established: unresolved effect intents of existing instances were not inspected

| command  | from    | to                             | guard                                  | effect | class        |
| -------- | ------- | ------------------------------ | -------------------------------------- | ------ | ------------ |
| checkout | idle    | on_loan                        | $loan_until_ms == $checkout_ms +       |        | explored     |
|          |         |                                | 1209600000                             |        |              |
| checkin  | on_loan | returned                       | $overdue == 0 \|\| recall_placed == 1  |        | declared     |
| extend   | on_loan | on_loan                        | $new_until_ms > loan_until_ms          |        | new declared |
| recall   | on_loan | on_loan                        |                                        |        | declared     |
| extend   | on_loan | (refused: IK_MACHINE_MISMATCH) |                                        |        | refused      |

Rules: given = supplied by the caller, stored = kept by the machine.

| name     | rule                                                | verbatim                                    |
| -------- | --------------------------------------------------- | ------------------------------------------- |
| checkout | loan_until_ms (given) is checkout_ms (given) + 14d  | $loan_until_ms == $checkout_ms + 1209600000 |
| checkin  | overdue (given) is 0 or recall_placed (stored) is 1 | $overdue == 0 \|\| recall_placed == 1       |
| extend   | new_until_ms (given) > loan_until_ms (stored)       | $new_until_ms > loan_until_ms               |

Exact evidence

## Behavior and what the evidence shows

- C0 [exercised claim declared]: Version 2 adds Extend for loans created under the new rules. While a loan is on loan, Extend can set a later deadline; an equal or earlier deadline is refused.
    evidence out/check-change.json#definition: declared (definition); new in version 2: command extend; removed: none
- C1 [exercised claim reachable]: the new definition still reaches returned
    evidence out/check-change.json#returned: witness_found (bounded_paths); bounded search over the supplied corpus; smallest found within the enumerated search
- C2 [exercised claim continuity]: instances created under version 1 keep version 1; recall still works on them; extend is refused on them with IK_MACHINE_MISMATCH
    evidence out/sc-change#loan-1/under_prior: committed (continuity_observation); instances created under version 1 keep version 1
    evidence out/sc-change#loan-1/recall: committed (continuity_observation); instances created under version 1 keep version 1; recall still works on them (presented version 1)
    evidence out/sc-change#loan-1/extend: refused (continuity_observation); instances created under version 1 keep version 1; extend is refused on them with IK_MACHINE_MISMATCH (presented version 2)

## Reading

established: instances created under version 1 keep version 1
established: checkout still works on them
established: recall still works on them
established: extend is refused on them with IK_MACHINE_MISMATCH
safe next action: existing instances stay on their version; moving one to version 2 in place is not supported

## Evidence index

- out/check-change.json#definition | ik.check.profile.v1 | declared | definition | declared
- out/check-change.json#returned | ik.check.profile.v1 | witness_found | bounded_paths | complete
- out/sc-change#loan-1/under_prior | ik.scenario.result.v1 | committed | continuity_observation | observed
- out/sc-change#loan-1/checkout | ik.scenario.result.v1 | committed | continuity_observation | observed
- out/sc-change#loan-1/extend | ik.scenario.result.v1 | refused | continuity_observation | observed
- out/sc-change#loan-1/recall | ik.scenario.result.v1 | committed | continuity_observation | observed
~~~~


### Questions

1. What changes for new instances?
2. What happens to existing ones?
3. Is a migration or permission needed?

## Journey 3 — DIAGNOSE / RECOVER

Incident under review: a lost reply after tagging an item. Scenario: `fixtures/scenarios/lost-reply.json`.

### Recovery handoff

`ik handoff behavior-recover.json --evidence out/sc-lost --kind recovery`

~~~~text
Ikaros recovery: item-lost
operation: op-tag

Demonstrated
the earlier operation committed; this lookup did not commit again

Uncertain
whether any handle from this lookup is current

Not established
whether the caller saved the operation id before sending (the kernel records it; the caller must keep its copy)

Scope
Checked: whether a recorded operation exists in the store under the given id and request.
Not checked: that the criteria are valid; that the code and effects performing a
transition do what is intended (execution is assumed perfect and reliable); that the
definition says what was needed; anything the requirement does not state.
Readiness: the owner's decision; this report does not state it.
Claim: Ikaros checked whether a recorded operation exists in the store under the given id and
request; it does not claim that the criteria are valid, that execution did what was intended, or
readiness, which is the owner's decision.

Who attested what

| duty               | expected party | recorded                                                    |
| ------------------ | -------------- | ----------------------------------------------------------- |
| Requirement source | Owner          | not recorded                                                |
| Independent review | Reviewer       | not recorded                                                |
| Approver identity  | Platform       | not recorded: an acceptance binds content, not who accepted |
| Parties in roles   |                | not recorded                                                |

events the evidence supports · lookup by operation id · never resend
```text
  ● [ idle ] ══▶ ⟪
      │
      │  tag
      ▼
  ○ [ ① request sent ]        observed
      │
      ▼
  ✂ [ ② reply lost ]          observed
      │
      ▼
  ○ [ ③ committed ]           found by lookup
      │
      ▼
  ○ [ ④ asked again ]         found by lookup  ◀── YOU ARE HERE
      │
      ▼
  ⟫ ══▶ ✔ [ tagged ]  END
Established: the earlier operation committed; this lookup did not commit again
Uncertain: whether any handle from this lookup is current
Safe next action: do not resend; the earlier result stands
```
legend: ◀── YOU ARE HERE · ✔ END · ✂ reply lost · ⟪ ⟫ one command, unfolded · ● initial
legend: ○ other

Safe next action
do not resend; the earlier result stands

To change the item again
read the item's current state for a fresh handle; this lookup's result is history, not a handle

| event        | how known       | note                                                   |
| ------------ | --------------- | ------------------------------------------------------ |
| request sent | observed        |                                                        |
| reply lost   | observed        | the caller never saw the result                        |
| committed    | found by lookup | found under the same operation id; not committed again |
| asked again  | found by lookup | the earlier result stands; it is not a current handle  |

Exact evidence

## Behavior and what the evidence shows

- R1 [exercised claim reconciled]: lookup of the dropped commit is committed_exact
    evidence out/sc-lost#item-lost/found: committed_exact (observed_trace); established: the earlier operation committed; this lookup did not commit again. uncertain: whether any handle from this lookup is current

## Reading

established: the earlier operation committed; this lookup did not commit again
uncertain: whether any handle from this lookup is current
safe next action: do not resend; the earlier result stands

## Evidence index

- out/sc-lost#item-lost/genesis | ik.scenario.result.v1 | committed | observed_trace | observed
- out/sc-lost#item-lost/tag | ik.scenario.result.v1 | committed | observed_trace | observed
- out/sc-lost#item-lost/lost | ik.scenario.result.v1 | reply_dropped | observed_trace | observed
- out/sc-lost#item-lost/found | ik.scenario.result.v1 | committed_exact | observed_trace | observed
- out/sc-lost#item-lost/again | ik.scenario.result.v1 | historical | observed_trace | observed
- out/sc-lost#item-lost/restart | ik.scenario.result.v1 | reopened | observed_trace | observed
~~~~


### Questions

1. Did the operation commit?
2. What is still uncertain?
3. What action is safe next?
