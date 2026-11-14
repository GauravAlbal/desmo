# B12 candidate-aware cross-check

Read-only. Expected behavior from `oracle/N16/{brief,change}.md` and `oracle/N17/{brief,change}.md` only. Deliveries: change-stage projects (v1 machine identical to that rep’s build machine). Columns:

- **N16r1** `runs-b12/N16/r1/change/project` (v1 `machines/card_desk.ik.toml`, v2 `card_desk_v2.ik.toml`, host `src/cli.mjs`)
- **N16r2** `…/N16/r2/change/project` (v1/v2 tomls, host `src/desk.mjs`)
- **N17r1** `…/N17/r1/change/project` (v1 `payout.ik.toml`, v2 `payout_v2.ik.toml`, host `src/desk.js`)
- **N17r2** `…/N17/r2/change/project` (v1/v2 under `machines/`, host `src/desk.mjs`)

Also listed as eight cells: each family×rep **build** (v1 only) and **change** (v1+v2). Build v1 files byte-identical to change v1.

Classes: **G** = machine `when`/outcome guard over stored facts/args (cite file:line); **T** = then/invalidation on the machine (facts cleared or rewritten; not a `when` predicate); **O** = trusted external observation (clock / provider body); **H** = host-computed or host-checked only (machine would accept a wrong value if the host submitted it). A cell may split (e.g. G+H).

Expressible? = could the written rule be a machine guard with string literals, `!=`, operand+operand, `*`/`/` const, exists/not_exists, comparisons.

Legend for weaker-than-brief: **W**.

## Table (rule × delivery × class)

| Rule | N16r1-build | N16r1-change | N16r2-build | N16r2-change | N17r1-build | N17r1-change | N17r2-build | N17r2-change |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| capture_deadline | G | G (v1+v2) | G | G (v1+v2) | n/a | n/a | n/a | n/a |
| capture_cap | G | G | G | G | n/a | n/a | n/a | n/a |
| fee (250 bps floor) | G | G | G | G | n/a | n/a | n/a | n/a |
| refund_gate | G | G | G | G | n/a | n/a | n/a | n/a |
| refund_amount | G | G | G | G (W: no `amount>=0`) | n/a | n/a | n/a | n/a |
| void_clear | n/a | T+G (v2) | n/a | T+G (v2) | n/a | n/a | n/a | n/a |
| daily_cap | n/a | G (v2) + H pre-refuse | n/a | G (v2) + H pre-refuse | n/a | n/a | n/a | n/a |
| payout_fee (2%) | n/a | n/a | n/a | n/a | G | G | G | G |
| hold_until (now+span) | n/a | n/a | n/a | n/a | G (now O) | G | G | G |
| hold_release_gate | n/a | n/a | n/a | n/a | G | G | G | G |
| dual_approval | n/a | n/a | n/a | n/a | G | G | G | G |
| approval_revision | n/a | n/a | n/a | n/a | T+G | T+G | T+G | T+G |
| risk_clear nonempty reviewer | n/a | n/a | n/a | n/a | n/a | **H** | n/a | **H** |
| risk_clear early end of hold | n/a | n/a | n/a | n/a | n/a | T (v2; no clock) | n/a | T (v2; no clock) |
| dual_threshold | n/a | n/a | n/a | n/a | G `==10000` | G v1 `10000` / v2 `5000` | G `==10000` | G v1 `10000` / v2 `5000` |

`NOW_MS` itself is **O** in every delivery (host parses env and passes `now_ms`). Comparisons against stored deadlines are **G**.

---

## Expressibility (all families)

| Rule | Expressible as supported guard? |
| --- | --- |
| capture_deadline | Yes: `$now_ms <= capture_deadline_ms` (brief: elapsed iff `NOW_MS > deadline`). Deadline identity `capture_deadline_ms == authorized_ms + window_ms` is operand+operand. |
| capture_cap | Yes: `$amount_cents <= authorized_cents`. |
| fee | Yes: `$fee_numerator == $captured_cents * 250 && $fee_cents == $fee_numerator / 10000` (or equivalent `/ 40`). |
| refund_gate | Yes: `settlement_hold == 0 && $now_ms > hold_until_ms`. |
| refund_amount | Yes: `$amount_cents <= captured_cents - refunded_cents` (operand+operand). |
| void_clear | Clearing is a `then`, not a `when`. Entry can be `remaining_hold > 0`. Target facts `remaining_hold == 0`, `capture_deadline_ms == 0`, `void_reason == ''` are expressible as post-state / then, not as a success guard on the old values. |
| daily_cap | Yes: `$day_after == day_authorized_cents + $amount_cents && $day_after <= 50000`. |
| payout_fee | Yes: `$fee_cents == amount_cents / 50` (2%). |
| hold_until | Yes: `$hold_until_ms == $now_ms + review_span_ms`. |
| hold_release_gate | Yes: `$now_ms >= hold_until_ms`. |
| dual_approval | Yes: `amount_cents > dual_threshold` vs `<=` plus `$approver_id != approver1`. |
| approval_revision | Invalidation is `then` (clear approvers, bump revision). Pay gate `approved_revision == revision` is a comparison. |
| risk_clear nonempty reviewer | Yes: `$reviewer_id != ""` (string literal + `!=`). **None of the deliveries do this.** |
| risk_clear early end | Yes as command with **no** `$now_ms >= hold_until_ms` and `then` hold_active=0 / deadline/reason cleared. |
| dual_threshold | Yes: `$dual_threshold == 5000` (revised) or `== 10000` (original) on `open`. |

---

## Per-delivery notes

### N16 r1 change (and r1 build = v1)

**capture_deadline — G.** v1 `card_desk.ik.toml:50` and v2 `:52` `save_capture` `when = "... && $now_ms <= capture_deadline_ms && ..."`. Authorize identity v1 `:43` / v2 `:45` `$capture_deadline_ms == $authorized_ms + $window_ms` and `$authorized_ms == $now_ms`. Host `cli.mjs:555-556` also refuses `capture_elapsed` before dispatch (defense in depth; machine still refuses a late `now_ms`). Expressible; not weaker (`<=` matches brief `>` elapsed).

**capture_cap — G.** v1 `:50` / v2 `:52` `$amount_cents <= authorized_cents`. Host `cli.mjs:552-553` mirrors. Not weaker.

**fee — G.** v1 `:60` / v2 `:60` complete_capture `$fee_numerator == $captured_cents * 250 && $fee_cents == $fee_numerator / 10000`. Host proposes via `completeArgs` `cli.mjs:373-378`. Machine rejects a wrong proposed fee. Expressible.

**refund_gate — G.** v1 `:83` / v2 `:83` `settlement_hold == 0 && ... && $now_ms > hold_until_ms`. Host `cli.mjs:669-670`. Hold duration at capture v1 `:60` `$hold_until_ms == $now_ms + 86400000`. Not weaker.

**refund_amount — G.** v1/v2 `:83` `$amount_cents <= captured_cents - refunded_cents && $refunded_after == refunded_cents + $amount_cents` plus `$amount_cents >= 0`. Host `cli.mjs:672-678`. Not weaker.

**void_clear — T+G (v2 only).** v2 `:86-90` `when = "remaining_hold > 0 && network == 'accepted'"` `then = { remaining_hold = "0", capture_deadline_ms = "0", void_reason = '""', network = '"none"' }`. Facts named in the change brief are machine-cleared. Host `cli.mjs:641-650` also hardcodes the success JSON. v1 has no `void`. Not weaker on the three facts; captured_cents left in place (brief does not require wiping them). After void, state is `open` so `invariants.captured_accepted` (`remaining_hold == authorized - captured`) does not apply.

**daily_cap — G (v2) + H pre-check.** v2 `:45` `$day_authorized_after == day_authorized_cents + $amount_cents && $day_authorized_after <= 50000`. Host `cli.mjs:497` `if (!loaded.legacy && dayAfter > DAILY_CAP_CENTS) refuse("daily_cap")`. Machine would still refuse 50001 if the host omitted the check. v1 authorize `:43` has **no** cap (matches old-instance policy). Extra conjunct `$day_authorized_after >= $amount_cents` is not weaker.

### N16 r2 change (and r2 build = v1)

**capture_deadline — G, with one host-only clock coupling.** v1 `:54` / v2 `:54` `$now_ms <= capture_deadline_ms`. Deadline identity v1 `:47` / v2 `:47` `$capture_deadline_ms == $authorized_ms + $window_ms`. **W relative to r1:** authorize does **not** require `$authorized_ms == $now_ms`. Host `desk.mjs:453-505` sets `authorized_ms` from `NOW_MS`. A host that submitted a mismatched pair still satisfying `deadline == authorized_ms + window_ms` would be accepted. Expressible (r1 form).

**capture_cap — G.** v1/v2 `:54` `$amount_cents <= authorized_cents`. Host `desk.mjs:524`.

**fee — G (equivalent form).** v1/v2 `:54` `$fee_cents == $amount_cents / 40` (integer; 250/10000). Host `desk.mjs:286-287` `Math.trunc(amount / 40)`. Accept on `accept_capture` only checks equality to stored `intended_fee_cents` (`:62`), so the formula is enforced at save time. Not weaker vs 250 bps floor for integer cents.

**refund_gate — G.** v1 `:76` / v2 `:76` `settlement_hold == 0 && $now_ms > hold_until_ms`. Host `desk.mjs:594`.

**refund_amount — G, W.** v1/v2 `:76` `$refunded_after == refunded_cents + $amount_cents && $refunded_after <= captured_cents`. Missing `$amount_cents >= 0` / `<= captured - refunded` on the arg. Negative `amount_cents` with matching `refunded_after` would satisfy the machine; host `desk.mjs:597` uses `after > captured` only. Brief equality/upper bound is expressible as r1’s form.

**void_clear — T+G (v2).** v2 `:79-83` `when = "remaining_hold > 0"` `then = { remaining_hold = "0", capture_deadline_ms = "0", void_reason = '""' }`. Does **not** clear `network` in the machine; host success JSON `desk.mjs:639` and `domainNetwork(open)` (`:111-114`) report `"none"`. Status after void is host-derived from named state. The three brief facts are machine-cleared. Slightly weaker than r1 (no `network == 'accepted'` entry guard).

**daily_cap — G+H.** v2 `:47` `$day_after == day_authorized_cents + $amount_cents && $day_after <= 50000`. Host `desk.mjs:467`. v1 has no cap.

### N17 r1 change (and r1 build = v1)

**payout_fee — G.** v1 `:82` / v2 `:90` prepare_pay `$fee_cents == amount_cents / 50`. Host `desk.js:650` `Math.trunc(... / 50)`. Machine rejects a wrong fee. Expressible (`* 2 / 100` not needed).

**hold_until — G, now O.** place_hold v1 `:66-67` / v2 `:66-67` `$hold_until_ms == $now_ms + review_span_ms && $hold_placed_ms == $now_ms`. Host `desk.js:533` supplies `now_ms`. `open` stores `review_span_ms == 5000` (`:43`).

**hold_release_gate — G.** v1 `:74-75` / v2 `:74-75` `$now_ms >= hold_until_ms` (brief: 5999 refuse, 6000 ok). Host `desk.js:563`. Pay also requires `hold_active == 0` (`prepare_pay` `:82` / v2 `:90`). Not weaker.

**dual_approval — G.** approve outcomes v1 `:57-59` / v2 same: one approver if `amount_cents <= dual_threshold`; two distinct (`$approver_id != approver1`) if `>`. Pay requires `approved_revision == revision` (`:82`).

**approval_revision — T+G.** edit_amount v1 `:50-51` `$revision == revision + 1` then `approval_count = 0`, `approver1/2 = ""`. Invariant `approved_matches_revision` `:96-97`. Expressible.

**risk_clear nonempty reviewer — H.** v2 `:78-84` args `reviewer_id` with **no** `$reviewer_id != ""`. Host `desk.js:579-580` `if (!isIdent(reviewerId)) refuse("bad_args")`. Machine would store an empty/garbage reviewer if dispatched. **Could have been a guard.**

**risk_clear early end — T (v2).** v2 `:82-83` from held_* with `edition == 'revised'`, **no** clock comparison; `then` `hold_active=0`, `hold_until_ms=0`, `hold_reason=""`, `hold_reviewer=$reviewer_id`. Matches early clear. Zero vs JSON null is host projection `desk.js:599-602`.

**dual_threshold — G.** v1 open `:43` `$dual_threshold == 10000`; v2 `:43` `$dual_threshold == 5000`. Host v2 open `desk.js:417` passes 5000 (would be refused if it passed 10000 on v2). Original desks keep v1. Not weaker.

### N17 r2 change (and r2 build = v1)

**payout_fee — G.** v1 `:79` / v2 `:87` save_payout `$fee_cents == amount_cents / 50`. Host `desk.mjs:250` / `:579-583`.

**hold_until — G.** v1 `:64` / v2 `:64` `$hold_until_ms == $now_ms + review_span_ms`. Host `desk.mjs:511-515`.

**hold_release_gate — G.** v1 `:71-72` / v2 `:71-72` `hold_active == 1 && $now_ms >= hold_until_ms`. Host `desk.mjs:533`. Pay: `hold_active == 0` (`:79` / v2 `:87`).

**dual_approval — G.** approve v1 `:55-57` / v2 `:55-57` same threshold split + `$approver_id != approver1`. Invariant `dual_bar` `:103-106` `approved && amount > dual_threshold ⇒ approval_count == 2`.

**approval_revision — T+G.** edit_amount v1 `:48-49` `revision = "+1"` then clear approvers/`approved_revision=0`. save_payout `:79` `approved_revision == revision`. Host does not re-check revision math beyond dispatch (`desk.mjs:466`).

**risk_clear nonempty reviewer — H.** v2 `:75-81` no nonempty/`!=` guard. Host `desk.mjs:545-546` `parseId`. **Expressible, not in machine.** Unlike r1, v2 risk_clear also has **no** `edition == 'revised'` (v2-only file; edition routing is host `desk.mjs:549`).

**risk_clear early end — T.** v2 `:79-80` no clock; clears `hold_active`, `hold_until_ms`, `hold_reason`, sets `hold_reviewer`. Host maps 0 → JSON null (`:556-558`).

**dual_threshold — G.** v1 `:41` `== 10000`; v2 `:41` `== 5000`. Host open `desk.mjs:426` passes 5000.

---

## Weaker-than-brief guards (summary)

1. **N16 r2 refund_amount:** `refunded_after <= captured_cents` instead of `amount <= captured - refunded` with `amount >= 0`.
2. **N16 r2 authorize:** no `$authorized_ms == $now_ms` (deadline still tied to `authorized_ms + window_ms`).
3. **N16 r2 void entry:** `remaining_hold > 0` only; r1 also requires `network == 'accepted'`.
4. **N17 r1/r2 risk_clear reviewer:** machine accepts any string; nonempty is host `isIdent`/`parseId` only.
5. **N17 r2 risk_clear edition:** not in the v2 `when` (host-only `legacy_request`).

No delivery uses a looser inequality where the brief requires equality for fee, dual_threshold, hold_until identity, or capture deadline identity (those equalities are present except N16 r2’s missing now=authorized_ms coupling).
