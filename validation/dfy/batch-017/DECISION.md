# Batch 017: decision record

Pre-registered in `PREREG.md`, frozen in `manifest.json` and committed before dispatch. Two fresh full-grammar families (N24 harbor launch-ramp booking desk, N25 ski-patrol equipment issue desk), two replicas each, BUILD then CHANGE, TypeScript, macOS arm64, packaged candidate with the bounded string predicate `text_in`. Per-rule rows for both scorings are in `results.json`.

## First scoring (the record)

| Stage | N24 r1 | N24 r2 | N25 r1 | N25 r2 |
|---|---|---|---|---|
| BUILD | fail | fail | fail | fail |
| CHANGE | fail | fail | fail | fail |

0 of 8 stages pass on first scoring. Every failing row is `twin_failed` (the evaluator's honest companion call did not commit, so no attack could count; an inconclusive instrument gap, never a pass and never an enforcement finding). No row reports `committed` for an attack on a grammar rule: no attack was accepted anywhere it was tried.

Rows affected: N24 r1 `clerk_id`, `skipper_id`, `slip_tag` (creating command); N24 r2 `launch_slots`, `launch_gate`, `boarded_n` (+ `extra_berth` in CHANGE); N25 r1 and r2 `kit_mass`, `deploy_gate`, `issued_n` (+ `extra_mass` in CHANGE).

## Instrument defect found after first scoring (disclosed)

The honest-twin filler could not satisfy delivered guards of three generic kinds: (1) equality between two clock arguments on a creating command (`$posted_ms == $now_ms`); (2) a character-set and length predicate on a digest-like effect argument (`text_in($payload_digest, 'a-f0-9', 64, 64)`) next to request-id echoes; (3) derived relations to stored facts (`$kit_mass == radio_count * 175`, `$issued_n == radio_count`, `launch_slots == trailer_count + extra_berths`).

Repair: guard text may only inform the twin's precondition. Equalities between declared arguments are unified, `text_in` constraints on non-target arguments yield values within the bounds built from the set, and integer relations over stored facts and constants are evaluated against the instance's stored facts; whatever cannot be satisfied stays `twin_failed`. The attack still changes only the target argument and passes only on `IK_INVARIANT_FAILED` with a rule and unchanged state. The repair was written by an evaluator-repair agent without access to any delivery, validated on toys only (new reference, host-only, missing-input and length-only toys per class; the existing selftest matrix unchanged for every toy), and made after the delivery rows had been seen. Because the repair followed the outcomes, the rescoring is not independent evidence.

## Rescoring with the repaired evaluator (reported separately)

8 of 8 stages pass in all four replicas; no row of any replica fails any criterion.

## The targeted quantity

Rule-instances through the CHANGE stage (rule x replica), criterion 2, i.e. a direct runtime command with each stated grammar attack is refused with a rule-attributable code and canonical state unchanged after an honest twin committed, and each valid-also value commits:

| Quantity | First scoring (conclusive rows only enforce; the rest inconclusive) | Rescoring |
|---|---|---|
| Identifier grammar rules (1 to 64, `A-Za-z0-9._-`) | 18 of 20 (2 inconclusive) | 20 of 20 |
| All grammar rules (identifiers, code, notes) | 30 of 32 (2 inconclusive) | 32 of 32 |
| Rules with a character class | 22 of 24 (2 inconclusive) | 24 of 24 |
| Command-specific input rules | 38 of 40 (2 inconclusive) | 40 of 40 |
| Benign free-form labels accepted | 11 of 12 (1 inconclusive) | 12 of 12 |

Batch 016 enforced only the non-empty part (12 of 12 identifier non-empty rules); the comparison is descriptive: families, briefs, evaluator and product differ.

## Diagnostic: how the agents reasoned (not scored)

All eight handoffs report the identifier, code and note grammars as machine guards written with `text_in`, and the opaque labels (`slip_tag`, `boat_name`, `callsign`, `cache_label`) as intentionally unguarded.

## What this supports and does not

Supports: on two fresh families and four replicas, agents given the packaged candidate with `text_in` delivered the complete stated length and character-class grammar of required identifiers, codes and notes as runtime-enforced behavior, with no attack accepted anywhere it was tried, free-form labels accepted, and old instances unchanged; and the full-grammar finding does not rely on host code.

Does not support: a rate, a comparison with earlier batches, a claim about other families or agent models, `DFYJourneyQualified`, or independence of the 8 of 8 (the first scoring is 0 of 8 and every failure is an inconclusive instrument gap; the 8 of 8 comes from an evaluator repaired after the rows were seen). n=2 per family, one model condition (not exposed), macOS arm64, no humans, no Linux x64.

## Next

- Evaluate the candidate-aware twin synthesis on an independent family set frozen before dispatch (the three defect classes above are now in the evaluator, but only validated on toys).
- Owner-facing leaves remain open and separate: worker and acceptance-authority separation, human review, consumer approval, Linux x64, licensing, publication, and the downstream note.

## Addendum: scorer completeness defect and reconciliation (new evidence result)

A review of the committed scorer found that its stage decision did not enforce completeness: a rule with only some of its required criteria present counted as passing, a non-boolean `pass` value was truthy, and the runner exit status and timeout were recorded but not part of the decision. This affects how the stage totals above may be read; it does not change the recorded rows.

Reconciliation (`reconciliation.json`): every stored stage was re-evaluated with a strict acceptance contract (`validation/dfy/acceptance/stage_acceptance.py`, tested by `test_stage_acceptance.py` with negative controls for missing criteria, partial output followed by timeout, non-zero exit, malformed values, duplicate and conflicting rows, and inconclusive versus failed rows), using the preregistered required coordinates: BUILD criteria 1 and 2, CHANGE criteria 1 to 4, for every scored rule including benign labels, with no exemption.

- Runner status: every recorded run exited 0 and none timed out.
- First scoring (the record): N24 stages are complete and fail (the diagnosed `twin_failed` rows); N25 stages are additionally incomplete.
- Rescoring (evaluator v2): N24 stages are complete and pass. N25 stages are **incomplete**, not pass: the N25 evaluator never emitted criterion 1 for its three benign labels (`callsign`, `cache_label`, `trail_mark`; N24 does emit it). The earlier 8 of 8 therefore overstated N25 under the preregistered contract.
- Correction (evaluator v3, a third scoring of the unchanged N25 deliveries, reported separately): the missing criterion 1 rows were added (valid-path acceptance of each label stored exactly as given) and validated on the full toy matrix with unchanged verdicts for every existing toy. All four N25 stages are complete and pass.

Net: the corrected reading is N24 4 of 4 and N25 4 of 4 stages on the repaired evaluators with complete required rows. This is still not the prospectively frozen qualification: the evaluator was repaired twice after the rows were seen. Inconclusive twin failures are not application violations; the first scoring says only that no stage satisfied the complete evaluation floor.

Evidence custody: an evaluator-unification worker's controls were lost when its workspace was deleted before its output had been delivered and hashed. They are being rebuilt in a durable, hashed location; freeze and cleanup are now conditional on that handoff.

## Addendum 2: capture-and-replay evaluator and run reliability (new evidence result)

The honest request was rebuilt without guard synthesis: a contract-valid operation is driven through the delivered public application, the requests reaching the runtime boundary are captured, the target-bearing command is replayed from an equivalent pre-state, and only the oracle-specified target argument is changed. Attack values and expectations come from the oracle tables, not the candidate. Commands that carry the target in more than one request are attacked in sequence from an honest prefix; a follow-up outcome record is prefix-only; incomplete public output, a captured request that does not match the public result, a stale handle, or an unsupported request kind is inconclusive and never a pass. The kit was rebuilt with its control matrix (references, alternate surfaces, create-time, echo, clock-equality, digest-charset, derived, effect-then-record, multi-command, host-only, missing-input, always-refuse, incomplete-output, capture-mismatch, stale-handle and duplicate-id toys), the N24 and N25 selftests unchanged, and delivered into a retained, hashed directory before any workspace was removed. Three defects found on retained deliveries during its construction (creating commands sent as a distinct request kind, a follow-up record after the guarded command, two commands carrying the same target) were fixed generically and on toys; no delivery was shown to the builders.

Fourth scoring of the unchanged deliveries (strict acceptance, complete required rows): 8 of 8 stages in two runs at two parallel workers. At four parallel workers, one of two runs had one transient single-rule failure that did not reproduce (a serial rerun of that stage four times, and of the other stage that failed once in an earlier partial run, passed every time); runs under load are therefore capped at two workers and any non-pass is rerun serially and both results kept. Runs of this kit that differ from the serial result are an instrument reliability finding, not an application finding.

This is still not the prospectively frozen qualification: the evaluator was repaired three times after rows were seen. It does remove the guard-text synthesis dependency that produced the original inconclusive rows.
