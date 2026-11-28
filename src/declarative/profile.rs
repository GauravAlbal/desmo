//! Versioned check profiles: useful-path goals judged from an existing exploration.
//!
//! A profile does not change machine semantics or `spec_hash`. It only names questions
//! and the bounds used to interpret the exploration the check already ran.

use super::analyze::{machine_graph, MachineGraph};
use super::def::Definition;
use super::explore::{Bounds, CaseCorpus, Exploration, WitnessStep};
use crate::canonical::digest_json;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

pub const DEFAULT_MAX_NODES: usize = 4096;
pub const DEFAULT_MAX_DEPTH: usize = 16;
pub const PROFILE_SCHEMA: &str = "ik.check.profile.v1";
pub const PLAN_SCHEMA: &str = "ik.check.plan.v1";

const WITNESS_FOUND: &str = "witness_found";
const NO_WITNESS_COMPLETE: &str = "no_witness_found_complete";
const NO_WITNESS_INCOMPLETE: &str = "no_witness_found_incomplete";
const INPUT_COVERAGE_GAP: &str = "input_coverage_gap";
const STUCK_FOUND: &str = "stuck_states_found";
const NO_STUCK: &str = "no_stuck_states_found";
const UNREACHABLE: &str = "unreachable within the supplied corpus";
const NO_CORPUS: &str = "no_corpus_case";
const ALL_REFUSED: &str = "all_cases_refused";
const NO_OUTGOING: &str = "no_outgoing_command";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub profile_version: u64,
    pub limits: Bounds,
    pub boundary: Option<String>,
    pub goals: Vec<Goal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Goal {
    pub id: String,
    pub required: bool,
    pub kind: GoalKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoalKind {
    ReachTerminal { terminal: String },
    ReachState { state: String },
    CommandExecutes { command: String },
    NoStuckStates { allow_waiting: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CheckPlan {
    pub schema: &'static str,
    pub profile_digest: String,
    pub machine_spec_hash: String,
    pub corpus_digest: String,
    pub bounds: Bounds,
    pub goals: Vec<PlanGoal>,
    pub scope: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlanGoal {
    pub id: String,
    pub kind: String,
    pub required: bool,
    pub content_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProfileEnvelope {
    pub schema: &'static str,
    pub profile_digest: String,
    pub machine_spec_hash: String,
    pub corpus_digest: String,
    pub bounds: Bounds,
    pub search_complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete_reason: Option<String>,
    pub goals: Vec<GoalResult>,
    /// Prior-machine evaluation of the same goals. Absent unless both a profile and a prior were supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<ProfileBaseline>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GoalResult {
    pub id: String,
    pub kind: String,
    pub required: bool,
    pub content_digest: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub witness: Option<Vec<WitnessStep>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gaps: Option<Vec<CommandGap>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stuck_states: Option<Vec<StuckState>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proven_deadlock: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommandGap {
    pub command: String,
    pub classification: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StuckState {
    pub state: String,
    pub classification: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    profile_version: u64,
    #[serde(default)]
    limits: LimitsFile,
    #[serde(default)]
    goals: Vec<GoalFile>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct LimitsFile {
    max_nodes: Option<usize>,
    max_depth: Option<usize>,
    boundary: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoalFile {
    id: String,
    kind: String,
    required: Option<bool>,
    terminal: Option<String>,
    state: Option<String>,
    command: Option<String>,
    allow_waiting: Option<Vec<String>>,
}

#[derive(Serialize)]
struct ProfileDocument {
    profile_version: u64,
    limits: Bounds,
    #[serde(skip_serializing_if = "Option::is_none")]
    boundary: Option<String>,
    goals: Vec<GoalDocument>,
}

#[derive(Serialize)]
struct GoalDocument {
    id: String,
    kind: String,
    required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    terminal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allow_waiting: Option<Vec<String>>,
}

pub fn load_profile(path: &Path) -> Result<Profile, String> {
    let text =
        std::fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    parse_profile(&text).map_err(|err| format!("profile {}: {err}", path.display()))
}

pub fn parse_profile(text: &str) -> Result<Profile, String> {
    let file: ProfileFile = toml::from_str(text).map_err(|err| err.to_string())?;
    if file.profile_version != 1 {
        return Err(format!(
            "unsupported profile_version {}; expected 1",
            file.profile_version
        ));
    }
    let mut seen = BTreeSet::new();
    let mut goals = Vec::new();
    for raw in file.goals {
        goals.push(parse_goal(raw, &mut seen)?);
    }
    Ok(Profile {
        profile_version: 1,
        limits: Bounds {
            max_nodes: file.limits.max_nodes.unwrap_or(DEFAULT_MAX_NODES),
            max_depth: file.limits.max_depth.unwrap_or(DEFAULT_MAX_DEPTH),
        },
        boundary: file.limits.boundary,
        goals,
    })
}

pub fn bind_profile(profile: &Profile, def: &Definition) -> Result<(), String> {
    if let Some(boundary) = &profile.boundary {
        super::def::validate_state_expr(def, boundary, "profile limits.boundary")?;
    }
    for goal in &profile.goals {
        bind_goal(goal, def)?;
    }
    Ok(())
}

pub fn plan(profile: &Profile, def: &Definition, corpus: &CaseCorpus) -> CheckPlan {
    CheckPlan {
        schema: PLAN_SCHEMA,
        profile_digest: profile_digest(profile),
        machine_spec_hash: digest_json(def),
        corpus_digest: corpus.digest(),
        bounds: profile.limits,
        goals: profile.goals.iter().map(plan_goal).collect(),
        scope: scope_sentence(profile.limits),
    }
}

pub fn evaluate(
    profile: &Profile,
    def: &Definition,
    corpus: &CaseCorpus,
    run: &Exploration,
) -> ProfileEnvelope {
    let graph = machine_graph(def);
    let plan = super::plan::verification_plan(def);
    let goals = profile
        .goals
        .iter()
        .map(|goal| {
            let mut result = evaluate_goal(goal, def, corpus, run, &graph);
            attach_impossibility_proof(&mut result, goal, &plan);
            result
        })
        .collect();
    ProfileEnvelope {
        schema: PROFILE_SCHEMA,
        profile_digest: profile_digest(profile),
        machine_spec_hash: run.spec_hash.clone(),
        corpus_digest: run.case_corpus_digest.clone(),
        bounds: run.bound,
        search_complete: run.complete,
        incomplete_reason: run.incomplete_reason.clone(),
        goals,
        baseline: None,
    }
}

pub fn required_goals_satisfied(envelope: &ProfileEnvelope) -> bool {
    envelope
        .goals
        .iter()
        .filter(|goal| goal.required)
        .all(|goal| goal_satisfied(goal, envelope.search_complete))
}

pub fn scope_sentence(bounds: Bounds) -> String {
    format!(
        "search up to {} nodes / depth {} over the supplied corpus; a witness proves reachability, absence of a witness does not prove unreachability unless the search is complete",
        bounds.max_nodes, bounds.max_depth
    )
}

pub fn render_plan(plan: &CheckPlan) -> String {
    let mut text = String::from("PLAN\n");
    text.push_str(&format!("  scope: {}\n", plan.scope));
    text.push_str(&format!(
        "  limits: max_nodes={} max_depth={}\n",
        plan.bounds.max_nodes, plan.bounds.max_depth
    ));
    text.push_str(&format!("  corpus_digest: {}\n", plan.corpus_digest));
    text.push_str(&format!("  profile_digest: {}\n", plan.profile_digest));
    text.push_str(&format!(
        "  machine_spec_hash: {}\n",
        plan.machine_spec_hash
    ));
    for goal in &plan.goals {
        text.push_str(&format!(
            "  goal {} kind={} required={} content_digest={}\n",
            goal.id, goal.kind, goal.required, goal.content_digest
        ));
    }
    text
}

pub fn render_profile(envelope: &ProfileEnvelope) -> String {
    let mut text = String::from("PROFILE\n");
    text.push_str(&format!("  profile_digest {}\n", envelope.profile_digest));
    text.push_str(&format!(
        "  machine_spec_hash {}\n",
        envelope.machine_spec_hash
    ));
    text.push_str(&format!("  corpus_digest {}\n", envelope.corpus_digest));
    text.push_str(&format!(
        "  bounds max_nodes={} max_depth={}\n",
        envelope.bounds.max_nodes, envelope.bounds.max_depth
    ));
    text.push_str(&format!("  search_complete {}\n", envelope.search_complete));
    for goal in &envelope.goals {
        append_goal(&mut text, goal);
    }
    if let Some(baseline) = &envelope.baseline {
        append_baseline(&mut text, baseline);
    }
    text
}

fn append_goal(text: &mut String, goal: &GoalResult) {
    text.push_str(&format!(
        "  goal {} {} required={} content_digest={}\n",
        goal.id, goal.status, goal.required, goal.content_digest
    ));
    if let Some(detail) = &goal.detail {
        text.push_str(&format!("    {detail}\n"));
    }
    if let Some(steps) = &goal.witness {
        text.push_str(&format!("    witness: {}\n", format_steps(steps)));
    }
    if let Some(gaps) = &goal.gaps {
        for gap in gaps {
            text.push_str(&format!(
                "    command {} classification={}\n",
                gap.command, gap.classification
            ));
        }
    }
    if let Some(stuck) = &goal.stuck_states {
        for state in stuck {
            text.push_str(&format!(
                "    state {} classification={}\n",
                state.state, state.classification
            ));
        }
    }
    if goal.proven_deadlock == Some(false) {
        text.push_str("    not a proven deadlock\n");
    }
}

fn format_steps(steps: &[WitnessStep]) -> String {
    if steps.is_empty() {
        return "(initial)".to_string();
    }
    steps
        .iter()
        .map(|step| match step.case.as_deref() {
            Some(case) => format!("{}[{case}]", step.command),
            None => step.command.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn goal_satisfied(goal: &GoalResult, search_complete: bool) -> bool {
    match goal.status.as_str() {
        WITNESS_FOUND => true,
        NO_STUCK => search_complete,
        _ => false,
    }
}

fn parse_goal(raw: GoalFile, seen: &mut BTreeSet<String>) -> Result<Goal, String> {
    if raw.id.is_empty() {
        return Err("goal id must be a non-empty string".into());
    }
    if !seen.insert(raw.id.clone()) {
        return Err(format!("duplicate goal id `{}`", raw.id));
    }
    let kind = parse_kind(&raw)?;
    Ok(Goal {
        id: raw.id,
        required: raw.required.unwrap_or(true),
        kind,
    })
}

fn parse_kind(raw: &GoalFile) -> Result<GoalKind, String> {
    match raw.kind.as_str() {
        "reach_terminal" => reach_terminal(raw),
        "reach_state" => reach_state(raw),
        "command_executes" => command_executes(raw),
        "no_stuck_states" => no_stuck_states(raw),
        other => Err(format!("unknown goal kind `{other}`")),
    }
}

fn reach_terminal(raw: &GoalFile) -> Result<GoalKind, String> {
    reject_field(raw.state.is_some(), "state", &raw.kind, &raw.id)?;
    reject_field(raw.command.is_some(), "command", &raw.kind, &raw.id)?;
    reject_field(
        raw.allow_waiting.is_some(),
        "allow_waiting",
        &raw.kind,
        &raw.id,
    )?;
    let terminal = required_field(raw.terminal.as_deref(), "terminal", raw)?;
    Ok(GoalKind::ReachTerminal {
        terminal: terminal.to_string(),
    })
}

fn reach_state(raw: &GoalFile) -> Result<GoalKind, String> {
    reject_field(raw.terminal.is_some(), "terminal", &raw.kind, &raw.id)?;
    reject_field(raw.command.is_some(), "command", &raw.kind, &raw.id)?;
    reject_field(
        raw.allow_waiting.is_some(),
        "allow_waiting",
        &raw.kind,
        &raw.id,
    )?;
    let state = required_field(raw.state.as_deref(), "state", raw)?;
    Ok(GoalKind::ReachState {
        state: state.to_string(),
    })
}

fn command_executes(raw: &GoalFile) -> Result<GoalKind, String> {
    reject_field(raw.terminal.is_some(), "terminal", &raw.kind, &raw.id)?;
    reject_field(raw.state.is_some(), "state", &raw.kind, &raw.id)?;
    reject_field(
        raw.allow_waiting.is_some(),
        "allow_waiting",
        &raw.kind,
        &raw.id,
    )?;
    let command = required_field(raw.command.as_deref(), "command", raw)?;
    Ok(GoalKind::CommandExecutes {
        command: command.to_string(),
    })
}

fn no_stuck_states(raw: &GoalFile) -> Result<GoalKind, String> {
    reject_field(raw.terminal.is_some(), "terminal", &raw.kind, &raw.id)?;
    reject_field(raw.state.is_some(), "state", &raw.kind, &raw.id)?;
    reject_field(raw.command.is_some(), "command", &raw.kind, &raw.id)?;
    Ok(GoalKind::NoStuckStates {
        allow_waiting: raw.allow_waiting.clone().unwrap_or_default(),
    })
}

fn required_field<'a>(
    value: Option<&'a str>,
    field: &str,
    raw: &GoalFile,
) -> Result<&'a str, String> {
    value.ok_or_else(|| format!("goal `{}` kind {} requires `{field}`", raw.id, raw.kind))
}

fn reject_field(present: bool, field: &str, kind: &str, id: &str) -> Result<(), String> {
    if present {
        Err(format!(
            "goal `{id}`: field `{field}` is not valid for kind `{kind}`"
        ))
    } else {
        Ok(())
    }
}

fn bind_goal(goal: &Goal, def: &Definition) -> Result<(), String> {
    match &goal.kind {
        GoalKind::ReachTerminal { terminal } => require_terminal(def, &goal.id, terminal),
        GoalKind::ReachState { state } => require_state(def, &goal.id, state),
        GoalKind::CommandExecutes { command } => require_command(def, &goal.id, command),
        GoalKind::NoStuckStates { allow_waiting } => {
            for state in allow_waiting {
                require_state(def, &goal.id, state)?;
            }
            Ok(())
        }
    }
}

fn require_state(def: &Definition, id: &str, name: &str) -> Result<(), String> {
    if def.states.contains_key(name) {
        Ok(())
    } else {
        Err(format!("goal `{id}` references undeclared state `{name}`"))
    }
}

fn require_terminal(def: &Definition, id: &str, name: &str) -> Result<(), String> {
    match def.states.get(name) {
        Some(state) if state.terminal => Ok(()),
        Some(_) => Err(format!(
            "goal `{id}` references undeclared terminal `{name}`"
        )),
        None => Err(format!("goal `{id}` references undeclared state `{name}`")),
    }
}

fn require_command(def: &Definition, id: &str, name: &str) -> Result<(), String> {
    if def.commands.contains_key(name) {
        Ok(())
    } else {
        Err(format!(
            "goal `{id}` references undeclared command `{name}`"
        ))
    }
}

fn attach_impossibility_proof(
    result: &mut GoalResult,
    goal: &Goal,
    plan: &super::plan::VerificationPlanV1,
) {
    if result.status == WITNESS_FOUND {
        return;
    }
    let Some(row) = matching_impossibility(goal, plan) else {
        return;
    };
    let proof = format!(
        "proved impossible ({}); basis {}; scope {}",
        row.class, row.basis, row.scope
    );
    result.detail = Some(match result.detail.take() {
        Some(existing) => format!("{existing}; {proof}"),
        None => proof,
    });
    result.status = NO_WITNESS_COMPLETE.to_string();
    result.witness = None;
}

fn matching_impossibility<'a>(
    goal: &Goal,
    plan: &'a super::plan::VerificationPlanV1,
) -> Option<&'a super::plan::ImpossibilityRecordV1> {
    let target = explicit_goal_target(goal)?;
    plan.impossibilities
        .iter()
        .find(|row| row.obligation_id == target)
}

fn explicit_goal_target(goal: &Goal) -> Option<String> {
    match &goal.kind {
        GoalKind::ReachTerminal { terminal } => Some(format!("terminal/{terminal}/reachable")),
        GoalKind::ReachState { state } => Some(format!("state/{state}/executable")),
        GoalKind::CommandExecutes { .. } | GoalKind::NoStuckStates { .. } => None,
    }
}

fn evaluate_goal(
    goal: &Goal,
    def: &Definition,
    corpus: &CaseCorpus,
    run: &Exploration,
    graph: &MachineGraph,
) -> GoalResult {
    match &goal.kind {
        GoalKind::ReachTerminal { terminal } => reachability(
            goal,
            run,
            witness_of(&run.witnesses.by_terminal, terminal),
            commands_on_paths_to(graph, &BTreeSet::from([terminal.clone()])),
            def,
            corpus,
        ),
        GoalKind::ReachState { state } => reachability(
            goal,
            run,
            witness_of(&run.witnesses.by_state_label, state),
            commands_on_paths_to(graph, &BTreeSet::from([state.clone()])),
            def,
            corpus,
        ),
        GoalKind::CommandExecutes { command } => {
            let mut needed = commands_on_paths_to(graph, &command_sources(graph, command));
            needed.insert(command.clone());
            reachability(
                goal,
                run,
                witness_of(&run.witnesses.by_command, command),
                needed,
                def,
                corpus,
            )
        }
        GoalKind::NoStuckStates { allow_waiting } => {
            stuck_result(goal, def, corpus, run, graph, allow_waiting)
        }
    }
}

fn reachability(
    goal: &Goal,
    run: &Exploration,
    witness: Option<&Vec<WitnessStep>>,
    needed: BTreeSet<String>,
    def: &Definition,
    corpus: &CaseCorpus,
) -> GoalResult {
    if let Some(steps) = witness {
        return finish(
            goal,
            WITNESS_FOUND,
            Some(steps.clone()),
            None,
            None,
            None,
            None,
        );
    }
    let gaps = coverage_gaps(&needed, def, corpus, run);
    if !gaps.is_empty() {
        return finish(
            goal,
            INPUT_COVERAGE_GAP,
            None,
            Some(
                "input coverage gap: a command needed on a path has no corpus case or every supplied case was refused"
                    .into(),
            ),
            Some(gaps),
            None,
            None,
        );
    }
    if !run.complete {
        let reason = run.incomplete_reason.as_deref().unwrap_or("bounds");
        return finish(
            goal,
            NO_WITNESS_INCOMPLETE,
            None,
            Some(format!(
                "search truncated by bounds ({reason}); absence of a witness is not unreachability"
            )),
            None,
            None,
            None,
        );
    }
    finish(
        goal,
        NO_WITNESS_COMPLETE,
        None,
        Some(UNREACHABLE.into()),
        None,
        None,
        None,
    )
}

fn stuck_result(
    goal: &Goal,
    def: &Definition,
    corpus: &CaseCorpus,
    run: &Exploration,
    graph: &MachineGraph,
    allow_waiting: &[String],
) -> GoalResult {
    let stuck = stuck_candidates(def, corpus, run, graph, allow_waiting);
    if stuck.is_empty() {
        return finish(
            goal,
            NO_STUCK,
            None,
            Some(
                "no stuck states found among executably reached states; a terminal state with no outgoing command is not stuck"
                    .into(),
            ),
            None,
            None,
            None,
        );
    }
    let coverage_limited = !run.complete
        || stuck
            .iter()
            .any(|state| state.classification == NO_CORPUS || state.classification == ALL_REFUSED);
    let detail = if coverage_limited {
        Some(
            "stuck-state candidates are not a proven deadlock when the search is incomplete or the corpus is missing cases"
                .into(),
        )
    } else {
        Some("reached non-terminal state has no declared outgoing command".into())
    };
    finish(
        goal,
        STUCK_FOUND,
        None,
        detail,
        None,
        Some(stuck),
        Some(false),
    )
}

fn finish(
    goal: &Goal,
    status: &str,
    witness: Option<Vec<WitnessStep>>,
    detail: Option<String>,
    gaps: Option<Vec<CommandGap>>,
    stuck_states: Option<Vec<StuckState>>,
    proven_deadlock: Option<bool>,
) -> GoalResult {
    GoalResult {
        id: goal.id.clone(),
        kind: kind_name(&goal.kind).to_string(),
        required: goal.required,
        content_digest: content_digest(goal),
        status: status.to_string(),
        witness,
        detail,
        gaps,
        stuck_states,
        proven_deadlock,
    }
}

fn coverage_gaps(
    needed: &BTreeSet<String>,
    def: &Definition,
    corpus: &CaseCorpus,
    run: &Exploration,
) -> Vec<CommandGap> {
    let graph = machine_graph(def);
    let mut gaps = Vec::new();
    for command in needed {
        if let Some(classification) = command_gap(command, def, corpus, run, &graph) {
            gaps.push(CommandGap {
                command: command.clone(),
                classification,
            });
        }
    }
    gaps
}

fn command_gap(
    command: &str,
    def: &Definition,
    corpus: &CaseCorpus,
    run: &Exploration,
    graph: &MachineGraph,
) -> Option<String> {
    if needs_args(def, command) && case_count(corpus, command) == 0 {
        return Some(NO_CORPUS.to_string());
    }
    if executed_command(run, command) {
        return None;
    }
    let attempted = command_sources(graph, command)
        .iter()
        .any(|source| state_expanded(run, graph, source));
    if attempted {
        Some(ALL_REFUSED.to_string())
    } else {
        None
    }
}

fn stuck_candidates(
    def: &Definition,
    corpus: &CaseCorpus,
    run: &Exploration,
    graph: &MachineGraph,
    allow_waiting: &[String],
) -> Vec<StuckState> {
    let mut found = Vec::new();
    for state in &run.coverage.executably_reached_states {
        if let Some(candidate) = stuck_candidate(state, def, corpus, run, graph, allow_waiting) {
            found.push(candidate);
        }
    }
    found
}

fn stuck_candidate(
    state: &str,
    def: &Definition,
    corpus: &CaseCorpus,
    run: &Exploration,
    graph: &MachineGraph,
    allow_waiting: &[String],
) -> Option<StuckState> {
    if is_terminal(graph, state) || allow_waiting.iter().any(|name| name == state) {
        return None;
    }
    let commands = outgoing_commands(graph, state);
    if commands
        .iter()
        .any(|command| executed_from(run, command, state))
    {
        return None;
    }
    let classification = stuck_classification(&commands, def, corpus, run, graph, state)?;
    Some(StuckState {
        state: state.to_string(),
        classification,
    })
}

fn stuck_classification(
    commands: &[String],
    def: &Definition,
    corpus: &CaseCorpus,
    run: &Exploration,
    graph: &MachineGraph,
    state: &str,
) -> Option<String> {
    if commands.is_empty() {
        return Some(NO_OUTGOING.to_string());
    }
    let missing = commands
        .iter()
        .any(|command| needs_args(def, command) && case_count(corpus, command) == 0);
    let all_missing = commands
        .iter()
        .all(|command| needs_args(def, command) && case_count(corpus, command) == 0);
    if all_missing {
        return Some(NO_CORPUS.to_string());
    }
    if !state_expanded(run, graph, state) {
        return None;
    }
    if missing {
        Some(NO_CORPUS.to_string())
    } else {
        Some(ALL_REFUSED.to_string())
    }
}

fn state_expanded(run: &Exploration, graph: &MachineGraph, state: &str) -> bool {
    if run
        .coverage
        .executed_command_sources
        .iter()
        .any(|row| row.source == state)
    {
        return true;
    }
    if initial_name(graph) == Some(state) && run.bound.max_nodes > 0 && run.nodes > 0 {
        return true;
    }
    run.complete && !is_terminal(graph, state) && reached(run, state)
}

fn executed_command(run: &Exploration, command: &str) -> bool {
    run.coverage
        .executed_commands
        .iter()
        .any(|name| name == command)
}

fn executed_from(run: &Exploration, command: &str, source: &str) -> bool {
    run.coverage
        .executed_command_sources
        .iter()
        .any(|row| row.command == command && row.source == source)
}

fn reached(run: &Exploration, state: &str) -> bool {
    run.coverage
        .executably_reached_states
        .iter()
        .any(|name| name == state)
}

fn needs_args(def: &Definition, command: &str) -> bool {
    def.commands
        .get(command)
        .and_then(|cmd| cmd.args.as_ref())
        .is_some_and(|args| !args.is_empty())
}

fn case_count(corpus: &CaseCorpus, command: &str) -> usize {
    corpus.commands.get(command).map(Vec::len).unwrap_or(0)
}

fn is_terminal(graph: &MachineGraph, state: &str) -> bool {
    graph
        .states
        .iter()
        .any(|item| item.name == state && item.terminal)
}

fn initial_name(graph: &MachineGraph) -> Option<&str> {
    graph
        .states
        .iter()
        .find(|state| state.initial)
        .map(|state| state.name.as_str())
}

fn outgoing_commands(graph: &MachineGraph, state: &str) -> Vec<String> {
    let mut commands = BTreeSet::new();
    for edge in &graph.edges {
        if edge.source == state {
            commands.insert(edge.command.clone());
        }
    }
    commands.into_iter().collect()
}

fn command_sources(graph: &MachineGraph, command: &str) -> BTreeSet<String> {
    graph
        .edges
        .iter()
        .filter(|edge| edge.command == command)
        .map(|edge| edge.source.clone())
        .collect()
}

fn witness_of<'a>(
    book: &'a BTreeMap<String, Vec<WitnessStep>>,
    key: &str,
) -> Option<&'a Vec<WitnessStep>> {
    book.get(key)
}

fn commands_on_paths_to(graph: &MachineGraph, targets: &BTreeSet<String>) -> BTreeSet<String> {
    let forward = forward_states(graph);
    let reverse = reverse_edges(graph);
    let mut commands = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    for target in targets {
        if forward.contains(target) {
            seen.insert(target.clone());
            queue.push_back(target.clone());
        }
    }
    while let Some(state) = queue.pop_front() {
        let Some(edges) = reverse.get(&state) else {
            continue;
        };
        for (source, command) in edges {
            if !forward.contains(source) {
                continue;
            }
            commands.insert(command.clone());
            if seen.insert(source.clone()) {
                queue.push_back(source.clone());
            }
        }
    }
    commands
}

fn forward_states(graph: &MachineGraph) -> BTreeSet<String> {
    let mut next: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in &graph.edges {
        next.entry(edge.source.as_str())
            .or_default()
            .push(edge.target.as_str());
    }
    let Some(initial) = initial_name(graph) else {
        return BTreeSet::new();
    };
    let mut seen = BTreeSet::from([initial.to_string()]);
    let mut queue = VecDeque::from([initial.to_string()]);
    while let Some(state) = queue.pop_front() {
        for target in next.get(state.as_str()).into_iter().flatten() {
            if seen.insert((*target).to_string()) {
                queue.push_back((*target).to_string());
            }
        }
    }
    seen
}

fn reverse_edges(graph: &MachineGraph) -> BTreeMap<String, Vec<(String, String)>> {
    let mut reverse = BTreeMap::new();
    for edge in &graph.edges {
        reverse
            .entry(edge.target.clone())
            .or_insert_with(Vec::new)
            .push((edge.source.clone(), edge.command.clone()));
    }
    reverse
}

fn kind_name(kind: &GoalKind) -> &'static str {
    match kind {
        GoalKind::ReachTerminal { .. } => "reach_terminal",
        GoalKind::ReachState { .. } => "reach_state",
        GoalKind::CommandExecutes { .. } => "command_executes",
        GoalKind::NoStuckStates { .. } => "no_stuck_states",
    }
}

fn plan_goal(goal: &Goal) -> PlanGoal {
    PlanGoal {
        id: goal.id.clone(),
        kind: kind_name(&goal.kind).to_string(),
        required: goal.required,
        content_digest: content_digest(goal),
    }
}

fn profile_digest(profile: &Profile) -> String {
    digest_json(&ProfileDocument {
        profile_version: profile.profile_version,
        limits: profile.limits,
        boundary: profile.boundary.clone(),
        goals: profile.goals.iter().map(goal_document).collect(),
    })
}

fn content_digest(goal: &Goal) -> String {
    digest_json(&goal_document(goal))
}

fn goal_document(goal: &Goal) -> GoalDocument {
    let (terminal, state, command, allow_waiting) = match &goal.kind {
        GoalKind::ReachTerminal { terminal } => (Some(terminal.clone()), None, None, None),
        GoalKind::ReachState { state } => (None, Some(state.clone()), None, None),
        GoalKind::CommandExecutes { command } => (None, None, Some(command.clone()), None),
        GoalKind::NoStuckStates { allow_waiting } => (
            None,
            None,
            None,
            Some(allow_waiting.clone()).filter(|names| !names.is_empty()),
        ),
    };
    GoalDocument {
        id: goal.id.clone(),
        kind: kind_name(&goal.kind).to_string(),
        required: goal.required,
        terminal,
        state,
        command,
        allow_waiting,
    }
}

pub const PROFILE_DIFF_SCHEMA: &str = "ik.profile.diff.v1";

/// Identity a profile evidence record must match. A goal id is not sufficient.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileIdentity {
    pub digest: String,
    pub goals: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProfileDiff {
    pub schema: &'static str,
    pub old_digest: String,
    pub new_digest: String,
    pub weakened: bool,
    pub changes: Vec<ProfileChange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProfileChange {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub weakened: bool,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProfileBaseline {
    pub goals: Vec<BaselineGoal>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BaselineGoal {
    pub id: String,
    pub candidate: String,
    pub prior: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Present only when a goal satisfied on the prior is not satisfied on the candidate.
    #[serde(skip_serializing_if = "not_regression")]
    pub regression: bool,
}

pub fn profile_identity(profile: &Profile) -> ProfileIdentity {
    let mut goals = BTreeMap::new();
    for goal in &profile.goals {
        goals.insert(goal.id.clone(), content_digest(goal));
    }
    ProfileIdentity {
        digest: profile_digest(profile),
        goals,
    }
}

pub fn diff_profiles(old: &Profile, new: &Profile) -> ProfileDiff {
    let mut changes = limit_changes(old, new);
    changes.extend(goal_changes(old, new));
    let old_digest = profile_digest(old);
    let new_digest = profile_digest(new);
    if old_digest != new_digest && changes.is_empty() {
        changes.push(digest_only_change());
    }
    let weakened = changes.iter().any(|change| change.weakened);
    ProfileDiff {
        schema: PROFILE_DIFF_SCHEMA,
        old_digest,
        new_digest,
        weakened,
        changes,
    }
}

pub fn render_profile_diff(diff: &ProfileDiff) -> String {
    let mut text = String::from("PROFILE DIFF\n");
    text.push_str(&format!("  old_digest {}\n", diff.old_digest));
    text.push_str(&format!("  new_digest {}\n", diff.new_digest));
    if diff.weakened {
        text.push_str("  weakened\n");
    }
    for change in &diff.changes {
        append_diff_change(&mut text, change);
    }
    text
}

/// Same goals, judged on the prior exploration. Goals whose referenced state or command
/// is absent from the prior are `not_comparable`; they are not treated as unreachable.
pub fn baseline_against(
    profile: &Profile,
    candidate: &ProfileEnvelope,
    prior_def: &Definition,
    prior_run: &Exploration,
    corpus: &CaseCorpus,
) -> ProfileBaseline {
    let graph = machine_graph(prior_def);
    let goals = profile
        .goals
        .iter()
        .map(|goal| baseline_goal(goal, candidate, prior_def, prior_run, corpus, &graph))
        .collect();
    ProfileBaseline { goals }
}

fn append_baseline(text: &mut String, baseline: &ProfileBaseline) {
    text.push_str("  baseline\n");
    for goal in &baseline.goals {
        text.push_str(&format!(
            "  goal {} candidate={} prior={}\n",
            goal.id, goal.candidate, goal.prior
        ));
        if let Some(reason) = &goal.reason {
            text.push_str(&format!("    {reason}\n"));
        }
        if goal.regression {
            text.push_str("    regression\n");
        }
    }
}

fn not_regression(value: &bool) -> bool {
    !*value
}

fn baseline_goal(
    goal: &Goal,
    candidate: &ProfileEnvelope,
    prior_def: &Definition,
    prior_run: &Exploration,
    corpus: &CaseCorpus,
    graph: &MachineGraph,
) -> BaselineGoal {
    let candidate_status = candidate_status(candidate, &goal.id);
    match bind_goal(goal, prior_def) {
        Err(err) => incomparable(goal, candidate_status, &err),
        Ok(()) => comparable_goal(
            goal,
            candidate,
            prior_def,
            prior_run,
            corpus,
            graph,
            candidate_status,
        ),
    }
}

fn incomparable(goal: &Goal, candidate_status: String, err: &str) -> BaselineGoal {
    BaselineGoal {
        id: goal.id.clone(),
        candidate: candidate_status,
        prior: "not_comparable".into(),
        reason: Some(format!("{err} on the prior machine")),
        regression: false,
    }
}

fn comparable_goal(
    goal: &Goal,
    candidate: &ProfileEnvelope,
    prior_def: &Definition,
    prior_run: &Exploration,
    corpus: &CaseCorpus,
    graph: &MachineGraph,
    candidate_status: String,
) -> BaselineGoal {
    let prior = evaluate_goal(goal, prior_def, corpus, prior_run, graph);
    let prior_ok = goal_satisfied(&prior, prior_run.complete);
    let candidate_ok = candidate_satisfied(candidate, &goal.id);
    BaselineGoal {
        id: goal.id.clone(),
        candidate: candidate_status,
        prior: prior.status,
        reason: None,
        regression: prior_ok && !candidate_ok,
    }
}

fn candidate_status(envelope: &ProfileEnvelope, id: &str) -> String {
    envelope
        .goals
        .iter()
        .find(|goal| goal.id == id)
        .map(|goal| goal.status.clone())
        .unwrap_or_else(|| "missing".into())
}

fn candidate_satisfied(envelope: &ProfileEnvelope, id: &str) -> bool {
    envelope
        .goals
        .iter()
        .find(|goal| goal.id == id)
        .is_some_and(|goal| goal_satisfied(goal, envelope.search_complete))
}

fn limit_changes(old: &Profile, new: &Profile) -> Vec<ProfileChange> {
    let mut changes = Vec::new();
    push_limit(
        &mut changes,
        "max_nodes",
        old.limits.max_nodes,
        new.limits.max_nodes,
    );
    push_limit(
        &mut changes,
        "max_depth",
        old.limits.max_depth,
        new.limits.max_depth,
    );
    changes
}

fn push_limit(changes: &mut Vec<ProfileChange>, field: &str, old: usize, new: usize) {
    if old == new {
        return;
    }
    changes.push(ProfileChange {
        kind: "limits_changed".into(),
        id: None,
        field: Some(field.into()),
        weakened: new < old,
        detail: format!("{field} {old} -> {new}"),
    });
}

fn goal_changes(old: &Profile, new: &Profile) -> Vec<ProfileChange> {
    let old_goals = index_goals(&old.goals);
    let new_goals = index_goals(&new.goals);
    let mut changes = Vec::new();
    for (id, goal) in &old_goals {
        if !new_goals.contains_key(id) {
            changes.push(removed_goal(goal));
        }
    }
    for (id, goal) in &new_goals {
        if !old_goals.contains_key(id) {
            changes.push(added_goal(goal));
        }
    }
    for (id, old_goal) in &old_goals {
        if let Some(new_goal) = new_goals.get(id) {
            changes.extend(changed_goal(old_goal, new_goal));
        }
    }
    changes
}

fn index_goals(goals: &[Goal]) -> BTreeMap<&str, &Goal> {
    goals.iter().map(|goal| (goal.id.as_str(), goal)).collect()
}

fn removed_goal(goal: &Goal) -> ProfileChange {
    ProfileChange {
        kind: "goal_removed".into(),
        id: Some(goal.id.clone()),
        field: None,
        weakened: goal.required,
        detail: format!("required={}", goal.required),
    }
}

fn added_goal(goal: &Goal) -> ProfileChange {
    ProfileChange {
        kind: "goal_added".into(),
        id: Some(goal.id.clone()),
        field: None,
        weakened: false,
        detail: format!("required={}", goal.required),
    }
}

fn changed_goal(old: &Goal, new: &Goal) -> Vec<ProfileChange> {
    let mut changes = Vec::new();
    if content_digest(old) != content_digest(new) {
        changes.push(content_change(old));
    }
    if old.required != new.required {
        changes.push(required_toggle(old, new));
    }
    if let Some(change) = allow_waiting_change(old, new) {
        changes.push(change);
    }
    changes
}

fn content_change(old: &Goal) -> ProfileChange {
    ProfileChange {
        kind: "goal_changed".into(),
        id: Some(old.id.clone()),
        field: Some("content_digest".into()),
        weakened: false,
        detail: format!("content digest changed for goal `{}`", old.id),
    }
}

fn required_toggle(old: &Goal, new: &Goal) -> ProfileChange {
    ProfileChange {
        kind: "required_toggled".into(),
        id: Some(old.id.clone()),
        field: Some("required".into()),
        weakened: old.required && !new.required,
        detail: format!("required {} -> {}", old.required, new.required),
    }
}

fn allow_waiting_change(old: &Goal, new: &Goal) -> Option<ProfileChange> {
    let old_waiting = waiting_names(old);
    let new_waiting = waiting_names(new);
    if old_waiting == new_waiting {
        return None;
    }
    let weakened = new_waiting
        .iter()
        .any(|name| !old_waiting.iter().any(|old_name| old_name == name));
    Some(ProfileChange {
        kind: "allow_waiting_changed".into(),
        id: Some(old.id.clone()),
        field: Some("allow_waiting".into()),
        weakened,
        detail: format!("allow_waiting {old_waiting:?} -> {new_waiting:?}"),
    })
}

fn waiting_names(goal: &Goal) -> Vec<String> {
    match &goal.kind {
        GoalKind::NoStuckStates { allow_waiting } => allow_waiting.clone(),
        _ => Vec::new(),
    }
}

fn digest_only_change() -> ProfileChange {
    ProfileChange {
        kind: "profile_digest".into(),
        id: None,
        field: Some("profile_digest".into()),
        weakened: false,
        detail: "profile digest changed without a goal, limit, or allow_waiting change".into(),
    }
}

fn append_diff_change(text: &mut String, change: &ProfileChange) {
    text.push_str(&format!("  {}", change.kind));
    if let Some(id) = &change.id {
        text.push_str(&format!(" id={id}"));
    }
    if let Some(field) = &change.field {
        text.push_str(&format!(" field={field}"));
    }
    text.push_str(&format!(" {}\n", change.detail));
    if change.weakened {
        text.push_str("    weakened\n");
    }
}
