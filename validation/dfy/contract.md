# Cold qualification of agent-delivered lifecycle engineering: contract

Status: prepared; results are recorded separately under `results/` and `decision.md` once executed. Nothing in
this directory is a human review, a platform qualification, a consumer receipt or a publication clearance.

## Question
From installed packages and shipped guidance only, can a fresh coding agent (1) build a useful, correctly
recovering lifecycle application, (2) change it without eroding unchanged requirements, and (3) diagnose and, when
authorized, safely recover an incident, with the human limited to behavior-level decisions? No comparison with
any other toolchain is claimed or supported by this floor.

## Families, stages, replicas
Three new project families (briefs authored by contexts that had not seen the candidate): two TypeScript (N1 order
cancellation and refund desk; N2 subscription plan-change scheduler) and one Python (N3 equipment loan desk with
notifications). Two replicas per family; the language is fixed within a family. Stages per replica, each in a
fresh context: BUILD, CHANGE (applied to that replica's own submitted build; labeled rescue-and-change if the build
failed an original requirement), DIAGNOSE/RECOVER on a seeded incident (investigate-only and authorized-recovery
variants, plus a control incident whose correct diagnosis is no incident). Go keeps its existing compatibility
path; no new Go claim.

## Inputs and protections
Frozen candidate: see `batch-001/MANIFEST.md` (artifact identities, source revision). Builders receive: the
installed packages (npm tarballs or Python wheel plus the packed `ik` binary), the shipped guide, the installed skill
(`ikaros skill install`), the family brief, and the fake service. They do not receive the evaluator, the oracle
authors' schedules, other runs, the earlier experiment, or this contract. Briefs, change requests, stubs,
evaluators and diagnose checkers are read-only to builders and maintainers. Oracle authors saw no candidate source.
A source-aware reviewer is a separate, labeled role.

## Outcomes (separate, never combined)
Build pass; change pass (maintenance / rescue-and-change / not assessable); diagnose correctness and safety per
variant (classification, read-only compliance in investigate-only mode, exactly-one external effect and no duplicate
request in authorized recovery, no over-diagnosis on the control incident); complete trajectory; unsafe persisted
outcomes or unauthorized effect requests; legitimate operations wrongly refused; human interventions; resource use
where measurable. Unknown stays unknown. Every attempt, failure and rescue is recorded.

## Validity
Run invalid only for predefined procedural defects (unequal access, leaked evaluator, broken fixture, a builder
reading protected material). Product or builder failures remain results. Defect controls were run against every
evaluator before dispatch (see `projects/*/NOTES.md`).

## Readiness records
EngineeringReady, DFYJourneyQualified and PublicReleaseReady are separate. This floor can inform only the second.
