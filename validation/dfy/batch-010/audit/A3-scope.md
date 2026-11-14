# A3 — DFY Ikaros surface scope and complexity

Inspector: A3Scope (read-only on invariant-kernel / ik-experiments except this report and `/tmp/A3Scope-scratch`).
Candidate: invariant-kernel `HEAD` `43ca0f4978d30ab78778f5fc5565be2c38017e6a` (`release/region-authoring-0.4.0`).
Diff base: `1bddcee^` = `3a11f6449fd0e7aab07051a7f21523490db13be9` (parent of the commit that added `skills/ikaros/`).
Introducing commit: `1bddcee` “DFY D0-D2 and D1: effect helper, error taxonomy, admission and invalidation patterns, skill router”.
Range shortstat `1bddcee^..HEAD`: 418 files, +10719 / −126.
Working tree at inspect time also had an uncommitted `skills/ikaros/build.md` fit-step expansion (one paragraph). Journey citations below use the working-tree guides.

Named ask (from this audit): agents BUILD / CHANGE / DIAGNOSE lifecycle apps with Ikaros.
Disclosed journey surfaces: `skills/ikaros/{SKILL.md,build.md,change.md,diagnose.md,routing.json}`.
Agent-use evidence: grepped delivered apps under `ik-experiments/dfy/runs-b9/{N7,N8,N9}/{r1,r2}/{build,change}/project` (README/source/tests only; KIT, `.agents`, `.venv`, `sealed/`, and evaluators excluded). Diagnose evidence: `diag/*/case/diagnosis.json` and `operator-report.md` (inputs), not evaluators.

## Boundary

**Observation.** `docs/design/dfy-d0.md:7-16` already listed check/scenario/report/regression/profile-diff/evidence-verify/graph-export/TLC/Hegel/`ikaros doctor` as landed *before* this track. It named as not-yet-landed: adapter starter, skill/router, typed effect helpers, store-not-found diagnostic, client admission, invalidation patterns.

**Inference.** “Added since `skills/ikaros/`” therefore mixes (a) new commands/APIs in `1bddcee^..HEAD` and (b) pre-existing commands newly *named* by the journeys. The table marks `new` vs `wired`.

## Journey requirements (guides)

Required = numbered step, not marked optional. Optional = “if the question needs it” / specialist.

| Journey | Required surfaces | Optional |
|---|---|---|
| BUILD (`skills/ikaros/build.md`) | Discover/`ikaros doctor` (Node) or `ik --version` (Python); fit; `ikaros init` if no machine; `ik identity`/`laws`/`schema`; `ikaros codegen`; error-class mapping (`classify` / `StoreNotFoundError`); `ik check --plan` then `ik check --profile` (with `ik corpus init` / `NEXT COMMAND` named at step 10); `ik scenario run`/`reproduce`; `ik report`; `ikaros trace`; public-API exercise + `ik handoff` | `ik analysis describe/run/validate-result/conformance`, `ik graph export`, TLC, Hegel |
| CHANGE (`skills/ikaros/change.md`) | Discover + `ik identity`; `ik profile diff`; `ikaros codegen`; **`aggregate.owner` / `Owner()`** (step 5a); `ik check --prior`; `ik scenario run`/`reproduce`; `ik evidence verify`; `ik regression promote`/`run`; `ik report`; `ik handoff` | analysis / graph export |
| DIAGNOSE (`skills/ikaros/diagnose.md`) | Discover; `ik … show`; **`lookupOperation` / `lookup_operation` / `LookupOperation`**; `replay`/`verify`; `ik scenario run`/`reproduce`; `ik check`; classification; `ik report`; `ik evidence verify`; `ik handoff`; `ikaros trace` (advisory last line) | analysis (forbidden in `investigate_only`); `ik regression` only in `authorized_repair` |

`routing.json` does not name TLC, Hegel, `ik handoff`, `ik corpus init`, `ikaros trace`, or `adapters/p`. It does name doctor/identity/laws/schema/check-plan/profile-diff/analysis-describe/evidence-verify/report/show/replay/verify/lookup-operation/graph-export/scenario-run/reproduce.

## Agent use (batch-009 delivered apps)

**Observation.** All 12 build+change apps are TypeScript (`N7`,`N8`) or Python (`N9`). No Go app.

Client APIs in agent *source* (not KIT/.venv):

| API | BUILD | CHANGE |
|---|---|---|
| `createWith` / `create_with` | 6/6 | 6/6 |
| `classify(` | 6/6 | 6/6 |
| `StoreNotFoundError` | 5/6 (missing `N7/r1/build` which uses only `classify(err)==="not_found"` at `src/desk.mjs:85,601`) | 5/6 (same replica uses classify-only at `src/desk.mjs:86,719`) |
| `recordEffectOutcome` / `record_effect_outcome` + `pendingIntent` | 5/6 (`N9/r1/build/desk.py` has neither; crash labels `after-record-before-exit` exist at lines 378–389 with no helper call) | 5/6 (`N9/r1/change` also omitted the helper) |
| `aggregate.owner` | 0/6 | **6/6** (`N7/r1` `desk.mjs:156`; `N7/r2` `main.mjs:170`; `N8/r1` `main.mjs:70-71,306-307`; `N8/r2` `main.js:147-149,298`; `N9/r1` `desk.py:134` and later `agg.owner.version`; `N9/r2` `desk.py:154`) |
| `lookupOperation` | 0/6 | 0/6 |
| Go `Owner()` | n/a | n/a |

**Observation.** Agent READMEs document `npm install`, `./run …`, and `node --test` / `pytest` only (example: `N7/r1/build/project/README.md:9-42`). Zero matches in agent-authored README/source/tests for: `ik handoff`, `ik corpus init`, `NEXT COMMAND`, `ik check`, `ik scenario`, `ik report`, `ikaros trace`, `ikaros doctor`/`init`/`codegen`/`skill`, `ik graph export`, `ik analysis`, `tlc`, `hegel`, `ik profile diff`, `ik evidence verify`, `ik regression`.

**Observation.** Every build/change project still has `.ik/checks.toml`, at least one `scenarios/*.json`, and agent-authored `*.cases.json` beside machines (KIT copies also exist and were ignored for API greps). That is artifact presence, not a recorded invocation.

**Observation.** All 18 `diagnosis.json` files cite the *application* CLI (`status` / `reconcile` / `renew` / `send`) and HTTP GET to the stub. None cite `ik show`, `lookup_operation`, `ik check`, `ik scenario`, `ik report`, or `ik handoff`. Example: `N7/r1/diag/a-lost-investigate/case/diagnosis.json` (`status o-41` + `GET …/v1/bookings/…`); recover cases run `./run reconcile`.

**Observation.** Agent tests (`test/desk.test.mjs`, `tests/test_desk.py`, etc.) do not import `classify`, `owner`, or `recordEffectOutcome`. They exercise `./run`.

**Inference.** Agents used the *client* surfaces the BUILD/CHANGE guides added for writers (admission, error class, effect helper, owner-based routing). They did not leave evidence of using the *workbench CLI* surfaces the same guides list as later steps, and DIAGNOSE agents recovered via the app’s reconcile path rather than kernel lookup.

## Delete-tests (scratch)

Copies under `/tmp/A3Scope-scratch/delete/d{1-5}-*` (no `target/`, no `node_modules`). Surfaces removed in the copy; remaining references grepped in the repo. Heavy `cargo test` not run (assignment: prefer test names and guide refs).

### D1 — delete `ik handoff`

Scratch: dropped `"handoff" => crate::handoff::run(rest)` from `src/cli.rs:42` and `pub mod handoff` from `src/lib.rs:29`; omitted `src/handoff.rs`. Scratch `cli.rs` contains no `handoff` token.

Breaks:

- Journey steps: BUILD 16 (`build.md:24`), CHANGE 10 (`change.md:17`), DIAGNOSE 13 (`diagnose.md:29`). Human templates in `SKILL.md:74-83` would still exist.
- Tests: entire `tests/handoff.rs` (`cited_committed_and_refused_steps_are_exercised_with_their_scope`, `claims_without_resolvable_evidence_are_never_exercised`, `a_failed_expectation_contradicts_the_claim_and_blocks_the_exit_code`, `unreadable_evidence_is_listed_and_blocks`, `a_decision_is_rendered_without_choosing_for_the_owner`, `malformed_behavior_files_are_usage_errors`).
- `typescript/src/skill.test.ts:114-129` (`every cited command flag exists`): skill markdown still cites `ik handoff … --evidence … --kind … --json`; `usageFor` would no longer contain subcommand `handoff`.
- Docs: `docs/workbench-report.md:84,105`.

`ik --help` USAGE (`src/cli.rs:108-118`) already omits `handoff`; `tests/cli_usage.rs` would not notice the deletion.

### D2 — delete `ik check` NEXT block (`NEXT COMMAND` / `next_actions`)

Scratch: stripped the `next_actions` JSON insert and `println!("NEXT COMMAND\n  {hint}")` from `emit_check` (`src/cli.rs:1301-1315`). `fn corpus_hint` left in place but unused by emit.

Breaks:

- BUILD step 10 (`build.md:18` “Rerun, read `NEXT COMMAND`…”). CHANGE/DIAGNOSE do not name it.
- `tests/check_next_step.rs:16-22` `uncovered_refusal_names_the_corpus_commands` asserts `text.contains("NEXT")` and `"ik corpus init"`.
- `tests/admission_patterns.rs:65` `corpus_less_check_refusal_names_the_next_command_without_changing_exit` asserts `stdout.contains("NEXT COMMAND\n  {next}")` (line 86) and JSON `next_actions` (lines 72, 114).
- `docs/ik-check.md:56`.

### D3 — delete `ik corpus init`, keep `ik check --corpus-template`

Scratch: removed `"corpus" => run_corpus(rest)` and `fn run_corpus` / `run_corpus_inner`. `--corpus-template` / `emit_corpus_template` remain. `corpus_hint` still *prints* `ik corpus init {} --out {}` (`cli.rs:1271`).

Breaks:

- BUILD step 10 (`build.md:18`).
- `tests/admission_patterns.rs:29` `corpus_init_emits_an_editable_example_and_int_boundaries`.
- `tests/check_next_step.rs:21` (string `ik corpus init`).
- `docs/ik-check.md:59` (shown as equivalent to `--corpus-template` at line 60).
- `src/declarative/check.rs:258`.

**Overlap:** `docs/ik-check.md:58-60` and CHANGELOG `38` present `ik corpus init` and `ik check --corpus-template` as two writers of the same starter corpus.

### D4 — delete `ik analysis conformance`

Scratch: removed `Some("conformance") => conformance_cli(...)` and the usage line from `src/analysis/cli.rs`; removed `mod conformance` from `src/analysis/mod.rs`. Leftover `conformance_cli` body would not compile (`super::conformance::run`).

Breaks:

- BUILD step 15 only, and only as optional specialist (`build.md:23`). CHANGE 11 / DIAGNOSE 9 name `describe`/`run`/`graph export`, not conformance.
- `tests/adapter_starter.rs`: `shipped_starter_passes_conformance_on_an_exported_graph` and seven variant tests (`wrong_request_id_variant_fails_the_correlated_case`, `outside_write_variant_fails_the_contained_case`, `exit_without_result_variant_fails_the_result_case`, `overclaim_on_incomplete_graph_fails_the_honesty_case`, `dropped_native_log_fails_the_log_case`, `timeout_hook_removed_fails_the_timeout_case`, `cancel_hook_removed_fails_the_cancellation_case`).
- Skill flag test if `build.md:23` still cites `conformance`.
- `docs/analysis-adapters.md`, `docs/agent.md:161,168`, `adapters/starter/README.md`.

Native `ik check` / `ik scenario run` would not break.

### D5 — delete `adapters/p`

Scratch: omitted the tree.

Breaks:

- No journey step. Not in `routing.json`. CHANGELOG `50` and `docs/analysis-adapters.md:310+` describe it as prepared, not qualified.
- `tests/adapter_starter.rs:349` `p_describe_reports_tool_unavailable_and_executes_nothing` (paths at 351, 385, 398).

## Table

| Surface | New vs wired | Journeys that need it | Agent use (b9 README/source/tests) | Delete-test | Disposition |
|---|---|---|---|---|---|
| `skills/ikaros/` router + BUILD/CHANGE/DIAGNOSE + `routing.json` | **new** (`A` at `1bddcee`) | All three (the procedures) | Installed under `.agents/skills/ikaros/` in projects; agent source does not quote them | not deleted | **keep** |
| `ikaros skill install` | **new** (`typescript/src/skill.ts`) | Install path in `SKILL.md:8`; not a journey step | none | — | **keep** (packaging) |
| Typed effect helper `recordEffectOutcome` / `record_effect_outcome` | **new** | BUILD 8/16 (effects); CHANGE/DIAGNOSE recovery of pending intents | **yes** 10/12 apps; `N9/r1` omitted it | — | **keep** |
| `Aggregate.owner` / `Owner()` | **new** | **CHANGE 5a only** | **yes** all 6 change apps; 0/6 builds | — | **keep** |
| Error taxonomy `IK_STORE_NOT_FOUND` + `StoreNotFoundError` + `classify()` | **new** | BUILD 8 (must map by class/code); DIAGNOSE classification | **yes** classify 12/12; StoreNotFoundError 10/12 | — | **keep** |
| `createWith` / `create_with` / `CreateWith` | **new** | BUILD 8 (atomic admit); not named as a CLI | **yes** 12/12 | — | **keep** |
| `ik handoff` | **new** (`src/handoff.rs`) | BUILD 16, CHANGE 10, DIAGNOSE 13 | **none** | D1: `tests/handoff.rs` (6 tests) + skill citation test; journey last-step citation renderer gone; templates remain | **delete-candidate**: overlaps `SKILL.md` handoff templates; agents never invoked it; USAGE already omits it |
| `ik corpus init` | **new** (`src/cli.rs` `run_corpus`, `src/declarative/corpus.rs`) | BUILD 10 | none as command; all projects have `*.cases.json` (hand-written or kit-copied possible) | D3: `corpus_init_emits_an_editable_example_and_int_boundaries`; NEXT text still names it | **merge** into `ik check --corpus-template` (`docs/ik-check.md:59-60`) |
| `ik check` NEXT / `next_actions` | **new** (`emit_check` `src/cli.rs:1301-1314`) | BUILD 10 only | none | D2: `uncovered_refusal_names_the_corpus_commands`; `corpus_less_check_refusal_names_the_next_command_without_changing_exit` | **delete-candidate**: UX duplicate of docs; agents wrote corpora without it; `parse_check_args` already 29/30 after this track |
| `ik check --corpus-template` | **new** (same change) | BUILD 10 (docs pair it with corpus init) | none | not deleted (D3 kept it) | **keep** one starter writer |
| `ikaros codegen` (TS “projection”) | **wired** (existed `1bddcee^:typescript/src/cli.ts:190`) | BUILD 7, CHANGE 5; N/A for Python | **none** (all TS apps hand-wrote `.mjs`/`.js`) | — | **keep** as optional TS generator; not a DFY-new surface |
| `ikaros trace` | **wired** (existed `1bddcee^` `traceCommand`) | BUILD 14; DIAGNOSE last line; not CHANGE | **none** | not in the five; would leave `typescript/src/trace.test.ts` if only un-wired from skill | **delete-candidate** *from the journeys* (advisory HTML; overlaps `ik report --html`); command itself is pre-DFY |
| `ik scenario run`/`reproduce`, `ik report` | **wired** (pre-existed; scenario `seam_fired` + report `expectation_failed` **changed** in this range) | BUILD 11–13; CHANGE 7,10; DIAGNOSE 6,10 | no command strings; scenario JSON present in every project | — | **keep** |
| `ik check --profile` / `--plan` / `--prior` | **wired** | BUILD 9–10; CHANGE 6; DIAGNOSE 7 | no command strings; `.ik/checks.toml` + `*.cases.json` present | — | **keep** |
| `ik profile diff`, `ik evidence verify`, `ik regression` | **wired** | CHANGE 3,8,9; DIAGNOSE 10–11 (verify/regression) | **none** | — | **keep** for CHANGE continuity evidence; unused in b9 artifacts |
| `ik identity`/`laws`/`schema`, `ikaros doctor`, `ikaros init` | **wired** (init extra files **added**) | Discover; BUILD 5–6 | none | — | **keep** Discover |
| `lookupOperation` | **wired** (client existed; DIAGNOSE names it) | BUILD client-recovery note; DIAGNOSE 3 | **none** in app source; diagnose used HTTP GET + `./run status` | — | **keep** (kernel attribution); b9 agents bypassed it |
| `ik analysis describe/run/validate-result` | **wired** | optional all three; DIAGNOSE `investigate_only` forbids `run` | **none** | — | **keep** optional |
| `ik analysis conformance` + `adapters/starter/` | **new** | optional BUILD 15 only | **none** | D4: eight `tests/adapter_starter.rs` conformance tests; no native check/scenario break | **delete-candidate** vs DFY journeys (third-party adapter authoring, not BUILD/CHANGE/DIAGNOSE) |
| `adapters/p` prepared route | **new** | **none** | **none** | D5: `p_describe_reports_tool_unavailable_and_executes_nothing` only | **delete-candidate** (explicitly not qualified; silent extra vs journeys) |
| TLC adapter + `ik graph export` | **wired** (TLC existed; DFY diff is `os.path.expanduser` on java/jar in `adapters/tlc/ik_tlc_adapter.py:93-97`) | optional specialist (`SKILL.md:38-45`); `routing.json` has graph-export only | **none** | — | **keep** optional; do not treat DFY as having added TLC |
| Hegel `examples/ik_hegel_adapter.rs` | **wired** (existed) | optional specialist | **none** | — | **keep** optional |

## Complexity hot spots

Repo floors from `.<gate-config>.json`: Python `max_complexity` 11; Rust `max_complexity` 30 (`tqs-complexity-v1`, calibrated). Measured with `tqs evidence complexity --authority audit` on DFY-touched production files: `src/handoff.rs`, `src/declarative/corpus.rs`, `src/analysis/conformance.rs`, `src/cli.rs`, `src/analysis/cli.rs`, `python/src/invariant_kernel/{client,errors}.py`, `adapters/starter/adapter.py`, `adapters/p/adapter.py`, `adapters/tlc/ik_tlc_adapter.py`.

| Function | Score | Threshold | Introduced by this range? |
|---|---|---|---|
| `adapters/tlc/ik_tlc_adapter.py::target_nodes` | **13** | Python 11 | **No.** Parent `1bddcee^` already 13 at line 675. DFY diff is expanduser on toolchain paths, not this function. |
| `src/cli.rs::parse_check_args` | 29 | Rust 30 | Grew 21 → 29 (parent line 966 → HEAD 1103, 36 → 53 lines) while adding `--corpus-template`. Under ceiling. |
| `src/cli.rs::emit_check` | 24 | 30 | Grew 14 → 24 with NEXT/`next_actions`. |
| `src/handoff.rs::parse_args` | 23 | 30 | New; under ceiling. |
| `src/cli.rs::run_corpus_inner` | 20 | 30 | New; under ceiling. |
| `src/analysis/conformance.rs::judge_case` | 14 | 30 | New; under ceiling. |
| `python/…/errors.py::_classify_error` | 9 | 11 | New-ish taxonomy; under ceiling. |
| `python/…/client.py::{_call_context,reducer}` | 11 | 11 | At ceiling; not shown as new vs parent in this pass. |

No **new** function in this range scored above the repo threshold. TypeScript (`client.ts`, `errors.ts`, `cli.ts`) **not measured** (tqs run was Rust/Python only). Go `Owner()` / `classify` **not measured**.

## Scope disposition (skill `scope`)

Relative to “agents BUILD/CHANGE/DIAGNOSE with Ikaros”:

- Client APIs (effect helper, owner, classify/StoreNotFound, createWith) and the skill router: **IN_ASK**. b9 source confirms they were the surfaces agents actually bound to.
- Workbench CLI named by the guides (`check`/`scenario`/`report`/`handoff`/trace/analysis): **IN_ASK** as disclosed procedure, even where b9 left no invocation evidence.
- `adapters/p` and `ik analysis conformance`/`adapters/starter`: **SILENT_EXPAND** vs the three journeys (optional BUILD 15 is adapter-authoring, not delivering an app). No leftover/snag in the inspected trees records them as separate work.
- `ik corpus init` vs `--corpus-template`, `ik handoff` vs `SKILL.md` templates, `ikaros trace` vs `ik report --html`: overlapping IN_ASK surfaces, not extra jobs.

## Not inspected

- Evaluator JSON / sealed truth (assignment: do not read evaluators).
- Agent session transcripts; cannot prove `ik check`/`ik scenario` were or were not run—only that delivered README/source/tests never mention them.
- TypeScript/Go cognitive complexity.
- Whether dirty `skills/ikaros/build.md` on the inspect worktree matches the SHA agents received in b9 (projects carry their own `.agents/skills/ikaros/` copies).
