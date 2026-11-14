# Batch 015: decision record

Pre-registered in `PREREG.md`, frozen in `manifest.json` and committed before dispatch. Two fresh families (N20 community garden plot desk, N21 school field-trip desk), two replicas each, BUILD then CHANGE, TypeScript, macOS arm64, candidate bytes with the `ik laws` advisory and the corrected guide dispositions. Per-rule rows for every tier are in `results.json`.

## First scoring (the record)

BUILD 0 of 4, CHANGE 0 of 4. The first scoring is the record and is not replaced.

Reviewing the failing rows against the briefs before reading anything into them showed that most were instrument defects, not findings about the deliveries (rows reported `unroutable` or `twin_failed`, or an oracle sequence error). Unroutable and twin-failed rows are never a pass, so the first scoring stands, but it could not show whether the rules were enforced.

## Instrument defects found after first scoring (disclosed)

1. **Creation-time rules** (N20 `bed_limit`): the attack seeded the aggregate from a hard-coded initial-state name and looked for a command in that state. Repaired to read the initial state and the creating command's declared parameters from the delivered machine's schema, with the honest-twin requirement unchanged.
2. **Honest twin for commands with brief-stated companion literals** (N21 `depart_at`): the twin filler did not supply the fixed literals the brief states for a lock-and-label command, so the honest call was refused (`twin_failed`). Repaired with role-keyed brief literals and, as a fallback, literals read from the delivered guard text; the guard-text fallback affects only the twin's precondition, never the attack's pass or fail.
3. **Oracle error in criterion 4** (N21 `dest_set`, `seat_bound`): the sequence sent the valid call first and the invalid call second on the same instance, so a brief-conformant program (status refusal before value refusal) correctly answered a status code. Repaired to send the invalid value first on a fresh instance, then the valid value; every criterion 1 to 4 row of both evaluators was audited for the same flaw.

All three were repaired by oracle agents given only the briefs and the evaluator code, validated on toys before any delivery was rescored, with new alternate-surface toys for each class. The repairs were written after the first-scoring row details on the deliveries had been read, and the three defect classes were identified from them; this is disclosed, not hidden.

4. **Machine discovery** (both families, found after the rescoring above): the evaluator's machine discovery walked into the staged kit directory and the agents' own scratch evidence, and picked a shipped example machine with a higher version as the one to seed. This made N20 `bed_limit` still `unroutable` after repair 1 even though the delivered `open` command carries the bound. Fixed by excluding the kit, scratch-evidence, skill and stub directories from discovery. This fix was made after reading the repaired-evaluator output on the deliveries. The toy matrix was re-run after it and is unchanged (`selftest-v2.1-matrix.txt`).

## Rescoring with the repaired evaluators (v2.1; reported separately)

BUILD 1 of 4 and CHANGE 1 of 4 (N21 r2 passes both stages; no other replica passes either). The intermediate tier before the discovery fix is in `results.json` and also scored 2 of 8 stages.

| Replica | Rules failing criterion 2 (all reported as `committed`) |
|---|---|
| N20 r1 | `steward_id` (BUILD and CHANGE), `abort_clear` (CHANGE, empty `officer_id`) |
| N20 r2 | `steward_id` (BUILD and CHANGE), `abort_clear` (CHANGE, empty `officer_id`) |
| N21 r1 | `aide_id` (BUILD and CHANGE), `coordinator_id` (CHANGE) |
| N21 r2 | none |

Every other row, including all derived-value rules, all command-specific input rules and all benign stored-only arguments, passed in every replica. In every failing row a direct runtime command with valid authority and an empty identifier committed: the identifier-format rule is stated in the briefs only through the shared identifier grammar (1 to 64 characters), and the machine does not enforce even its non-empty part.

## The targeted quantity

Input-validity rule-instances enforced by the runtime: 14 of 20.

- Command-specific rules (`crop_bound`, `bed_limit`, `abort_note`, `dest_set`, `seat_bound`, `cancel_note`): 12 of 12.
- Rules stated only through the shared identifier grammar (`steward_id`, officer, `aide_id`, `coordinator_id`): 2 of 8. Only N21 r2 enforced them, and it enforced them as non-empty guards.

For context, batch 014 enforced 6 of 8 with different families, briefs and evaluator; the two are not comparable and no improvement or regression is claimed.

## Diagnostic: did the agents use the advisory?

Not scored. From the agents' own handoffs, all four BUILD replicas cite the unguarded-argument inventory. Three (N20 r1, N20 r2, N21 r1) described the identifier grammar as "unguarded stored strings", "not expressible as a guard" or "host-checked (no identifier type)" and left it in host code, which the guide's disposition (d) treats as an unmet requirement and not as an assumption. N21 r2 reported an empty inventory and had guarded every identifier it carried. The language has no string-length or character-class predicate, so the full grammar is genuinely not expressible, but its non-empty part is (`$id != ''`), and the three agents that left it in host code did not enforce that part.

## What this supports and does not

Supports: on two fresh families and four replicas, command-specific input-validity rules and the benign-argument and derived-value rules were enforced by the runtime in every replica (12 of 12 command-specific input rules), and the shared-grammar identifier rule remained host-only in 6 of 8 instances even where the advisory flagged it. The miss is a guide and language question, not an advisory-visibility question: the advisory showed the arguments and agents chose not to guard the expressible part.

Does not support: a rate, a comparison with batch 014, any claim about other families, `DFYJourneyQualified`, or that the advisory prevents the miss (it did not here). n=2 per family, one model condition, macOS arm64, no humans. Evaluator repairs were made after the first-scoring details and again after the v2 output were seen (disclosed above), by oracle agents, validated on toys.

## Next

- Guide: say that when an argument's full contract grammar is not expressible, the expressible subset (at least non-empty, equality with a literal set) MUST still be a guard and the residual is reported as an unmet requirement. The language has no length or character-class predicate; adding one is a product decision, not part of this check.
- A fresh-agent check of that wording on new families, with candidate-aware evaluation from the start.
