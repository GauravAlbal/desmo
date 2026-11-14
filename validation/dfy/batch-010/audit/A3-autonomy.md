# A3 autonomy and human-question audit (batch-009)

Inspected 30 parked batch-009 agents (builds `U9N7r1`–`U9N9r2`, changes `V9N7r1`–`V9N9r2`, diagnoses `W9N7r1A`–`W9N9r2C`), their `agent://` yields, `history://` summaries, session jsonl under `~/.omp/agent/sessions/.../U9|V9|W9*.jsonl`, the briefs/CHANGE/diagnose prompts those runs used, and Ikaros guidance.

Guidance the agents actually executed is the copy in each project `.agents/skills/ikaros/`. Current `invariant-kernel/skills/ikaros/SKILL.md`, `change.md`, and `diagnose.md` match that copy. `build.md` does not: repo HEAD adds a stricter fit step (one aggregate; no host saga/sidecar; if no human can answer, deliver only what one aggregate can honor). Batch-009 builders did not see that sentence.

Classification of every human-facing question / uncertainty / “decision needed”:

- **(a)** genuine business-behavior or permission decision the brief left undefined
- **(b)** engineering burden the guidance or tools should have let the agent resolve
- **(c)** not a question (note): an assumption they already made, a difficulty, or an unrun optional checker

PROMPT.md required them to *report* unsure requirements and at most one genuine behavior/permission decision. Listing an assumption after acting on it is (c), not a question. No agent used an `ask` tool. None stopped mid-run waiting for a human answer.

## Counts

| class | count |
|---|---|
| (a) questions | **1** |
| (b) questions | **0** |
| (c) notes | **33** |
| agents with decision=none / undecided=none | **29 / 30** |
| agents that blocked on a human | **0** |

The single (a) is `V9N9r1` (N9 change, r1), asked after the change was implemented.

## Builds (`U9*`)

All six yielded a delivery report with `decision` / `decision_needed` / `Remaining decision` = none. Unsure lines are post-hoc notes of defaults they already shipped.

### U9N7r1 — warehouse (Node)

Yield `decision_needed: none`. Transcript `history://U9N7r1` final prose same.

| item | class | why |
|---|---|---|
| expire on already-cancelled treated as success returning `cancelled` | (c) | BRIEF.md cancel is idempotent for cancelled/expired; expire-on-cancelled is unspecified. They chose cancel-like idempotency and did not ask. |
| parcel ids exclusive globally, not per facility | (c) | BRIEF says `parcel_taken` if held by another live order; they decided global exclusivity. |

Handoff vs transcript: TAP `# pass 11`, `ik check` ACCEPT, `all expectations met` on scenarios. Prose claimed crash-label + lost-reply tests, `ik check` ACCEPT, scenarios `order-book` and `order-stale`. Structured yield omitted Exercised/Not run; prose filled it. First `yield()` was empty, then an idle reminder, then a second yield with the report. Work was not left unfinished; they did not wait on a human.

### U9N7r2 — warehouse (Node)

Yield `decision: none`.

| item | class | why |
|---|---|---|
| expire/cancel of already-cancelled succeeds returning current object | (c) | same unspecified corner as r1; they implemented and noted it |
| `NOW_MS` required on order/pack/book/cancel/expire/status (`bad_args` if missing) | (c) | BRIEF requires `NOW_MS` on clock-sensitive commands; missing-env code is unspecified. They picked `bad_args`. |

Handoff claimed 10/10 `node:test`, `ik check` ACCEPT, scenario `book.json`. Tool results show TAP `# pass 10`, ACCEPT, scenario expectations met. `not_run`: TLC/Hegel (optional; listed). Matches.

### U9N8r1 — subscription (Node)

Yield `Decision: none`. Unsure string, split:

| item | class | why |
|---|---|---|
| missing open → `not_open` | (c) | BRIEF does not name that code |
| missing env → `bad_args` | (c) | R14 says missing `SERVICE_URL`/`NOW_MS` is a refusal; code unspecified |
| cancelled/expired renew → `cancelled`/`expired` | (c) | R7 says refused; codes unspecified |
| paid renew always no-op vs new cycle | (c) | BRIEF already says repeating renew after paid is a success no-op. They implemented that and still listed doubt. Not a question. |
| GET after grace when a charge id is saved | (c) | BRIEF already requires GET first when an attempt exists. They implemented GET then adopt-or-expire. |
| callback success JSON `{ignored:true}` | (c) | BRIEF: well-formed callback may be ignored |

Handoff claimed 6/6 tests, `ik check` ACCEPT, scenario `renew-paid`. TAP `# pass 6`, ACCEPT, scenario text present. Last source edit (`src/main.mjs`) is immediately before yield; jsonl bash argv is truncated with `…`, so a post-test edit cannot be proven or disproven from the stored command tail. `NotRun`: TLC/Hegel. Matches what was listed.

### U9N8r2 — subscription (Node)

Yield `decision: none`. Six notes, all (c): ignored well-formed callback; `retry_exhausted` when attempts already at limit; crash before first POST retries the saved id; next-cycle id only after `paid`; cancel after saved/paid is `charge_started`; missing `SERVICE_URL`/`STATE_DIR` → `bad_args`. Several of those are restatements of BRIEF R4/R7/R8, not open questions.

Handoff claimed 9 tests, `ik check` accept, scenarios `open-cancel`, `charge-adopt`, `stale-handle`. TAP `# pass 9`, ACCEPT, scenario text. Matches.

### U9N9r1 — e-sign (Python)

Yield `Remaining decision: none`.

| item | class | why |
|---|---|---|
| exact success JSON for send/callback | (c) | BRIEF specifies create/sign/decline/archive/status/reconcile shapes and allows extra fields; they noted send/callback extras |
| shared namespace for envelope_id vs document_id | (c) | R6 says ids are namespace-safe / `id_in_use`; they treated one pool |

Handoff claimed 11 process tests, `ik check` ACCEPT, scenario `useful.json`. Python test run is in the transcript (`tests/test_desk.py`); ACCEPT and “all expectations met” appear in tool results. `Not run`: TLC/Hegel, `ik analysis`, `ik handoff` CLI — matches (those commands are absent). A `desk.py` write sits ~300ms after the last test invocation in jsonl timestamps; that is a possible skipped retest, not a human question.

### U9N9r2 — e-sign (Python)

Yield `decision: none`.

| item | class | why |
|---|---|---|
| exact success JSON extras for send/reconcile-already-sent/callback | (c) | extras allowed; they noted them |
| kernel state `attempting` mapped to draft + `attempt_saved` | (c) | engineering projection, not a requester decision |

Handoff claimed `tests/test_desk.py` passed, crash/lost-reply/unrelated bytes. Transcript ran `.venv/bin/python tests/test_desk.py` (ok) and `ik check`. Structured yield did not list `ik check`/scenario; those ran. No (a)/(b).

## Changes (`V9*`)

PROMPT: existing stores need not be migrated unless CHANGE says otherwise; report what happens to them. `change.md` (shipped): ask only when intent changes ambiguously or exceeds delegated authority; keep prior definitions for persisted instances.

### V9N7r1 — warehouse multi-parcel

Yield `decision: none`. Notes (c): duplicate parcel ids → `bad_id`; idempotent recreate compares sorted lists while storage keeps caller order; v1 packed cancel/unpack not migrated. CHANGE.md: existing one-parcel orders keep original behavior; packed cancel/unpack are the new rules. They followed that and noted it.

Handoff: 14+1 node tests, continuity packed v1 book, `ik check` v2 `--prior` v1 ACCEPT, scenarios. Transcript: full `node --test test/desk.test.mjs` (97-line TAP) then a one-test `--test-name-pattern` run (TAP `# pass 1`). Continuity store commands present. `ik evidence verify` / `ik handoff` not in the command trace and not listed as Not run. Tests/continuity claims match; evidence-verify was skipped silently.

### V9N7r2 — warehouse multi-parcel

Yield `decision: none`.

| item | class | why |
|---|---|---|
| whether packed cancel/unpack should apply to pre-change v1 packed orders | (c) | CHANGE says keep original behavior; they kept v1 frozen and reported that. Not asked. |

Handoff claimed `node --test tests/desk.test.mjs` 11 pass, `ik check` `--prior`, scenarios, continuity `/tmp/wh-desk-continuity-N7r2`. Eval argv is stored as `{}` in jsonl, but tool results contain TAP `# pass 11`, ACCEPT, scenario text, and continuity status/book JSON for `o-old`. Handoff matches those outputs. `not_run`: TLC/Hegel. `ik evidence verify` not run and not listed.

### V9N8r1 — subscription downgrade

Yield `decision: none`. Notes (c): successful downgrade JSON fields; upgrade/wrong-id refused; `charge_started` if an attempt is saved. CHANGE specifies the command and “upgrade or invalid subscription/status is refused” without codes.

Handoff: 10/10 tests, continuity, `ik check` `--prior`, scenarios `renew-paid` and `downgrade-then-pay`. TAP `# pass 10`. Honest `not_run`: TLC/Hegel, `ik evidence verify`, `ik handoff`. Matches.

### V9N8r2 — subscription downgrade

Yield `decision: none`. Notes (c): upgrade → `bad_plan`; wrong id → `bad_args`; `credit_cents` and `proration_cents` are the same stored credit; TLC/Hegel not run (also a Not-run).

Handoff: 12 tests, `ik check` `--prior`, four scenarios. TAP `# pass 12`, scenario run in bash. Matches. Evidence verify not run, not listed.

### V9N9r1 — e-sign two-signer — only (a) in the batch

Yield `decision`: “Should extend-deadline work on pre-change v1 envelopes (C4), or is refusing those acceptable so v1 stays frozen?”

Transcript `history://V9N9r1` L137–139, same question after the work was done:

> Should `extend-deadline` work on **pre-change v1** envelopes (C4), or is refusing those acceptable so we never mutate the frozen v1 machine?

| item | class | why |
|---|---|---|
| **extend-deadline on pre-change v1 envelopes** | **(a)** | CHANGE C4: “Existing one-signer envelopes may use it, but their original expiry rule remains in force.” Ikaros `change.md`: keep the previous definition for persisted aggregates; do not rewrite stored spec hashes. That is a user-visible behavior fork the brief left in tension with identity freeze. They implemented refuse-on-v1 (`invalid_state`) and asked. Not a checker/corpus/schema question (those are banned). Not (b): the tools cannot decide whether old envelopes get the new command. |
| duplicate addresses allowed; only duplicate signer ids are `bad_args` | (c) | CHANGE: malformed or duplicate signer is `bad_args`; they decided addresses may collide |
| grace stored as `decision_until_ms` at create/extend | (c) | implementation mapping |

They did not stop for the answer. Tests: unittest 14 OK; continuity; `ik check` `--prior`; `ik evidence verify` (this is the only change agent whose transcript shows that command). Handoff matches. Unrun specialists not named in the structured yield (prose also omitted TLC).

### V9N9r2 — e-sign two-signer

Yield `remaining_decision: none`. Unresolved (c): pre-change v1 stores cannot use `extend-deadline` without changing the frozen spec; new six-arg creates are v2 and can extend. Same C4 tension as r1; they resolved it as no-migration and did not ask.

Handoff: `tests/test_desk.py`; `ik check` `--prior`; scenarios `happy.json` and `dual.json`; continuity status/send/sign. Transcript shows those commands (python tests, scenario `happy.json`/`dual.json`). `Not run`: TLC/Hegel. Evidence verify not run, not listed. Two yields after an idle reminder; work was already done.

## Diagnoses (`W9*`)

Mode A = investigate-only, B = authorized-recovery of a lost reply, C = authorized-recovery control (no incident). Case PROMPT: facts from application commands + provider public lookup only; write `diagnosis.json`; report classification, provider commit, evidence, local-record action, anything undecided.

All 18 yielded, all wrote `diagnosis.json`, all `undecided`/`undecidable` = none except `W9N8r1C`.

| agent | classification (yield) | undecided | class of that item |
|---|---|---|---|
| W9N7r1A | committed_reply_lost | none | — |
| W9N7r1B | committed_reply_lost; ran reconcile | none | — |
| W9N7r1C | no_incident (cancelled, no request_id) | none | — |
| W9N7r2A | committed_reply_lost | none | — |
| W9N7r2B | committed_reply_lost; ran reconcile | none | — |
| W9N7r2C | no_incident; reconcile `not_applicable` | none | — |
| W9N8r1A | committed_reply_lost | none | — |
| W9N8r1B | committed_reply_lost; ran reconcile | none | — |
| **W9N8r1C** | no_incident (`not_due`) | operator report did not name `NOW_MS`; used `NOW_MS=0` matching `not_due` | **(c)** they derived the clock from the symptom and did not ask |
| W9N8r2A | committed_reply_lost | none | — |
| W9N8r2B | committed_reply_lost; ran reconcile | none | — |
| W9N8r2C | no_incident; no charge_id so no GET | none | — |
| W9N9r1A | committed_reply_lost | none | — |
| W9N9r1B | committed_reply_lost; ran reconcile | none | — |
| W9N9r1C | no_incident (archived; send `invalid_state`) | none | — |
| W9N9r2A | committed_reply_lost; claimed did not reconcile | none | — |
| W9N9r2B | committed_reply_lost; ran reconcile | none | — |
| W9N9r2C | no_incident (archived) | none | — |

`W9N8r1C`’s operator-report.md names `STATE_DIR` and `SERVICE_URL` only. Symptom is `not_due` with payment none. Using `NOW_MS=0` is an assumption they recorded, not a request for a clock from the human.

Handoff vs transcript (diagnose):

- Investigate-only A agents: status + public GET, no `./run reconcile` in bash. `W9N9r2A` grepped BRIEF for “reconcil” (false-positive if scanning the word); the actual run is `./run status` + `curl` GET. Yield “changed nothing / did not reconcile” matches.
- Recovery B agents: status, GET, then `./run reconcile`. Matches “local_record: ran reconcile”.
- Control C: N7 cancelled / N8 `not_due` / N9 archived + refused send. N7/N8 correctly skipped provider GET when no saved id (`W9N8r2C` says so). N9 C looked up the original send (allowed; archived envelope still has a request id).
- None called kernel `lookup_operation` / `ik show`. Case PROMPT forbids private state and tells them to use app commands + public lookup. That is following the assignment, not a skipped required step of *this* brief. Shipped `diagnose.md` still names `lookup_operation`; they preferred the case prompt.
- None ran `ik scenario run` / specialist analysis. Case prompt does not require them. Skill says list unrun claims; diagnose yields mostly omit a Not-run line.
- None asked the human to pick a checker (banned by SKILL.md).

No diagnose agent stopped early. All reported a classification and wrote the case file.

## Guidance vs what they asked

Shipped SKILL.md: “Ask the human at most one consequential behavior or permission question. Never ask them to choose a checker, write a corpus, inspect a payload schema, or find writers.”

Observed:

- Zero checker / corpus / schema / writer questions. **(b) = 0.**
- Zero mid-run blocking questions.
- One end-of-report behavior question (`V9N9r1`), which is the template the skill allows.
- Sibling `V9N9r2` treated the same C4 line as a note and did not ask. The batch therefore shows both “ask the one real fork” and “assume no migration.”

PROMPT.md: “The human … is not available to answer routine engineering questions: derive requirements from the brief, decide technical details yourself, and report at the end.” That is what 29/30 agents did. Unsure lists are mostly unspecified error codes and JSON extras — reported, not escalated.

## Stopped early / skipped verification / unfinished-while-success

Stopped early waiting on a human: **no**. All 30 terminal-yielded. `U9N7r1`, `V9N9r1`, `V9N9r2` yielded twice after an idle reminder; the report already existed.

Required-by-PROMPT tests (process-restart / crash-label / lost-reply) + run them: TAP or python-run evidence is present for every build and change agent. Claimed pass counts match TAP `# pass` where TAP exists (U9N7r2 10, U9N8r1 6, U9N8r2 9, V9N7r2 11, V9N8r1 10, V9N8r2 12, V9N9r1 14). Jsonl `bash`/`eval` argv is often truncated (`…`) or empty (`eval` `{}`), so command text alone is not a reliable negative; tool **results** are.

Skill steps often skipped, with mixed disclosure:

| step | who ran it | disclosure |
|---|---|---|
| `ik check` (+ `--prior` on change) | builds and changes, in tool results | usually claimed |
| `ik scenario run` | builds and changes, in tool results | usually claimed |
| existing-instance continuity (change.md 5a) | all six V9, in commands/outputs | claimed |
| `ik evidence verify` | **only V9N9r1** | V9N8r1 listed it Not run; V9N7r1/r2, V9N8r2, V9N9r2 did not |
| `ik handoff` CLI | **none** | U9N9r1 and V9N8r1 listed it; others omitted |
| TLC / Hegel | **none** | most listed Not run; optional per SKILL.md |
| diagnose `lookup_operation` / scratch scenario | **none** | case prompt used app+GET instead |

These skips are **not** human questions. They are incomplete skill journeys while reporting success. Where Not run is listed, handoff matches the transcript. Where it is omitted (`ik evidence verify` on five change agents; `ik handoff` on most), the handoff is thinner than the skill template, not contradicted on the claims it *does* make.

No agent left the diagnose case without `diagnosis.json`. No investigate-only agent’s yield claimed a store write it did not make.

## Does batch-009 support “the agent needed no human engineering intervention”?

**Supports (observation):**

- 30/30 finished without a blocking human turn. No `ask` tool. No “Decision:” mid-flight.
- **(b) = 0.** They did not push checker choice, corpus authoring, payload-schema inspection, writer-finding, or `ik` argv onto the requester.
- 29/30 `decision=none`. Unspecified error codes and JSON extras were chosen and footnoted (c), which is what PROMPT.md asked.
- Diagnoses classified from `./run status` + public GET (and reconcile only in authorized B), without asking which recovery command to run.

**Does not support (insufficiency):**

- Product correctness, lawfulness of machines, or hidden-oracle pass. This audit did not re-execute apps; A2 inspects that.
- That C4 on N9 is settled. `V9N9r1` asked (a); `V9N9r2` assumed freeze. Batch-009 does not say which existing-envelope behavior the requester wanted.
- That every Ikaros skill step ran. `ik evidence verify` and `ik handoff` were mostly skipped. Optional specialists were not run (correctly optional, but then “full skill journey” is untrue).
- That listed unsure items are harmless. Several (expire-on-cancelled, missing-env codes, v1 packed-cancel, v1 extend) are real behavior forks the brief left loose; agents did not need a human *in session*, which is not the same as the brief being complete.
- That the stricter HEAD `build.md` fit rule was followed. Agents never saw it. U9N7r1’s yield even names `desk-aux.json` for cross-aggregate recovery — a host sidecar the current skill now forbids. That is guidance drift, not a human question.

**Boundary:** evidence is this batch’s yields + transcripts + the briefs those runs had. It cannot speak to other batches, to whether a later reviewer had to patch the trees, or to production permission.

## Could not inspect

- Full untruncated `bash`/`eval` argv: jsonl stores many commands with Unicode `…` or `eval` args `{}`. Used tool-result TAP/ACCEPT/HTTP bodies instead.
- `history://` is a summary; jsonl is the session log. Parked roster listed all 30 ids.
- A2 reports in `dfy/audit/A2-N*.md` were not used (read tool refused them as oversize). This document does not inherit A2 findings.
- Live diagnose `STATE_DIR` bytes were not dumped; classify-from-commands only.
- Whether crash-label tests inside the Node/Python files actually honor every `CRASH_AT` string — that is implementation inspection, not autonomy.
