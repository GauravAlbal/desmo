# Answer key

Withheld from the reader. Derived from the journey fixtures' contracts and from evidence the packed binary recorded in this gate. Pointers use the relative finding ids the readout prints (`out/...`), plus the fixture files under `../journeys/fixtures/`.

## BUILD

Questions: What will the application do? What is not yet established? Does anything require your decision?

### What will the application do?

A valid tag on a new item commits. That is a demonstrated runtime outcome (`new_mutation`). The required terminal `tagged` was witnessed in the supplied cases, but only as a bounded search (explored only; not a runtime commit).

Evidence:

- Delivery handoff first block: Demonstrated `a valid tag on a new item commits`; Explored only names the tagged witness as bounded search, not a runtime commit.
- Exact evidence B1: `exercised claim new_mutation` on `out/sc-build#item-1/tag`.
- Exact evidence B2: `exercised claim reachable` on `out/check-build.json#tagged`.

### What is not yet established?

Readiness is not established. The tagged witness is a bounded search, not a runtime commit. This packet does not prove unsupplied cases, unreachability, or a production commit.

Evidence:

- Delivery handoff first block: a `Scope` block (checked: state transitions as defined; not checked: criteria validity, execution, whether the definition says what was needed; `Readiness: the owner's decision; this report does not state it.`); a `Who attested what` table (rule acceptance 1 of 1 awaiting, independent review not recorded); Explored only on B2.
- `Where each criterion came from` (BUILD): the tag grammar (character set, minimum and maximum length) is chosen by the author and not acknowledged; the requirement says only "a valid tag". (CHANGE): `extend`'s `>` is stated by the requirement sentence "a later deadline", and the requirement is agent-restated. A stated origin is not a claim that the span means what the author says.
- `What is missing` (BUILD): the machine accepts any `label` (no guard reads it) and declares no invariant; no guard refusal is cited for `tag`; the requirement lists read `none found`, which is a statement about what the author atomized, not a claim that nothing is left out. (CHANGE): `deposit_id` on `checkout` is unguarded, no invariant is declared, `recall` has no guard, no run of this definition is cited at all (the observed runs are continuity observations on version 1 and are not counted), and the change lists cover new or changed commands only. The list is not a completeness claim.
- Exact evidence B2 scope: bounded search over the supplied corpus.

### Does anything require your decision?

Yes. Accept or reject the `tag` rule as the business rule: the identifier must be 1 to 64 characters from `A-Za-z0-9._-`. The handoff says the definition declares that rule and the kernel checks it before the move to `tagged`. It does not say the rule matches the business intent; that is the reader's decision. The handoff status is `DECISION REQUIRED`.

Evidence:

- Delivery handoff first block: `Status: DECISION REQUIRED`; `Decision needed` lists rule `tag` as `needs your acceptance`.

## CHANGE

Questions: What changes for new instances? What happens to existing ones? Is a migration or permission needed?

### What changes for new instances?

Version 2 adds a command `extend`. It moves a loan's deadline to a later time; the new deadline must be after the current one. That is what the new definition declares. This packet does not prove that a live loan has already used `extend`.

Evidence:

- Journey 2 fixture `fixtures/loan_v2.ik.toml`: comment and `[commands.extend]` (`new_until_ms` must be later than `loan_until_ms`).
- Change handoff first block, under New in this version (labelled declared): states that Extend sets a later deadline on loans created under the new rules and refuses an equal or earlier one. A reader who says only "the output names Extend" has found the gap the first version had; accept "it lets a loan's due date be moved later" as correct. A reader may also correctly say the output does not show that Extend was ever run (declared, not demonstrated).

### What happens to existing ones?

Loans created under version 1 stay on version 1. Checkout and recall still work on them. They do not gain `extend`.

Evidence:

- Change handoff first block Existing instances lines naming kept version and working checkout/recall.
- Continuity scenario `fixtures/scenarios/existing-loan.json`.

### Is a migration or permission needed?

A decision is needed, not a migration: no running instance can be moved to version 2 in place. The reader chooses between keeping existing loans on version 1 until they finish (they cannot use `extend`) and letting them finish, then creating new instances on version 2 (this does not move an unfinished loan to version 2, and recreating a business operation can repeat external work). The reader also accepts or rejects the new `extend` rule. The store holds 1 unfinished loan on version 1 (in `on_loan`). The compatibility table finds no difference in `idle` or `on_loan` apart from `on_loan` gaining `extend`, and says stored values and unresolved effects were not checked. The reason for the change is quoted from the author, with its source, and is not verified by evidence.

Evidence:

- Change handoff: `Status: DECISION REQUIRED`; `Why` with source; `Decision needed` with the rule table and the two options; `Found: version 1 instances in the store: 1 unfinished`; the compatibility table; `Not established` lines for stored values and effect intents.

## DIAGNOSE / RECOVER

Questions: Did the operation commit? What is still uncertain? What action is safe next?

### Did the operation commit?

Yes. The earlier operation committed; this lookup did not commit again. Subject `item-lost`, operation `op-tag`. R1 is `exercised claim reconciled`.

Evidence:

- Recovery handoff first line: `Ikaros recovery: item-lost` and `operation: op-tag`.
- First block Demonstrated: `the earlier operation committed; this lookup did not commit again`.
- Exact evidence R1: `exercised claim reconciled` on `out/sc-lost#item-lost/found` `committed_exact`.

### What is still uncertain?

Whether any handle from this lookup is current. Whether the caller saved the operation id before sending is also not established. Readiness is not established (scenario-only evidence).

Evidence:

- Recovery handoff first block: Uncertain `whether any handle from this lookup is current`; Not established `whether the caller saved the operation id before sending`; the `Scope` block (checked: whether a recorded operation exists in the store; `Readiness: the owner's decision; this report does not state it.`) and the `Who attested what` table.

### What action is safe next?

Do not resend; the earlier result stands. No decision is asked.

Evidence:

- Recovery handoff first block: Safe next action `do not resend; the earlier result stands`; `Decision needed` / `none`.

## Glance (maps)

Withheld until scored. Derived from `ik diagram` analytics on the journey fixtures. Structural numbers ignore guard values.

### BUILD (tagged item map)

1. 1 step (structural).
2. No. Every state reaches an end (structural).
3. You are here at `idle`; next is `tag` (guarded); shortest finish from here is 1 (structural).

### CHANGE (loan v1 to v2 map)

1. No. Existing instances do not get `extend` (continuity: refused `IK_MACHINE_MISMATCH` when that evidence is present; otherwise the Existing column lists only the old commands).
2. Yes. Existing states drain under the old rules; reachability is unchanged (structural).
3. `extend` is added (`✚`). Reachability: unchanged (structural).

### RECOVER (lost-reply map)

1. You are here at the last evidenced event (asked again, found by lookup).
2. No.
3. Lookup by operation id; never resend.
