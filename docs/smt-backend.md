# SMT backend (prototype)

The kernel owns machine semantics. This backend translates the existing
parsed guard AST and the schema-1 lowered definition into SMT-LIB2. It does
not keep a second hand-written model of any machine.

## Proof scopes

| Query | Question |
|---|---|
| Local selectability | For one `(command, source state, outcome index)`, do any well-typed facts and arguments make that arm selectable? |
| Bounded model check | Is there a concrete trace of length `N` from the initial state to a target? Skipped when the lowered transition list has more than 40 rows (product machines); those unrollings are multi-megabyte QF_LIA and are not a useful first query. |
| Horn reachability | From the initial state, can a target state, taken outcome, or terminal ever occur? |

A target is a lowered state name, a `terminal/.../reachable` coordinate, or an
`outcome/{command}/{index}/executable` coordinate.

## What is exact vs Unknown

Exact, in this fragment:

- Integer facts and arguments as SMT `Int` with explicit `i64` range.
- Checked add/sub/mul: overflow is an evaluation error, not wrapping, and not
  `false`.
- Division by a positive integer constant uses toward-zero `i64` division
  (`(-7)/2 = -3`), not SMT Euclidean `div`.
- Unset facts: reading one is an error. `exists` / `not_exists` test presence.
- Short-circuit `&&` / `||` with errors, matching the shared evaluator.
- Ordered outcomes: arm `i` is selectable iff the source is legal, every
  earlier guard evaluates successfully to false, and guard `i` evaluates
  successfully to true (or is missing). An earlier error is a refusal.
- `then` assignments: literal, `$arg`, counter `+N` with checked add.
- String equality as integer identity. Collected literals occupy distinct
  ids `0..k-1`. String facts and arguments range over non-negative `i64`
  (remainder ids `k, k+1, …` stay distinct). A single OTHER bucket is exact
  **only** for comparisons against literals. Two non-literal strings must
  not collapse: that would make `$a == $b` always true and `$a != $b` a
  false impossibility.

Unknown, never approximated:

- String ordering (`<`, `>`, `<=`, `>=`) or any use of SMT string theory.
- `text_in` (bounded string membership). The encoder reports Unsupported
  rather than approximating with SMT strings. Kernel evaluation of the same
  AST still uses the shared `holds()` implementation.
- Queries outside the fragment, solver timeout, resource limit, or incomplete
  reconstruction of a Horn counterexample.

Rust strings include code points above U+2FFFF. SMT-LIB string theory does
not, so substituting SMT strings would exclude legal inputs. Equality among
those strings is still exact under the integer encoding.

## Admission gate

Zero-mismatch differential testing against the kernel evaluator is an
**admission gate** for this encoder (sampled guards, pinned values, a finite
set of machines). It is not a proof of universal encoder equivalence.

## Certificate checking

Three results only:

- **Witness** — a concrete trace. It is reported only after replay through the
  compiled kernel (`Machine::decide` / `apply`). A solver model that does not
  replay is not a witness.
- **ProvenImpossible** — a checked argument. Local selectability `unsat` on
  every source, confirmed by a second solver, is a check of "no assignment
  selects this arm". For unbounded unreachability, Horn `sat` means an
  inductive invariant exists; that invariant is **checked** only when the
  three conditions (initial states satisfy `I`, every transition preserves
  `I`, `I` excludes the target) are discharged by a second query, and by
  cvc5 when present. An unsat core alone is not a certificate.
- **Unknown** — unsupported fragment, timeout, disagreement, or
  solver-asserted-only Horn `sat` whose invariant was not extracted.

## Trust boundary

| Label | Meaning |
|---|---|
| `SolverAsserted` | The solver said so; not a checked certificate. A Z3/cvc5 disagreement is this label. |
| `CheckedInvariant` | An inductive invariant `I` (including a large n-ary disjunction of reachable configurations — there is no 64-disjunct cutoff) passed three independent queries (`init => I`, `I /\\ step => I'`, `I` excludes the target) with **both** Z3 and cvc5 `unsat`. Symbolic integer arguments are quantified. |
| `CheckedExhaustive` | A complete kernel-driven finite-model exploration over a **finite** domain (bools and equality-partitioned strings, plus a hash of the reachable set), or a concrete trace replayed through the compiled kernel. Integer arguments or facts are sampled, not enumerated; that sample is not `CheckedExhaustive`. |

A proof of impossibility never satisfies an authored or explicit coverage
requirement. That policy lives elsewhere.

Host `ik.analysis.result.v1` currently accepts `observed_trace` and
`bounded_paths` on the result document. The three-result document is
`smt-result.json`. `checked_induction_or_certificate` is declared on the
adapter manifest.

## Pinning

The adapter never downloads a solver. Pin procedure:

1. Install Z3 and cvc5 by some offline means (for example a local virtualenv
   used only as a subprocess backend, the same way the TLC adapter pins a
   jar). The pip `cvc5` wheel has no CLI; pin `adapters/smt/cvc5_stdin.py`
   (or a copy whose shebang is the same interpreter) as the `cvc5` path.
2. Put absolute paths in `adapters/smt/toolchain.json`.
3. Set `z3_sha256` / `cvc5_sha256` to the lowercase SHA-256 of those
   executables. The template `0…0` digest is a placeholder, not a pin.
4. Record `z3 --version` and `cvc5 --version` in the same file.
5. Set `manifest.template.json` `invocation.argv[0]` to the absolute path of
   `adapter.py` and `executable_sha256` to that file's digest.

A digest mismatch, a placeholder digest, a placeholder path, or a binary that
does not answer its version probe loads no solver, and `ik check` prints one
`feasibility adapter configured but unusable: ...` line per problem on stderr
instead of behaving as if no adapter were set. Only `IK_SMT_ADAPTER=auto`
discovers unpinned solvers; use it for exploration, not for a pinned record.

`scripts/pin-smt-toolchain.py OUT_DIR --z3 <path> [--cvc5 <path>]` writes a
toolchain file with paths, SHA-256 digests and versions read from the real
executables into a private directory (the shipped template is never rewritten);
point `IK_SMT_ADAPTER` at that directory.

Missing binaries are `tool_unavailable`.

## Integer decision

Checked `i64` is modeled as SMT `Int` plus range assertions
`[-2^63, 2^63-1]` and an error predicate. Overflow of `+`, `-`, or `*` sets
the error bit. The query does not fall through to a later ordered outcome.

64-bit bitvectors with overflow predicates are an equivalent alternative.
`Int` was chosen so Horn clauses stay in integer arithmetic and so
toward-zero division is a sign-aware `ite` over SMT `div` of the absolute
value.
