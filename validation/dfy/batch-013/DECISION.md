# Batch 013: requalification of the coverage-contract bytes (decision record)

Not a qualification batch and not an agent wave: no new transfer claim is made. It records what was verified on the packed bytes listed in `manifest.json` after the two-tier coverage contract and the optional checked feasibility adapter landed. Batch 011 and batch 012 stay attached to their own bytes and do not transfer to these.

## What was run

- Packed-binary regressions: seam fixtures, TypeScript client and skill tests (66), Python client tests (149 passed, 1 skipped), artifact qualification; the Rust suite (573) on the source revision.
- Two consumer models, A and H (lifecycle machines supplied by a downstream consumer; not committed here), with the adapter off and on (`consumer-checks.json`).
- A symbolic-versus-sampled mutant of H.

## Results

| Check | Result |
|---|---|
| Model A, adapter off and on | ACCEPT, identical plan digest: the adapter changes nothing where nothing is needed |
| Model H, adapter off | REFUSED: 46 in-house impossibility records; five generated `classify` outcome copies (17 to 21) stay unwitnessed |
| Model H, adapter on | ACCEPT: 51 records; the five carry trust `CheckedInvariant` (certificate re-verified by two independent solvers, symbolic integer arguments); all 37 authored arms were already satisfied by committed witnesses before and after |
| Mutant of H whose shadow breaks only at an unsampled integer value (`attempts == 1000`, with the bounding invariant relaxed) | adapter refuses to discharge outcomes 17 and 18 and discharges only 19 to 21. Ground truth by the kernel: with a corpus case that reaches `attempts == 1000`, real committed witnesses exist for 17 and 18; 19 to 21 stay unreachable because the attempt-cap arm fires first. No false discharge on a value the exhaustive sample did not contain |
| Adapter given as an explicit directory whose `toolchain.json` still holds placeholder paths | no solvers found, nothing discharged (safe); it is silent, which is a usability gap, not a soundness one |

## Why H becomes ACCEPT, and what that does and does not mean

The five obligations were generated product-context copies, not authored behavior: at the only reachable source, earlier arms always fire first. A complete explicit-state exploration (292 configurations) and a symbolic inductive-invariant check agree, so they are discharged with recorded basis, scope and certificate digest. Authored goals are untouched: an always-true first arm or a dead authored command still REFUSES, and a solver-asserted-only result never discharges. The adapter is optional, pinned by operator configuration, and absent it `ik check` output is byte-identical to the previous behavior.

Known-solution validation (a separate suite of 67 machines with ground truth from construction, kernel replay and mechanical exhaustive exploration) found one error in the hand-written truth (a checked multiplication case), which was corrected by kernel replay. The in-house engine and the solver backend both scored zero unsound results; solver precision 69 of 76 provable cases, in-house 36 of 76. Of the unreachable obligations, 34 are verified by exhaustive kernel exploration over a finite domain and 39 only by sampled integers plus by-construction reasoning; those 39 are stated as such, not as exhaustive.

## Limits

Solvers are discovered, not digest-pinned, in these checks; pinning is a documented operator step and the shipped manifest still holds placeholders by design. Two consumer models and a handful of mutants are not a general proof of the encoder; the fidelity and known-solution gates are admission tests, not equivalence proofs. Not run: Linux x64, human participants, a new agent wave. `DFYJourneyQualified` is still not asserted.
