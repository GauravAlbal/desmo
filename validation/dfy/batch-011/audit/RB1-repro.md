# RB1-repro: lineage-2 A2 findings (N13, N14, N15)

Reproducer: ReproN13N15. Date: 2026-11-09.

Read-only against delivered trees under `dfy/runs-b11/{N13,N14,N15}` (copies in `/tmp/ReproN13N15-scratch`). Binary: `dfy/batch-011/ik-packed` (0.5.0). Node 22.17.1. Python 3.9 venv from each N14 project. Expected behavior taken only from each tree's `BRIEF.md` / `CHANGE.md`. Inspector reports `A2-N13.md`, `A2-N14.md`, `A2-N15.md` were hypotheses, not oracles.

Stubs (ephemeral ports this sitting):

- N13 ILL: `SERVICE_URL=http://127.0.0.1:60511` (control `60512`)
- N14 claims: `SERVICE_URL=http://127.0.0.1:60513` (control `60514`)
- N15 escrow: `SERVICE_URL=http://127.0.0.1:60515` (control `60516`)

Dead provider used where named: `SERVICE_URL=http://127.0.0.1:1`.

`ik show` / `ik replay` are JSON-line session ops (`protocol.major=1`, `op=show|replay`). Host files in every STATE_DIR used here were only the named sqlite store (no JSON overlay).

Classification key:

- **CONFIRMED defect** — reproduced a violation of a quoted BRIEF/CHANGE line, or a valid current caller of the delivered owning machine committed a transition that line forbids.
- **NOT REPRODUCED** — inspector claim not observed on this sitting.
- **NOT A CONTRACT VIOLATION** — observed (or source-true) but BRIEF/CHANGE does not forbid it. Guidance mismatches are noted under responsible layer.

Layers: **domain code** (agent-generated app/machine), **shipped guidance** (Ikaros docs/skills), **Ikaros product**, **oracle-contract gap** (inspector vs letter of BRIEF).

---

## Legitimate progress (still works)

### N13 r2/change and r1/change (new v2 request, not overdue)

STATE_DIR `/tmp/ReproN13N15-scratch/state/n13-r2c-legit` (r2) and `n13-r1c-legit` (r1). `NOW_MS=1000`. No `CRASH_AT`.

```
./run status
./run file ILL-1 PAT-9 OCLC-42
./run locate LEND-A
./run transmit
./run receive
./run checkout
./run checkin
```

r2 checkout: `{status:"out",checkout_ms:1000,loan_until_ms:604801000,overdue:0}` (7-day span). r2 checkin: `{status:"returned",overdue:0}`. r1 same checkout/checkin shape (`loan_req_id` is `LR-…` not `L.ILL-1`). Host files: `ill-desk.sqlite` only.

Overdue-with-recall also works (CHANGE C1/C3): `n13-r2c-recall-cli` / `n13-r1c-recall-cli`, checkout at 1000, then `NOW_MS=605801001`:

```
./run checkin          # {"error":"recall_required"} — status stays out
./run place-recall     # recall_placed:1, status out
./run checkin          # {status:"returned",overdue:1}
```

### N14 r2/change

STATE_DIR `n14-r2c-legit`, `NOW_MS=0`:

```
./run intake RX-1 PAT-1 NDC1 30 2 172800000   # edition v2
./run verify RX-1
./run fill RX-1 10                            # pickup_deadline_ms 86400000 (24h cap)
./run claim                                   # accepted, one provider effect
./run reconcile <claim_id>                    # lookup:accepted while stub up
./run claim                                   # success no-op
./run pickup                                  # phase verified, remaining_qty 20
```

### N15 r1/change and r2/change (promote happy path JSON)

STATE_DIR `n15-r1c-promote` / `n15-r2c-promote`:

```
NOW_MS=0    ./run open L1 S1 10000 2000 5000
NOW_MS=0    ./run bid L1 O1 B1 10000
NOW_MS=0    ./run bid L1 O2 B2 9000
NOW_MS=1000 ./run accept L1 O1     # revised deadline 6000, deposit_cents 200
NOW_MS=1100 ./run deposit L1       # under_contract / funded
NOW_MS=1100 ./run fallthrough L1   # fallen / released
NOW_MS=1100 ./run promote L1       # CLI: offered, backup null, deposit_id null
```

CLI promote/status match CHANGE C1 JSON. Persistence does not (findings 3 and 5).

---

## (1) N13 version-2 checkin / `recall_required` bypass via command interface

**Expected (CHANGE.md):** “If `NOW_MS > loan_until_ms` and `recall_placed` is not `1`, `checkin` returns `recall_required` and changes nothing.” C3: “On a new request, `checkin` when overdue without a recall is `recall_required` and leaves `status:"out"`."

v2 machines (`N13/r1/change/machines/ill_desk_v2.ik.toml` 71–73; `N13/r2/change/desk_v2.ik.toml` 72–74): `from=["out"]` `to="returned"` with no `when`. Host enforces the rule (`desk.js` 364–367; `desk.mjs` 528–530).

**Sequence (r2/change)** — STATE_DIR `.../state/n13-r2c-bypass`, `NOW_MS=1000` then `605801001`:

```
./run file ILL-1 PAT-9 OCLC-42
./run locate LEND-A
./run transmit
./run receive
./run checkout
NOW_MS=605801001 ./run checkin    # {"error":"recall_required"}
# then runtime current caller, not ./run:
# Kernel(store).machine(desk_v2.ik.toml).show("desk").execute({name:"checkin", args:{}})
NOW_MS=605801001 ./run status
```

**Actual:** CLI checkin → `{"error":"recall_required"}`, status still `out`, `recall_placed:0`. Direct `execute`: `DIRECT_CHECKIN_OK` named `returned`, facts `recall_placed:0`. Subsequent `./run status`: `status:"returned"`, `recall_placed:0`.

`ik --store ill-desk.sqlite --machine desk_v2.ik.toml --session` `op=replay`: `state:"returned"`, `recall_placed:0`. Journal seq 8 is `checkin` with empty args. Aggregate `terminal:1`. Host files: `ill-desk.sqlite` only.

**r1/change** — same CLI refuse, then `agg.command("checkin", {})` → `returned`, `recall_placed:0`. Same `./run status` lie.

**Layer:** domain code (host-only law). Not Ikaros product: the machine accepted a command the definition allows.
**Severity:** high.
**Classification:** **CONFIRMED defect.** CHANGE C3 is false of the delivered v2 command. Honest `./run checkin` still refuses (legitimate path above).

---

## (2) N13 r1/change `file` HistoricalResult → undefined `machine.show`

**Expected (BRIEF.md):** “A second file is `already_filed`.” Ikaros `createWith` + same `operationId` should return history, not throw. `src/desk.js:291`: `created instanceof HistoricalResult ? await machine.show(AGG)` but `openMachines` returns `{kernel, v1, v2}` — `machine` is unbound. r1/build binds `machine` from `openMachine` and does not have this line.

**Sequence A (honest `./run`, new operation id each process)** — STATE_DIR `n13-r1c-file-twice`:

```
NOW_MS=1000 ./run file ILL-1 PAT-9 OCLC-42   # filed
NOW_MS=1000 ./run file ILL-1 PAT-9 OCLC-42   # {"error":"already_filed"}
```

**Sequence B (exact-repeat createWith, same `operationId=op-repeat-file`)** against a fresh store, using `@ikaros/runtime` from the r1/change tree:

1. `v2.createWith({aggregateId:"desk", command:{name:"file", args:{req_id:ILL-1,…}}, operationId:"op-repeat-file"})` → filed, not HistoricalResult.
2. Repeat identical `createWith` → `HistoricalResult`.
3. `await machine.show("desk")` with unbound `machine` → `TypeError: Cannot read properties of undefined (reading 'show')`.

**Sequence C (CLI path with `Kernel.prototype.newOperationId` pinned to `fixed-op-id`)** — `runCommand` twice: first `{status:"filed"}`; second throws `already_filed` (the `file` catch at 293–296 sees the bound desk after the TypeError and maps it). Inspector's “CLI maps that to `bad_args`” is **not** what `runCommand` does; `run` would map an *uncaught* TypeError to `bad_args`, but `file`'s catch converts it.

**Layer:** domain code.
**Severity:** high on the operation-id recovery path; honest second `file` is BRIEF-correct.
**Classification:** **CONFIRMED defect** for the exact-repeat HistoricalResult line (undefined binding). **NOT REPRODUCED** as `./run` → `bad_args`. Lost-create retry via same operation id therefore cannot return the historical filed object; depending on catch vs `run`, the caller sees `already_filed` or `bad_args` instead of success-from-history. BRIEF does not require operation-id replay (“one process at a time”); that part is shipped-guidance (`docs/client-recovery.md`), not a BRIEF miss.

---

## (3) N15 r1/change promote JSON cleared; store facts drive reconcile → `corrupt_state`

**Expected (CHANGE.md C1):** promote “moves that backup to primary, clears deposit fields, returns `phase:"offered"`” and “clears backup, deposit id, deposit cents, accepted_ms, and deposit_deadline_ms”. BRIEF reconcile: “No saved deposit is `no_deposit`”.

v2 `promote` then (`desk_v2.ik.toml` 139–144) sets primary from backup, `has_backup=0`, `escrow_code=0`. It does not clear `deposit_id` / `deposit_cents` / `accepted_ms` / `deposit_deadline_ms` / `deposit_offer_id` / `deposit_buyer_id`. Host `cmdPromote` hardcodes `deposit_id:null`; `depositFieldsForStatus` nulls those fields whenever public phase is `offered`.

**Sequence** — STATE_DIR `n15-r1c-promote`:

```
NOW_MS=0    ./run open L1 S1 10000 2000 5000
NOW_MS=0    ./run bid L1 O1 B1 10000
NOW_MS=0    ./run bid L1 O2 B2 9000
NOW_MS=1000 ./run accept L1 O1
NOW_MS=1100 ./run deposit L1          # deposit_id df21107d899acc21cbd7891109a26656e
NOW_MS=1100 ./run fallthrough L1
NOW_MS=1100 ./run promote L1          # CLI deposit_id null
NOW_MS=1100 ./run status              # deposit_id/accepted_ms/deadline null
NOW_MS=1100 ./run reconcile L1 df21107d899acc21cbd7891109a26656e
```

**Actual CLI:** promote/status `{deposit_id:null,deposit_cents:null,accepted_ms:null,deposit_deadline_ms:null,phase:"offered",edition:"revised"}`.

**`ik show` / sqlite `aggregates.state` after promote:** `named: offered` with `deposit_id: df21107d899acc21cbd7891109a26656e`, `deposit_cents:200`, `accepted_ms:1000`, `deposit_deadline_ms:6000`, `deposit_offer_id:O1`, `deposit_buyer_id:B1`, `escrow_code:0`, `primary_offer_id:O2`, `has_backup:0` (backup_offer_id string O2 remains). Journal seq 11 is `promote`.

**Reconcile:** `{"error":"corrupt_state"}`. Status after still `offered` / CLI-null deposit. Host files: `desk.sqlite` only.

**Layer:** domain code (machine `then` + host view). Reconcile reads store facts, not the JSON overlay.
**Severity:** high.
**Classification:** **CONFIRMED defect** of CHANGE C1 (clears deposit fields) and of BRIEF reconcile (`no_deposit` when none saved).

---

## (4) N14 r2 reconcile requires a live provider after the claim is terminal

**Expected (BRIEF.md reconcile):** “Lookup 200 with matching ids … commits accepted state and returns the accepted success object plus `lookup:"accepted"`; … transport failure is `service_unavailable`.” Repeat `claim` after accepted/cash is “a success no-op without a provider call.” r1/change skips GET when `claim in (accepted, rejected, cash)`.

r2 `do_reconcile` always GET (`desk.py` 682–686).

**Sequence** — STATE_DIR `n14-r2c-dead-recon`, `NOW_MS=0`:

```
./run intake RX-1 PAT-1 NDC1 30 2 86400000
./run verify RX-1
./run fill RX-1 10
./run claim                          # accepted, claim_id 8a26e053d1861f854080c4c2fd9dca8d
SERVICE_URL=http://127.0.0.1:1 ./run reconcile 8a26e053d1861f854080c4c2fd9dca8d
SERVICE_URL=http://127.0.0.1:1 ./run claim
```

**Actual:** reconcile → `{"error":"service_unavailable"}`. Repeat claim → accepted success no-op. `ik show` still `claim:"accepted"`, `state:"held"`. Same for cash: STATE_DIR `n14-r2c-cash-dead`, cash-pay id `d876ae74cceb0a9efbcc5fedbf8ceea2`, dead reconcile → `service_unavailable`, dead `claim` no-op.

**r1/change control** (`n14-r1c-dead-recon`): dead reconcile after accepted → exit 0 `{lookup:"accepted"}`.

**Layer:** domain code vs inspector; **oracle-contract gap** vs BRIEF letter (transport failure *is* `service_unavailable`; reconcile is specified as a lookup, not as a local no-op).
**Severity:** medium as product quality (unknown-outcome mishandle of a locally terminal fact); not high vs BRIEF letter.
**Classification:** **NOT A CONTRACT VIOLATION** of the quoted reconcile sentence. Observation **confirmed**. Repeat `claim` is the BRIEF no-op and still works.

---

## (5) Effect-outcome binding inaccuracies

### 5a. N13 r1 outcome digest = payload digest

**Expected:** BRIEF has no digest text. Shipped `KIT/docs/typescript.md`: record the HTTP body digest; a later different `outcome_digest` is `EffectConflictError`. r1 `acceptLoan` records `cur.pendingIntent.payloadDigest` (`desk.js` 238–243).

**Sequence** — STATE_DIR `n13-r1c-digest`:

```
NOW_MS=1000 ./run file ILL-1 PAT-9 OCLC-42
NOW_MS=1000 ./run locate LEND-A
NOW_MS=1000 ./run transmit    # loan_req_id LR-9fd987e77f23b9c3cd5758d1
```

**Actual store:** `aggregates.effect_json` and receipt seq 4 `type:record` have

- `payload_digest`: `a4c2fc6a8ccf2a31c1b019e41301f07f0647af943197a69ca8d661ef2c7d5742`
- `outcome_digest`: **same** `a4c2fc6a…`

That hex is `sha256(JSON.stringify({loan_req_id,req_id,lender_id,oclc}))`. Stub body adds `outcome:"accepted"`; `sha256` of that object is `9d614257978c3eea992f8f343091a136fda5acec5c2fd2233f439cdf18c73673` (different). Transmit succeeded; recovery that later recorded the real body digest would conflict.

r2/change on the bypass store recorded **distinct** payload `bb6588bb…` vs outcome `9fa3556b…` (body digest). r1-only bug.

**Layer:** domain code vs shipped guidance. BRIEF silent.
**Severity:** medium.
**Classification:** **NOT A CONTRACT VIOLATION** of BRIEF. **CONFIRMED** inaccuracy vs Ikaros record-outcome guidance. Binding defect in the delivered r1 writer.

### 5b. N15 r2 leftover `deposit_id` hidden by `has_deposit=0`

**Expected (CHANGE C1):** promote “clears … deposit id”. r2 `listing_v2.ik.toml` 108–113 zeros `accepted_ms`, `deposit_deadline_ms`, `deposit_cents`, `has_deposit`, `has_backup` but does not clear `deposit_id` (or `backup_offer_id`). Host `liveDepositId` / `cmdReconcile` key off `has_deposit` (`desk.mjs` 169–175, 606).

**Sequence** — STATE_DIR `n15-r2c-promote`, same open/bid/accept/deposit/fallthrough/promote as (3). deposit_id `da39cb9932202d03a72b07b47`.

**Actual CLI status:** `deposit_id: null`. **`ik show`:** `has_deposit:0`, `deposit_cents:0`, `accepted_ms:0`, `deposit_deadline_ms:0`, **`deposit_id` still `da39cb9932202d03a72b07b47`**, `hold_key` same, `backup_offer_id:O2`. Reconcile of that id: `{"error":"no_deposit"}` (not `corrupt_state`).

**Layer:** domain code.
**Severity:** medium (shadow fact; reconcile does not go corrupt).
**Classification:** **CONFIRMED defect** of CHANGE C1 “clears deposit id” in the runtime store. CLI JSON is a second view.

---

## (6) Every other A2 finding

### N13

| ID | Replica | Classification | Sev | Layer | Evidence |
|---|---|---|---|---|---|
| F3 `isLegacy` uses `owner.version===1` not `identity.equals` | r1/change | NOT A CONTRACT VIOLATION of CHANGE old-instance policy on this two-file tree (commands still hit owner via `owningMachine`). Weaker than `docs/machine-evolution.md`. | medium | domain + shipped guidance | `desk.js` 116–118. Not exercised with a colliding extra definition. |
| F4 no `lookupOperation`; r2 binds no operation ids | all | NOT A CONTRACT VIOLATION (BRIEF: one process; HTTP GET recovery). Miss vs `docs/operation-identity.md`. | medium vs guidance | domain + shipped guidance | grep: no `lookupOperation` in N13/N14/N15 writers. r1 `createWith` uses `kernel.newOperationId()` only on `file`. |
| F6 unknown command on empty desk is `bad_status` | r2 build+change | **CONFIRMED defect** | medium | domain | BRIEF: “An unknown command is `bad_args`.” Empty STATE_DIR `n13-r2c-empty` / `n13-r2b-empty`: `./run foobar` → `{"error":"bad_status"}`. r1 empty: `bad_args`. After a bound desk (`n13-r2c-legit`): r2 `foobar` → `bad_args`. |
| F7 broad `catch` / callback swallow | r2 | callback well-formed on `out`: `{ok:true}`, status unchanged (**BRIEF R6 holds**). Swallow of `corrupt` not injected. | medium | domain | `desk.mjs` 406–410 catch empty. |
| F8 `reconcile` after `returned` → `bad_args` | r2/change | **CONFIRMED defect** vs named reconcile codes | medium | domain | `n13-r2c-reconcile-after-returned`: after place-recall+checkin, `./run reconcile L.ILL-1` → `{"error":"bad_args"}`. BRIEF reconcile list is `no_loan` / `loan_mismatch` / `lookup_conflict` / `service_unavailable` / success. |
| F9 7 vs 14 day span is host-computed | both change | NOT A CONTRACT VIOLATION of CHANGE C4 as observed through `./run` (new requests got `604801000`). Machine `checkout` args are unconstrained ints. Same host-authority pattern as (1). | medium | domain | r2 `desk.mjs` 507–511 `loanSpan(legacy)`. |
| F10 `L.${req_id}` can exceed 64 | r2 | **CONFIRMED defect** | low | domain | BRIEF identifiers 1–64. `req_id` 63 × `R`+`x`*62; transmit succeeded with `loan_req_id` length 65 (`L.Rxxx…`). Stub accepted it. |
| F11 corrupt mapped by message text | r1 | source-confirmed; wrong code **NOT REPRODUCED** on a real corrupt file this sitting | low | domain vs `skills/ikaros/build.md` step 8 | `run` / `main.js`: `msg.includes("corrupt")`. |
| F12 `transmit_noop` unused | r1 | NOT A CONTRACT VIOLATION (repeat transmit after accepted is a success no-op). | low | domain | r1 `n13-r1c-repeat-tx`: second transmit success, no extra POST required by BRIEF R5. |
| F13 no change handoff; README still v1 | both change | NOT A CONTRACT VIOLATION of BRIEF/CHANGE (CHANGE is the request, not a shipped handoff). | low | domain docs vs `skills/ikaros/change.md` | r1 README: authority `machines/ill_desk.ik.toml` only; no `place-recall`. |
| F14 first-POST snapshot mismatch → `service_unavailable` not `lookup_conflict` | r2 | **NOT REPRODUCED** against the shipped stub (it always echoes matching fields). Source: `firstTransmit` 308–310: non-200-match except 409 is `service_unavailable`. GET mismatch **is** `lookup_conflict` (326). | low | domain | Would need a custom provider body. |

r2 `ack_transmit` on repeat transmit (`n13-r2c-repeat-tx` second transmit succeeded). Whether that journaled a mutation was not separately dumped; not filed as a BRIEF miss (success no-op JSON).

### N14

| ID | Replica | Classification | Sev | Layer | Evidence |
|---|---|---|---|---|---|
| F1 host remaining/deadline/rush/restore | all | NOT A CONTRACT VIOLATION of BRIEF through honest `./run` (v2 fill cap `86400000` then pickup at `86400001` → `pickup_lapsed`, remaining restored to 30). Amounts are host args the machine writes. Same pattern as N13 (1). | high vs guidance | domain + brownfield guidance | `n14-r2c-v2-hold-cap`. |
| F2 r1 `already_bound` host-only | r1 | NOT A CONTRACT VIOLATION of BRIEF (second intake is `already_bound`: `n14-r1c-already-bound`). Machine `bind` `from=["intake"]` would still allow a second bind if the host check were removed. | medium | domain | toml + `rx_desk.py` 296–297. |
| F3 r1 `status` hides `pickup_deadline_ms==0` | r1/change | **CONFIRMED defect** | low | domain | BRIEF status should report the deadline. `NOW_MS=0` `hold_ms=0` fill returns `pickup_deadline_ms:0`; `status` returns `null`. |
| F4 record then mark are two commits | r1+r2 | NOT REPRODUCED as a mixed store (no unlabeled kill between the pair). Sequential two calls, no `transaction()`. | medium | domain | source only this sitting. |
| F5 r2 `CRASH_AT=after-claim-saved-before-request` is after GET | r2 | **CONFIRMED** vs BRIEF letter (“before sending a request”) | low | domain | `n14-r2c-crash`: claim exit -9, status `claim:"saved"`, `/posts` count unchanged (no new POST). Source GET-then-crash-then-POST. |
| r1 cash reconcile `lookup:"accepted"` | r1 | NOT A CONTRACT VIOLATION of mutation; misleading label (BRIEF lookup names are accepted/rejected/not_sent). | low | domain | `n14-r1c-cash-lookup`: live and dead reconcile both `{lookup:"accepted"}` while `claim:"cash"`. |
| r1 corrupt + missing NOW_MS → `bad_time` | r1 | **CONFIRMED defect** | medium | domain | BRIEF precedence row 1: `corrupt_state` first. `n14-r1c-corrupt-precedence`: garbage `ikaros.sqlite`, no `NOW_MS` → `bad_time`; with `NOW_MS=0` → `corrupt_state`. r2 same garbage, no NOW → `corrupt_state` (holds). |
| fill from `lapsed` → `already_moved` | r2 | NOT A CONTRACT VIOLATION (BRIEF does not name fill-from-lapsed; `already_moved` is a verify code). | low | domain | After accepted pickup-at-86400001: phase `lapsed`, remaining 20; `fill RX-1 5` → `already_moved`. |
| r2 cash+`rejected` adopt | r2 | NOT REPRODUCED (shipped stub never rejects `payor:cash`). Source would `adopt_rejected`. | — | domain | latent. |
| r2 v2 `rush` no `when` | r2/change | same host-authority pattern as F1; honest CLI not broken this sitting | high vs guidance | domain | `rx_desk_v2.ik.toml` 97–101. |

### N15

| ID | Replica | Classification | Sev | Layer | Evidence |
|---|---|---|---|---|---|
| F4/F7 host deadline/`deposit_cents`; r2 v1 will store a revised deadline | all / r2 C4 | C4 **holds** on honest `./run`: build-program open/bid then change-program accept at 1000 records `deposit_deadline_ms:2000`; deposit at 3000 is `deposit_elapsed` (`n15-r2-orig-deadline-C4`, `n15-r1-orig-deadline-C4`). Machine does not encode original `accept_by_ms` on r2 v1. | high (r2 sufficiency) / medium | domain | CLI contract holds; host `if` is the only C4 wall. |
| F5 no `lookupOperation` | all | NOT A CONTRACT VIOLATION of BRIEF HTTP recovery | low/medium vs guidance | domain + shipped guidance | grep empty. |
| F6 r2 unexpected kernel refuse → `desk_full` / `no_backup` | r2 | NOT REPRODUCED (no mismatch injected). Source `cmdBid`/`cmdPromote` catch. | low | domain | `desk.mjs` ~395 / 713–715. |
| F8 r1 edition from `owner.version` | r1/change | NOT A CONTRACT VIOLATION on this two-definition tree. Command routing uses `identity.equals`. | low | domain | `editionOf` in `cli.mjs`. |
| F9 no handoff; README names only v1 | both change | NOT A CONTRACT VIOLATION of BRIEF/CHANGE | medium vs change.md skill | domain docs | r2 README: `listing.ik.toml` only, no `promote`. |
| F10 original-instance tests are `/tmp` fixtures | r1+r2 change tests | CONFIRMED as test design from source; fixture currency **not re-run** (would mutate shared `/tmp`, out of project). | high as tests | domain tests | `test/desk.test.mjs` r1: `/tmp/desk-orig-N15r1/{accepted,fallen,mid}`; r2: `/tmp/desk-continuity-r2`. |
| F11 tautology | r2 test | CONFIRMED hollow test (source) | low | domain tests | `assert.ok(posts.count === 0 \|\| posts.count >= 0)`. |
| F12 corpora omit `fund` / `mark_fallen` | r2 | CONFIRMED from source (grep no matches in cases json) | low | domain | hollow vs `ik check`. |
| F13 r1 `callback` on empty store is success | r1 build+change | **CONFIRMED defect** | medium | domain | BRIEF refusal 8: no bound listing except `open` and `status` → `not_open`. `n15-r1c-callback-empty` / `n15-r1b-callback-empty`: well-formed callback → null status object exit 0. r2: `not_open`. |
| F14 r2 `ignore_callback` journaled no-op | r2 | NOT A CONTRACT VIOLATION (BRIEF allows ignore). | low | domain | not separately dumped this sitting. |

N15 r1 `accept` on a revised listing recorded `deposit_deadline_ms:6000` (CHANGE worked clock). r1 v1 original still `2000`.

---

## Method notes

- Copies under `/tmp/ReproN13N15-scratch/{N13,N14,N15}-r*-{build,change}` with `node_modules` / `.venv` / `KIT` symlinked. Delivered trees not written.
- Direct machine callers used `@ikaros/runtime` from the same replica (`show`/`command`/`execute`/`createWith`).
- `ik` one-shot `show <id>` is invalid (`unexpected argument`); session JSON with `protocol`+`op` works. `replay` without stdin blocks (waits for a request line).
- Did not read evaluator, controls, authority, diagnose, sealed, or oracle material. Did not run in-project unittest modules (would write `__pycache__` / mutate `/tmp` fixtures).
- Stubs killed with the driver process; ports above are this sitting only.

## Index (named assignment items)

| # | Finding | Class |
|---|---|---|
| 1 | N13 v2 checkin host-only `recall_required`; runtime command bypass | CONFIRMED defect |
| 2 | N13 r1 `file` HistoricalResult undefined `machine` | CONFIRMED (TypeError path); NOT REPRODUCED as `./run`→`bad_args` |
| 3 | N15 r1 promote CLI-null vs leftover facts → reconcile `corrupt_state` | CONFIRMED defect |
| 4 | N14 r2 reconcile needs live provider when claim terminal | observation confirmed; NOT A CONTRACT VIOLATION of BRIEF transport-failure text |
| 5a | N13 r1 outcome_digest == payload_digest | CONFIRMED vs Ikaros guidance; NOT A CONTRACT VIOLATION of BRIEF |
| 5b | N15 r2 leftover `deposit_id` with `has_deposit=0` | CONFIRMED defect of CHANGE C1 |
