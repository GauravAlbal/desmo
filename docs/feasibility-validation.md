# Known-solution feasibility validation

A backend may discharge a generated success obligation only after it passes this
suite. The in-house engine, and any later solver, is scored against ground truth
that does not come from that backend.

## Layout

```
tests/fixtures/feasibility_truth/
  machines/*.ik.toml
  corpus/*.cases.json
  truth.json
tests/feasibility_truth.rs
scripts/score-known-feasibility.py
```

Each case in `truth.json` names a machine, a finite case corpus, a ground-truth
method, and the obligations under test. An obligation is one of:

| `truth` | Meaning |
|---|---|
| `REACHABLE` | A kernel-replayable `trace` commits the obligation. |
| `UNREACHABLE` | Successful commit is impossible in the stated `scope`. |
| `UNKNOWN` | The bounded method did not decide. A backend `unknown` is never wrong. |

Methods: `by_construction` (hand-reasoned, notes on the case), `brute_force`
(exhaustive walk of the real `Machine` API over the listed corpus and depth),
`published_pattern` (classic safety/reachability shape re-expressed in the
declarative language), `bounded_unknown` (finite sample or depth is incomplete),
`kernel_replay` (a concrete trace replayed after a mechanical explorer
disagreed with hand truth).

Every `UNREACHABLE` row also carries independent mechanical fields from the
kernel-driven explorer (not the in-house feasibility engine):

| Field | Meaning |
|---|---|
| `mechanical` | `kernel_exhaustive` (finite bool/string domain closed), `unbounded_sample` (integer arguments or facts; sample only), or `authored_goal` (authored coordinate; impossibility never discharges). |
| `domain` | Stated string/int/bool partition used by the explorer. |
| `complete` | Whether that exploration closed. |
| `nodes` | Reachable configurations in the sample. |

## Scoring a backend

```sh
python3 scripts/score-known-feasibility.py \
  --truth tests/fixtures/feasibility_truth/truth.json \
  --backend './target/debug/ik feasibility-backend --engine in-house' \
  --ik "$(pwd)/target/debug/ik"

python3 scripts/score-known-feasibility.py \
  --truth tests/fixtures/feasibility_truth/truth.json \
  --backend './target/debug/ik feasibility-backend --engine smt' \
  --ik "$(pwd)/target/debug/ik"
```

`my-backend` is invoked once per case:

```
CMD --machine PATH --corpus PATH --case-id ID
```

It prints JSON: `{ "obligations": [ { "id", "status", "trust", "trace"? } ] }`
or a bare list of those rows.

| `status` | Rule |
|---|---|
| `impossible` on `REACHABLE` | `UNSOUND`, fatal |
| `witness` that does not replay | fatal |
| `witness` that replays on `UNREACHABLE` | `UNSOUND`, fatal |
| `unknown` | never wrong |
| `impossible` on `UNREACHABLE` | true positive |

`trust` is `checked` (the runner verified a replay) or `solver` (backend-asserted).
Solver-asserted counts are reported separately from checked true positives.

Precision is the fraction of `UNREACHABLE` obligations the backend reports
`impossible`. It is information, not a pass criterion. Soundness is.

Self-test:

```sh
python3 scripts/score-known-feasibility.py \
  --truth tests/fixtures/feasibility_truth/truth.json \
  --self-test
```

A fake that marks a `REACHABLE` obligation `impossible` must be caught. A fake
that always returns `unknown` must be sound with precision 0.

## How truth was verified

`tests/feasibility_truth.rs` does the following, independently of the engine:

1. Every `REACHABLE` trace is replayed through `Machine::decide` / `apply`.
2. Brute force walks the real kernel twice, with opposite candidate order, over
   the case corpus and declared node/depth bounds. The reached obligation sets
   must match.
3. Brute force must not produce a witness for `UNREACHABLE` or `UNKNOWN`.
   A `REACHABLE` row whose brute walk misses still passes if its construction
   trace replays (the listed sample may be one witness, not an enumeration).
4. Construction and brute force disagreements fail the test until resolved.
5. Every `UNREACHABLE` obligation is re-checked by the kernel explorer. A
   witness means the hand truth was wrong (or the explorer was); the suite
   fails until one of them is fixed. `mul_overflow_not_false` was corrected
   this way: `$n * 2 < $n` holds for `n = -1` under checked i64 multiply
   (`-2 < -1`), so those rows are `REACHABLE` with a kernel-replayed trace.

`UNKNOWN` rows are those whose finite sample is acknowledged incomplete
(unbounded string order, a far integer cut, a long counter climb). Completing
the walk over *that* sample must still not find a witness.

## In-house engine

The suite runs `check_machine` (and `ik check --json` on one fixture). The engine
may only emit `impossible` (an impossibility record) or stay silent (`unknown`).
It does not emit witnesses. Pass criteria:

- zero `impossible` records on `REACHABLE` obligations
- zero discharges of a witness-replayable obligation
- precision printed as information

Authored goals stay mandatory even when a generated copy is proved impossible.

## Categories

Cases cover dead commands, ordered-outcome shadowing, string/int/bool
partitions, a non-negative counter, i64 overflow as an error rather than false,
mutex-style flag products, unreachability behind an invariant, fact-flow
implication, parent fallback vs child override, unrelated-region addition, and
adversarial shapes: string ordering, overflow, negative-dividend toward-zero
division, unset facts, short-circuit with errors, and an earlier guard error
that must not fall through.

## Kani capability proof

The historical record documents a Kani 0.68 / CBMC 6.11 bounded run of
`validate_capability` for arbitrary independent `u64` generations and independent
ASCII nonces of length 0–4, including NUL. Its original positive, four cover and
false-control outputs were unavailable in the retained-evidence audit on
2026-11-17; this is not a fresh qualification or evidence that the historical run
failed. The source harness remains available, but source inspection and a passing
product gate cannot substitute for those solver results.

For an explicitly authorized reproduction, select the positive harness:

```sh
RUSTFLAGS=-Zmir-opt-level=2 cargo kani --lib --harness capability_contract --no-assertion-reach-checks -Z unstable-options --unwind 24
```

`--no-assertion-reach-checks` removes only Kani's auxiliary assertion-reachability
reporting; it does not remove user assertions, safety checks, or unwind checks.
The explicitly selected positive harness matters because the other harness is
intentionally false. Its negative control is expected to exit 1 at the deliberate
assertion; the expectation alone is not an observed counterexample:

```sh
RUSTFLAGS=-Zmir-opt-level=2 cargo kani --lib --harness intentionally_wrong_matching_is_rejected --no-assertion-reach-checks -Z unstable-options --unwind 24 --cbmc-args --stop-on-fail --trace
```

Kani is not part of the acceptance scripts or release gate.

### Scoped launch wording

The following is the evidence-conditioned scope as of 2026-11-17, not a claim
that a new verifier run passed:

> The **bounded proofs** reported to date are helper-scoped: a historical Kani 0.68 / CBMC 6.11 run is documented for `validate_capability`, with independent arbitrary `u64` generations, independent ASCII nonces of length 0–4 (including NUL), unwind 24, and safety/unwinding checks retained. Its assertions cover matching acceptance and generation-before-nonce diagnostic codes/rules, not exact diagnostic message bytes. The original positive, four cover and deliberately false helper-control outputs are unavailable in the retained audit, so those outcomes have not been independently reconfirmed here. Full command/effect entrypoint authority, runtime-owned cleanup and exact historical-receipt preservation remain unproved: retained positive entrypoint attempts timed out, and Verus stopped at owned JSON-type admission. This evidence is not bound to the published `0.5.0` binaries and establishes no whole-core, persistence, cryptographic or host guarantee.

An observed false assertion in an effect-replay control witnesses that case; it
does not establish the positive entrypoint theorem or replace the missing helper
control/cover results. Stronger wording requires actual qualified results and
their source/artifact boundary, not an installation, intended assertion or
successful unrelated gate.
