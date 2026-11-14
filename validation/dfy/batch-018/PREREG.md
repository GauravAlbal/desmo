# Batch 018: prospective two-family confirmation, pre-registered

Written and committed before any agent run, on a frozen candidate and a frozen evaluator. Retains every earlier result unchanged (batches 015 to 017 stay as recorded, including batch 017's first scoring of 0 of 8 and its later rescorings).

## Scope amendment (stated openly)

The original qualification requirement was three fresh families with two replicas each. This batch is two fresh families with one replica each (four stages in BUILD and CHANGE pairs: two BUILD, two CHANGE). It is a finite confirmation, not a rate, and it does not retroactively satisfy the original three-family, two-replica wording. The reduction is a decision to right-size the gate; it is not evidence that the larger exercise occurred.

## Question

On a frozen packaged candidate that includes the bounded string predicate `text_in` and the corrected readout surfaces, and under a frozen capture-and-replay evaluator with the strict acceptance contract, does a fresh agent deliver the complete stated grammar of required identifiers, codes and notes as runtime-enforced behavior, with free-form labels accepted and existing instances unchanged?

## Candidate and instrument (frozen before dispatch; digests in `manifest.json`)

- Candidate: public export of the source revision in the manifest, packaged binary and npm tarballs, macOS arm64.
- Evaluator: the retained capture-and-replay kit (`evalkit-v2`) and two fresh family evaluators, authored by oracle authors that received requirements and tooling but no delivery. The kit is not changed after freeze. If a defect in it is found afterward, the first outcome stands and any repair and rescoring is reported separately.
- Scoring contract: exact required coordinates (BUILD criteria 1 and 2; CHANGE criteria 1 to 4; every scored rule including benign labels, no exemption), strict boolean results, runner exit and timeout part of the decision. Rows reported `twin_failed` or `unroutable` are inconclusive: never a pass, never an application finding.
- Execution: at most two parallel workers; one replica's build and change stages never run concurrently; stage scoring happens in isolated scratch, `TMPDIR` and working directory.
- Failure handling (declared in advance): the first outcome of each stage is the record. A serial rerun may be run to diagnose a non-pass; it is reported next to the first outcome and never replaces it. A four-worker transient seen earlier is an unresolved load-dependent anomaly; no four-worker claim is made.

## Families and scored rules

Rule ids are those the evaluator emits.

- N26 community-garden tool-lending desk. BUILD: `loan_grams`, `soak_at`, `checkout_gate`, `plot_set`, `crate_bound`, `loaned_n`, `steward_id`, `plot_id`, `tender_id`, `shed_id`, `bin_code`, `care_note`, `nick`, `shed_label`. CHANGE adds `extra_grams`, `void_clear`, `void_code`, `warden_id`, `hedge_mark`. Identifier rules: `steward_id`, `plot_id`, `tender_id`, `shed_id`, `warden_id`. Grammar rules: the identifiers plus `bin_code`, `care_note`, `void_code`. Benign: `nick`, `shed_label`, `hedge_mark`.
- N27 bicycle-shop repair-ticket desk. BUILD: `labor_total`, `ready_at`, `dispatch_gate`, `frame_set`, `hours_bound`, `billed_n`, `tech_id`, `bay_id`, `rider_id`, `bin_id`, `seal_code`, `shop_note`, `rack_nick`, `bin_mark`. CHANGE adds `extra_labor`, `void_clear`, `void_note`, `clerk_id`, `scrap_tag`. Identifier rules: `tech_id`, `bay_id`, `rider_id`, `bin_id`, `clerk_id`. Grammar rules: the identifiers plus `seal_code`, `shop_note`, `void_note`. Benign: `rack_nick`, `bin_mark`, `scrap_tag`.

Grammar attacks and valid-also values are those of batch 017 (identifier 1 to 64 of `A-Za-z0-9._-`; code exactly six of `A-Z0-9`; note 1 to 80 any characters; free-form labels unbounded).

## Pass criteria (per replica and stage; all required rows)

1. Valid path with expected values. 2. An honest captured request, replayed from an equivalent pre-state, commits; each oracle-specified attack on the target argument alone is refused with `IK_INVARIANT_FAILED` and a rule, with canonical state unchanged; each valid-also value commits. 3. Invalidation leaves no stale or `corrupt_state` facts. 4. Existing instances keep original rules. BUILD counts criteria 1 and 2; CHANGE counts 1 to 4.

## Self-test (required before dispatch; recorded)

Through the scorer path on the frozen bytes, both families: references, alternate-surface, echo, create-time, derived, effect-then-record, state-invariant and status-before-value toys pass; host-only variants fail criterion 2 on every rule; missing-input variants fail exactly the input-validity rules; the shared-type-only control fails exactly the identifier rules; the non-empty-only control fails exactly the grammar rules; the length-only controls fail exactly the rules with a character class; the over-constrained control fails the benign rows; stale, always-refuse and old-instance-breaking controls fail.

## Reporting

Table of rule by stage by criterion; counts of grammar-rule, identifier-rule, command-specific input-rule and benign-label instances enforced or accepted; inconclusive rows listed separately; the delivered guard text for each failed rule; first outcomes and any diagnostic reruns side by side; and what the result does and does not support: n=1 per family, one model condition, macOS arm64, no humans, no rate, no claim about other families, and not a substitute for human review, consumer acceptance, native Linux x64, publication, or any trust-boundary claim.
