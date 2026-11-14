# Batch 012: finite transfer check, pre-registered

Written and committed before any agent run. Not a qualification batch and not a held-out lineage.

## Claim this check can support

Only this: with the repaired candidate bytes and the updated build and change guides, builders on two fresh families, two replicas each, did or did not enforce every contract-named derived value through a machine guard in this finite sample. It does not support a rate, a comparison, `DFYJourneyQualified`, or any claim about other families. Batch 011 remains the scored result for its exact bytes and does not transfer to these bytes.

## Candidate and host

- Candidate: public export of the source revision recorded in `manifest.json` (macOS arm64), which includes the coverage-contract repair (`0f41079`). An earlier export (`d0927f4`) was built and passed its targeted regressions, then superseded by that repair; it is not the subject of this check. The exact binary and package digests are recorded in `manifest.json` before dispatch.
- Instrument limits stated in advance: both families are TypeScript deliveries; the direct-command harness routes by the delivered machine's identity and command names, and a criterion 2 case it cannot route is reported as unroutable (never counted as pass) and cross-checked by a candidate-aware read of the delivered machine; criterion 2 is counted per rule only where criterion 1 passes for that rule; the runtime-use check from batch 011 (store sufficiency) is not part of this check.
- Agent: general-purpose agent with a shell, same condition as batch 011. Model identity is not exposed.
- Two fresh families (names assigned by the oracle author, not previously used), two replicas each, BUILD then CHANGE, sequential, evaluators sealed off disk during agent waves.

## Fresh-family requirements (given to a blind oracle author)

Each family's brief must name, in plain domain language and without any hint about implementation, at least three consequential rules of these kinds: an amount computed from stored quantities (multiplication, subtraction or integer division), a deadline computed from a stored time plus a stored span, and a gate that depends on a stored flag and a time comparison. The change brief must add one rule of the same kinds and one invalidation (a command that must clear every fact a later projection reads). Briefs must not mention guards, arithmetic forms or the machine language.

## Pre-registered pass criteria (per replica; all must hold)

For every consequential rule named in the brief or change:
1. Valid path: the legitimate request sequence through the delivered runtime is accepted and projects the expected values.
2. Direct-command refusal: a direct runtime command with valid authority and a deliberately wrong derived value (wrong amount, wrong deadline, gate bypass) is refused by the runtime with a rule-attributable code, using only supported entry points (no store tampering).
3. Invalidation: after the invalidating command, a following reconcile or projection is not `corrupt_state` and shows no stale value.
4. Existing instances: the continuity check from batch 011 (old instances keep original rules).

A rule that the delivered host checks only in host code fails criterion 2 for that rule. Per-rule results are reported, not only the replica total. Results are kept as first scored; no rescoring or hand repair.

## Reporting

A table of rule by replica by criterion, the delivered guard text for each failed rule, and an honest statement of what the result does and does not support. Exposed fixtures from batch 011 (N13, N15) are not reused as held-out content.
