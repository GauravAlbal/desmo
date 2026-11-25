# IK-14 greenfield tranche 2 contract

Status: authored fixture set; no builders dispatched and no comparison results exist. This tranche is additional evidence, not pooled with tranche 1 without review.

## Scope and topology
Three bounded local lifecycles: G4 equipment reservation, G5 finite inspection schedule, G6 account notification. Each has one persistent local store, restart, concurrent local processes, and a supplied deterministic fake external service. G5 has a finite count; all three include uncertain external outcomes; G4/G5/G6 include terminal boundaries and G5 explicit crash points. This does not establish generality beyond the fixtures.

## Arms and parity
Arm A is a competent native single-store transactional implementation: atomic command transactions, per-aggregate currentness check, idempotent operation record, unique outbox/pending-effect record, and audit/replay consistency check. Arm B uses the frozen Desmo candidate for lifecycle state transitions, with ordinary libraries allowed elsewhere. Both arms receive identical brief, change request, public stub contract, runtime and time budget. Neither may edit briefs, evaluator, controls, or stub. The runtime owns transitions only; it does not establish criteria validity or execution fidelity.

## Role separation and evaluator secrecy
Builders receive only staged `BRIEF.md`, `CHANGE.md` (during maintenance), and `stub/CONTRACT.md` plus `stub/server.mjs`; they MUST NOT receive `evaluator/`, `controls/`, harness sources, or control-route descriptions. Evaluator control routes MUST live in a separate harness process/source tree outside the project staging copy. The public service contract and service server MUST expose only lifecycle API; fault injection/control API is hosted by an evaluator-owned sidecar bound to loopback and is configured by evaluator environment, not embedded in stub source. Control routes are not documented in builder-visible files. Verify isolation mechanically before dispatch: stage each project into an empty workspace, inventory all staged paths, search staged text for control-route names and control port, then confirm all evaluator/control files remain outside staged tree. Record command output in `validation.log`. Do not use prior tranche's `stage.py` unchanged: it copies stub source to the builder and thereby exposes any embedded evaluator routes.

## Independent parity review
For each G4-G6, a reviewer must compare the A and B brief inputs byte-for-byte, confirm every R requirement has a distinct evaluator assertion and corresponding per-requirement defect control, confirm change requirements and original requirements are both exercised after maintenance, and inspect both service call transcripts for identical fault schedules. The reviewer checks baseline competence checklist above before outcomes are read. Any unresolved mismatch blocks dispatch.

## Validation protocol
Run evaluator against (a) one reference-correct public-surface stub per arm and (b) one deliberately defective stub per requirement. Each defect control must fail exactly its mapped requirement and pass all other requirement assertions. Controls are isolated by requirement; no mutation may accidentally trigger a second requirement. Log command, exit status and per-requirement outcome in `validation.log`. No arm implementation is an evaluator control. No builder is dispatched until controls are green.

## Outcomes and validity
Report initial pass, post-change pass/rescue/not assessable, trajectory, unsafe outcomes, unauthorized effect requests, false refusals, uncertainty reconciliation, interventions and costs separately. Keep all attempts and failures. Invalidate only for unequal inputs/budget, evaluator leakage, broken common fixture, or protected-material access. A single tranche is descriptive; no superiority claim follows.
