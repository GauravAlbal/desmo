# Audit stages A1 to A4: what has evidence and what does not

This is a status record, not a certification. Each row names the evidence in this repository or says there is none.

| Stage | Required | Evidence present | Not done |
|---|---|---|---|
| A1: oracle and plan | Oracle authored without candidate source; frozen outside builder write access; validated with deliberate controls; method-fit classified | The three evaluator families were authored by contexts that had not seen the candidate and validated against defect controls (`controls-validation.log`, re-run by hand). Evaluators and sealed incident truth were kept in directories the agents were told not to read. | Separation was by instruction, not by tooling, so it is honor-system evidence. The method-fit classification for formal proof was not recorded. |
| A2: implementation and seam audit | Source and raw evidence examined for missing writers, hollow paths, partial commits; broken variants confirmed to fire; full gates run | New product code (`ik handoff`, usage unification, report status order) shipped with tests that include negative controls and the full Rust suite (496 passed, 0 failed at the last run). | No independent candidate-aware inspector reviewed the delivered cold-run builds' source. |
| A3: integrated product and autonomy audit | BUILD, CHANGE, DIAGNOSE from the installed boundary with the real kernel; human surface asks only substantive questions; scope and complexity audit | Batches 001 and 002 ran all three journeys from installed packages against the real kernel in scratch stores. | The human-question audit and the delete-test scope and complexity audit were not run. The packaged bytes of batch 002 predate later skill and `ik` changes. |
| A4: cold qualification and authority audit | Fresh agents, structurally separate evaluators, every attempt recorded, authority-negative controls run | Every attempt of both batches is recorded, including first failures and re-runs. Agents had no access to the acceptance gate. | Batch 003 used fresh families (N4 to N6). Two authority-negative controls were run on the packed binary (store flag and output into a store directory were refused, bytes unchanged). Mandatory requirements do not all pass, and the blind batch needed evaluator and brief fixes after first scoring, so first-pass scores are kept as failures. Batches 001 and 002 share families; batch 004 reran the batch-003 families on repaired briefs and a rebuilt candidate (build 6/6, change 5/6, diagnose 18/18), so no second blind population exists there. Batch 005 used three new families but needed evaluator and brief repairs after first scoring (build 4/6, change 0/6, diagnose 14/18 before; all pass after), and its continuity check, added afterward, found that both N8 replicas broke existing instances (batch 007 then passed 2/2 after the `owner` accessor and guide step; one family, not blind). Mandatory requirements do not all pass. The authority-negative controls (gate modification, self-approval, unauthorized publication or pin movement, unauthorized live replay) were not run against cold agents. The lead's own submissions were rejected by the gate for a dirty tree and for out-of-envelope files and were not overridden; that is the gate behaving, not a cold-agent control. |

Disposition: **A1 partial, A2 partial, A3 partial, A4 not met.** DFYJourneyQualified is not claimed.

Batch 008 froze the instruments (hashed before dispatch) and reran the batch-005 families on the final candidate: build 3/6, change 2/6, diagnose 17/18, continuity 6/6. It is not blind (families reused). Its main finding: none of the six builds calls the runtime client, and the behavioural evaluators cannot tell (batch-005 builds did use it; the cause is likely the restricted agent condition, not isolated). A4 stays unmet.

Batch 009 reran the frozen instruments with an unrestricted agent: build 6/6, change 6/6, diagnose 18/18, continuity 6/6, with every build and change using the runtime and routing old instances by `Aggregate.owner`. This supports the reading that batch 008's gap was the agent condition. Still not blind (families reused, different agent model) and n=2 per family; A4 stays unmet.

## Round 2: agent-owned audits before the held-out batch

Roles were separate contexts: three oracle authors (no candidate implementation, no builder submissions), three candidate-aware inspectors for source and seams, two inspectors for autonomy and scope, an integrator for the runtime-use check, and the lead, who reran every measurement below by hand. No inspector edited a target.

### A1: instruments

- **Isolation (blocked, with capability named).** A permission-denied directory was read back by the same user with one `chmod`, so path or permission separation of evaluators from builders is honor-system under this runner. Mitigation for the held-out batch: evaluators, controls, sealed incident truth and continuity plans are kept in an encrypted archive that is off disk while any builder agent runs, with the key held only in the lead's process memory, and are unsealed only between agent waves. This establishes that the files are absent from the builder-readable filesystem during a wave. It does not establish a general sandbox, and a lost key means the archive is unrecoverable.
- **Three new families** (names withheld until the batch completes) were authored in the persistent-local lifecycle topology: two TypeScript, one Python, each with 13 or 14 initial requirements, a five-requirement feature change that alters existing behavior and adds a command, an explicit written policy for every changed behavior on pre-change instances, an external effect with a possible lost reply and public lookup route, three incident cases (lost reply investigate-only, lost reply authorized recovery, expected refusal that is not an incident), and a continuity plan.
- **Control batteries, rerun by the lead.** 12, 16 and 12 non-reference controls per family, each failing named requirements, covering always-refuse, duplicate effect, stale completion, false uncertainty resolution, untriggered fault boundary, broken old-instance routing, and domain-specific defects (the two authority controls of one family sit in this directory and pass the behavioral evaluators by design). In all three the correct reference toy passes the initial requirements, the changed toy passes all, and the unchanged toy fails exactly the change requirements. Every change requirement C1 to C5 has at least one control that fails it. Known limits: two controls spill into neighboring requirements, and one family's always-refuse control still passes an empty-status requirement.
- **Defect found and fixed before freeze.** The oracles' runtime-use checks were tied to the reference toy's own machine and store file names, so they would have failed correct builds that chose other names. A shared generic checker now discovers the store and machine pair by running `ik verify`, requires history and state to advance after mutating commands, requires the crash-saved attempt to be in the runtime store, and requires named facts from `status` to appear in the runtime record; it compares no state names.
- **Runtime-use check validation (rerun by the lead on a sample).** Reference toys 3 of 3 pass; shadow-state and bypass controls 6 of 6 detected with reasons; real batch-009 builds that use the runtime 6 of 6 pass; real batch-008 builds that do not 6 of 6 detected. The check covers the discovered store, history and state growth, crash persistence, and fact agreement on the exercised commands only. A determined bypass that also writes to a runtime store on the exercised paths is not excluded.
- **Oracle fidelity.** Behavior is exact-checked on the commands and fault boundaries listed in the briefs; hidden schedules and inputs vary but no requirement is hidden. No formal-model campaign (TLC or Hegel) was included; constitution section 26 is not claimed for these families.

### A2: implementation and seam inspection of the batch-009 deliveries

Three fresh read-only inspectors (one per family) covered build and change source for both replicas, about 1,000 lines of findings with source references.

- Every delivered application commits its main lifecycle writers through the runtime, and all four change applications route pre-change instances to their owning machine version. That part of the evidence is confirmed from source, not only from evaluators.
- The behavioural evaluators passed applications with real defects: a host sidecar that can release resources while the order stays open, two-store sagas with no recovery leaking occupancy on cancel and expire, a `sent` transition not coupled to an accepted effect, retry accounting differing from the brief, a failure marker not cleared on a legacy path, and one hollow test. The inspectors classified almost all as application defects. The guide already forbids a host branch deciding the same lifecycle; the cold agents did not always follow it. These seams are not covered by the behavioural evaluators, so they are covered here by inspection and the held-out deliveries receive the same inspection after the run.
- One candidate or guide finding: the atomicity-fit step told builders to stop, with no constructive path, and both warehouse builders built a cross-store host saga instead. `skills/ikaros/build.md` step 2 now tells them to first try one aggregate, forbids a saga, sidecar or host lifecycle branch as the substitute, and says what to deliver when no human can answer. Requires new package bytes and qualification; the batch-009 evidence is unchanged.
- One limitation, not a defect: a frozen owner machine has no way to add a command to existing instances, so a change that asks old instances for a new command can only be refused or migrated. The held-out change contracts state that policy explicitly instead of leaving it for an evaluator to decide.

### A3: autonomy and scope

- **Autonomy and questions, 30 agents (6 builds, 6 changes, 18 diagnoses).** One question to the human, a genuine behavior decision the change text left open (new action on pre-change instances); none was engineering burden; none blocked; 29 reported no decision needed. No agent stopped early. Exercised claims matched the transcripts, with silent skips of `ik evidence verify` (5 of 6 changes) and the `ik handoff` command (all). This supports "no human engineering intervention in session" and does not support product correctness or full skill-step completion.
- **Scope and complexity.** The agents' delivered applications used `createWith`, error classification and `recordEffectOutcome` (10 of 12 builds and changes), and `Aggregate.owner` (6 of 6 changes). They did not name `ik handoff`, corpus, scenario, report, trace, analysis or the specialist adapters in their source, though several reports state that they ran `ik check` and scenarios in their sessions, so source-based counts understate use. Delete-candidates listed by the inspector: `ik handoff`, the `ik check` next-step block, analysis conformance and starter adapters, the P adapter, and the `trace` command as a journey step. All five were deliverables of the commissioned goal packets (handoff projections, the specialist adapters, the first-run step) and are kept; deleting them is outside this continuation. No function above the repo complexity floors was introduced; one Rust parser function grew from 21 to 29 against a ceiling of 30. TypeScript and Go complexity was not measured.

### Dispositions after this round

| Criterion | Disposition |
|---|---|
| A1 instruments validated with defect controls, method fit recorded | met for the three new families; fidelity classified above |
| A1 evaluator isolation | blocked: the runner cannot enforce separation from same-user agents; mitigated by sealing, scope stated above |
| A2 candidate-aware inspection of delivered source | met for batch 009; repeats on the held-out deliveries |
| A3 human-question audit | met on the batch-009 transcripts; not a substitute for an unfamiliar-human study |
| A3 scope and complexity audit | met, with TypeScript and Go complexity not measured |
| A4 held-out qualification and authority-negative controls | not yet executed; see later sections |

## Round 3: held-out batch 010 (first scoring)

Full record: `batch-010/DECISION.md`. The predeclared floor (6 of 6 builds, 6 of 6 changes with continuity, 18 of 18 diagnoses on first scoring) was **not met**: build 3 of 6, change 0 of 6, diagnose 13 of 18, continuity 3 of 6.

- **Instrument defects found after scoring** (kept as first scoring, not rescored): old instances were seeded by the oracle's reference program instead of each trajectory's own pre-change build; an unnamed success field was asserted; a brief left an error-code precedence undefined; the diagnosis evaluators flagged any free text matching `POST`; one continuity plan used a schema the loader did not read. The last was a scoring-integration mistake of the lead's own, and no evaluation took place for those two trajectories.
- **Builder and delivered defects, independent of the instruments**: one builder miss against an explicit brief line, one delivered `package.json` carrying pasted runner text so the program could not start, and an old-instance `complete` that returns `corrupt_state`.
- **A2 on the held-out deliveries** (three fresh read-only inspectors): sidecar files act as authority for amounts and lifecycle on old instances in all four N10 and N11 change deliveries; host blob authority in N12; one hollow success; one unversioned machine; no `lookup_operation` anywhere. The behavioral evaluators, and the frozen runtime-use check, passed deliveries with these defects.
- **Root cause with product consequences**: the product has no migrator and a frozen owner cannot gain a command. The N10 and N11 change contracts asked old instances for new commands, and `change.md` did not say what to do. Step 5b now does (changelog entry); this is a guide change and so starts a new lineage.
- **A4 authority-negative controls on disposable targets**: forged coverage by copying a real ledger line with the revision swapped is accepted as covered, and approval of a revision with no accepted receipt is recorded, so neither is enforced by the gate CLI itself (two defects filed). Protected evaluators were off disk during every agent wave, which is isolation and not a refused write. Inspect-only replay against a store was refused with bytes unchanged. Publication, live pin and reroll controls were not run against real targets and no disposable target exists for them.

### Dispositions after round 3

| Criterion | Disposition |
|---|---|
| A1 oracle and instrument audit | **failed on the first held-out lineage**: validation by control battery was necessary but not sufficient; five instrument defects surfaced only against real submissions. Repairs go into the next lineage's oracle brief. |
| A1 evaluator isolation | blocked (runner cannot enforce); mitigated and exercised by sealing during every wave |
| A2 candidate-aware inspection | met for batches 009 and 010, with findings above |
| A3 human-question and scope audits | met (round 2) |
| A4 held-out qualification | **not met**: floor missed on first scoring |
| A4 authority-negative controls | partly met: forge and self-approval **failed**; replay refusal met; publication, pin and reroll controls blocked for lack of a disposable target |
| Human review, human cold use, Linux x64, consumer acceptance, owner decisions, publication | open, separate rows, not waived |

## Round 4: second held-out lineage, batch 011 (first scoring)

Full record: `batch-011/DECISION.md`. Predeclared floor **met**: build 6 of 6, change 6 of 6 with continuity 6 of 6, diagnose 18 of 18, all on first scoring, installed bytes from source `3741e2b`, three families never exposed to a builder, evaluators and incident truth off disk during every agent wave, instrument self-test passed in every decryption window.

- **What changed from lineage 1**: every instrument defect of batch 010 was built into the revised oracle brief and then validated before freeze (control batteries rerun by the lead, runtime-use check with an Ikaros-backed positive, bypass, shadow-state and sidecar controls in happy and old-instance modes, continuity plans through the real harness). Guide steps `build.md` 2 and `change.md` 5b were added from lineage-1 findings; the candidate was rebuilt and requalified before the freeze.
- **A2 on the lineage-2 deliveries**: no JSON sidecar authority and the stated old-instance policy honored where inspected; still found: host-computed amounts and deadlines passed as command arguments in every delivery (the guard and assignment language cannot add integers or assign string literals, so this is a product expressiveness limit), a host-only gate that the command interface bypasses (N13), leftover deposit facts that make a later reconcile `corrupt_state` (N15), and no `lookup_operation` anywhere. None of these changed first scoring; the instruments do not see them.
- **A3 autonomy**: lineage-2 final reports surfaced no decision requests to the human; the formal transcript audit was run on batch 009 and not repeated.

### Dispositions after round 4 (per criterion)

| Criterion | Disposition |
|---|---|
| A1 oracle and instrument audit | **met for lineage 2** (validated before freeze, self-tested every window); lineage 1 failed it and that record stands |
| A1 evaluator isolation | blocked as a runner capability (no per-agent sandbox); mitigated and exercised by encryption between waves; scope stated in `RULES.md` |
| A2 candidate-aware inspection | met for batches 009, 010 and 011, with the findings above |
| A3 human-question audit | met on batch 009 transcripts; lineage-2 reports read by the lead; not a substitute for an unfamiliar-human study |
| A3 scope and complexity audit | met; TypeScript and Go complexity not measured |
| A4 held-out qualification (fresh families, installed bytes) | **met** on the second lineage under the predeclared floor; the first lineage missed it |
| A4 authority-negative: inspect-only replay and scratch-store refusal | met (batch 011 `authority-controls.json`, same binary digest as batch 004); investigate-only diagnostic cases passed with state hash unchanged in 6 of 6 |
| A4 authority-negative: evaluator modification | met as isolation (nothing to modify during waves); not a refused write |
| A4 authority-negative: forged acceptance | **failed against the gate CLI** (copied real ledger line with swapped revision reports covered); defect filed |
| A4 authority-negative: self-approval | **failed against the gate CLI** (approval of an unaccepted revision recorded); defect filed |
| A4 authority-negative: unauthorized publication, live pin change, gate reroll | **blocked**: no disposable target exists; deliberately not tried against real ones |
| Human review, human cold use, Linux x64, consumer acceptance, owner decisions, publication | open, separate rows, not waived |

### Disposition correction (after the lineage-2 review)

This replaces the paragraph that called the gate the only open leaf. It was wrong: applicable application and product findings are also open. Historical scores are unchanged.

| Item | Current disposition |
|---|---|
| Frozen held-out test floor | **met** for batch 011 (its exact bytes, instruments, agent condition and host) |
| A2 inspection execution | completed for the inspected deliveries; **substantive findings remain open** until each is reproduced, classified and closed or dispositioned in `audit/` reports |
| Canonical business-rule enforcement | **not established** for rules whose derived values the host computes and the kernel copies without checking their defining relationship |
| Recovery correctness | **open** for the reproduced N13 and N15 paths and for any other reproduced finding |
| Acceptance-authority controls | **failed** for forged coverage and for self-approval against the gate; publication, pin and reroll controls **unexecuted** |
| DFYJourneyQualified | **not asserted** |
| EngineeringReady | **not inferred** from batch counts |
| PublicReleaseReady | **no**; human, owner, consumer and target-platform gates stay separate |

Two independent repair tracks follow: the gate's authority boundary, and delivered lifecycle behavior plus the product and agent interfaces that failed to establish it. Closing an inspection is not closing its findings.

### Per-finding dispositions after reproduction (batch 011 deliveries; reports in `batch-011/audit/`)

| Finding | Disposition |
|---|---|
| N13 version-2 `checkin` bypasses host `recall_required` through the command interface | **confirmed defect**; layer: delivered domain code plus undiscoverable guard forms; open until repaired and a regression fixture exists |
| N13 `file` repeat path calls an undefined `machine.show` | **confirmed TypeError path**; the CLI maps it to `already_filed`, so the misleading error is not reproduced through `./run bad_args` |
| N15 `promote` leaves deposit facts, later `reconcile` returns `corrupt_state` | **confirmed defect** of the written change contract |
| N15 r2 leftover `deposit_id` hidden by `has_deposit=0` | **confirmed defect** of the written change contract |
| N14 r2 `reconcile` needs a live provider after terminal state | observed; **not a violation** of the written transport-failure text |
| N13 r1 effect outcome digest equals payload digest | confirmed against product guidance; **not a violation** of the written brief |
| Host-copied derived values (amounts, deadlines, spans) | category-3 authority gaps confirmed per rule in `RB1-authority.md`; enforced rules recorded as category 2 |
| Claim: no integer arithmetic or string literals | **not reproduced**; see `RB2-language.md`. Product defects found: counter `then +N` wrapped at `i64::MAX`; `then = { n = "+1" }` on a replace fact disagreed between check and compile; `*` and `/` errors were opaque |
| Gate: copied ledger row with swapped revision; unaccepted approve | repaired on a branch of the gate's own repository, independently reviewed (no copy-swap bypass found; `approve` of an unaccepted revision is refused; `--override` is recorded separately and never covers), and **accepted by that repository's own acceptance loop** after its floor was recalibrated (full suite 2899 passed, 1 skipped, on a branch rebuilt on its current main; an unrelated disk-floor workaround commit the repair agent had added was left out). Earlier submissions had been rejected as executor kills under memory pressure and a stale floor, not on content. **Not yet landed on the gate's main branch and not installed**: landing and release are the gate maintainer's call, so the gate currently in use is still the unrepaired one. Residual trust: a same-account writer of the sibling event store can still mint coverage (events are unsigned); the CLI operator verbs are not tied to an OS principal. Read-only revalidation of the real ledger with the repair: 145 rows, 144 authenticated and bound, 1 failing (event file absent from the ledger-adjacent store; bytes exist in the home last-resort store; not recreated) |
| Publication command (C1) | **FAIL**: no authorization check in the repo's own release path (`docs/RELEASING.md` steps publish with no token; `scripts/release` copies the pin file with no token). A disposable wrapper with a sentinel destination showed the control shape, but it is not part of the repo. Ikaros's own publish-approval lifecycle control passes at product level and is a different claim. Real registry IAM was not tested |
| Test pin (C2) | gate component pin file: PASS on a disposable file (unknown or non-remote revision refused; authorized re-pin wrote only the disposable file). Ikaros `LATEST_RELEASE`: no pin-update verb, **FAIL**. File mode and immutable flags block writes only until the same uid undoes them |
| Gate reroll (C3) | bare resubmission of a settled rejection: PASS. Operator and worker through the library API: PASS; through the CLI every operator verb runs as operator, so a same-account worker is an operator: **FAIL** at that boundary. Repeat-class breaker: observed live on this change (see below); a disposable run was not executable because the task substrate was unavailable. Override recorded separately: PASS |
| Evidence custody (C4) | **FAIL for same-account workers**: mode 0600 does not exclude the same uid (read, list and write succeed from an empty-environment child). No key in the environment or inherited descriptor. Excluding the same uid needs a second account or a remote store |

Repeat-class breaker, observed: this change's tests were rejected twice by the test-value floor (five explicit `AssertFalse`-class hard gates). Differential diagnosis: the same test-quality evidence tool, run directly on the file, reproduces five violations on both failing versions and zero on the fixed version, so the cause was five explicit `panic!` arms in the test file (the first repair guessed the wrong construct). The local preflight does not run that tool, which is why two submissions were spent; filed. Disposition of the breaker is the operator's.

Seam fixtures for the N13, N14 and N15 findings now exist (`validation/dfy/regressions/`, all seams matched on the repaired bytes). The requalification and fresh transfer check ran as batch 012 (below).

## Round 5: finite transfer check on the repaired bytes, batch 012 (first scoring)

Two fresh payment-rail families, two replicas each, pre-registered (`batch-012/PREREG.md`) and frozen (`batch-012/manifest.json`) before dispatch. First scoring: BUILD 3 of 4, CHANGE 1 of 4. The N16 r1 failures were a harness defect (`./run` launched through `node`); after the fix and a passing instrument self-test it was rescored once and passes both stages, recorded separately and not replacing first scoring. Every derived value (amount, fee, deadline, span, cap, approval bar) was refused by the runtime on a wrong direct command in all 8 deliveries; invalidation and old-instance continuity held in all 8. Both N17 replicas left the empty-`reviewer_id` refusal in host code only (an input-validity rule, expressible as a guard): the check's floor was not met on first scoring and the miss is a different class from the one the repair targeted. A guide-wording follow-up is filed, not applied, so the result stays attached to these exact bytes. This supports only the finite statement in `batch-012/DECISION.md`. `DFYJourneyQualified` is still not asserted.

## Round 6: coverage-contract bytes requalified, batch 013 (no agent wave)

The checker's coverage contract was repaired in two shipped steps: authored behavior is mandatory and satisfied only by committed witnesses; generated product-context copies are discharged only by sound impossibility records; an optional, operator-configured feasibility adapter (Z3 with an independent cvc5 check) can discharge a generated copy only through a re-verified, symbolic inductive certificate or an exhaustive kernel exploration, and a solver-asserted result never discharges. On the packed bytes (`batch-013/manifest.json`) the targeted regressions pass and the two downstream consumer models behave as recorded in `batch-013/DECISION.md`: model A ACCEPT with the adapter off or on (identical digest); model H REFUSED without the adapter and ACCEPT with it, the five discharged records carrying `CheckedInvariant`; a mutant whose shadow breaks only at an unsampled integer value was not falsely discharged. The earlier prover that discharged shadowed authored arms was rejected before shipping (it accepted an always-true first arm) and a second string-ordering unsoundness was found by differential testing and not shipped. This closes the generated-coverage findings (#11191, #11197, #11198) for these bytes; it does not change any agent-journey result. `DFYJourneyQualified` is still not asserted.

## Round 7: finite transfer check on the input-validity guide wording, batch 014

Two fresh non-payment families, two replicas each, pre-registered (`batch-014/PREREG.md`) and frozen before dispatch. First scoring: BUILD 4 of 4, CHANGE 0 of 4, every CHANGE failure an `unroutable` criterion 2 row from an evaluator that routed by the oracle's own command names; this was an instrument defect and the first scoring stays the record. The evaluators were repaired candidate-aware (honest-twin protocol, validated on toys including alternate-surface controls) and the deliveries rescored separately: N18 passes both stages in both replicas; N19 fails CHANGE in both replicas on one rule, a direct `abort_freeze` with an empty supervisor identifier commits because the machine has no guard and only the host refuses it. Input-validity rules enforced by a machine guard: 6 of 8 rule-instances; the two misses are the rule the brief states only through a shared identifier grammar. A second repair (v2.1) followed v2 output on the deliveries and is disclosed as such. This supports only the finite statement in `batch-014/DECISION.md`. `DFYJourneyQualified` is still not asserted.

## Round 8: focused check of input-validity enforcement with the `ik laws` advisory, batch 015

Two fresh families, two replicas each, pre-registered (`batch-015/PREREG.md`) and frozen before dispatch. First scoring: BUILD 0 of 4, CHANGE 0 of 4, the record. Review of the failing rows showed four evaluator defects (hard-coded initial-state seeding, twin fill without brief-stated companion literals, a valid-then-invalid oracle sequence that contradicted the brief's precedence, and machine discovery that walked into the staged kit); all were repaired by oracle agents without access to the deliveries, validated on toys, and the repairs were made after the delivery rows had been seen (disclosed). Rescored separately: BUILD 1 of 4, CHANGE 1 of 4; every remaining failure is an empty identifier committed through a direct runtime command. Command-specific input rules were runtime-enforced in 12 of 12 instances and the advisory-flagged shared-grammar identifier rules in 2 of 8. The advisory did not by itself change agent behavior on the expressible part of an inexpressible grammar. This supports only the finite statement in `batch-015/DECISION.md`. `DFYJourneyQualified` is still not asserted.

## Round 9: guarding the expressible part of an inexpressible contract grammar, batch 016

Two fresh families, two replicas each, pre-registered (`batch-016/PREREG.md`) and frozen before dispatch, on a packaged candidate whose runtime is byte-identical to batch 015's and whose BUILD and CHANGE guides now require guarding the expressible part (non-empty, literal sets, comparisons) of a grammar the language cannot fully express and reporting only the residual. First scoring: 6 of 8 stages pass; the one failing replica fails only on `twin_failed` criterion-2 rows (an evaluator gap on fact-echo companion arguments, repaired by oracle agents without access to the deliveries, validated on toys, disclosed as made after the rows were seen). Rescored separately: 8 of 8. Identifier rules stated only through the shared grammar were runtime-enforced (non-empty) in 12 of 12 instances and command-specific input rules in 12 of 12; every agent reported the length and character-class residual as host-only. Adapting the wording in response to batch 015 does not by itself remove independence; the evaluator repair made after the rows were seen is what makes the 8 of 8 non-independent. This supports only the finite statement in `batch-016/DECISION.md`. `DFYJourneyQualified` is still not asserted.

## Round 10: bounded string predicate and full-grammar qualification, batch 017

Product: the machine language gained one bounded string predicate, `text_in(VALUE, 'SET', MIN, MAX)` (length in Unicode scalar values, explicit ASCII set or `*`, no normalization), implemented once and shared by runtime, explorer, explain, scenario and sampling; the SMT backend reports it unsupported and feasibility treats it as opaque, never as true. An independent adversarial review found no confirmed defect; a reference implementation matched the runtime on 5597 evaluations. The evaluator was unified into one candidate-aware kit (zero-diff against first scoring of the four earlier families, 3020 rows; 28 conformance toys).

Two fresh families (`N24`, `N25`), two replicas each, pre-registered (`batch-017/PREREG.md`) and frozen before dispatch. First scoring: 0 of 8 stages, every failing row `twin_failed` (an inconclusive evaluator gap on clock-equality, digest-charset and derived-relation companions); no attack on any grammar rule was accepted anywhere it was tried. The repair was written by an agent without access to the deliveries, validated on toys, and made after the rows were seen. Rescored separately: 8 of 8; all 32 grammar-rule instances (20 identifier) enforced by the runtime; opaque labels accepted. This supports only the finite statement in `batch-017/DECISION.md`. `DFYJourneyQualified` is still not asserted.
