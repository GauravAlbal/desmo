# Batch 016: decision record

Pre-registered in `PREREG.md`, frozen in `manifest.json` and committed before dispatch. Two fresh families (N22 community radio program-slot desk, N23 clinic supply-room restock desk), two replicas each, BUILD then CHANGE, TypeScript, macOS arm64, packaged candidate whose runtime is byte-identical to batch 015's; only the BUILD and CHANGE guide text differs (it now requires guarding the expressible part of an inexpressible contract grammar and reporting only the residual). Per-rule rows for both tiers are in `results.json`.

## First scoring (the record)

Per replica and stage:

| Stage | N22 r1 | N22 r2 | N23 r1 | N23 r2 |
|---|---|---|---|---|
| BUILD | pass | **fail** | pass | pass |
| CHANGE | pass | **fail** | pass | pass |

6 of 8 stages pass on first scoring. The only failing replica, N22 r2, fails criterion 2 on `cart_total` and `air_gate` (BUILD and CHANGE) and `extra_cart` (CHANGE), every one reported `twin_failed`: the honest twin did not commit, so no attack could count. Unroutable and twin-failed rows are never a pass, so the first scoring stands.

One N23 r1 builder run failed at startup with a provider network error (`ECONNRESET`, 6 seconds, no work done) and was re-dispatched on the same untouched run directory; disclosed, not scored.

## Instrument defect found after first scoring (disclosed)

The delivered N22 r2 send command declares companion arguments whose guard requires them to equal stored facts and to echo the request id (`$play_slot_id == slot_id && $play_producer_id == producer_id && $play_band == band && $effect_id == $play_req_id && $idempotency_key == $play_req_id && $payload_digest != ''`). The evaluator's honest-twin filler filled such string companions by type, so the honest call was refused. That is an instrument gap and not evidence about enforcement.

Repair: for a non-target parameter, honest candidates come from the briefs' role table, then from equalities in the delivered guard text (`$param == fact`, `$p == $q`, effect-identity companions copying the request id), then a non-empty placeholder for a `!= ''`-only parameter. Guard text only affects the twin's precondition; the attack still changes only the target argument, passes only on `IK_INVARIANT_FAILED` with a rule and unchanged state, and only after the honest twin commits. The repair was written by oracle agents without access to the deliveries and validated on toys only: three new echo toys per family (reference passes everything, host-only twin fails criterion 2 on every rule as `committed`, missing-input twin fails exactly the input rules), the old behavior was shown to fail the echo reference before the fix, and every existing toy kept its verdict. The repair was made after the `twin_failed` rows on N22 r2 had been seen; this is disclosed.

## Rescoring with the repaired evaluator (reported separately)

8 of 8 stages pass in all four replicas. No row of any replica fails any criterion.

## The targeted quantity

Rule-instances stated only through the shared identifier grammar (`producer_id`, `host_id`, `engineer_id` in N22; `clerk_id`, `reviewer_id`, `pharmacist_id` in N23), scored as: a direct runtime command with an empty value is refused with a rule-attributable code and state unchanged, with an honest twin committing: **12 of 12** (batch 015: 2 of 8, different families, briefs and wording; the comparison is descriptive only). Command-specific input rules (`band_set`, `mic_bound`, `kill_note`, `item_set`, `carton_bound`, `void_note`): 12 of 12 (batch 015: 12 of 12). Benign stored-only and opaque-label arguments were accepted for ordinary values in every replica (no over-constraint row failed).

## Diagnostic: how the agents reasoned (not scored)

All four BUILD handoffs and all four CHANGE handoffs say the machine guards the non-empty part (`$arg != ''`) and that the 1-to-64-character and character-class residual is host-only because the language cannot express it; the CHANGE replicas name the residual as unresolved or unmet with the argument names. This is the behavior the revised wording asks for. The N23 brief states in one sentence that the reviewer identifier is required, which is a command-specific statement of one of its three identifier rules; the other five identifier rules in the two families are stated only through the grammar.

## What this supports and does not

Supports: on two fresh families and four replicas, agents given the revised wording delivered the non-empty part of every required identifier, every command-specific input rule, every derived-value rule and the benign arguments as runtime-enforced behavior, and reported the inexpressible residual. The residual (length bound, character class) is not machine-enforced and is not scored; the product still has no length or character-class predicate, so the full contract grammar remains host-enforced.

Does not support: a rate, a comparison with batch 015 (different families, briefs, evaluator and wording), a claim about other families or agent models, `DFYJourneyQualified`, or that the wording alone caused the change: it was written in response to the batch 015 finding and the batches differ in more than the wording. Adapting the guide wording in response to an earlier batch does not by itself make a later batch non-independent; what makes the 8 of 8 non-independent is that the evaluator was repaired after the failing rows had been seen. n=2 per family, one model condition (not exposed), macOS arm64, no humans, no Linux x64. The first scoring is 6 of 8 stages; the 8 of 8 comes from an evaluator repaired after the failing rows were seen.

## Next

- Product decision (owner-facing, not taken here): whether to add a string-length or character-class predicate to the machine language so the residual becomes enforceable.
- Evaluate the wording on an independent family set the oracle authors write without sight of this record, ideally with a candidate-aware evaluator frozen before dispatch.
