# Batch 016: focused check of guarding the expressible part of an inexpressible contract grammar, pre-registered

Written and committed before any agent run. Not a qualification batch and not a held-out lineage. It does not reopen or change any earlier batch result.

## Question

Batch 015 showed command-specific input rules enforced by the runtime in 12 of 12 instances, but rules stated only through the shared identifier grammar (1 to 64 characters of `A-Za-z0-9._-`) host-only in 6 of 8 instances: an empty identifier committed through a direct runtime command even though all four agents cited the `ik laws` inventory. The language has no length or character-class predicate, so only the non-empty part of that grammar is machine-expressible. The BUILD and CHANGE guides now say: guard the expressible part (non-empty, equality with each literal of a stated set, a numeric comparison) on every argument that carries the rule, and report only the residual as unmet.

Does a fresh agent given the packaged candidate with that wording deliver the non-empty part of required identifiers as a runtime guard, without over-constraining opaque labels or benign stored-only arguments?

This tests a wording written in response to the batch 015 finding. It is therefore not independent of that finding, and a pass would show only that the revised wording coincided with the behavior on two new families, not that it generalizes.

## What is scored (delivered behavior only)

Whether an agent ran `ik laws` or quoted the guide is recorded as a diagnostic and is not an acceptance criterion. The length bound and the character class of the identifier grammar are not machine-expressible and are not scored (`not_scored: inexpressible`); an agent is not required to reject a 65-character identifier or a punctuation character, and an agent that does so in a way that breaks a legitimate value fails on the over-constraint rows.

## Candidate and host

- Candidate: public export of the source revision in `manifest.json`, packaged binary and npm tarballs, macOS arm64, feasibility adapter off. The runtime binary is byte-identical to batch 015's; the BUILD and CHANGE guide text is the only difference, so the manifest records the staged skill digests.
- Agent: general-purpose agent with a shell, the same condition as batches 011 to 015. Model identity is not exposed.
- Two fresh families (`N22` community radio program-slot desk, `N23` clinic supply-room restock desk), two replicas each, BUILD then CHANGE, TypeScript, evaluators sealed off disk during agent waves. Oracle authors are independent of the builders; they were given the batch 014 and 015 lessons: every command a brief names is defined; input rules are stated both as command-specific sentences and through the shared identifier grammar; briefs never mention guards, machines or the advisory. The N23 brief states in a sentence that one identifier is required, which makes that one rule command-specific in statement; the other identifier rules are stated only through the grammar.

## Rule sets (fixed before dispatch)

Rule ids are those the evaluator emits; identifier rules are the scored shared-type rules.

- N22 BUILD: `cart_total`, `air_at`, `air_gate`, `band_set`, `mic_bound`, `producer_id`, `host_id`, `studio_matches`. CHANGE adds `extra_cart`, `kill_clear`, `kill_note`, `engineer_id`. Input-validity: `band_set`, `mic_bound`, `producer_id`, `host_id`, `kill_note`, `engineer_id`. Identifier rules: `producer_id`, `host_id`, `engineer_id`. Benign: `show_tag`, `voice_color`, `desk_mark`.
- N23 BUILD: `fill_units`, `due_at`, `receive_gate`, `item_set`, `carton_bound`, `clerk_id`, `reviewer_id`, `received_n`. CHANGE adds `extra_buffer`, `void_clear`, `void_note`, `pharmacist_id`. Input-validity: `item_set`, `carton_bound`, `clerk_id`, `reviewer_id`, `void_note`, `pharmacist_id`. Identifier rules: `clerk_id`, `reviewer_id`, `pharmacist_id`. Benign: `ticket_tag`, `shelf_code`, `lot_code`.

## Pass criteria (per replica and stage; all required rows)

As in batch 015: 1. valid path with expected values; 2. through the runtime with valid authority, an honest twin commits and the deliberately invalid value (for an identifier rule, an empty value) is refused with a rule-attributable code (`IK_INVARIANT_FAILED`) with canonical state unchanged; 3. invalidation leaves no stale or `corrupt_state` facts; 4. existing instances keep their original rules (invalid value first, then valid, on a fresh old instance, because the briefs' precedence is status before value). BUILD counts criteria 1 and 2 on the original rules; CHANGE counts 1 to 4 on all rules. Candidate-aware mapping: the evaluator discovers the delivered commands, their declared parameters and the initial state from the delivered machine schema, excludes the staged kit, scratch evidence, skills and stub from machine discovery, fills companion literals from the briefs' role table and then from the delivered guard text (precondition only), and requires the honest twin to commit before any attack counts; unroutable and twin-failed rows are non-pass with the reason.

Results are kept as first scored. If an instrument defect is found afterward, any rescoring is reported separately and first scoring stays the record.

## Instrument self-test (required before dispatch; recorded)

Through the scorer path on these bytes, both families: reference, alternate-surface reference, state-invariant and status-before-value toys pass; host-only and alternate-surface host-only fail criterion 2 on every rule; missing-input and alternate-surface missing-input fail criterion 2 on exactly the input-validity rules; the shared-type-only-missing control fails criterion 2 on exactly the identifier rules; the over-constrained control fails; stale, always-refuse and old-instance-breaking controls fail.

## Reporting

A table of rule by replica by criterion; the count of identifier-rule instances enforced by the runtime (the targeted quantity) against batch 015's 2 of 8, stating the families, briefs and wording differ and that the comparison is descriptive; the count of command-specific input rules (the control quantity, 12 of 12 in batch 015); the delivered guard text for each failed rule; the diagnostic on advisory use; an honest statement of what the result does and does not support (n=2 per family, one model condition, macOS arm64, no humans, no rate, no claim about other families).
