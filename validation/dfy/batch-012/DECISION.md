# Batch 012: finite transfer check on the repaired bytes (decision record)

Not a qualification batch and not a held-out lineage (see `PREREG.md`, frozen with `manifest.json` before any agent ran). Two fresh payment-rail families, two replicas each, BUILD then CHANGE, installed bytes from the manifest, TypeScript deliveries, general-purpose agent with a shell, evaluators sealed during all agent waves.

## First scoring (the record)

| Family / replica | BUILD | CHANGE |
|---|---|---|
| N16 card authorization r1 | FAIL (instrument defect, see below) | FAIL (instrument defect) |
| N16 card authorization r2 | pass | pass |
| N17 payout desk r1 | pass | FAIL: `risk_clear` criterion 2 |
| N17 payout desk r2 | pass | FAIL: `risk_clear` criterion 2 |

First scoring: BUILD 3 of 4, CHANGE 1 of 4. Per-rule results for every delivery are in `results.jsonl`.

## Instrument defect found after scoring (reported separately, first scoring unchanged)

The N16 harness launched `./run` through `node`, so a delivered `./run` written as a bash wrapper (N16 r1) failed criterion 1 for every rule before any rule was tested. The brief specifies an executable `./run`, not a Node script, so this is a harness defect, not an agent defect. The harness was changed to execute `./run` directly, the instrument self-test (positives pass, negatives fail, through the scorer path) was re-run and passed, and N16 r1 was rescored once: BUILD pass and CHANGE pass, every rule at every criterion. That rescoring is recorded as `v2-harness-fix-reported-separately` in `results.jsonl`. The fix was made after seeing the first results; it is not part of the first scoring and it does not change the N17 results.

## What the per-rule results show

- Derived values (amounts, fees, deadlines, spans, caps, approval bars): across all 8 deliveries, every one was refused by the runtime when proposed wrongly through a direct command with valid authority (criterion 2), including with the corrected N16 r1 scoring. The class that failed in batch 011 (host-computed values copied unchecked) did not recur in this finite sample. The delivered N16 r2 guards use the new division and multiplication forms.
- Invalidation: after the invalidating command (void, edit-after-approval, risk clear) reconcile and projections were not `corrupt_state` and showed no stale facts in all deliveries (criterion 3).
- Existing instances kept original rules in all deliveries (criterion 4).
- Failure: in both N17 replicas the brief's rule that an empty or malformed `reviewer_id` is refused is enforced only by host code; the machine accepts an empty reviewer id through a direct command. Both machines could have expressed it as a guard (`$reviewer_id != ""`). A different class from the one the repair targeted: an input-validity refusal, not a derived value. It is an authority gap for any claim that the runtime enforces that rule.
- A candidate-aware reading of every delivered machine (`crosscheck.md`) agrees with the automated results and adds observations the evaluator did not test: weaker-than-brief guards in N16 r2 (refund amount lower bound; authorize time equality) and a host-only edition check in N17 r2 `risk_clear`.

## What this does and does not support

It supports: on these bytes, in this finite sample, agents following the shipped guidance enforced the derived-value rules through machine guards for every named amount, deadline and span, and handled invalidation and old instances correctly. It does not support a rate, a comparison, any claim about other families or languages, or `DFYJourneyQualified`. The first-scoring floor of this check (all stages) was not met; the residual miss is the input-validity class. The guide names amounts, deadlines, approval predicates and invalidations but not "any refusal the contract names"; that wording is a candidate product follow-up (filed), not applied here, so these results stay attached to these exact bytes.

## Limits

No Linux x64, no human participants, agent model identity not exposed, both families TypeScript, N17 has no host-only BUILD-stage control toy, the runtime-use (store sufficiency) check from batch 011 was not part of this check, and same-account custody limits apply (the key was held in process memory during the waves and saved only afterwards). Delivered projects are in `submissions.tar.gz` (text files only, paths cleaned). N16 and N17 are now exposed and become regression fixtures, not held-out tests.
