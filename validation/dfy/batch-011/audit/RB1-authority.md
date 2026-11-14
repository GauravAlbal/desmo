# RB1 — Authority per consequential rule (lineage-2 N13 / N14 / N15)

```text
ASSESS
  boundary           CHANGE and BUILD deliveries N13, N14, N15, both replicas.
                     Written contract is BRIEF.md (CHANGE.md on change trees).
                     Runtime law is the .ik.toml when/then/from path plus store facts.
  not in scope       evaluator, controls, authority, diagnose, sealed, oracle, other
                     run directories; SQL tampering; privileged store edits
  assumption         NOW_MS / $now_ms is a trusted CAT-1 clock; caller ids and
                     caller-supplied quantities (qty, units, ask_cents, offer amounts,
                     hold_ms, accept_by_ms, deposit_window_ms) are observations
  method             classify each consequential amount/deadline/approval/invalidation
                     as CAT-1/2/3 from machine vs host; for every CAT-3 rule, copy
                     the sqlite store after legitimate ./run setup and issue the
                     host’s Ikaros client API with a deliberately incorrect derived
                     value (or the forbidden command). API: Aggregate.command on
                     N13/N15 r1 and N14; Aggregate.execute on N13/N15 r2.
  proof note         honest ./run green is WRONG_METHOD for “the runtime enforces
                     this formula”: that check never supplies an out-of-formula arg.
                     Direct command on a store copy is the seam that can expose
                     absence of the machine relation. A2 inspector notes are
                     hypotheses until this sitting’s command matched.
  instrument         Node 22.17.1; each project’s @ikaros/runtime or .venv + KIT/bin/ik;
                     scratch /tmp/authority-scratch (deleted after this report)
```

**Classification.**

| CAT | Meaning |
|---|---|
| 1 | Externally supplied observation whose producer the contract explicitly trusts (clock `NOW_MS`, caller identifiers and caller-supplied quantities, matching provider HTTP body). |
| 2 | Derived value computed or independently checked on the authoritative runtime transition (`when` guard and/or `then` assignment that encodes the defining relationship). |
| 3 | Host-derived proposal the kernel copies (or a host-only refuse) without checking the defining relationship. An authority gap for any claim that the **runtime** enforces that rule. |

Honest `./run` still implements the contract on the CLI path in every delivery exercised below. CAT-3 is about store/machine sufficiency, not about whether `./run` is polite.

---

## N13 — Interlibrary loan desk

Trees: `N13/r{1,2}/{build,change}/project`. Store `STATE_DIR/ill-desk.sqlite`. Aggregate `desk`. r1 API: `agg.command` (`src/desk.js` `apply`). r2 API: `agg.execute` (`src/desk.mjs`).

### Contract rules (BRIEF + CHANGE)

| ID | Rule | Source |
|---|---|---|
| N13-A1 | `checkout_ms = NOW_MS` | BRIEF checkout |
| N13-A2 | BUILD / pre-change: `loan_until_ms = checkout_ms + 1209600000` | BRIEF; CHANGE old-instance |
| N13-A3 | CHANGE new: `loan_until_ms = checkout_ms + 604800000` | CHANGE checkout |
| N13-A4 | `overdue = 1` iff status `out` and `NOW_MS > loan_until_ms` | BRIEF status / checkin |
| N13-A5 | BUILD: `checkin` allowed when overdue | BRIEF checkin |
| N13-A6 | CHANGE new: overdue `checkin` without `recall_placed=1` is `recall_required`, no mutation | CHANGE checkin; C3 |
| N13-A7 | CHANGE new: `place-recall` on `out` records `recall_placed=1`; repeat is no-op | CHANGE place-recall; C1–C2 |
| N13-A8 | CHANGE: `place-recall` on a pre-change instance is `legacy_request`, no mutation | CHANGE old-instance; C2 |
| N13-A9 | Status-gated invalidations: `checkout` only `received`; `checkin` only `out`; `receive` only `requested`+accepted; `cancel` not while `attempt_saved`; terminals stay terminal | BRIEF commands |
| N13-A10 | Saved lender snapshot (`req_id`, `lender_id`, `oclc`) and same `loan_req_id` on retry | BRIEF transmit |

### Per-delivery classification

**N13-A1 `checkout_ms = NOW_MS`.** CAT-1 recording of the trusted clock. Machines take `$checkout_ms` as an int with no clock of their own (`ill_desk.ik.toml` / `desk.ik.toml` `commands.checkout`). Lying about the clock is lying about a CAT-1 observation, not a derived amount.

**N13-A2 / N13-A3 loan span.** CAT-3 in **all four** deliveries. Host computes the sum and passes `loan_until_ms`.

- r1/build host: `src/desk.js:16` `LOAN_SPAN`, `:309–310` `loanUntil = Number(nowMs + LOAN_SPAN)` then `apply(..., {checkout_ms, loan_until_ms})`. Machine `machines/ill_desk.ik.toml:64–68` `then = { checkout_ms = "$checkout_ms", loan_until_ms = "$loan_until_ms" }` — no `when` on the difference.
- r1/change host: `src/desk.js:18–19`, `:122` `loanSpan`, `:351–356`. v2 machine `ill_desk_v2.ik.toml:65–69` same unguarded copy. v1 machine identical.
- r2/build host: `src/desk.mjs:14`, `:454–456`. Machine `desk.ik.toml:66–70` (project root, not `machines/`).
- r2/change host: `src/desk.mjs:14–15,128,509–511`. Machine `desk_v2.ik.toml:66–70` (project root).

Direct `checkout {checkout_ms:1000, loan_until_ms:1001}` on a copy after legitimate file/locate/transmit/receive:

| Delivery | Runtime | Persisted |
|---|---|---|
| r1/build | **ACCEPTED** `command` | `named:out`, `loan_until_ms:1001` (honest CLI: `1209601000`) |
| r1/change | **ACCEPTED** `command` | `loan_until_ms:1001` (honest CLI: `604801000`) |
| r2/build | **ACCEPTED** `execute` | `loan_until_ms:1001` (honest CLI: `1209601000`) |
| r2/change | **ACCEPTED** `execute` | `loan_until_ms:1001` (honest CLI: `604801000`) |

**N13-A4 overdue flag.** Status projection from CAT-1 `NOW_MS` vs stored `loan_until_ms`. Not a stored fact. Host: r1/change `src/desk.js:364`; r2/change `src/desk.mjs:527`. Not a runtime transition.

**N13-A5 BUILD overdue checkin allowed.** CAT-2 / not a gap: v1 `checkin` `from=["out"]` `to="returned"` with no overdue `when` **is** the BRIEF rule. Not reproduced as a defect.

**N13-A6 CHANGE new `recall_required`.** CAT-3 on both change replicas. v2 `checkin` has no `when` (`ill_desk_v2.ik.toml:71–73`; `desk_v2.ik.toml:72–74`). Host only: r1/change `src/desk.js:364–366`; r2/change `src/desk.mjs:528–529`.

After honest 7-day checkout (`loan_until_ms=604801000`, `recall_placed=0`):

- CLI `NOW_MS=604801001 checkin` → `{"error":"recall_required"}` (r1/change and r2/change). Store stays `out`.
- Direct `checkin {}` on a copy → **ACCEPTED**, persisted `named:returned`, `recall_placed:0`.

Inspector F1 **reproduced** as a contract-authority gap (CHANGE C3). The runtime does not enforce the new law.

**N13-A7 `recall_placed=1`.** Split.

- r2/change: CAT-2. `desk_v2.ik.toml:85–90` `when = "recall_placed == 0 && $recall_placed == 1"`. Direct `{recall_placed:0}` → **REFUSED** `IK_INVARIANT_FAILED` `place_recall.when`. Direct `{recall_placed:1}` → **ACCEPTED**, `recall_placed:1`. Repeat-as-no-op is still a host short-circuit (`src/desk.mjs:455`).
- r1/change: CAT-3. `ill_desk_v2.ik.toml:75–79` copies `$recall_placed` with no `when`. Direct `{recall_placed:0}` → **ACCEPTED**, persisted `recall_placed:0`. Direct `{recall_placed:1}` → **ACCEPTED**, `recall_placed:1`. Host always passes 1 (`src/desk.js:321`).

**N13-A8 `legacy_request`.** Host maps owner before the command (r1/change `src/desk.js:116–118` version-only; r2/change `src/desk.mjs:123–128` `identity.equals`). v1 machines have **no** `place_recall`, so a v1 handle would refuse the unknown command. That is CAT-2 absence of the new command on the old definition, not a derived-amount gap. Direct CAT-3 counterexample is not applicable on a v2 handle. Not treated as a runtime amount/deadline gap.

**N13-A9 status gates.** CAT-2 in all four: `from`/`when` on locate, receive, checkout, checkin, cancel, declare_lost. r2 `save_attempt` also binds snapshot: `desk.ik.toml:44–46` `$req_id == req_id && $lender_id == lender_id && $oclc == oclc` (CAT-2). r1 `save_attempt` does not check snapshot fields (not an amount rule).

**N13-A10 loan id.** CAT-1 application-created id, persisted before POST. Effect `id = "$loan_req_id"`. Not a derived amount.

---

## N14 — Pharmacy refill desk

Trees: `N14/r{1,2}/{build,change}/project`. r1 store `ikaros.sqlite`, aggregate `desk`, `agg.command`. r2 store `desk.sqlite`, aggregate `prescription`, `agg.command`. Python `invariant_kernel.Kernel`.

### Contract rules (BRIEF + CHANGE)

| ID | Rule | Source |
|---|---|---|
| N14-A1 | Intake `remaining_qty = qty` | BRIEF intake |
| N14-A2 | Fill deducts `units` from `remaining_qty`; `held_units = units` | BRIEF fill; R3 |
| N14-A3 | BUILD / v1 fill: `pickup_deadline_ms = NOW_MS + hold_ms` | BRIEF fill |
| N14-A4 | CHANGE v2 fill: `pickup_deadline_ms = NOW_MS + min(hold_ms, 86400000)` | CHANGE; C3 |
| N14-A5 | `amount_cents = units * 175` (held lot and claim snapshot) | BRIEF amounts / claim |
| N14-A6 | Unpaid lapse restores `remaining_qty += held_units`; paid lapse does not | BRIEF pickup |
| N14-A7 | Successful pickup: remaining>0 stays remaining; remaining=0 and refills>0 → `remaining_qty=qty`, `refills-=1`; else dispensed | BRIEF pickup; R4 |
| N14-A8 | Pickup unpaid and on time is `unpaid`, no mutation | BRIEF pickup; R7 |
| N14-A9 | Pickup lapsed only when `NOW_MS > pickup_deadline_ms` | BRIEF pickup |
| N14-A10 | CHANGE v2 `rush`: `pickup_deadline_ms = min(current, NOW_MS+3600000)`; never extends; `rush_ms` = that result | CHANGE; C1–C2 |
| N14-A11 | CHANGE: `rush` on v1 is `legacy_rx`, no mutation | CHANGE old-instance |
| N14-A12 | Second intake is `already_bound` | BRIEF intake |
| N14-A13 | Fill `units` in `1..remaining_qty`; fill only from `verified` | BRIEF fill |
| N14-A14 | Claim snapshot units = `held_units`, amount = `held_units*175` | BRIEF claim |

### Per-delivery classification

**N14-A1 remaining = qty on intake.** CAT-2 all four. r1 `bind` `then remaining_qty = "$qty"` (`rx_desk.ik.toml:35` / v2 `:36`). r2 `intake` same (`rx_desk.ik.toml:36`).

**N14-A2 remaining after fill.** CAT-3 all four. `when` checks `$units <= remaining_qty` then **overwrites** remaining with a host arg (`$remaining_qty` / `$remain_after`), not `remaining_qty - $units`.

- r1/build `rx_desk.py:376–391`; machine `rx_desk.ik.toml:43–48`.
- r1/change `rx_desk.py:390–412`; v2 `rx_desk_v2.ik.toml:44–49`.
- r2/build `desk.py:384–393`; `rx_desk.ik.toml:42–47`.
- r2/change `desk.py:401–405`; v2 `:42–47`.

Direct fill `units=1` with `remaining_qty`/`remain_after=0`, `amount_cents=1`, `deadline=0` after honest intake+verify:

| Delivery | Runtime | Persisted |
|---|---|---|
| r1/build | **ACCEPTED** | `held`, `remaining_qty:0`, `amount_cents:1`, `pickup_deadline_ms:0`, `held_units:1`, `qty:10` |
| r1/change | **ACCEPTED** | same lie, `edition:v2` |
| r2/build | **ACCEPTED** | `remaining_qty:0`, `amount_cents:1`, `pickup_deadline_ms:0`, `units:1` |
| r2/change | **ACCEPTED** | same lie |

Honest CLI fill `units=4` still yields `remaining_qty:6` and v1 deadline `172800000` / v2 cap `86400000`.

**N14-A3 / N14-A4 pickup deadline on fill.** CAT-3 all four (same fill `then pickup_deadline_ms = "$…"`). Host: r1/build `:376`; r1/change `:400–403` `min(hold, HOLD_CAP)`; r2/build `:384`; r2/change `:401–405`. Direct deadline `0` accepted as above. Honest v2 CLI fill at `NOW_MS=0` / `hold_ms=172800000` persisted `86400000` (cap held on the CLI path only).

**N14-A5 amount = units×175.** CAT-3 at fill in all four (fill writes `$amount_cents`). After an **honest** fill:

- r1 save_insurer `when = "... && $amount_cents == amount_cents && $units == held_units"` (`rx_desk.ik.toml:54`, v2 `:62`). Direct `amount_cents=1` → **REFUSED** `save_insurer.when`. CAT-2 consistency with the **already stored** fill amount, not with `×175`. The `×175` gap is at fill.
- r2 save_insurer `when = "claim == 'none'"` only (`rx_desk.ik.toml:53`; v2 `:53`). Direct `{units:99, amount_cents:1, claim_st:"saved"}` → **ACCEPTED**, persisted `units:99`, `amount_cents:1`, `claim:saved` while `held_units` stayed `4`. CAT-3 for snapshot amounts.

**N14-A6 / N14-A7 restore arithmetic.** CAT-3. r1 `lapse_unpaid` / `pickup_ok` copy `$remaining_qty` / `$refills` (`rx_desk.ik.toml:105–110`; host r1/build `rx_desk.py:597–607` lapse and `:619–638` pickup_ok; r1/change lapse at `:622`). Direct `lapse_unpaid remaining_qty=0` on an unpaid held lot (remaining 6, held 4, deadline still in the future) → **ACCEPTED**, `named:verified`, `remaining_qty:0` (should have been 10, and should not have lapsed yet). r2 `pickup` copies `$remain_after` / `$qty_restore` / `$refills_after` (`rx_desk.ik.toml:86–95`). Direct `now_ms=172800001` (past deadline) `remain_after=0` → **ACCEPTED**, `remaining_qty:0` (should be 10). Same on r2/change at `now_ms=86400001`.

**N14-A8 unpaid on-time is non-mutating.** CAT-2 on r2: no pickup outcome for unpaid ∧ `now_ms <= deadline`. Direct `now_ms=1` (before deadline) `remain_after=0` → **REFUSED** `pickup.when`. r1: host returns `unpaid` without a command (r1/build `rx_desk.py:607`; r1/change `:632`). Direct `lapse_unpaid` still mutates (A6), so the **clocked** unpaid-no-op is host-only (CAT-3 selector).

**N14-A9 lapse only after deadline.** r1: CAT-3. Machine `lapse_unpaid` has no clock (`rx_desk.ik.toml:105–110`); host chooses the command. Direct lapse before deadline accepted (A6). r2: CAT-2 relative to **supplied** `$now_ms` (`$now_ms > pickup_deadline_ms`). Lying about `now_ms` is lying about CAT-1 clock, not a derived amount. The restore value remains CAT-3.

**N14-A10 rush formula.** Split.

- r1/change: non-extension CAT-2 (`rx_desk_v2.ik.toml:51–56` `$pickup_deadline_ms <= pickup_deadline_ms && $rush_ms == $pickup_deadline_ms`). Direct `{pickup_deadline_ms:999999999}` → **REFUSED** `rush.when`. Exact `min(current, NOW+3600000)` is CAT-3: direct `{pickup_deadline_ms:1, rush_ms:1}` → **ACCEPTED**, persisted `pickup_deadline_ms:1`, `rush_ms:1` (honest CLI at `NOW_MS=0` after a 24h-capped fill: `3600000`). Host: `rx_desk.py:780–794`.
- r2/change: CAT-3 fully. `rx_desk_v2.ik.toml:97–101` no `when`. Direct `{deadline:999999999, rush_at:999999999}` → **ACCEPTED** (extension). Direct `{deadline:1, rush_at:1}` → **ACCEPTED**. Host: `desk.py:442–457`.

**N14-A11 `legacy_rx`.** Host edition/owner check (r1/change `rx_desk.py:773–778`; r2/change `desk.py:435–441`). v1 machines have no `rush`. CAT-2 absence of the new command on v1, same shape as N13-A8.

**N14-A12 already_bound.** Split.

- r2: CAT-2. `intake` `from=["empty"]`. Direct second `intake` from `verified` → **REFUSED** `IK_ILLEGAL_STATE` `intake`.
- r1: CAT-3. Host `if agg is not None: already_bound` (build `rx_desk.py:274–275`; change `:296–297`). Machine `bind` `from=["intake"]` (`rx_desk.ik.toml:31–35` / v2 `:32–36`). Direct second `bind` from `intake` with `qty=1` → **ACCEPTED**, persisted `qty:1`, `remaining_qty:1` (overwrote 10). From `verified`, bind is illegal (`from` does not include verified) — that refuse is CAT-2 and is **not** the `already_bound` rule.

**N14-A13 units bounds / verified-only fill.** CAT-2: fill `when` `$units >= 1 && $units <= remaining_qty` and `from=["verified"]`. Direct `units=1` with remaining 10 is legal; the lie is the leftover remaining (A2).

**N14-A14 claim snapshot.** See A5. r1 CAT-2 vs stored fill facts; r2 CAT-3.

Inspector F1 (host remaining/deadline/rush/restore) **reproduced**. Inspector F2 (r1 `already_bound` host-only) **reproduced** from `intake`. Inspector “r2 rush has no `when`” **reproduced** (extension accepted).

---

## N15 — Property offer and escrow desk

Trees: `N15/r{1,2}/{build,change}/project`. Store `desk.sqlite`. Aggregate `desk`. r1 API: `agg.command` (`src/cli.mjs` `exec`). r2 API: `agg.execute` (`desk.mjs`).

### Contract rules (BRIEF + CHANGE)

| ID | Rule | Source |
|---|---|---|
| N15-A1 | `deposit_cents = floor(primary.amount_cents / 50)` | BRIEF accept; R3 (worked: 10000 → 200; counter 11000 → 220) |
| N15-A2 | BUILD / original: `deposit_deadline_ms = accept_by_ms` | BRIEF accept; CHANGE C4 |
| N15-A3 | CHANGE revised: `deposit_deadline_ms = accepted_ms + deposit_window_ms` | CHANGE; C3 (worked: accept at 1000, window 5000 → 6000) |
| N15-A4 | Accept only if `NOW_MS <= accept_by_ms`; else durable `expired` / `window_elapsed` | BRIEF accept / bid |
| N15-A5 | Deposit after `deposit_deadline_ms` is `deposit_elapsed`, durable expire, no POST | BRIEF deposit |
| N15-A6 | CHANGE `promote` on fallen revised with backup: backup becomes primary `countered:false`; **clears** deposit id/cents/`accepted_ms`/`deposit_deadline_ms`; `phase:offered`, `escrow:none` | CHANGE promote; C1 |
| N15-A7 | CHANGE: `promote` on original is `legacy_listing`, no mutation | CHANGE old-instance; C2 |
| N15-A8 | Bid/backup/third-offer/`desk_full`; counter once on primary | BRIEF bid/counter |
| N15-A9 | Funded deposit no-op; fallthrough only `under_contract`+funded | BRIEF deposit/fallthrough |

### Per-delivery classification

**N15-A1 `floor(amount/50)`.** CAT-3 in **all four**. Machines require `$deposit_cents >= 0` only.

- r1/build host `src/cli.mjs:436–442`; machine `machines/desk.ik.toml:87–92`.
- r1/change host `src/cli.mjs:492–498`; v2 `desk_v2.ik.toml:88–93`.
- r2/build host `desk.mjs:452–462`; `listing.ik.toml:67–72`.
- r2/change host `desk.mjs:490–504`; `listing_v2.ik.toml:67–72`.

Direct `accept` with `deposit_cents:1` (honest is 200 for amount 10000):

| Delivery | Other args | Runtime | Persisted |
|---|---|---|---|
| r1/build | `deposit_deadline_ms=2000` (legal original deadline) | **ACCEPTED** | `deposit_cents:1`, `deposit_deadline_ms:2000` |
| r1/build | `deposit_cents:1`, `deadline:9999` | **REFUSED** `accept.when` (deadline law, not cents) | unchanged `offered` |
| r1/change (v2) | `cents:1`, `deadline:9999` | **ACCEPTED** | `deposit_cents:1`, `deposit_deadline_ms:9999` |
| r2/build | `cents:1`, `deadline:9999` | **ACCEPTED** | `deposit_cents:1`, `deposit_deadline_ms:9999` |
| r2/change | `cents:1`, `deadline:9999` | **ACCEPTED** | `deposit_cents:1`, `deposit_deadline_ms:9999` |

Honest CLI accept: r1/build and r2/build `deposit_cents:200`, `deposit_deadline_ms:2000`; change replicas `200` / `6000`.

**N15-A2 original deadline = `accept_by_ms`.** Split.

- r1/build (v1): CAT-2. `desk.ik.toml:91` `$deposit_deadline_ms == accept_by_ms`. Direct `deadline:6000` (revised formula) → **REFUSED**. Direct `deadline:2000` → **ACCEPTED**.
- r2/build (v1): CAT-3. `listing.ik.toml:69–72` no deadline equality. Direct `deadline:6000` → **ACCEPTED** (`deposit_deadline_ms:6000` on a BUILD instance). Direct `deadline:9999` → **ACCEPTED**. C4-shaped original deadline is **not** a machine law on r2 even at BUILD.
- r1/change original listings keep the v1 machine (owner route). That v1 equality remains CAT-2 **if** commands run on the v1 handle. New v2 listings: see A3.
- r2/change original: v1 `accept` still has no deadline equality (CAT-3). Host `desk.mjs:491–495` is the only C4 branch. Direct `accept` with `deadline:6000` on a v2 handle (new listing) is also accepted; a v1 handle would likewise copy the arg.

**N15-A3 revised deadline = `accepted_ms + deposit_window_ms`.** CAT-3 on both change replicas. r1 v2 `desk_v2.ik.toml:92` `$deposit_deadline_ms >= $now_ms` (partial lower bound only). r2 v2 no deadline relation. Direct `deadline:9999` accepted (A1 table). Direct `deadline:2000` (original formula) also **ACCEPTED** on r1/change and r2/change v2 handles — the runtime will store an original-style deadline on a revised listing. Host formula: r1/change `src/cli.mjs:322–326`; r2/change `desk.mjs:492–495`.

**N15-A4 accept-by window.** CAT-2 all four: `$now_ms <= accept_by_ms` on accept, and expire outcomes when `$now_ms > accept_by_ms`. `now_ms` is the CAT-1 clock observation passed in.

**N15-A5 deposit elapsed.** CAT-2 vs stored `deposit_deadline_ms` and supplied `now_ms` (r1 `plan_deposit` / `expire_deposit`; r2 `deposit_gate`). The **value** of that stored deadline is CAT-3 wherever A2/A3 are CAT-3.

**N15-A6 promote clears deposit fields.** Split; both are authority gaps vs CHANGE C1.

- r1/change: CAT-3 for clearing. `desk_v2.ik.toml:139–144` moves backup → primary (`when` `$offer_id == backup_offer_id && $amount_cents == backup_amount_cents` — that identity check is CAT-2) and sets `has_backup=0`, `escrow_code=0`. It does **not** clear `deposit_id`, `deposit_cents`, `accepted_ms`, `deposit_deadline_ms`. Direct `promote {offer_id:O2, buyer_id:B2, amount_cents:9000}` after legitimate accept/deposit/fallthrough → **ACCEPTED** `named:offered`, persisted `deposit_cents:200`, `deposit_deadline_ms:6000`, `deposit_id:<hex>`, `accepted_ms:1000`, `has_backup:0`. CLI `promote` JSON nulls those fields (`src/cli.mjs:704–705`; `depositFieldsForStatus` `:131–134`) while the store keeps them. Inspector F1 **reproduced**.
- r2/change: partial CAT-2. `listing_v2.ik.toml:108–113` zeros `accepted_ms`, `deposit_deadline_ms`, `deposit_cents`, `has_deposit`, `has_backup`. Direct promote → **ACCEPTED**, those zeros persisted. `deposit_id` string remains `d766659e83c48d8c9bfc7cd52` (CAT-3 leftover id). CLI JSON `deposit_id:null` via `has_deposit` (`desk.mjs:169–175`). Inspector F3 **reproduced**.

**N15-A7 `legacy_listing`.** Host refuse before command (r1/change `src/cli.mjs:683`; r2/change `desk.mjs:691`). v1 has no `promote`. Same CAT-2 “command absent on old definition” as N13-A8 / N14-A11.

**N15-A8 bid/counter capacity.** CAT-2: `has_primary` / `has_backup` / `$offer_id != primary` / `primary_countered == 0` / `$amount_cents >= 1`. Caller offer **amounts** are CAT-1.

**N15-A9 funded / fallthrough.** CAT-2 escrow_code / `has_deposit` gates (`complete_hold`, `deposit_funded_noop`, `plan_release` / `mark_fallen`).

Inspector F4/F7 (host deadline/`deposit_cents`; r2 original deadline not machine-enforced) **reproduced**. Inspector F1 leftover deposit facts after r1 promote **reproduced**. Inspector F3 leftover `deposit_id` on r2 promote **reproduced**.

---

## Direct-command counterexamples (CAT-3 only)

All against **copies** of stores produced by legitimate `./run` (stub `SERVICE_URL`, `NOW_MS` as in BRIEF examples). API = the host’s Ikaros client. No SQL.

### N13 loan span (A2/A3) — all four

Setup: `file ILL-1 PAT-9 OCLC-42` → `locate LEND-A` → `transmit` → `receive` at `NOW_MS=1000`. Copy `ill-desk.sqlite`.

```text
# r1: Aggregate.command   r2: Aggregate.execute
checkout { checkout_ms: 1000, loan_until_ms: 1001 }
```

**ACCEPTED** every replica. Persisted `loan_until_ms=1001`. Honest CLI on a sibling copy: build `1209601000`, change `604801000`.

### N13 CHANGE overdue checkin without recall (A6) — r1/change, r2/change

Setup: honest checkout at `NOW_MS=1000` (`loan_until_ms=604801000`, `recall_placed=0`). Copy store.

```text
# CLI at NOW_MS=604801001
./run checkin   →  {"error":"recall_required"}  (store stays out)

# Direct on the copy (r1 command / r2 execute)
checkin {}
```

**ACCEPTED**. Persisted `named:returned`, `recall_placed:0`.

### N13 r1/change `recall_placed` (A7)

On the honest-out copy:

```text
place_recall { recall_placed: 0 }  → ACCEPTED, recall_placed:0
place_recall { recall_placed: 1 }  → ACCEPTED, recall_placed:1
```

r2/change `{recall_placed:0}` **REFUSED** (`place_recall.when`) — CAT-2, preserved.

### N14 fill remaining / amount / deadline (A2/A3/A4/A5) — all four

Setup: `intake RX1 PAT1 00000000001 10 1 172800000` → `verify RX1` at `NOW_MS=0`. Copy store.

```text
# r1 command fill
{ rx_id:"RX1", units:1, remaining_qty:0, pickup_deadline_ms:0, amount_cents:1, claim:"none", payor:"none" }

# r2 command fill  (change adds rush_clear:-1)
{ units:1, remain_after:0, deadline:0, amount_cents:1 }
```

**ACCEPTED** all four. Persisted `remaining_qty:0`, `amount_cents:1`, `pickup_deadline_ms:0`, `held_units:1`, `qty:10`. Honest CLI `fill RX1 4`: remaining 6; v1 deadline 172800000; v2 deadline 86400000.

### N14 r1 lapse restore without clock (A6/A9)

After honest fill (remaining 6, held 4):

```text
lapse_unpaid { remaining_qty:0, claim:"none", payor:"none" }
```

**ACCEPTED** r1/build and r1/change. Persisted `named:verified`, `remaining_qty:0` (defining restore is 10; defining clock has not lapsed).

### N14 r2 lapse restore with past `$now_ms` (A6)

```text
pickup { now_ms:172800001, remain_after:0, refills_after:0, qty_restore:10,
         claim_reset:"none", empty:"", zero:0, neg:-1 }
```

**ACCEPTED** r2/build. Persisted `remaining_qty:0` (should restore to 10). r2/change same with `now_ms:86400001` on a v2 held lot. On-time `now_ms=1` unpaid pickup **REFUSED** (`pickup.when`) — CAT-2 preserved.

### N14 r1 second bind from intake (A12)

After honest `intake` only:

```text
bind { rx_id:"RX1", ..., qty:1, refills:0, hold_ms:0, edition:"v1"|"v2", ... }
```

**ACCEPTED**. Persisted `qty:1`, `remaining_qty:1` (was 10). r2 second `intake` from `verified` **REFUSED** — CAT-2 preserved.

### N14 r2 claim snapshot (A5/A14)

After honest fill (`held_units=4`, stored `amount_cents=700`):

```text
save_insurer { claim_id:"C-LIE", units:99, amount_cents:1, payor:"insurer",
               claim_st:"saved", effect_id:"C-LIE", payload_digest:"aa",
               idempotency_key:"C-LIE" }
```

**ACCEPTED** r2/build and r2/change. Persisted `units:99`, `amount_cents:1`, `claim:saved`. r1 equivalent **REFUSED** (`$amount_cents == amount_cents`) — CAT-2 vs stored fill amount.

### N14 CHANGE rush (A10)

After honest v2 fill (deadline 86400000):

```text
# r1/change
rush { pickup_deadline_ms:999999999, rush_ms:999999999 }  → REFUSED rush.when
rush { pickup_deadline_ms:1, rush_ms:1 }                  → ACCEPTED deadline=1

# r2/change
rush { deadline:999999999, rush_at:999999999 }            → ACCEPTED (extension)
rush { deadline:1, rush_at:1 }                            → ACCEPTED
```

Honest CLI `rush RX1` at `NOW_MS=0`: `pickup_deadline_ms=3600000`.

### N15 deposit_cents and deadlines (A1/A2/A3)

Setup: `open L1 S1 10000 2000 5000` → `bid L1 O1 B1 10000` → `bid L1 O2 B2 9000` at `NOW_MS=1000`. Copy `desk.sqlite`.

```text
# r1 command accept (also deposit_offer_id/buyer_id)
{ offer_id:"O1", now_ms:1000, deposit_cents:1, deposit_deadline_ms:<D>,
  deposit_offer_id:"O1", deposit_buyer_id:"B1" }

# r2 execute accept
{ offer_id:"O1", now_ms:1000, deposit_cents:1, deposit_deadline_ms:<D> }
```

| Delivery | D | Result |
|---|---|---|
| r1/build | 2000 | **ACCEPTED** `deposit_cents:1` (cents gap; deadline law held) |
| r1/build | 9999 or 6000 | **REFUSED** `accept.when` (deadline CAT-2) |
| r1/change v2 | 9999 | **ACCEPTED** cents=1, deadline=9999 |
| r1/change v2 | 2000 | **ACCEPTED** (original formula on a revised listing) |
| r1/change v2 | 6000 | **ACCEPTED** (honest formula, still not checked as a relation) |
| r2/build | 9999 / 2000 / 6000 | **ACCEPTED** all three |
| r2/change | 9999 / 2000 / 6000 | **ACCEPTED** all three |

Honest CLI accept: build `deposit_cents:200` `deadline:2000`; change `200` / `6000`.

### N15 CHANGE promote clear (A6)

Setup: open/bid/bid/accept/deposit/fallthrough (stub up). Copy store. Direct:

```text
promote { offer_id:"O2", buyer_id:"B2", amount_cents:9000 }
```

- r1/change: **ACCEPTED** `named:offered`, `has_backup:0`, **deposit facts retained** (`deposit_cents:200`, `deposit_deadline_ms:6000`, `accepted_ms:1000`, `deposit_id` set). CLI JSON `deposit_id/cents: null`.
- r2/change: **ACCEPTED** `named:offered`; numeric deposit fields zeroed; **`deposit_id` string retained**. CLI JSON `deposit_id: null`.

---

## Category-2 rules that are enforced (preserve)

These were witnessed as runtime refuses or as `then` assignments that encode the contract relation:

- N13: `from`/`when` on file, locate, receive, checkout, checkin, cancel, declare_lost; r2 `save_attempt` snapshot equality; r2/change `place_recall` `0→1` only; BUILD overdue checkin unguarded **matches BRIEF**.
- N14: intake/bind `remaining_qty = $qty`; fill `from=verified` and `$units ∈ 1..remaining_qty`; r1 `save_insurer`/`save_cash` `$units == held_units && $amount_cents == amount_cents`; r1/change rush **non-extension**; r2 `intake` only from `empty`; r2 unpaid on-time pickup has no outcome; r2 lapse vs not-lapse selected by `$now_ms > pickup_deadline_ms`; r1 `pickup_dispensed` `remaining_qty==0 && refills==0`; claim `from=held` / `claim==none|rejected` as written.
- N15: `ask_cents >= 1`; bid/backup/counter `from`/`has_*`/`amount >= 1`; accept `$offer_id == primary_offer_id && $now_ms <= accept_by_ms`; r1 v1 `$deposit_deadline_ms == accept_by_ms`; r1 `$deposit_offer_id/$deposit_buyer_id` bound to primary; r1 v2 / r2 v2 promote backup identity (and r1 amount) match; expire/deposit_gate vs supplied `now_ms` and stored deadline; escrow_code / `has_deposit` fund and release gates; r2/change promote **zeros** `deposit_cents`, `deposit_deadline_ms`, `accepted_ms`, `has_deposit`.

CAT-1 (trusted observations, not derived): `NOW_MS`; caller ids and caller-supplied `qty` / `hold_ms` / `units` / offer `amount_cents` / `ask_cents` / `accept_by_ms` / `deposit_window_ms`; application-created loan/claim/deposit ids; matching provider HTTP bodies (`outcome:"accepted"|"rejected"|"held"`). Direct commands that only lie about `NOW_MS`/`now_ms` are not counted as CAT-3.

---

## Judgment

**[observation]** Twelve deliveries (3 families × 2 replicas × 2 stages) persist lifecycle in one Ikaros sqlite. Honest `./run` computed the contract formulas in the host and passed them as command args. Direct client commands that supplied a wrong derived value were accepted whenever the machine `then` copied the arg without a `when` on the defining relation. Accepted/refused rows and persisted facts are in the counterexample section (copied stores; host client API).

**[inference]** A store-sufficiency or `./run`-only suite that never issues an out-of-formula command will not see these gaps: after a compliant CLI fill/checkout/accept the facts look consistent. That check is `WRONG_METHOD` for a runtime-enforcement claim (`proof` skill).

**[judgment]** (confidence: high on reproduced accepted/refused rows; evidential force: those rows establish the CAT-3 classification for the named rule on that delivery; they do not establish that every untested invalidation is unenforced.) The consequential **derived** amounts and deadlines in these contracts — loan span, fill remaining, fill/rush/pickup deadlines, `units×175`, `floor(amount/50)`, revised vs original deposit deadline, unpaid restore, r2 claim snapshot, r1/change recall value, CHANGE overdue checkin, r1 promote deposit-clear — are **not** runtime-enforced except where the table marks CAT-2. r1 N15 v1 original deadline, r1 N14 rush non-extension, and r2 N13 `place_recall` `0→1` are the notable CAT-2 exceptions among the new/derived rules.

**[exception — not a contract violation]** BUILD N13 overdue checkin (BRIEF allows it). N13/N14/N15 `legacy_*` host mapping of “command does not exist on v1” (runtime would still refuse the missing command on a v1 handle). Status `overdue` as a host view of CAT-1 clock vs stored until.

**[insufficiency]** Inspector F-ids are marked reproduced only where this sitting’s direct command matched. r2 pickup `now_ms=1` unpaid was **REFUSED** (not a clock bypass). r1 second bind from `verified` was **REFUSED** (the `already_bound` gap is from `intake` only). ATS IR lint was not run; this file is the assessment artifact the brief named.
