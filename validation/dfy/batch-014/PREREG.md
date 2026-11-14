# Batch 014: finite transfer check on the input-validity guide wording, pre-registered

Written and committed before any agent run. Not a qualification batch and not a held-out lineage.

## Claim this check can support

Only this: with the candidate bytes recorded in `manifest.json` and the build and change guides that now say input-validity rules are `when` guards, builders on two fresh families, two replicas each, did or did not enforce every consequential rule named in the brief and the change, including input-validity rules, through a machine guard in this finite sample. It does not support a rate, a comparison against batch 012, `DFYJourneyQualified`, or any claim about other families. Batches 011 and 012 remain the scored results for their own bytes and do not transfer to these bytes. The only guide change since batch 012 is the added input-validity wording in `skills/ikaros/build.md` and `change.md`; the runtime differences since batch 012 are the coverage-contract repair and the optional feasibility adapter (off in this check: no solver is configured for the builders).

## Candidate and host

- Candidate: public export of the source revision in `manifest.json` (macOS arm64). Binary and package digests are recorded before dispatch.
- Instrument limits stated in advance: both families are TypeScript deliveries; the evaluator routes direct commands by the delivered machine's identity and command names, and a criterion 2 case it cannot route is reported as unroutable and never counted as pass; the runtime-use check from batch 011 (store sufficiency) is not part of this check; same-account isolation limits from the custody check still apply.
- Agent: general-purpose agent with a shell, same condition as batches 011 and 012. Model identity is not exposed.
- Two fresh families (`N18`, `N19`, names and domains assigned by independent oracle authors, not previously used), two replicas each, BUILD then CHANGE, sequential, evaluators sealed off disk during agent waves.

## Fresh-family requirements (given to blind oracle authors)

Each BUILD brief names at least five consequential rules covering: an amount computed from stored quantities, a deadline computed from a stored time plus a stored span, a gate depending on a stored flag and a time comparison, an input-validity rule on a command argument that is not a derived value, and a rule relating a command argument to a stored fact. The change brief adds one amount or deadline rule, one more input-validity rule and one invalidation, with old instances keeping original rules. Briefs do not mention guards, arithmetic forms or the machine language.

## Rule sets (fixed before dispatch)

- N18 BUILD: `chem_volume`, `soak_ready`, `unload_gate`, `paper_grade`, `collector_differs`; CHANGE adds `print_sheets`, `abort_clear`, `abort_note`. Input-validity rules: `paper_grade`, `abort_note`.
- N19 BUILD: `pack_mass`, `freeze_until`, `freeze_release_gate`, `lane_bound`, `distinct_receiver`; CHANGE adds `abort_clear`, `retain_vial`. Input-validity rules: `lane_bound`, `abort_clear`.

## Pre-registered pass criteria (per replica; all must hold)

For every rule: 1. valid path accepted with expected values; 2. a direct runtime command with valid authority and a deliberately wrong value (wrong derived value, bypass, or invalid input) is refused by the runtime with a rule-attributable code, using only supported entry points; 3. (invalidating rule) after the invalidating command a following reconcile or projection is not `corrupt_state` and shows no stale value; 4. existing instances keep original rules (continuity from the trajectory's own build). BUILD stage counts criteria 1 and 2 on the original rules; CHANGE counts 1 to 4 on all rules.

A rule enforced only in host code fails criterion 2 for that rule. Per-rule results are reported, not only totals. Results are kept as first scored; no rescoring or hand repair. If a harness defect is found after first scoring, any rescoring is reported separately and first scoring stays the record.

## Instrument self-test (required before dispatch and recorded)

Through the scorer path on the candidate bytes: the reference build passes both stages; the host-only control fails criterion 2 on every rule; a control that enforces everything except the input-validity rules fails criterion 2 on exactly those rules; stale-fact, always-refuse and old-instance-breaking controls fail.

## Reporting

A table of rule by replica by criterion, the delivered guard text for each failed rule, the count of input-validity rules enforced by a machine guard (the quantity this check targets), and an honest statement of what the result does and does not support.
