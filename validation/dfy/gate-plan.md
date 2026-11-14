# Right-sized gate plan for the scoped public alpha

Each gate is stated as the claim it must support, the cheapest evidence that supports that claim, what is deliberately dropped, and who acts. Nothing is waived: a gate either keeps a smaller named proof or is removed because the claim it protected is not made.

Claim of the alpha: *an agent, and a person reading the tool's output, can build, change and diagnose a durable lifecycle application with Ikaros, and the runtime enforces what the machine declares, on the stated topology.*

| Gate | Protects the claim that | Minimum proof kept | Dropped or shrunk | Actor |
|---|---|---|---|---|
| Acceptance gate on every change | shipped bytes match what was tested | keep; batch record-only edits into one change each | many tiny record commits | agent |
| Fresh-agent qualification (D8) | agents succeed on unseen families | one frozen confirmation under the corrected instrument: re-score retained deliveries (no new agents), then one fresh family pair, one replica each | no more family sets, no replica growth, no new guide amendments | agent |
| Evaluator correctness | the pass/fail instrument is sound | strict stage acceptance (done), capture-and-replay honest request with its control matrix, artifacts hashed before workspace deletion | no universal constraint solver; guard-synthesis kept only as bounded fallback | agent |
| Human review (IK-06, IK-13) | a person can read the readouts and the agent instructions and describe how the machine was constructed | one unfamiliar reader, at most 20 minutes, reading the product's actual behavior handoffs for BUILD, CHANGE and DIAGNOSE (`human/HUMAN-READOUT.md`, answers withheld, key in `human/ANSWER-KEY.md`); the reader does not decode kernel JSON, author predicates or build corpora. The raw-JSON journey sheet stays as an agent/interface fixture. This is a reduced replacement for the earlier human review and cold-use obligations, not a record that either occurred | no full cold run by a human, no multi-reviewer panel, no usability study | one human |
| Linux x64 (IK-11) | the packaged bytes work on the second supported platform | packaged-artifact qualification and cross-platform spec_hash parity in an emulated x86_64 container, stated as emulated | no native-hardware run, no full source suite on x64, no contention measurement on Linux | agent |
| Consumer acceptance (IK-10) | the first consumer can adopt the release | existing scratch differential run stands; owner states yes or no on the pin move | no live host run before the owner decides | owner |
| Licensing and disclosure (IK-12) | the public source may be released | owner chooses a license and a security contact in one reply; agent applies it | no history rewrite unless the owner asks | owner |
| Worker/acceptance separation | no actor certifies its own work | document the cooperative same-account boundary as a disclosed limit; make no no-self-certification claim | no new accounts, services or mandatory signing | agent (disclosure), owner only if the claim is wanted |
| Publication and pin controls (IK-16) | release actions are authorized | owner performs or approves the visibility, registry and pin actions | none | owner |
| Launch film (IK-15) | promoted-launch comprehension | not required for a quiet alpha; removed from the alpha path | whole gate | none |
| Comparative claims (IK-14) | superiority wording | no comparative language is used; descriptive results stay as recorded | whole gate | none |
| Downstream note | the downstream team knows about the change | one-page note delivered only on explicit permission and a named path | none | owner permission |

## Order on the critical path

1. Evaluator kit with capture-and-replay and its control matrix (agent, in progress).
2. Linux x64 emulated qualification and journey state model with human sheet (agent, in progress).
3. Re-score retained batch 017 deliveries under the finished instrument; then freeze and run one fresh family pair, one replica each.
4. Hand the owner the three small decisions: pin move yes/no, license and security contact, note permission; hand one person the human sheet.
5. Update the readiness ledger from these results and stop.

## What this does not claim

It does not claim `DFYJourneyQualified`, human review, consumer acceptance, native Linux x64, publication, or that separation of worker and acceptance authority exists. Those remain stated limits, not waived gates.

## Dispositions (2026-11-12)

- **Worker and acceptance-authority audit: completed at its stated scope.** Cooperative same-account operation is confirmed. Executed on disposable fixtures against the gate tool version 0.3.54: operator-only calls through the restricted interface were denied; an edited ledger row, a forged receipt subject and a receipt row for a different revision were denied; blocking git hooks denied. Executed and allowed: operator mode through the plain command line, candidate test code reaching authority files, and bypassing hooks with the documented switches. Worker isolation and mechanical prevention of self-certification are not provided. Earlier receipt-binding results remain historical evidence at the tool version they were run against; they were not re-run on 0.3.54.
- **Gate reliability repairs: closed at observed scope.** Two timing-sensitive tests were made deterministic without weakening their timeout contracts and pass repeatedly under CPU contention where the originals failed; dependency preparation for restricted-network gates is scripted and integrity-checked. This is not a claim of universal stability.
- **Human comprehension exercise: not yet run.** It needs an unfamiliar reader and the regenerated product handoffs; it replaces, and does not record the occurrence of, the earlier human review and cold-use obligations.
- **Live consumer cutover, publication and the live pin: not performed.** Each is a separate owner authorization.
