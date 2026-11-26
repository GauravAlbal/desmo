//! Declarative machine support: inspectable `.ik.toml` definitions, a bounded guard
//! grammar, strict static checks, a compiled [`Machine`] implementation, and
//! derived developer-facing analysis (explain / graph / explore / diff) and a
//! verification-plan compiler.

pub mod analyze;
pub mod challenge;
pub mod chart;
pub mod check;
pub mod compat;
pub mod compile;
pub mod corpus;
pub mod counterexample;
pub mod def;
pub mod diff;
pub mod evaluate;
pub mod evidence;
pub mod explain;
pub mod explore;
pub mod gaps;
pub mod expr;
mod feasibility;
pub mod lint;
pub mod local;
pub mod mutation;
pub mod plan;
pub mod profile;
pub mod region;
pub mod release;
pub mod report;
pub mod schema;
pub mod seed;
mod text_in;
pub mod points;
pub mod witness;

pub use analyze::{analyze, machine_graph, MachineGraph, StructuralAnalysis};
pub use check::{check_machine, standard_lane, CheckResult};
pub use compile::{
    load_def, load_definition, parse_and_compile, parse_definition, CompiledMachine,
};
pub use corpus::{starter_corpus, STARTER_NOTE};
pub use counterexample::{
    classify_replay, CounterexampleV1, INVALID_NONDETERMINISTIC_COUNTEREXAMPLE,
};
pub use chart::{
    analyze_chart, check_consistent, drop_first_edge, Chart, ChartAnalytics, ChartEdge, ChartError,
    ChartState, EdgeKey, EdgeProvenance, GroupChart, JointChart, RegionChart,
};
pub use def::{Definition, StaticErrors};
pub use diff::{diff_machines, MachineDiff};
pub use evaluate::{GuardEvaluation, OutcomeSelection};
pub use evidence::{
    discharge_exploration, discharge_graph, discharge_hollow, transition_evidence,
    ObligationDischarge, TransitionEvidenceV1,
};
pub use explain::{explain, Explanation};
pub use explore::{explore, Bounds, CaseCorpus, Exploration};
pub use lint::{render_lints, structural_lints, StructuralLint};
pub use local::{change_local_guards, LocalAdequacy, LocalNeighbor};
pub use seed::{auto_seed_corpus, smart_starter_corpus, AUTO_SEED_NOTE};
pub use mutation::{
    curated_stale_capability_killed, mutation_report, run_guard_boundary_mutant, MutationReport,
    MutationResult,
};
pub use plan::{
    plan_diff, verification_plan, AuthoredGoalV1, CoordinateDelta, ImpossibilityRecordV1,
    SemanticChallengeV1, VerificationObligationV1, VerificationPlanDiff, VerificationPlanV1,
    PLAN_SCHEMA_BASE, PLAN_SCHEMA_VERSION,
};
pub use release::{ObligationStatus, ReleaseEvidence, ReleaseJudgment};
pub use report::{
    judge_acceptance, report_from_evidence, verification_report, AcceptJudgment, ChallengeCounts,
    ObligationCounts, VerificationReportV1,
};
