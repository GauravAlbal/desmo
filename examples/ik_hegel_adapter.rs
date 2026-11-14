//! Hegel executable adapter.
//!
//! Sampled execution of a declarative machine against a fresh `Store`.
//! This is property-based testing, not a proof. It never reports
//! `holds_in_scope`.

use hegel::generators as gs;
use hegel::{HealthCheck, Hegel, Settings, TestCase, Verbosity};
use ik::canonical::canonical_json;
use ik::declarative::def::Definition;
use ik::declarative::expr::{self, EvalCtx};
use ik::declarative::load_def;
use ik::identity::{AggregateId, Capability, MachineIdentity};
use ik::machine::Machine;
use ik::operation::{OperationId, OperationOutcome};
use ik::store::{CommandRequest, Projection, Store};
use serde_json::{json, Map, Value};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const RESULT_PROTOCOL: &str = "ik.analysis.result.v1";
const REQUEST_PROTOCOL: &str = "ik.analysis.request.v1";
const AGGREGATE: &str = "agg-1";

const SAFETY: &[&str] = &[
    "refusal_is_atomic",
    "store_verifies",
    "stale_capability_refused",
    "declared_invariants_hold",
];
const REACH: &[&str] = &["reach_state", "reach_terminal"];

fn main() {
    let code = match std::env::args().nth(1).as_deref() {
        Some("describe") => describe(),
        Some("run") => run_from_args(),
        _ => usage(),
    };
    std::process::exit(code);
}

fn usage() -> i32 {
    eprintln!("usage: ik_hegel_adapter describe");
    eprintln!("       ik_hegel_adapter run --request <request.json> --out <dir>");
    2
}

fn describe() -> i32 {
    let doc = json!({
        "adapter": {"id": "hegel-runtime", "version": "1"},
        "engine": {"id": "hegeltest", "version": "0.30.0"},
        "commands": ["describe", "run"],
        "inputs": ["ik.runtime-driver.v1", "executable_driver"],
        "properties": ["safety", "reachability"],
        "evidence": ["sampled_execution"],
        "goals": SAFETY.iter().chain(REACH.iter()).collect::<Vec<_>>(),
        "holds_in_scope": false,
        "note": "Hegel sampled execution is property-based testing, not a proof."
    });
    println!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default());
    0
}

fn run_from_args() -> i32 {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let parsed = match parse_run_args(&args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("ik_hegel_adapter: {message}");
            return 2;
        }
    };
    match execute(&parsed.0, &parsed.1) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("ik_hegel_adapter: {message}");
            1
        }
    }
}

fn parse_run_args(args: &[String]) -> Result<(PathBuf, PathBuf), String> {
    let mut request = None;
    let mut out = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--request" => request = Some(required_path(args, &mut index)?),
            "--out" => out = Some(required_path(args, &mut index)?),
            other => return Err(format!("unknown argument `{other}`")),
        }
        index += 1;
    }
    match (request, out) {
        (Some(request), Some(out)) => Ok((request, out)),
        _ => Err("--request and --out are required".into()),
    }
}

fn required_path(args: &[String], index: &mut usize) -> Result<PathBuf, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .map(PathBuf::from)
        .ok_or_else(|| "missing argument value".to_string())
}

fn execute(request_path: &Path, out: &Path) -> Result<(), String> {
    fs::create_dir_all(out).map_err(|err| err.to_string())?;
    let request = read_request(request_path)?;
    let outcome = match prepare(&request) {
        Ok(prepared) => drive(&prepared, out),
        Err(early) => early,
    };
    write_result(out, &outcome)
}

fn read_request(path: &Path) -> Result<ik::analysis::AnalysisRequest, String> {
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let request: ik::analysis::AnalysisRequest =
        serde_json::from_str(&text).map_err(|err| err.to_string())?;
    if request.protocol != REQUEST_PROTOCOL {
        return Err("request protocol mismatch".into());
    }
    Ok(request)
}

struct Prepared {
    request: ik::analysis::AnalysisRequest,
    machine_path: PathBuf,
    machine_rel: String,
    enabled: BTreeSet<String>,
    reach: ReachGoal,
    mutant: MutantChoice,
    cases: u64,
    steps: i64,
    seed: u64,
}

enum ReachGoal {
    None,
    Terminal,
    State(String),
}

enum MutantChoice {
    None,
    Unavailable,
    Unknown,
    #[cfg(feature = "test-support")]
    Active(ik::law_mutant::LawMutant),
}

enum Early {
    Unsupported { reason: String },
    Failed { reason: String },
    Ready(Prepared),
}

fn prepare(request: &ik::analysis::AnalysisRequest) -> Result<Prepared, Outcome> {
    let class = request.property_class.as_str();
    let goals: BTreeSet<String> = request.goals.iter().map(|goal| goal.id.clone()).collect();
    let disabled = disabled_goals(&request.controls);
    if let Some(reason) = unsupported_question(class, &goals) {
        return Err(outcome_unsupported(request, &reason));
    }
    let mutant = mutant_choice(request.controls.get("law_mutant").map(String::as_str));
    if matches!(mutant, MutantChoice::Unavailable | MutantChoice::Unknown) {
        let reason = match mutant {
            MutantChoice::Unknown => "unknown law_mutant",
            _ => "law_mutant requires a test-support adapter build",
        };
        return Err(outcome_unsupported(request, &reason));
    }
    let (machine_rel, machine_path) = match machine_file(request) {
        Ok(found) => found,
        Err(reason) => return Err(outcome_failed(request, &reason)),
    };
    let loaded = match load_def(&machine_path) {
        Ok(loaded) => loaded,
        Err(errors) => return Err(outcome_failed(request, &errors.join("; "))),
    };
    if !identity_matches(&loaded, request) {
        return Err(outcome_failed(
            request,
            "loaded machine identity does not match the request subject",
        ));
    }
    if let Err(reason) = args_supported(&loaded.definition().commands) {
        return Err(outcome_unsupported(request, &reason));
    }
    let reach = reach_goal(class, &goals, &request.controls);
    if matches!(reach, ReachGoal::State(ref name) if name.is_empty()) {
        return Err(outcome_unsupported(
            request,
            "reach_state requires controls.target",
        ));
    }
    Ok(Prepared {
        request: request.clone(),
        machine_path,
        machine_rel,
        enabled: enabled_goals(class, &goals, &disabled),
        reach,
        mutant,
        cases: bounded_u64(request.controls.get("cases"), 16, 64),
        steps: bounded_i64(request.controls.get("steps"), 6, 16),
        seed: request
            .controls
            .get("seed")
            .and_then(|text| text.parse().ok())
            .unwrap_or(1),
    })
}

fn unsupported_question(class: &str, goals: &BTreeSet<String>) -> Option<String> {
    let allowed: &[&str] = match class {
        "safety" => SAFETY,
        "reachability" => REACH,
        _ => {
            return Some(format!(
                "{class} is not a sampled-execution question this adapter answers"
            ))
        }
    };
    if goals.iter().all(|goal| allowed.contains(&goal.as_str())) {
        None
    } else {
        Some("goal id is not supported for this property class".into())
    }
}

fn disabled_goals(controls: &BTreeMap<String, String>) -> BTreeSet<String> {
    controls
        .get("disable")
        .map(|text| {
            text.split(',')
                .map(|item| item.trim().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn enabled_goals(
    class: &str,
    goals: &BTreeSet<String>,
    disabled: &BTreeSet<String>,
) -> BTreeSet<String> {
    if class != "safety" {
        return BTreeSet::new();
    }
    goals.difference(disabled).cloned().collect()
}

fn reach_goal(
    class: &str,
    goals: &BTreeSet<String>,
    controls: &BTreeMap<String, String>,
) -> ReachGoal {
    if class != "reachability" {
        return ReachGoal::None;
    }
    if goals.contains("reach_state") {
        return ReachGoal::State(controls.get("target").cloned().unwrap_or_default());
    }
    if goals.contains("reach_terminal") {
        return ReachGoal::Terminal;
    }
    ReachGoal::None
}

fn mutant_choice(raw: Option<&str>) -> MutantChoice {
    let Some(name) = raw.map(str::trim).filter(|name| !name.is_empty()) else {
        return MutantChoice::None;
    };
    #[cfg(feature = "test-support")]
    {
        return match named_mutant(name) {
            Some(mutant) => MutantChoice::Active(mutant),
            None => MutantChoice::Unknown,
        };
    }
    #[cfg(not(feature = "test-support"))]
    {
        let _ = name;
        MutantChoice::Unavailable
    }
}

#[cfg(feature = "test-support")]
fn named_mutant(name: &str) -> Option<ik::law_mutant::LawMutant> {
    use ik::law_mutant::LawMutant::*;
    match name {
        "AcceptStaleCapability" => Some(AcceptStaleCapability),
        "SkipGenerationIncrement" => Some(SkipGenerationIncrement),
        "SkipNonceRotation" => Some(SkipNonceRotation),
        "AllowTerminalMutation" => Some(AllowTerminalMutation),
        "SkipInvariantValidation" => Some(SkipInvariantValidation),
        "AllowSecondPendingEffect" => Some(AllowSecondPendingEffect),
        "AllowEffectConflict" => Some(AllowEffectConflict),
        "MutateOnDuplicate" => Some(MutateOnDuplicate),
        "RerunLiveGuardsOnReplay" => Some(RerunLiveGuardsOnReplay),
        "PersistOnRefusal" => Some(PersistOnRefusal),
        "CommitPartialTransaction" => Some(CommitPartialTransaction),
        "SkipOperationBinding" => Some(SkipOperationBinding),
        "SkipDuplicateGate" => Some(SkipDuplicateGate),
        "OmitCapabilityFromFingerprint" => Some(OmitCapabilityFromFingerprint),
        "SkipGenesisValidation" => Some(SkipGenesisValidation),
        _ => None,
    }
}

fn machine_file(request: &ik::analysis::AnalysisRequest) -> Result<(String, PathBuf), String> {
    let rel = request
        .controls
        .get("machine")
        .cloned()
        .or_else(|| {
            request
                .inputs
                .iter()
                .map(|input| input.path.clone())
                .find(|path| path.ends_with(".ik.toml"))
        })
        .ok_or_else(|| "request bundle has no machine .ik.toml".to_string())?;
    if rel.contains("..") || Path::new(&rel).is_absolute() {
        return Err("machine path must stay inside the bundle".into());
    }
    let path = request.bundle_dir.join(&rel);
    if !path.is_file() {
        return Err(format!("machine not found at {}", path.display()));
    }
    Ok((rel, path))
}

fn identity_matches(
    machine: &ik::declarative::CompiledMachine,
    request: &ik::analysis::AnalysisRequest,
) -> bool {
    let identity = machine.identity();
    identity.machine_id == request.subject.identity.machine_id
        && identity.spec_hash.0 == request.subject.identity.spec_hash
}

fn args_supported(
    commands: &BTreeMap<String, ik::declarative::def::CommandDef>,
) -> Result<(), String> {
    for (name, cmd) in commands {
        for (arg, ty) in cmd.args.clone().unwrap_or_default() {
            if !matches!(ty.as_str(), "int" | "string" | "bool") {
                return Err(format!(
                    "command `{name}` argument `{arg}` type `{ty}` is unsupported"
                ));
            }
        }
    }
    Ok(())
}

fn bounded_u64(raw: Option<&String>, default: u64, cap: u64) -> u64 {
    raw.and_then(|text| text.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
        .min(cap)
}

fn bounded_i64(raw: Option<&String>, default: i64, cap: i64) -> i64 {
    raw.and_then(|text| text.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
        .min(cap)
}

struct Outcome {
    request: ik::analysis::AnalysisRequest,
    execution: String,
    rows: Vec<Row>,
    artifacts: Vec<String>,
    reason: String,
}

struct Row {
    goal_id: String,
    finding: String,
    coverage_complete: bool,
    coverage_reason: String,
    reproduction: String,
    artifact: Option<String>,
}

fn outcome_unsupported(request: &ik::analysis::AnalysisRequest, reason: &str) -> Outcome {
    Outcome {
        request: request.clone(),
        execution: "unsupported".into(),
        rows: rows_for(
            request,
            "inconclusive",
            false,
            reason,
            "not_attempted",
            None,
        ),
        artifacts: Vec::new(),
        reason: reason.into(),
    }
}

fn outcome_failed(request: &ik::analysis::AnalysisRequest, reason: &str) -> Outcome {
    Outcome {
        request: request.clone(),
        execution: "failed".into(),
        rows: rows_for(
            request,
            "inconclusive",
            false,
            reason,
            "not_attempted",
            None,
        ),
        artifacts: Vec::new(),
        reason: reason.into(),
    }
}

fn rows_for(
    request: &ik::analysis::AnalysisRequest,
    finding: &str,
    complete: bool,
    reason: &str,
    reproduction: &str,
    artifact: Option<String>,
) -> Vec<Row> {
    request
        .goals
        .iter()
        .map(|goal| Row {
            goal_id: goal.id.clone(),
            finding: finding.into(),
            coverage_complete: complete,
            coverage_reason: reason.into(),
            reproduction: reproduction.into(),
            artifact: artifact.clone(),
        })
        .collect()
}

fn drive(prepared: &Prepared, out: &Path) -> Outcome {
    let searched = search_with_mutant(prepared);
    finish_search(prepared, out, searched)
}

fn search_with_mutant(prepared: &Prepared) -> SearchEnd {
    #[cfg(feature = "test-support")]
    if let MutantChoice::Active(mutant) = prepared.mutant {
        let (end, fired) =
            ik::law_mutant::observe_law_mutant(mutant, || search_and_replay(prepared));
        return stamp_seam(end, fired);
    }
    search_and_replay(prepared)
}

fn search_and_replay(prepared: &Prepared) -> SearchEnd {
    let mut end = hegel_search(prepared);
    if let SearchEnd::Violation(hit) = &mut end {
        replay_blob(prepared, hit);
        replay_scenario(prepared, hit);
    }
    end
}

fn stamp_seam(end: SearchEnd, fired: u32) -> SearchEnd {
    if fired > 0 {
        return end;
    }
    SearchEnd::Failed("law mutant seam was never reached".into())
}

struct Hit {
    goal: String,
    blob: Option<String>,
    steps: Vec<StepRec>,
    log: String,
    reproduction: String,
    artifact: Option<String>,
}

enum SearchEnd {
    Passed { witness: Option<Vec<StepRec>> },
    Violation(Hit),
    Failed(String),
}

fn hegel_search(prepared: &Prepared) -> SearchEnd {
    install_slot(prepared);
    witness_clear();
    let settings = search_settings(prepared);
    let (caught, log) = capture(|| {
        catch_unwind(AssertUnwindSafe(|| {
            Hegel::new(|tc| run_case(&tc)).settings(settings).run();
        }))
    });
    match caught {
        Ok(()) => SearchEnd::Passed {
            witness: witness_take(),
        },
        Err(payload) => classify_panic(payload, &log),
    }
}

fn search_settings(prepared: &Prepared) -> Settings {
    Settings::new()
        .test_cases(prepared.cases)
        .stateful_step_count(prepared.steps)
        .seed(Some(prepared.seed))
        .derandomize(false)
        .database(None)
        .verbosity(Verbosity::Quiet)
        .print_blob(true)
        .suppress_health_check(HealthCheck::all())
}

fn run_case(tc: &TestCase) {
    trace_clear();
    saw_clear();
    let driver = match Driver::boot(tc) {
        Ok(driver) => driver,
        Err(message) => panic!("infrastructure: {message}"),
    };
    hegel::stateful::run(driver, tc.clone());
    note_witness();
}

fn classify_panic(payload: Box<dyn std::any::Any + Send>, log: &str) -> SearchEnd {
    let message = panic_text(&payload);
    if let Some(goal) = message.strip_prefix("property violation: ") {
        return SearchEnd::Violation(Hit {
            goal: goal.trim().to_string(),
            blob: blob_from(log),
            steps: trace_clone(),
            log: log.to_string(),
            reproduction: "not_attempted".into(),
            artifact: None,
        });
    }
    SearchEnd::Failed(message)
}

fn panic_text(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    if let Some(text) = payload.downcast_ref::<&str>() {
        return (*text).to_string();
    }
    "unknown panic".into()
}

fn replay_blob(prepared: &Prepared, hit: &mut Hit) {
    let Some(blob) = hit.blob.clone() else {
        return;
    };
    let settings = search_settings(prepared).print_blob(false);
    let caught = catch_unwind(AssertUnwindSafe(|| {
        Hegel::new(|tc| run_case(&tc))
            .settings(settings)
            .reproduce_failure(blob)
            .run();
    }));
    if caught.is_err() {
        let steps = trace_clone();
        if !steps.is_empty() {
            hit.steps = steps;
        }
    }
}

fn replay_scenario(prepared: &Prepared, hit: &mut Hit) {
    if hit.steps.is_empty() || hit.steps.iter().any(StepRec::synthetic) {
        hit.reproduction = "model_only".into();
        return;
    }
    let staging = std::env::temp_dir().join(format!(
        "ik-hegel-scenario-{}-{}",
        std::process::id(),
        hit.goal
    ));
    let _ = fs::remove_dir_all(&staging);
    if fs::create_dir_all(&staging).is_err() {
        hit.reproduction = "not_reproduced".into();
        return;
    }
    let machine_copy = staging.join("machine.ik.toml");
    if fs::copy(&prepared.machine_path, &machine_copy).is_err() {
        hit.reproduction = "not_reproduced".into();
        return;
    }
    let scenario = scenario_value(&hit.steps);
    let scenario_path = staging.join("input-scenario.json");
    if write_pretty(&scenario_path, &scenario).is_err() {
        hit.reproduction = "not_reproduced".into();
        return;
    }
    let bundle = staging.join("bundle");
    let code = ik::scenario::run(&[
        "run".into(),
        machine_copy.display().to_string(),
        scenario_path.display().to_string(),
        "--out".into(),
        bundle.display().to_string(),
    ]);
    hit.reproduction = if code == 0 {
        "concrete_reproduced".into()
    } else {
        "not_reproduced".into()
    };
    // The caller copies the bundle into the adapter out directory.
    let _ = STAGING.with(|cell| {
        *cell.borrow_mut() = Some(Staging {
            goal: hit.goal.clone(),
            bundle,
            scenario,
        });
    });
}

struct Staging {
    goal: String,
    bundle: PathBuf,
    scenario: Value,
}

thread_local! {
    static STAGING: RefCell<Option<Staging>> = const { RefCell::new(None) };
}

fn finish_search(prepared: &Prepared, out: &Path, end: SearchEnd) -> Outcome {
    match end {
        SearchEnd::Failed(message) => outcome_failed(&prepared.request, &message),
        SearchEnd::Passed { witness } => passed_outcome(prepared, out, witness),
        SearchEnd::Violation(hit) => violation_outcome(prepared, out, hit),
    }
}

fn passed_outcome(prepared: &Prepared, out: &Path, witness: Option<Vec<StepRec>>) -> Outcome {
    let reason = "sampled execution over a finite case and step budget; not an exhaustive search and not a proof";
    if prepared.request.property_class == "reachability" {
        return reach_outcome(prepared, out, witness, reason);
    }
    Outcome {
        request: prepared.request.clone(),
        execution: "completed".into(),
        rows: prepared
            .request
            .goals
            .iter()
            .map(|goal| safety_row(&goal.id, &prepared.enabled, reason))
            .collect(),
        artifacts: Vec::new(),
        reason: reason.into(),
    }
}

fn safety_row(goal: &str, enabled: &BTreeSet<String>, reason: &str) -> Row {
    if enabled.contains(goal) {
        Row {
            goal_id: goal.into(),
            finding: "no_violation_found".into(),
            coverage_complete: false,
            coverage_reason: reason.into(),
            reproduction: "not_attempted".into(),
            artifact: None,
        }
    } else {
        Row {
            goal_id: goal.into(),
            finding: "inconclusive".into(),
            coverage_complete: false,
            coverage_reason: "property disabled by request control".into(),
            reproduction: "not_attempted".into(),
            artifact: None,
        }
    }
}

fn reach_outcome(
    prepared: &Prepared,
    out: &Path,
    witness: Option<Vec<StepRec>>,
    incomplete: &str,
) -> Outcome {
    let found = witness.as_ref().is_some_and(|steps| !steps.is_empty());
    let mut artifact = None;
    let mut reproduction = "not_attempted".to_string();
    let mut artifacts = Vec::new();
    if found {
        let mut hit = Hit {
            goal: prepared
                .request
                .goals
                .iter()
                .find(|goal| REACH.contains(&goal.id.as_str()))
                .map(|goal| goal.id.clone())
                .unwrap_or_else(|| "reach_terminal".into()),
            blob: None,
            steps: witness.unwrap_or_default(),
            log: String::new(),
            reproduction: "not_attempted".into(),
            artifact: None,
        };
        replay_scenario(prepared, &mut hit);
        if let Some(relative) = publish_bundle(out, &hit.goal) {
            artifact = Some(relative.clone());
            artifacts.push(relative);
        }
        reproduction = hit.reproduction;
    }
    let finding = if found {
        "witness_found"
    } else {
        "no_witness_found"
    };
    let reason = if found {
        "a sampled execution reached the requested state; this is a witness, not an exhaustive search"
    } else {
        incomplete
    };
    Outcome {
        request: prepared.request.clone(),
        execution: "completed".into(),
        rows: prepared
            .request
            .goals
            .iter()
            .map(|goal| Row {
                goal_id: goal.id.clone(),
                finding: finding.into(),
                coverage_complete: false,
                coverage_reason: reason.into(),
                reproduction: reproduction.clone(),
                artifact: artifact.clone(),
            })
            .collect(),
        artifacts,
        reason: reason.into(),
    }
}

fn violation_outcome(prepared: &Prepared, out: &Path, hit: Hit) -> Outcome {
    let mut artifacts = Vec::new();
    if let Some(relative) = write_native(out, &hit) {
        artifacts.push(relative);
    }
    let scenario_rel = publish_bundle(out, &hit.goal);
    if let Some(relative) = &scenario_rel {
        artifacts.push(relative.clone());
    }
    let artifact = if hit.reproduction == "concrete_reproduced" {
        scenario_rel.clone()
    } else {
        scenario_rel.clone()
    };
    let reason = format!(
        "sampled execution found a violation of {}; finite budget is not a proof of any other case",
        hit.goal
    );
    Outcome {
        request: prepared.request.clone(),
        execution: "completed".into(),
        rows: prepared
            .request
            .goals
            .iter()
            .map(|goal| violation_row(goal.id.as_str(), &hit, &reason, artifact.clone()))
            .collect(),
        artifacts,
        reason,
    }
}

fn violation_row(goal: &str, hit: &Hit, reason: &str, artifact: Option<String>) -> Row {
    if goal == hit.goal {
        Row {
            goal_id: goal.into(),
            finding: "violation".into(),
            coverage_complete: false,
            coverage_reason: reason.into(),
            reproduction: hit.reproduction.clone(),
            artifact,
        }
    } else if SAFETY.contains(&goal) {
        Row {
            goal_id: goal.into(),
            finding: "inconclusive".into(),
            coverage_complete: false,
            coverage_reason: "search stopped at another property violation".into(),
            reproduction: "not_attempted".into(),
            artifact: None,
        }
    } else {
        Row {
            goal_id: goal.into(),
            finding: "inconclusive".into(),
            coverage_complete: false,
            coverage_reason: reason.into(),
            reproduction: "not_attempted".into(),
            artifact: None,
        }
    }
}

fn write_native(out: &Path, hit: &Hit) -> Option<String> {
    let dir = out.join("native");
    fs::create_dir_all(&dir).ok()?;
    let mut text = String::new();
    if let Some(blob) = &hit.blob {
        text.push_str("To reproduce this failure, add the attribute below #[hegel::test]:\n");
        text.push_str(&format!("    #[hegel::reproduce_failure(\"{blob}\")]\n"));
    }
    if !hit.log.is_empty() {
        text.push_str(&hit.log);
        if !hit.log.ends_with('\n') {
            text.push('\n');
        }
    }
    if text.is_empty() {
        return None;
    }
    fs::write(dir.join("hegel-reproduce.txt"), text).ok()?;
    Some("native/hegel-reproduce.txt".into())
}

fn publish_bundle(out: &Path, goal: &str) -> Option<String> {
    let staging = STAGING.with(|cell| cell.borrow_mut().take())?;
    if staging.goal != goal {
        return None;
    }
    let relative = format!("findings/{goal}");
    let dest = out.join(&relative);
    let _ = fs::remove_dir_all(&dest);
    copy_dir(&staging.bundle, &dest).ok()?;
    Some(format!("{relative}/scenario.json"))
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn scenario_value(steps: &[StepRec]) -> Value {
    json!({
        "scenario_version": 1,
        "machine": "machine.ik.toml",
        "aggregate": AGGREGATE,
        "replayable": true,
        "steps": steps.iter().filter_map(StepRec::scenario_json).collect::<Vec<_>>()
    })
}

fn write_result(out: &Path, outcome: &Outcome) -> Result<(), String> {
    let body = result_value(outcome);
    let tmp = out.join("result.json.tmp");
    write_pretty(&tmp, &body)?;
    fs::rename(&tmp, out.join("result.json")).map_err(|err| err.to_string())
}

fn result_value(outcome: &Outcome) -> Value {
    let properties: Vec<Value> = outcome
        .rows
        .iter()
        .map(|row| {
            let mut item = json!({
                "goal_id": row.goal_id,
                "property_class": outcome.request.property_class,
                "finding": row.finding,
                "evidence_class": "sampled_execution",
                "coverage": {"complete": row.coverage_complete, "reason": row.coverage_reason},
                "reproduction": {"status": row.reproduction}
            });
            if let Some(artifact) = &row.artifact {
                item["reproduction"]["artifact"] = json!(artifact);
            }
            item
        })
        .collect();
    json!({
        "protocol": RESULT_PROTOCOL,
        "request_id": outcome.request.request_id,
        "run_id": outcome.request.run_id,
        "subject": outcome.request.subject,
        "goals": outcome.request.goals.iter().map(|goal| json!({
            "id": goal.id,
            "content_digest": goal.content_digest
        })).collect::<Vec<_>>(),
        "profile_digest": outcome.request.profile_digest,
        "inputs": outcome.request.inputs,
        "execution_status": outcome.execution,
        "properties": properties,
        "artifacts": outcome.artifacts
    })
}

fn write_pretty(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|err| err.to_string())?;
    fs::write(path, bytes).map_err(|err| err.to_string())
}

fn capture<R>(f: impl FnOnce() -> R) -> (R, String) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    let result = hegel::with_output_override(
        Arc::new(move |line| {
            if let Ok(mut guard) = sink.lock() {
                guard.push(line.to_string());
            }
        }),
        f,
    );
    let text = lines
        .lock()
        .map(|guard| guard.join("\n"))
        .unwrap_or_default();
    (result, text)
}

fn blob_from(log: &str) -> Option<String> {
    for line in log.lines() {
        let marker = "reproduce_failure(\"";
        let Some(start) = line.find(marker) else {
            continue;
        };
        let rest = &line[start + marker.len()..];
        let end = rest.find('"')?;
        let blob = &rest[..end];
        if !blob.is_empty() {
            return Some(blob.to_string());
        }
    }
    None
}

struct Slot {
    machine_path: PathBuf,
    enabled: BTreeSet<String>,
    reach: ReachGoal,
}

thread_local! {
    static SLOT: RefCell<Option<Slot>> = const { RefCell::new(None) };
    static TRACE: RefCell<Vec<StepRec>> = const { RefCell::new(Vec::new()) };
    static WITNESS: RefCell<Option<Vec<StepRec>>> = const { RefCell::new(None) };
    static SAW_TERMINAL: Cell<bool> = const { Cell::new(false) };
    static SAW_STATE: Cell<bool> = const { Cell::new(false) };
}

fn install_slot(prepared: &Prepared) {
    SLOT.with(|cell| {
        *cell.borrow_mut() = Some(Slot {
            machine_path: prepared.machine_path.clone(),
            enabled: prepared.enabled.clone(),
            reach: match &prepared.reach {
                ReachGoal::None => ReachGoal::None,
                ReachGoal::Terminal => ReachGoal::Terminal,
                ReachGoal::State(name) => ReachGoal::State(name.clone()),
            },
        });
    });
}

fn trace_clear() {
    TRACE.with(|cell| cell.borrow_mut().clear());
}

fn trace_push(step: StepRec) {
    TRACE.with(|cell| cell.borrow_mut().push(step));
}

fn trace_clone() -> Vec<StepRec> {
    TRACE.with(|cell| cell.borrow().clone())
}

fn witness_clear() {
    WITNESS.with(|cell| *cell.borrow_mut() = None);
    saw_clear();
}

fn saw_clear() {
    SAW_TERMINAL.with(|cell| cell.set(false));
    SAW_STATE.with(|cell| cell.set(false));
}

fn note_witness() {
    let reached = SAW_TERMINAL.with(Cell::get) || SAW_STATE.with(Cell::get);
    if !reached {
        return;
    }
    WITNESS.with(|cell| {
        if cell.borrow().is_none() {
            *cell.borrow_mut() = Some(trace_clone());
        }
    });
}

fn witness_take() -> Option<Vec<StepRec>> {
    WITNESS.with(|cell| cell.borrow_mut().take())
}

#[derive(Clone)]
enum StepRec {
    Command {
        label: String,
        name: String,
        args: Map<String, Value>,
        operation_id: String,
        handle: Option<String>,
        use_handle: Option<String>,
        expect: String,
        synthetic: bool,
    },
    Duplicate {
        label: String,
        of: String,
    },
    Reopen {
        label: String,
    },
}

impl StepRec {
    fn synthetic(&self) -> bool {
        matches!(
            self,
            Self::Command {
                synthetic: true,
                ..
            }
        )
    }

    fn scenario_json(&self) -> Option<Value> {
        if self.synthetic() {
            return None;
        }
        Some(match self {
            Self::Command {
                label,
                name,
                args,
                operation_id,
                handle,
                use_handle,
                expect,
                synthetic: _,
            } => {
                let mut item = json!({
                    "kind": "command",
                    "label": label,
                    "name": name,
                    "args": args,
                    "operation_id": operation_id,
                    "expect": expect
                });
                if let Some(handle) = handle {
                    item["handle"] = json!(handle);
                }
                if let Some(use_handle) = use_handle {
                    item["use_handle"] = json!(use_handle);
                }
                item
            }
            Self::Duplicate { label, of } => json!({"kind": "duplicate", "label": label, "of": of}),
            Self::Reopen { label } => json!({"kind": "reopen", "label": label}),
        })
    }
}

struct CmdSchema {
    name: String,
    args: Vec<(String, String)>,
}

struct Retained {
    label: String,
    capability: Capability,
}

struct PriorOp {
    label: String,
    name: String,
    args: Map<String, Value>,
    capability: Capability,
    operation_id: String,
}

#[derive(Clone, PartialEq, Eq)]
struct Snap {
    state: String,
    generation: u64,
    sequence: u64,
    terminal: bool,
    effect: String,
}

struct Driver {
    path: PathBuf,
    store: Option<Store>,
    machine: ik::declarative::CompiledMachine,
    commands: Vec<CmdSchema>,
    aggregate: AggregateId,
    identity: MachineIdentity,
    enabled: BTreeSet<String>,
    reach_terminal: bool,
    reach_state: Option<String>,
    retained: Vec<Retained>,
    priors: Vec<PriorOp>,
    step_index: u32,
    last_refusal: Option<(Snap, Snap)>,
    last_stale: Option<(String, Snap, Snap)>,
}

impl Driver {
    fn boot(tc: &TestCase) -> Result<Self, String> {
        let slot = SLOT.with(|cell| {
            cell.borrow().as_ref().map(|slot| Slot {
                machine_path: slot.machine_path.clone(),
                enabled: slot.enabled.clone(),
                reach: match &slot.reach {
                    ReachGoal::None => ReachGoal::None,
                    ReachGoal::Terminal => ReachGoal::Terminal,
                    ReachGoal::State(name) => ReachGoal::State(name.clone()),
                },
            })
        });
        let slot = slot.ok_or_else(|| "adapter slot was not installed".to_string())?;
        let machine = load_def(&slot.machine_path).map_err(|errors| errors.join("; "))?;
        let commands = command_schemas(machine.definition());
        let path = fresh_store_path();
        let mut store = Store::create(&path).map_err(|err| err.to_string())?;
        let identity = machine.identity().clone();
        let aggregate = AggregateId(AGGREGATE.into());
        store
            .create_aggregate(&machine, &aggregate, &identity)
            .map_err(|err| err.to_string())?;
        let _ = tc;
        let (reach_terminal, reach_state) = match slot.reach {
            ReachGoal::Terminal => (true, None),
            ReachGoal::State(name) => (false, Some(name)),
            ReachGoal::None => (false, None),
        };
        Ok(Self {
            path,
            store: Some(store),
            machine,
            commands,
            aggregate,
            identity,
            enabled: slot.enabled,
            reach_terminal,
            reach_state,
            retained: Vec::new(),
            priors: Vec::new(),
            step_index: 0,
            last_refusal: None,
            last_stale: None,
        })
    }

    fn store_mut(&mut self) -> &mut Store {
        self.store.as_mut().expect("store")
    }

    fn show(&mut self) -> Projection {
        let aggregate = self.aggregate.clone();
        self.store_mut().show(&aggregate).expect("show aggregate")
    }
}

fn command_schemas(def: &Definition) -> Vec<CmdSchema> {
    def.commands
        .iter()
        .map(|(name, cmd)| CmdSchema {
            name: name.clone(),
            args: cmd.args.clone().unwrap_or_default().into_iter().collect(),
        })
        .collect()
}

fn fresh_store_path() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(format!("hegel-case-{n}"));
    fs::create_dir_all(&dir).expect("case dir");
    dir.join("store.sqlite")
}

#[hegel::state_machine]
impl Driver {
    #[rule]
    fn submit_declared(&mut self, tc: TestCase) {
        tc.assume(!self.commands.is_empty());
        let drawn = draw_command(&tc, &self.commands);
        self.submit_declared_body(&drawn.0, drawn.1);
    }

    #[rule]
    fn submit_stale(&mut self, tc: TestCase) {
        tc.assume(!self.commands.is_empty());
        let drawn = draw_command(&tc, &self.commands);
        let current = self.show().capability;
        if let Some(index) = stale_index(&self.retained, &current) {
            let retained = self.retained[index].clone_parts();
            self.submit_stale_body(&drawn.0, drawn.1, retained.1, Some(retained.0));
            return;
        }
        let shown = self.show();
        let capability = Capability {
            generation: shown.generation.saturating_add(1),
            nonce: "synthetic-stale".into(),
        };
        self.submit_stale_body(&drawn.0, drawn.1, capability, None);
    }

    #[rule]
    fn duplicate_operation(&mut self, tc: TestCase) {
        tc.assume(!self.priors.is_empty());
        let index = draw_index(&tc, self.priors.len());
        let prior = self.priors[index].clone_parts();
        let before = snap_of(&self.show());
        let outcome = self.command_with(&prior.1, &prior.2, &prior.3, Some(&prior.4));
        let after = snap_of(&self.show());
        if matches!(outcome, Observed::Refused(_)) && before != after {
            self.last_refusal = Some((before, after));
        }
        if matches!(outcome, Observed::Historical) {
            trace_push(StepRec::Duplicate {
                label: self.next_label(),
                of: prior.0,
            });
        }
    }

    #[rule]
    fn reopen_store(&mut self, tc: TestCase) {
        let _ = tc;
        let before = snap_of(&self.show());
        let path = self.path.clone();
        self.store.take();
        self.store = Some(Store::open(&path).expect("reopen"));
        let after = snap_of(&self.show());
        if before != after && self.enabled.contains("store_verifies") {
            panic!("property violation: store_verifies");
        }
        trace_push(StepRec::Reopen {
            label: self.next_label(),
        });
    }

    #[invariant]
    fn properties_hold(&mut self, tc: TestCase) {
        let _ = tc;
        self.check_refusal();
        self.check_stale();
        self.check_invariants();
        self.check_verify();
    }
}

impl Retained {
    fn clone_parts(&self) -> (String, Capability) {
        (self.label.clone(), self.capability.clone())
    }
}

impl PriorOp {
    fn clone_parts(&self) -> (String, String, Map<String, Value>, Capability, String) {
        (
            self.label.clone(),
            self.name.clone(),
            self.args.clone(),
            self.capability.clone(),
            self.operation_id.clone(),
        )
    }
}

fn stale_index(retained: &[Retained], current: &Capability) -> Option<usize> {
    retained.iter().position(|item| &item.capability != current)
}

fn draw_index(tc: &TestCase, len: usize) -> usize {
    if len <= 1 {
        return 0;
    }
    let drawn = tc.draw(
        gs::integers::<i32>()
            .min_value(0)
            .max_value((len - 1) as i32),
    );
    usize::try_from(drawn).unwrap_or(0)
}

fn draw_command(tc: &TestCase, commands: &[CmdSchema]) -> (String, Map<String, Value>) {
    let index = draw_index(tc, commands.len());
    let cmd = &commands[index];
    let mut args = Map::new();
    for (name, ty) in &cmd.args {
        args.insert(name.clone(), draw_arg(tc, ty));
    }
    (cmd.name.clone(), args)
}

fn draw_arg(tc: &TestCase, ty: &str) -> Value {
    match ty {
        "int" => json!(tc.draw(gs::integers::<i64>().min_value(-1).max_value(5))),
        "bool" => json!(tc.draw(gs::booleans())),
        "string" => json!(tc.draw(gs::text().alphabet("ab").min_size(0).max_size(3))),
        other => panic!("infrastructure: unsupported arg type {other}"),
    }
}

impl Driver {
    fn submit_declared_body(&mut self, name: &str, args: Map<String, Value>) {
        let capability = self.show().capability.clone();
        let before = snap_of(&self.show());
        let label = self.next_label();
        let operation_id = format!("op-{label}");
        let outcome = self.command_with(&name, &args, &capability, Some(&operation_id));
        let after_view = self.show();
        let after = snap_of(&after_view);
        let expect = expect_of(&outcome);
        let handle = match &outcome {
            Observed::Committed(next) => {
                self.retained.push(Retained {
                    label: label.clone(),
                    capability: capability.clone(),
                });
                self.priors.push(PriorOp {
                    label: label.clone(),
                    name: name.to_string(),
                    args: args.clone(),
                    capability,
                    operation_id: operation_id.clone(),
                });
                let _ = next;
                Some(label.clone())
            }
            Observed::Refused(_) => {
                self.last_refusal = Some((before, after));
                None
            }
            Observed::Historical | Observed::Noop => None,
        };
        self.note_reach(&after_view);
        trace_push(StepRec::Command {
            label,
            name: name.to_string(),
            args,
            operation_id,
            handle,
            use_handle: None,
            expect,
            synthetic: false,
        });
    }

    fn submit_stale_body(
        &mut self,
        name: &str,
        args: Map<String, Value>,
        capability: Capability,
        use_handle: Option<String>,
    ) {
        let before = snap_of(&self.show());
        let label = self.next_label();
        let operation_id = format!("op-{label}");
        let outcome = self.command_with(name, &args, &capability, Some(&operation_id));
        let after = snap_of(&self.show());
        let code = match &outcome {
            Observed::Refused(code) => code.clone(),
            _ => "committed".into(),
        };
        self.last_stale = Some((code, before, after));
        trace_push(StepRec::Command {
            label,
            name: name.to_string(),
            args,
            operation_id,
            handle: None,
            use_handle: use_handle.clone(),
            expect: expect_of(&outcome),
            synthetic: use_handle.is_none(),
        });
    }

    fn command_with(
        &mut self,
        name: &str,
        args: &Map<String, Value>,
        capability: &Capability,
        operation_id: Option<&str>,
    ) -> Observed {
        let req = CommandRequest {
            aggregate_id: self.aggregate.clone(),
            machine: self.identity.clone(),
            capability: capability.clone(),
            command: json!({"name": name, "args": args}),
            observation: None,
        };
        let mut store = self.store.take().expect("store");
        let result = match operation_id {
            Some(raw) => {
                let id = OperationId::parse(raw).expect("operation id");
                store.command_op(&self.machine, &req, &id)
            }
            None => store
                .command(&self.machine, &req)
                .map(OperationOutcome::Committed),
        };
        self.store = Some(store);
        observed(result)
    }

    fn next_label(&mut self) -> String {
        let label = format!("s{}", self.step_index);
        self.step_index += 1;
        label
    }

    fn note_reach(&self, view: &Projection) {
        if self.reach_terminal && view.terminal {
            SAW_TERMINAL.with(|cell| cell.set(true));
        }
        if let Some(target) = &self.reach_state {
            if state_name(&view.state) == *target {
                SAW_STATE.with(|cell| cell.set(true));
            }
        }
    }

    fn check_refusal(&self) {
        if !self.enabled.contains("refusal_is_atomic") {
            return;
        }
        let Some((before, after)) = &self.last_refusal else {
            return;
        };
        if before != after {
            panic!("property violation: refusal_is_atomic");
        }
    }

    fn check_stale(&self) {
        if !self.enabled.contains("stale_capability_refused") {
            return;
        }
        let Some((code, before, after)) = &self.last_stale else {
            return;
        };
        if code != "IK_CAPABILITY_STALE" || before != after {
            panic!("property violation: stale_capability_refused");
        }
    }

    fn check_invariants(&mut self) {
        if !self.enabled.contains("declared_invariants_hold") {
            return;
        }
        let view = self.show();
        if let Err(message) = invariants_hold(self.machine.definition(), &view.state) {
            let _ = message;
            panic!("property violation: declared_invariants_hold");
        }
    }

    fn check_verify(&mut self) {
        if !self.enabled.contains("store_verifies") {
            return;
        }
        let aggregate = self.aggregate.clone();
        let mut store = self.store.take().expect("store");
        let verified = store
            .verify(&self.machine, Some(&aggregate))
            .map(|summary| summary.verified)
            .unwrap_or(false);
        self.store = Some(store);
        if !verified {
            panic!("property violation: store_verifies");
        }
    }
}

enum Observed {
    Committed(Capability),
    Noop,
    Historical,
    Refused(String),
}

fn observed(
    result: Result<OperationOutcome<ik::store::CommandResult>, ik::diagnostic::Diagnostic>,
) -> Observed {
    match result {
        Err(err) => Observed::Refused(err.code.as_str()),
        Ok(OperationOutcome::Historical(_)) => Observed::Historical,
        Ok(OperationOutcome::Committed(result)) if result.mutation => {
            Observed::Committed(result.capability)
        }
        Ok(OperationOutcome::Committed(_)) => Observed::Noop,
    }
}

fn expect_of(outcome: &Observed) -> String {
    match outcome {
        Observed::Committed(_) => "committed".into(),
        Observed::Noop => "noop".into(),
        Observed::Historical => "committed_exact".into(),
        Observed::Refused(code) => format!("refused:{code}"),
    }
}

fn snap_of(view: &Projection) -> Snap {
    Snap {
        state: canonical_json(&view.state),
        generation: view.generation,
        sequence: view.sequence,
        terminal: view.terminal,
        effect: canonical_json(&view.effect),
    }
}

fn state_name(state: &Value) -> String {
    state
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn invariants_hold(def: &Definition, state: &Value) -> Result<(), String> {
    let name = state_name(state);
    let facts = facts_of(state);
    let args = BTreeMap::new();
    let ctx = EvalCtx {
        facts: &facts,
        args: &args,
        state: &name,
    };
    for (inv_name, inv) in &def.invariants {
        if !gate_open(&inv.when, &ctx)? {
            continue;
        }
        let assertion = expr::parse(&inv.assert).map_err(|err| err)?;
        let holds = expr::eval(&assertion, &ctx).map_err(|err| format!("{inv_name}: {err}"))?;
        if !holds {
            return Err(inv_name.clone());
        }
    }
    Ok(())
}

fn facts_of(state: &Value) -> BTreeMap<String, Value> {
    let mut facts = BTreeMap::new();
    let Some(object) = state.as_object() else {
        return facts;
    };
    for (key, value) in object {
        if key != "state" {
            facts.insert(key.clone(), value.clone());
        }
    }
    facts
}

fn gate_open(when: &Option<String>, ctx: &EvalCtx<'_>) -> Result<bool, String> {
    let Some(text) = when else {
        return Ok(true);
    };
    let expr = expr::parse(text).map_err(|err| err)?;
    expr::eval(&expr, ctx).map_err(|err| err)
}
