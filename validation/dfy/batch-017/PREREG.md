# Batch 017: fresh-family qualification of the full identifier and text grammar, pre-registered

Written and committed before any agent run. Not a held-out lineage. It does not reopen or change any earlier batch result; batches 015 and 016 stay as recorded.

## Question

Batches 015 and 016 showed that, without a length or character-class predicate, only the non-empty part of a stated identifier grammar could be enforced by the runtime. The runtime now has one bounded string predicate, `text_in(VALUE, 'SET', MIN, MAX)`: length in Unicode scalar values, an exact ASCII character set (or `*` for any scalar), no normalization. The BUILD and CHANGE guides now say to encode the declared grammar with that predicate, to leave opaque labels unguarded, and to report only a residual that is not a length-bounded character set.

Does a fresh agent given the packaged candidate deliver the complete stated grammar (length and character class) of required identifiers, codes and notes as a runtime guard, while free-form labels remain accepted and the lifecycle behavior is unchanged?

The guide text and the product capability were written in response to batches 015 and 016. A pass shows the combination works on two new families, not that it generalizes.

## What is scored (delivered behavior only)

Delivered behavior through the runtime. Use of the guide, `ik laws`, or the predicate by name is a diagnostic only.

Grammar table per family (fixed before dispatch; stated to builders only in plain domain language):

- g1 shared identifier: 1 to 64 characters of `A-Za-z0-9._-`, on five required arguments across several commands (one added by CHANGE).
- g2 command code: exactly 6 characters of `A-Z0-9`, command-specific error code.
- g3 note: 1 to 80 characters, any characters including spaces and non-ASCII, command-specific error code (BUILD note and CHANGE note).
- g4 free-form labels (three per family, one added by CHANGE): any characters, stored as given, no length limit; scored as benign (an over-constrained delivery fails).

Attacks per rule, each from an isolated pre-state after the honest twin committed, changing only the target argument; a rule passes only if every attack is refused with `IK_INVARIANT_FAILED` and canonical state is unchanged:

- g1: empty; 65 characters; space; `/`; `:`; a non-ASCII letter; a full-width digit; an astral emoji. Extra valid values that must commit: 64 characters; one using `.`, `_`, `-`; mixed case with digits.
- g2: too short; too long; lower case; `-`; empty; non-ASCII. Must commit: digits only; letters only.
- g3: empty; 81 characters. Must commit: 80 characters; a note with spaces, a non-ASCII letter and an emoji.
- g4: honest and extra values (`Blue Jay`, `Ünïcode label`, a 300-character string) must commit.

## Rule sets (fixed before dispatch)

Rule ids are those the evaluator emits. Machine-readable tables are in `manifest.json` only by digest; the ids:

- N24 harbor launch-ramp booking desk. BUILD: `launch_slots`, `due_at`, `launch_gate`, `ramp_set`, `trailer_bound`, `clerk_id`, `skipper_id`, `marshal_id`, `berth_id`, `lane_code`, `desk_note`, `boarded_n`, `slip_tag`, `boat_name`. CHANGE adds `extra_berth`, `abort_clear`, `abort_note`, `harbor_id`, `wharf_nick`. Identifier rules: `clerk_id`, `skipper_id`, `marshal_id`, `berth_id`, `harbor_id`. Grammar rules: identifier rules plus `lane_code`, `desk_note`, `abort_note`. Benign: `slip_tag`, `boat_name`, `wharf_nick`.
- N25 ski-patrol equipment issue desk. BUILD: `kit_mass`, `return_at`, `deploy_gate`, `slope_set`, `radio_bound`, `issued_n`, `ranger_id`, `base_id`, `partner_id`, `cache_id`, `kit_code`, `brief_note`, `callsign`, `cache_label`. CHANGE adds `extra_mass`, `scrap_clear`, `scrap_note`, `medic_id`, `trail_mark`. Identifier rules: `ranger_id`, `base_id`, `partner_id`, `cache_id`, `medic_id`. Grammar rules: identifier rules plus `kit_code`, `brief_note`, `scrap_note`. Benign: `callsign`, `cache_label`, `trail_mark`.

## Pass criteria (per replica and stage; all required rows)

1. Valid path with expected values. 2. Through the runtime with valid authority, an honest twin commits and every attack is refused as above, with every extra valid value committing. 3. Invalidation leaves no stale or `corrupt_state` facts. 4. Existing instances keep original rules (invalid value first, then valid, on a fresh old instance; status is checked before value). BUILD counts criteria 1 and 2 on the original rules; CHANGE counts 1 to 4 on all rules.

The evaluator is the unified, candidate-aware kit: the oracle's tables decide what must be true, the adapter only discovers how to invoke it from the delivered schema. Rows the adapter cannot route, or whose honest twin does not commit, are inconclusive: never a pass, never an enforcement or product finding.

First scoring is the record. If an instrument defect is found afterward, any rescoring is reported separately with the post-outcome repair disclosed; evaluator repairs are validated on toys only.

## Instrument (frozen before dispatch)

- The evaluator kit was unified and frozen with zero-diff regressions against the first scoring of the four earlier families (3020 rows) and a 28-toy conformance matrix on the batch 017 binary.
- The two families were authored by oracle authors that received requirements and tooling, not any prior delivery, and that did not see any delivered project.
- Self-test through the scorer path, both families: references, alternate-surface, echo, create-time, state-invariant and status-before-value toys pass; host-only variants fail criterion 2 on every rule; missing-input variants fail criterion 2 on exactly the input-validity rules; the shared-type-only control fails exactly the identifier rules; the non-empty-only control (batch 016 behavior) fails exactly the grammar rules; the length-only control fails exactly the rules with a character class; the over-constrained control fails the benign rows; stale, always-refuse and old-instance-breaking controls fail.
- Evaluators are sealed off disk before dispatch.

## Reporting

Table of rule by replica by criterion; the count of grammar-rule instances enforced by the runtime (the targeted quantity) with the length-only and charset-only breakdown; the count of identifier-rule instances against batch 016's non-empty-only 12 of 12 (descriptive; families and briefs differ); command-specific input rules (control); benign-label acceptance; the delivered guard text for each failed rule; inconclusive rows listed separately; the diagnostic on predicate and guide use; and what the result does and does not support (n=2 per family, one model condition, macOS arm64, no humans, no rate, no claim about other families).
