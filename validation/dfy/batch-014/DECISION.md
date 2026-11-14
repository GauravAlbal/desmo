# Batch 014: decision record

Pre-registered in `PREREG.md`, frozen in `manifest.json` and committed before dispatch. Two fresh families (N18 darkroom session desk, N19 laboratory sample chain-of-custody), two replicas each, BUILD then CHANGE, TypeScript, macOS arm64. Per-rule rows for every tier are in `results.json`.

## First scoring (the record)

| Stage | N18 r1 | N18 r2 | N19 r1 | N19 r2 |
|---|---|---|---|---|
| BUILD | pass | pass | pass | pass |
| CHANGE | fail | fail | fail | fail |

BUILD 4 of 4. CHANGE 0 of 4. In every CHANGE failure the failing rows were criterion 2 reported `unroutable` (runtime error `IK_COMMAND_INVALID`) on N18 `print_sheets`, `abort_clear`, `abort_note` and on N19 `abort_clear`; all other rules passed all four criteria. Both BUILD stages passed their input-validity rules (`paper_grade`, `lane_bound`) through machine guards.

## Instrument defect found after first scoring

Two independent replicas failing the same rows pointed at the instrument before the builders. Confirmed: the evaluator's direct-command attacks routed by command names and argument names taken from the oracle's reference build (`send_prints`, `abort` with cleared-fact arguments). A delivered machine that names its commands differently (`save_print`, `plan_print`) or clears facts by a literal inside the machine received `IK_COMMAND_INVALID` and was scored as a failure although its guards were present (`print_count == tray_count * 6`, `$note != ''`). Unroutable is never a pass, so the first scoring stands as the record and the defect is a limit of what that scoring could show.

The repair (v2) is candidate-aware: it discovers commands and declared parameters from the delivered machine, requires an honest twin (the same call with the honest value must commit) before an attack can pass, and defines `abort_clear` for a command with no parameter able to carry a cleared fact as: no such parameter, facts read as cleared through the runtime after an honest abort, and no declared way to leave them set. It was validated only on toys before it touched a delivery: the reference and three alternate-surface references pass; host-only controls (reference surface and alternate surface) fail every c2 row; missing-input controls fail c2 on exactly the input-validity rules; stale, always-refuse and breaks-old controls fail. A second repair (v2.1) was made after the first v2 run on the deliveries showed `twin_failed` rows for N19 (the honest twin supplied an original-edition send count on a revised sample and did not supply the freeze placed time and reason the delivered command declares). v2.1 was again validated on toys, with two new alternate-surface variants for those defect classes, before the deliveries were rescored. That v2.1 repair was made after seeing v2 output on the deliveries; it is disclosed as such, and the pre-repair v2 rows are kept in `results.json`.

## Rescoring with v2.1 (reported separately, not replacing first scoring)

| Stage | N18 r1 | N18 r2 | N19 r1 | N19 r2 |
|---|---|---|---|---|
| BUILD | pass | pass | pass | pass |
| CHANGE | pass | pass | fail | fail |

N19 CHANGE fails one rule in both replicas: `abort_clear` criterion 2. A direct runtime `abort_freeze` with an empty `supervisor_id` commits in both deliveries; the machines declare `args = { supervisor_id = "string" }` and no guard on it, and the host (`parseId`) refuses the empty identifier before calling the runtime. That is the input-validity miss class from batch 012, reproduced in a new family and a new command. All other rules pass all four criteria in all four CHANGE deliveries.

## The quantity this check targets

Input-validity rules enforced by a machine guard (checked through the runtime, not the CLI): 6 of 8 rule-instances. `paper_grade` 2 of 2 (BUILD), `lane_bound` 2 of 2 (BUILD), `abort_note` 2 of 2 (CHANGE), supervisor non-empty 0 of 2 (CHANGE). The brief states the supervisor rule only through the shared identifier grammar (1 to 64 characters) rather than as a command-specific sentence; the other three input rules are stated explicitly where they apply. That difference in how the oracle stated the rules is a confound and is not resolved by this check.

## Other findings

- The original N18 brief mentions `abort-session` in its requirements and precedence text without defining the command, which both N18 BUILD replicas reported. It does not change any scored BUILD rule. It is a brief defect and is recorded.
- A builder-reported language limit that held up: a guard cannot multiply two facts, so `print_count == tray_count * 6 && sheets_per_tray == 6` was used. The check still holds the relationship.
- Every derived-value rule (amount, deadline, gate, relation) passed criterion 2 in every delivery where the instrument could route it, which is consistent with batch 012.

## What this supports and does not

Supports: on two fresh families and two replicas, the added input-validity wording coincided with machine guards on 6 of 8 input-validity rule-instances, including all three rules the briefs stated as command-specific sentences, and with all derived-value rules passing; and a specific remaining miss class, an identifier-format rule stated only in a shared grammar, left host-only in 2 of 2 deliveries of one family.

Does not support: a rate, a comparison with batch 012 (different families, briefs and evaluator), any claim about other families, or `DFYJourneyQualified`. First scoring is BUILD 4 of 4 and CHANGE 0 of 4 and is not replaced by the rescoring. The v2 and v2.1 evaluators were authored by oracle agents with knowledge of the first-scoring failure pattern. n=2 per family, TypeScript only, macOS arm64, feasibility adapter off, no Linux x64, no human participants, agent model not exposed.

## Next

- Product: generate a machine guard (or a corpus-case check) for identifier-format and non-empty rules on declared string arguments; the guide wording is not enough on its own for rules stated only in a shared grammar. Candidate: an `ik check` advisory that flags a declared string argument with no guard referencing it, since the check already reads the machine.
- Instrument: make direct-command routing candidate-aware from the start for the next family, and keep the honest-twin protocol.
