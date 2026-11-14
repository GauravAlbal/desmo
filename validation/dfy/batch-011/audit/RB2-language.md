# RB2: declarative language — integer arithmetic and string literals

**Claim checked:** “the declarative language has no integer arithmetic or string literals” (batch-011 `DECISION.md` §Independent inspection, and cold-agent paraphrases below).

**Verdict:** **not reproduced.** Candidate `3741e2b` (frozen binary `dfy/batch-011/ik-packed`, `invariant-kernel 0.5.0 protocol 1.0`) has a bounded guard grammar with string literals, `exists` / `not_exists`, integer `+INT` / `-INT` on one operand, and `then` literals / `$arg` / counter `+N`. What is actually missing for some lineage-2 formulas is **variable+variable arithmetic, multiplication, division, and `then` expressions** — not “no arithmetic or strings.”

**Method:** read `cand11/src/declarative/{expr.rs,def.rs,compile.rs}` (grammar matches `git show 3741e2b:src/declarative/expr.rs`); read shipped `docs/` + `skills/ikaros/` + examples; author scratch machines under `/tmp/LanguageClaims-scratch`; run them with `ik check` / `ik identity` / `ik scenario run` against the frozen binary only. Expected lineage-2 behavior taken only from `BRIEF.md` / `CHANGE.md`. Inspector hypotheses were not treated as defects unless the binary and those contracts agreed.

Binary: `<work>/batch-011/ik-packed`.

Source grammar (`expr.rs`):

```text
atom        := '(' expr ')' | cmp | 'exists' '(' IDENT ')' | 'not_exists' '(' IDENT ')'
cmp         := operand ( '==' | '!=' | '<' | '<=' | '>' | '>=' ) operand
operand     := value ( ('+' | '-') INT )?
value       := '$' IDENT | IDENT | STRING | INT | 'true' | 'false' | 'state'
```

`then` (static check + compile): `literal | $arg | +N`, where `parse_literal` accepts `true` / `false` / `i64` text / a value that **begins and ends with `"`**.

---

## Operation table

| Operation | Classification | Evidence (command and output) |
|---|---|---|
| String literal operands in guards | **supported** | `when = "status == \"paid\""` and `when = "status == 'paid'"` both `ik identity` exit 0; `ik scenario run` committed. Mismatch `status == 'paid'` with fact `"open"` refused `IK_INVARIANT_FAILED` message `no outcome of command \`go\` applies in state \`idle\``. Shipped examples use `state == 'verified'` / `'authorized'`. |
| String literal assignment in `then` (e.g. `then status = "paid"`) | **supported-but-undiscoverable** | Bare TOML `then = { status = "paid" }` → `ik identity: \`then\` value \`paid\` for \`status\` is not a literal or \`$arg\`` (exit 2). Nested quotes work: `then = { status = '"paid"' }` and `then = { status = "\"paid\"" }` compile; scenario facts after `go` are `status: "paid"`. Empty clear `then = { deposit_id = '""' }` sets `deposit_id: ""`. Inner single quotes fail: `then = { status = "'paid'" }` → `\`then\` value \`'paid'\` for \`status\` is not a literal or \`$arg\``. **Where a cold agent looks:** `docs/machine-language.md` never states then-literal syntax; `docs/region-lowering.md` says only “`then` uses the existing assignment-expression syntax”; greenfield fixture and `examples/approval-revision` assign only `$arg` / `+N`. They miss `parse_literal` in `def.rs` / `compile.rs` (requires the `"` characters **inside** the TOML string). |
| `not_exists` | **supported-but-undiscoverable** | `when = "not_exists(token)"` on an unset replace fact: identity 0, scenario committed (facts `{}`). Same guard with `initial = "x"`: refused `IK_INVARIANT_FAILED`. **Where a cold agent looks:** grammar lives in `expr.rs` and `RESERVED` in `def.rs`. No hit in `docs/*.md`, `skills/ikaros/*`, or shipped `*.ik.toml`. They will try `!exists` / `not exists` (below) and conclude absence tests do not exist. `exists(token)` is discoverable (`verification_attempt`, `approval_revision`). |
| `!exists` / `!exists()` | **unsupported** | `when = "!exists(token)"` and `when = "! exists(token)"` → `ik identity: command \`go\` guard parse error: expected an operand`. `when = "not exists(token)"` → `expected a comparison operator`. Binary never prints `!exists() is not valid`. |
| Integer constant add/subtract on an integer operand in a guard | **supported** | `when = "n + 1 == 6"` with `n=5`: committed. `n - 1 == 4`: committed. `$x + 2 == 7` with `x=5`: committed. `n + 1 == 7`: refused `no outcome of command \`go\` applies in state \`idle\``. Shipped `machines/verification_attempt.ik.toml` `reject` uses `attempt_count + 1 < max_attempts`; `ik scenario run` of start/submit/reject committed `attempt_count: 1`, state `repairable`. Overflow is checked: `n = 9223372036854775807`, `n + 1 == 0` refused; trace `guard \`commands.go.when\` could not be evaluated: integer arithmetic overflow`. |
| fact+fact or fact−fact | **unsupported** | `when = "a + b == 3"` → `command \`go\` guard parse error: expected an integer after \`+\``. `when = "a - b == 3"` → `expected an integer after \`-\``. |
| fact+argument (and arg+fact) | **unsupported** | `when = "a + $x == 3"` and `when = "$x + a == 3"` → `expected an integer after \`+\``. Same for `remaining_qty - $units`: `command \`fill\` guard parse error: expected an integer after \`-\``. |
| Arithmetic in assignments | **unsupported** (with a **defective** `+N` footgun on replace facts) | `then = { n = "n + 1" }` → `\`then\` value \`n + 1\` for \`n\` is not a literal or \`$arg\``. Unquoted `then = { n = 1 }` is a TOML type error, not a language eval: `invalid type: integer \`1\`, expected a string`. Quoted `then = { n = "1" }` assigns integer `1` (scenario facts `n: 1`). **Defect:** `then = { n = "+1" }` on a **replace** int with initial 5 compiles (static `parse_literal("+1")` sees `i64` 1) but runtime `compile_then` treats any `+N` as `CounterIncr`; scenario facts `n: 6`. Check and compile disagree. |
| Counter increment | **supported** | `policy = "counter"`, `then = { count = "+1" }`: committed, facts `count: 1`. `then = { count = "1" }` → `counter fact \`count\` accepts only \`+N\` increments, got \`1\``. Shipped `verification_attempt` / `approval_revision` / `docs/patterns.md` show `+1`. **Defect (overflow):** counter initial `9223372036854775807` then `+1` **commits** with `count: -9223372036854775808` (wrapping). Guards on the same `i64::MAX + 1` refuse. Assignment uses `current + n` (`compile.rs`), not `checked_add`. |
| Comparison of two integer facts | **supported** | `when = "a == b"` (both 3) committed; `when = "a < b"` committed. Shipped `approved_revision == current_revision`. |
| Equality of a fact to an argument | **supported** | `when = "a == $x"` with `x=9` committed; `when = "token == $x"` with strings committed. Shipped `$revision == current_revision`, `$artifact_digest == artifact_digest`. |
| Integer division or rounding | **unsupported** | `when = "amount / 50 == 200"` → `command \`go\` guard parse error: expected a comparison operator`. `when = "units * 175 == 350"` → same. `when = "$deposit_cents == amount / 50"` → `unexpected token Op("") at position 24`. Tokenizer turns `*`/`/` into empty `Op("")` and fails; it does **not** hang (identity returned in <1s with timeout 3s). No toward-zero / floor operator exists. |
| Deadline-style `now_ms <= accepted_ms + window_ms` where both addends are facts/arguments | **unsupported** as written; **supported** if the addend is an integer constant | `$now_ms <= accepted_ms + window_ms` → `expected an integer after \`+\``. `$now_ms <= accepted_ms + 5000` with `accepted_ms=1000`: `now_ms=6000` committed, `now_ms=6001` refused `no outcome of command \`go\` applies`. Host-supplied pair with a **constant** span is enforceable: `$loan_until_ms == $checkout_ms + 1209600000` committed for `1209600000` and refused for `604800000`. `$deposit_deadline_ms == $accepted_ms + deposit_window_ms` → `expected an integer after \`+\``. Original N15 law `$deposit_deadline_ms == accept_by_ms` committed for 2000 and refused a host lie of 6000. |
| Percentage `amount * pct / 100` | **unsupported** | `when = "amount * pct / 100 == 200"` → `expected a comparison operator`. Same empty-`Op("")` path as division. |

---

## Cold-agent messages vs frozen binary

None of the five paraphrases is emitted verbatim. Accuracy of the **claim**:

| Agent text | Binary text on the frozen `ik` | Accurate? |
|---|---|---|
| `then-assignments accept only $arg or numeric-string literals` | ``then` value `paid` for `status` is not a literal or `$arg`` | **Partly.** That is the error for the TOML spelling `status = "paid"` (deserializer yields `paid` without quotes). False as a language theorem: `'"paid"'` / `"\"paid\""` assign strings; `"1"` assigns int; `true`/`false` assign bools; counters need `+N`. |
| `integer 1 in then is a parse error` | `TOML parse error … then = { n = 1 } … invalid type: integer `1`, expected a string` | **Accurate for unquoted TOML 1.** Not a guard/then grammar refusal. `then = { n = "1" }` works. |
| `!exists() is not valid` | `command \`go\` guard parse error: expected an operand` | **Accurate that `!exists` is invalid.** Incomplete: `not_exists(IDENT)` is valid and implemented. |
| `guards cannot add ints` | (no such message.) `n + 1` works. `a + b` prints `expected an integer after \`+\`` | **False.** Constant add/sub is in the grammar, the greenfield fixture, and the binary. |
| `then cannot assign fact-to-fact` | ``then` value `a` for `b` is not a literal or `$arg``; `then = { b = "$a" }` → ``then` for `b` references unknown arg `$a`` | **Accurate as a limitation.** Copying a fact requires an `$arg` the host already holds. |

`DECISION.md`’s product sentence “cannot add integers or assign string literals” is the same false generalization. Inspectors who wrote that host arithmetic was forced by the language mixed **undiscoverable-but-present** forms with **actually missing** variable-variable / mul / div / `then` expressions.

---

## Discoverability (why cold agents miss supported forms)

Followed paths that **do not** state the grammar:

- `docs/machine-language.md` — schema-2 regions; mentions `when`/`then` without `STRING`, `+INT`, `exists`, `not_exists`, or then-literal quoting.
- `skills/ikaros/{SKILL,build,change}.md` — journey/check/scenario, not the expression language.
- `docs/greenfield.md` — “write a `.ik.toml`; start from `machines/verification_attempt.ik.toml`.” That fixture **does** show `attempt_count + 1`, `exists(...)`, `then = { … = "+1" }`, and `state == 'verified'`. An agent who opens it would not conclude “no integer arithmetic.” An agent who only reads `machine-language.md` would.
- `docs/region-lowering.md:145–146` — “`then` uses the existing assignment-expression syntax” with no definition of that syntax.

Present only in source / one example:

| Form | Cold-agent lookup |
|---|---|
| `not_exists(IDENT)` | `expr.rs` grammar + `def.rs` `RESERVED`. Zero docs/examples. |
| `then` string literal | `parse_literal`: must keep `"` inside the TOML string. No example assigns `"paid"`. |
| Guard `+INT`/`-INT` | `expr.rs`; **also** `verification_attempt.ik.toml` `attempt_count + 1 < max_attempts`. |
| Counter `+N` | `docs/patterns.md` + approval-revision example. Discoverable if they open examples. |

---

## Lineage-2: only the minimal missing op, or an already-supported path

Do **not** add a general expression evaluator. Semantics below apply only if a listed op is added. Integers are signed `i64` (tokenizer `parse::<i64>()`, literals non-negative, negatives via `x - N`). Guard arithmetic already uses `checked_add` / `checked_sub` and refuses `integer arithmetic overflow`. Unset replace facts are absent from the fact map; comparing them is `fact \`name\` is not set`, wrapped as `guard \`…\` could not be evaluated: …` (`IK_INVARIANT_FAILED`); evaluation errors **do not fall through** (`evaluate.rs`). `NOW_MS` is a host argument; the kernel has no clock. Determinism: integer-only, no floats, no implicit rounding.

**Identity / replay:** adding a new operand form is additive if the compiler encoding of existing `Arith { base, add: bool, amount: i64 }` is unchanged — old `spec_hash` values stay. Changing overflow of **existing** counter `+N` from wrap to refuse would be a replay break for any journal that already stored `i64::MIN`; lineage-2 amounts (hold_ms ≤ 2.592e9, spans 6.048e8 / 1.2096e9, cents) cannot hit `i64::MAX`. If arithmetic is extended, overflow must **refuse** like guards, not wrap like today’s counter `then`.

### N13 (ILL desk) — 14d/7d span, overdue checkin, recall

Already-supported authoritative path (no new op):

1. Checkout: host passes `$checkout_ms` and `$loan_until_ms`. Guard `$loan_until_ms == $checkout_ms + 1209600000` (v1) or `+ 604800000` (v2). Reproduced: good sum commits; 7-day sum against a 14-day constant refuses. The host still **computes** the number; the machine **rejects a lie**.
2. Overdue checkin: `$now_ms <= loan_until_ms || recall_placed == 1` (recall fact initial 0). Reproduced: overdue with `recall_placed=0` refuses; `recall_placed=1` commits. A2-N13’s `$overdue == 0 || recall_placed == 1` is the same idea with a derived arg.

Nothing here requires var+var, mul/div, or string `then`. `not_exists` is not required if `recall_placed` is a zero-initialized int. Unguarded v2 `checkin` in the delivered trees is therefore **not** a language limit.

### N14 (pharmacy) — remaining, hold cap, rush, amount

| Formula | Path |
|---|---|
| `units <= remaining_qty` | **Already supported** (`$units <= remaining_qty`). Reproduced. |
| `remaining_after == remaining_qty - units` | **Not expressible.** Weaker supported path stores host `$remain_after` after the `<=` guard; reproduced lie: remaining 10, units 4, `remain_after` 99 **commits** `remaining_qty: 99`. **Minimal missing op:** subtract one integer operand (fact or arg) from another **in a comparison** (so the host-supplied remainder can be checked). `i64`, `checked_sub`, overflow refuse, missing fact eval-error (no fallthrough). Do not add `then remaining_qty = remaining_qty - $units` unless that same binary op exists in guards; one comparison op is enough: `when = "$remain_after == remaining_qty - $units"` then `then remaining_qty = "$remain_after"`. |
| `pickup_deadline_ms = NOW_MS + hold_ms` and v2 `NOW_MS + min(hold_ms, 86400000)` | ` $pickup_deadline_ms == $now_ms + 86400000` is **supported** (arg+const). `$now_ms + hold_ms` is **not**. Two outcomes can encode the cap **only** if the uncapped branch can add two operands. Reproduced const-only cap when `hold_ms > 86400000`. **Minimal missing op:** add two integer operands (fact or arg) in a comparison, `checked_add`, same missing/overflow rules. `min` is **not** required: two ordered outcomes, `hold_ms <= 86400000 && $d == $now + hold_ms` else `$d == $now + 86400000`. |
| `amount_cents = held_units * 175` | **Not expressible.** Already-supported: store host `$amount_cents`. **Minimal missing op:** multiply an integer operand by a **positive integer constant** in a comparison (`$amount_cents == held_units * 175`). `checked_mul`, overflow refuse. No general `*` of two facts. |
| `rush`: `min(current, NOW_MS + 3600000)` | **Already supported** if the host supplies `$deadline` and the machine checks it. Reproduced: current 100000, `now=0`, honest `$deadline=100000` commits both facts to 100000; current 10000000, honest `$deadline=3600000` commits 3600000; lying `$deadline=1` refuses. Outcomes: `pickup_deadline_ms <= $now_ms + 3600000 && $deadline == pickup_deadline_ms` else `… > … && $deadline == $now_ms + 3600000`. Copying the chosen deadline into `rush_ms` uses the same `$deadline` arg (no fact-to-fact `then`). |

### N15 (escrow) — deposit cents, revised deadline, promote clears

| Formula | Path |
|---|---|
| `deposit_cents = floor(amount_cents / 50)` (BRIEF R3; amounts ≥ 0) | **Not expressible.** Already-supported: host `$deposit_cents` with `$deposit_cents >= 0` (weaker). **Minimal missing op:** divide an integer operand by a **positive integer constant** in a comparison (`$deposit_cents == amount / 50`). Semantics: `i64` toward-zero division (for non-negative BRIEF amounts this is floor); remainder discarded; constant divisor ≠ 0 so no div-by-zero diagnostic is required for this contract; overflow does not arise. Do not add rounding modes or `%`. |
| Original `deposit_deadline_ms = accept_by_ms` | **Already supported:** `$deposit_deadline_ms == accept_by_ms`. Reproduced: 2000 commits; host lie 6000 refuses. r1 v1 already did this; r2 leaving it in the host is not forced. |
| Revised `deposit_deadline_ms = accepted_ms + deposit_window_ms` | **Not expressible** (`expected an integer after \`+\``). Same **minimal missing op** as N14 deadline: add two integer operands in a comparison, then `then deposit_deadline_ms = "$deposit_deadline_ms"`. If `deposit_window_ms` were a machine constant, `$now_ms + 5000` would already suffice; the contract stores a per-listing window fact. |
| Promote “clears deposit fields” including string ids | **Already supported** via then string literals (`deposit_id = '""'`, reproduced empty string). Agents who believed string `then` was impossible left leftover ids; that is not a language gap. |

---

## Defects observed on the frozen binary (not the original claim)

1. **Counter `then +N` wraps** at `i64::MAX` (`count: -9223372036854775808`); guard `+` refuses. Replay of a wrapping increment would persist the wrapped value.
2. **`then = { n = "+1" }` on replace facts increments** (5→6) while static check accepts `+1` as a literal integer. Identity hash encodes the compiled increment.
3. **`*` / `/` parse errors are opaque** (`expected a comparison operator` or `unexpected token Op("")`), which feeds the “no arithmetic” story.

These do not justify the claim that string literals or integer `+const` are absent.
