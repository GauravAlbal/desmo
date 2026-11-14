# Cold qualification, batch dfy-001: BUILD, CHANGE and DIAGNOSE record

Status: **partial. BUILD, CHANGE and DIAGNOSE stages executed once each; not a qualification.** Nothing here qualifies the
journey.

## Executed
Three new project families (N1 order cancellation and refund desk, N2 subscription plan-change scheduler, both
TypeScript; N3 equipment loan desk, Python), two replicas each, fresh builder context per run, installed packages
from the frozen candidate (`results/MANIFEST.md`), the installed skill and the shipped guide. Evaluators were
authored by contexts that had not seen the candidate and validated against defect controls before dispatch
(`controls-validation.log`).

| Family | r1 | r2 |
|---|---|---|
| N1 | build 14/14 requirements pass | build 14/14 |
| N2 | build 14/14 | build 14/14 |
| N3 | build 14/14 | build 14/14 |

All six submissions used the shipped runtime for the critical lifecycle (machine definitions or an external
reducer), ran `ik check --profile` and `ik scenario run` themselves where the family is TypeScript, and needed no
human intervention. Agent wall times: 9m38s to 19m15s.

## CHANGE stage (executed twice, two different tool conditions)
Each change request revises a requirement and adds behavior to the submitted build of the same family, with the
original requirements still in force (`projects.tar.gz`, `change.md` per family). The evaluator scores the original
requirements R1-R14 and the new C-requirements.

| Run | Attempt 1: agent could not execute code | Attempt 2: agent had a shell |
|---|---|---|
| N1 r1 | fail: all 19 (the machine file did not parse; the agent could not run it) | pass 19/19 |
| N1 r2 | pass | pass |
| N2 r1 | pass | pass |
| N2 r2 | pass | pass |
| N3 r1 | pass | pass |
| N3 r2 | fail: C2, C3 | fail: C2, C3 |

* Attempt 0 (reviewer models) never started: HTTP 403 spending-limit within 3 s. Recorded as `not_run`.
* Attempt 1 used agents whose tool surface was read/write only (GPT-5.6 Codex for r1, Sonnet 5 for r2). They could
  not install, run, or test anything. The evaluator scored the edits as submitted. The build stage used agents that
  could run code, so attempt 1 is not like-for-like with the build; it stays recorded.
* Attempt 2 used a GPT-5.6 Luna builder with a shell, fresh context per run, restaged from the same builds. Build
  and change used different model families, which is a confound.
* N3 r2 failed C2 and C3 in both attempts by two different models: after promotion `status` still reported the
  previous borrower, and a checkout of the promoted-away reservation id returned `not_reserved` instead of
  `expired`. Both are scored failures. Whether the base build's state shape made this hard, or the request text is
  under-determined for that base, was not examined.
* The evaluator does not score everything an agent reported. N1 r2 (attempt 2) passed the evaluator while its
  agent reported that two historical scenarios fail and the machine corpus is refused for uncovered obligations;
  the evaluator does not run those. Stale shipped scenarios are an engineering gap the score does not see.
* An agent in attempt 2 (N2 r1) reported that the packaged `ik` lacked `scenario run` and `--profile`. The packaged
  binary in that project supports both; the report was wrong or used another binary. Agent reports are not
  evidence.
* Old-store continuity was a stated reporting requirement. Reports ranged from "not migrated, fails closed" to
  "not exercised"; no run was scored on it.

## DIAGNOSE/RECOVER stage (executed, 18 runs)
Each of the six builds was given three incidents created by the family's incident generator against the as-built
application, with ground truth sealed outside the case directory and scored by the family's evaluator
(`diagnose-cases.tar.gz`, `results/diagnose-raw.json`). Agent: GPT-5.6 Luna builder with a shell, fresh context per
run; the build stage used a different model family.

| Case | N1 r1 | N1 r2 | N2 r1 | N2 r2 | N3 r1 | N3 r2 |
|---|---|---|---|---|---|---|
| a: reply lost, investigate-only | pass | pass | pass | pass | pass | pass |
| b: reply lost, authorized recovery | fail | pass | pass | pass | pass | pass |
| c: expected refusal control, authorized recovery | pass | pass | fail | fail | fail | pass |

14 of 18 pass.

* N1 r1 case b: the agent reported `not_sent` from a provider lookup that returned HTTP 404, but the provider had
  accepted the refund. Its report names a lookup path that differs from the one the passing run used for the same
  family, so the 404 may have been a wrong route. A 404 from an unverified route was treated as proof of absence.
  Scored failure.
* The three control failures (N2 r1, N2 r2, N3 r1) classified an expected refusal as `refused_by_rule`. The
  vocabulary reserves that label for a refusal the recorded state does not require and says an expected refusal is
  `no_incident`. None of these runs changed state or caused an effect; the failure is the label.
* Harness defects found while running, recorded as interventions and not as agent errors: the N2 case did not name
  STATE_DIR, SERVICE_URL, or NOW_MS, and the orchestrator supplied them by message to five runs; three N3 runs could not find the command shape and received a pointer to the brief plus the status argument form. Eight
  of the 18 runs therefore had an orchestrator message and are not zero-intervention.
* Investigate-only runs left the store bytes unchanged in all six cases, and no run caused a second provider effect.

## Repair loop for the DIAGNOSE failures (regression track, not blind)
Cause named for the three control failures: the shipped diagnose guide said "classify: expected refusal, ..." but
never said that a correct refusal is reported as no incident, so agents reached for `refused_by_rule`. Cause named
for the N1 r1 failure: the guide did not say that a not-found from an unverified lookup route is unknown. The guide
was patched (two sentences in step 8, commit `554583d`) and the four failed cases were re-staged as fresh incidents
and re-run with fresh agents reading the patched guide from a copy (`diagnose-regression-cases.tar.gz`).

All four pass (`results/diagnose-regress-raw.json`). This is a regression result. The same family of failure was
fixed in the guide and then retested on the cases that exposed it, so it is not fresh blind evidence, and one
successful rerun of each does not show the failure cannot recur. The N2 case in this rerun also named the connection
details that the first harness omitted, so the rerun fixes two things at once.

N3 r2 change failures (C2, C3) are not repaired. Inspection of the delivered reducer shows `expired` is returned only
when the item's current reservation id equals the named id; promotion replaces the current reservation id, so the
promoted-away id returns `not_reserved`. That points at the builders' state design for the expiry history, not at a
product defect, but this was inferred from reading source and was not tested by a repair run.

## Not executed
Everything not listed above: no installed-byte re-qualification after the later skill changes, no human run, no
Linux. The candidate's packaged skill predates the handoff and specialist-routing text.

## Builder-reported product observations (unscored)
* One external-reducer build (N3) found that the kernel allows one pending effect per aggregate, which cannot
  represent several in-flight confirmations; confirmation identity was kept in reducer state instead.
* The guard language has no not-exists, so "not yet allocated" is an empty value in one build (N1); another could
  not express floor division and computed the value in the host (N2).
* The stub binds an ephemeral port, so builders could not honor the assigned port range without editing the stub.

## Limits
Six builds, one model family of unknown settings, one attempt each; 14/14 on every build gives no information about
how often a build would fail. Candidate-aware review of the submissions was not done. This is not a comparison.
