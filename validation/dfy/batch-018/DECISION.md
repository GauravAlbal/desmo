# Batch 018: decision record

Pre-registered in `PREREG.md`, frozen in `manifest.json` and committed before dispatch. Two fresh families (N26 community-garden tool-lending desk, N27 bicycle-shop repair-ticket desk), one replica each, BUILD then CHANGE, TypeScript, macOS arm64, packaged candidate with the bounded string predicate `text_in`, frozen capture-and-replay evaluator, strict acceptance contract, two parallel workers. Per-rule rows are in `results.json`.

## First scoring (the record, and the only scoring)

| Stage | N26 r1 | N27 r1 |
|---|---|---|
| BUILD | pass | pass |
| CHANGE | pass | pass |

4 of 4 stages pass. Every required coordinate was present exactly once with a boolean result, every runner exited 0 without timeout, and no row was inconclusive (no `twin_failed`, no `unroutable`). The evaluator and candidate were not changed after freeze; no rescoring and no diagnostic rerun was needed.

## Targeted quantities (CHANGE stage, rule instances, criterion 2)

| Quantity | Enforced or accepted |
|---|---|
| Identifier grammar rules (1 to 64 of `A-Za-z0-9._-`) | 10 of 10 |
| All grammar rules (identifiers, code, note) | 16 of 16 |
| Rules with a character class | 13 of 13 |
| Command-specific input rules | 20 of 20 |
| Free-form labels accepted | 6 of 6 |

Each attack changed one argument of an honestly captured request and was refused at the runtime boundary with a rule and unchanged state; each valid-also value committed.

## Diagnostic (not scored)

Both deliveries used `text_in` in their machine definitions (N26 eight uses in the revised machine, N27 seven) and both left opaque labels unguarded.

## What this supports and does not

Supports: on two fresh families, one replica each, a frozen candidate and a frozen instrument produced a first-scoring pass on every stage, with the complete stated string grammar enforced at the runtime boundary and free-form labels accepted.

Does not support: a rate, the original three-family two-replica requirement (this is the declared reduced scope), a claim about other families or agent models, `DFYJourneyQualified`, human review, consumer acceptance, native Linux x64, publication, or any trust-boundary claim. n=1 per family, one model condition (not exposed), macOS arm64, no humans.

## Relation to earlier results

Batches 015 to 017 stay as recorded. Batch 017's 8 of 8 came from evaluator repairs made after its rows were seen; this batch is the first run in which the evaluator and the candidate were fixed before any agent ran and in which the first scoring is the result. The packaged candidate for this batch predates the later plain-language report and handoff text changes; those are qualified separately at the packaged interfaces.
