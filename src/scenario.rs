//! Exact-runtime scenario driver.
//!
//! `ik scenario run` executes a version-1 scenario against a fresh scratch store
//! under `--out` and writes a reproduction bundle. It never accepts a caller
//! store path and never opens a store file that already exists.

use crate::canonical::{canonical_json, sha256_hex};
use crate::declarative::load_def;
use crate::diagnostic::Diagnostic;
use crate::effect::{EffectOutcomeV1, EffectStatus};
use crate::identity::{AggregateId, Capability, EffectId, MachineIdentity};
use crate::machine::Machine;
use crate::operation::{Lookup, OperationHistory, OperationId, OperationOutcome, OperationRequest};
use crate::protocol::RUNTIME_VERSION;
use crate::store::{
    CommandRequest, CommandResult, EffectOutcomeRequest, Projection, Store, SCHEMA_VERSION,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const USAGE: &str = "\
usage: ik scenario run <machine.ik.toml> <scenario.json> --out <dir>
       ik scenario reproduce <bundle-dir> --out <dir>

The run creates a fresh scratch store at <dir>/scratch/store.sqlite.
There is no --store flag. An output directory or scratch path that already
contains a store file is refused.
";

const STORE_FILE: &str = "store.sqlite";
const RESULT_SCHEMA: &str = "ik.scenario.result.v1";
const TRACE_SCHEMA: &str = "ikaros.trace.v2";

struct RunArgs {
    machine: PathBuf,
    scenario: PathBuf,
    out: PathBuf,
}

struct ReproduceArgs {
    bundle: PathBuf,
    out: PathBuf,
}

enum Invocation {
    Help,
    Run(RunArgs),
    Reproduce(ReproduceArgs),
}

/// CLI entry for `ik scenario`. Exit 0 met/reproduced, 1 expectation or
/// reproduction mismatch, 2 usage or refusal to touch an existing store.
pub fn run(args: &[String]) -> i32 {
    match parse_invocation(args) {
        Ok(Invocation::Help) => {
            println!("{USAGE}");
            0
        }
        Ok(Invocation::Run(parsed)) => execute_run(&parsed),
        Ok(Invocation::Reproduce(parsed)) => execute_reproduce(&parsed),
        Err(message) => usage_error(&message),
    }
}

fn usage_error(message: &str) -> i32 {
    eprintln!("ik scenario: {message}");
    eprintln!("{USAGE}");
    2
}

fn parse_invocation(args: &[String]) -> Result<Invocation, String> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        return Ok(Invocation::Help);
    }
    if args.is_empty() {
        return Err("scenario command is required".into());
    }
    if args.iter().any(|arg| arg == "--store") {
        return Err(
            "scenario runs do not accept --store; they create a fresh scratch store under --out"
                .into(),
        );
    }
    match args[0].as_str() {
        "run" => parse_run(&args[1..]).map(Invocation::Run),
        "reproduce" => parse_reproduce(&args[1..]).map(Invocation::Reproduce),
        other => Err(format!("unknown scenario command `{other}`")),
    }
}

fn parse_run(args: &[String]) -> Result<RunArgs, String> {
    let mut positionals = Vec::new();
    let out = take_out(args, &mut positionals)?;
    if positionals.len() != 2 {
        return Err("run requires <machine.ik.toml> <scenario.json> --out <dir>".into());
    }
    Ok(RunArgs {
        machine: PathBuf::from(&positionals[0]),
        scenario: PathBuf::from(&positionals[1]),
        out,
    })
}

fn parse_reproduce(args: &[String]) -> Result<ReproduceArgs, String> {
    let mut positionals = Vec::new();
    let out = take_out(args, &mut positionals)?;
    if positionals.len() != 1 {
        return Err("reproduce requires <bundle-dir> --out <dir>".into());
    }
    Ok(ReproduceArgs {
        bundle: PathBuf::from(&positionals[0]),
        out,
    })
}

fn take_out(args: &[String], positionals: &mut Vec<String>) -> Result<PathBuf, String> {
    let mut out = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                index += 1;
                let path = args
                    .get(index)
                    .ok_or_else(|| "--out requires a directory".to_string())?;
                out = Some(PathBuf::from(path));
            }
            flag if flag.starts_with('-') => return Err(format!("unknown flag `{flag}`")),
            positional => positionals.push(positional.to_string()),
        }
        index += 1;
    }
    out.ok_or_else(|| "--out <dir> is required".to_string())
}

fn execute_run(args: &RunArgs) -> i32 {
    match run_bundle(args) {
        Ok(code) => code,
        Err(message) => usage_error(&message),
    }
}

fn run_bundle(args: &RunArgs) -> Result<i32, String> {
    let scenario_bytes = read_bytes(&args.scenario)?;
    let scenario = parse_scenario(&scenario_bytes)?;
    let machine_path = resolve_scenario_machine(&args.scenario, &scenario.machine)?;
    same_machine_bytes(&args.machine, &machine_path)?;
    let store_path = fresh_store_path(&args.out)?;
    let machine = load_def(&args.machine).map_err(|errors| errors.join("; "))?;
    let identity = machine.identity().clone();
    let prior_loaded = load_prior_machine(&args.scenario, &scenario)?;
    let store = Store::create(&store_path).map_err(store_create_error(&store_path))?;
    let driven = drive(
        store,
        &store_path,
        &machine,
        &identity,
        prior_ref(prior_loaded.as_ref()),
        &scenario,
    )?;
    let result = result_document(&scenario_bytes, &identity, &scenario.aggregate, &driven)?;
    write_bundle(
        &args.out,
        &scenario_bytes,
        &args.machine,
        prior_loaded.as_ref().map(|(path, _)| path.as_path()),
        &result,
        &driven.events,
    )?;
    print_run_status(&driven);
    Ok(exit_for(&driven))
}

fn execute_reproduce(args: &ReproduceArgs) -> i32 {
    match reproduce(args) {
        Ok(code) => code,
        Err(message) => usage_error(&message),
    }
}

/// Comparison of a fresh scenario run with the bundle's recorded result.
///
/// A runtime-identity difference is listed and does not, by itself, fail the comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reproduction {
    pub reproduced: bool,
    pub lines: Vec<String>,
}

/// Re-run `bundle` in a fresh scratch store under `out` and compare `result.json`.
///
/// `ik scenario reproduce` and `ik regression run` both call this. It does not
/// rewrite the bundle's expectations.
pub fn reproduce_bundle(bundle: &Path, out: &Path) -> Result<Reproduction, String> {
    let scenario_path = bundle.join("scenario.json");
    let machine_path = bundle.join("machine.ik.toml");
    let recorded = read_json(&bundle.join("result.json"))?;
    let scenario_bytes = read_bytes(&scenario_path)?;
    if !machine_path.is_file() {
        return Err(format!("bundle is missing {}", machine_path.display()));
    }
    let scenario = parse_scenario(&scenario_bytes)?;
    let store_path = fresh_store_path(out)?;
    let machine = load_def(&machine_path).map_err(|errors| errors.join("; "))?;
    let identity = machine.identity().clone();
    let prior_loaded = load_prior_machine(&scenario_path, &scenario)?;
    let store = Store::create(&store_path).map_err(store_create_error(&store_path))?;
    let driven = drive(
        store,
        &store_path,
        &machine,
        &identity,
        prior_ref(prior_loaded.as_ref()),
        &scenario,
    )?;
    let result = result_document(&scenario_bytes, &identity, &scenario.aggregate, &driven)?;
    write_bundle(
        out,
        &scenario_bytes,
        &machine_path,
        prior_loaded.as_ref().map(|(path, _)| path.as_path()),
        &result,
        &driven.events,
    )?;
    Ok(reproduction_of(&recorded, &result))
}

fn reproduce(args: &ReproduceArgs) -> Result<i32, String> {
    let report = reproduce_bundle(&args.bundle, &args.out)?;
    for line in &report.lines {
        println!("{line}");
    }
    Ok(if report.reproduced { 0 } else { 1 })
}

fn store_create_error(path: &Path) -> impl Fn(Diagnostic) -> String + '_ {
    move |err| format!("refusing to open a store at {}: {err}", path.display())
}

fn print_run_status(driven: &Drive) {
    if driven.all_met {
        println!("all expectations met");
        return;
    }
    let label = driven.failed_label.as_deref().unwrap_or("unknown");
    println!("expectation failed: {label}");
}

fn exit_for(driven: &Drive) -> i32 {
    if driven.all_met {
        0
    } else {
        1
    }
}

fn reproduction_of(recorded: &Value, fresh: &Value) -> Reproduction {
    let reproduced = deterministic_eq(recorded, fresh);
    let mut lines = Vec::new();
    if reproduced {
        lines.push("reproduced".to_string());
    } else {
        lines.push("not_reproduced".to_string());
        match first_difference(recorded, fresh) {
            Diff::Step(label) => lines.push(format!("first differing step: {label}")),
            Diff::Field(field) => lines.push(format!("first differing field: {field}")),
        }
    }
    for field in runtime_differences(recorded, fresh) {
        lines.push(format!("runtime difference: {field}"));
    }
    Reproduction { reproduced, lines }
}

enum Diff {
    Step(String),
    Field(String),
}

fn first_difference(recorded: &Value, fresh: &Value) -> Diff {
    let left = recorded.get("steps").and_then(Value::as_array);
    let right = fresh.get("steps").and_then(Value::as_array);
    if let (Some(left), Some(right)) = (left, right) {
        let shared = left.len().min(right.len());
        for index in 0..shared {
            if left[index] != right[index] {
                let label = left[index]
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                return Diff::Step(label.to_string());
            }
        }
        if left.len() != right.len() {
            return Diff::Field("steps".into());
        }
    }
    Diff::Field(first_field(
        &without_runtime(recorded),
        &without_runtime(fresh),
    ))
}

fn first_field(left: &Value, right: &Value) -> String {
    let Some(left_map) = left.as_object() else {
        return "result".into();
    };
    let Some(right_map) = right.as_object() else {
        return "result".into();
    };
    let mut keys = BTreeSet::new();
    keys.extend(left_map.keys());
    keys.extend(right_map.keys());
    for key in keys {
        if left_map.get(key) != right_map.get(key) {
            return key.clone();
        }
    }
    "result".into()
}

fn deterministic_eq(recorded: &Value, fresh: &Value) -> bool {
    without_runtime(recorded) == without_runtime(fresh)
}

fn without_runtime(value: &Value) -> Value {
    let mut copy = value.clone();
    if let Some(map) = copy.as_object_mut() {
        map.remove("runtime");
    }
    copy
}

fn runtime_differences(recorded: &Value, fresh: &Value) -> Vec<String> {
    let mut fields = Vec::new();
    let Some(left) = recorded.get("runtime").and_then(Value::as_object) else {
        return fields;
    };
    let Some(right) = fresh.get("runtime").and_then(Value::as_object) else {
        return fields;
    };
    for key in ["binary_sha256", "ik_version", "store_schema"] {
        if left.get(key) != right.get(key) {
            fields.push(key.to_string());
        }
    }
    fields
}

fn fresh_store_path(out: &Path) -> Result<PathBuf, String> {
    if let Some(reason) = existing_store(out) {
        return Err(reason);
    }
    let scratch = out.join("scratch");
    fs::create_dir_all(&scratch).map_err(|err| format!("create {}: {err}", scratch.display()))?;
    let store_path = scratch.join(STORE_FILE);
    if store_path.exists() || existing_store(out).is_some() {
        return Err("scratch path already contains a store file".into());
    }
    Ok(store_path)
}

fn existing_store(out: &Path) -> Option<String> {
    if out.is_file() || directory_has_store(out) {
        return Some("output directory already contains a store file".into());
    }
    let scratch = out.join("scratch");
    if scratch.is_file() || directory_has_store(&scratch) {
        return Some("scratch path already contains a store file".into());
    }
    None
}

fn directory_has_store(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| is_store_name(&entry.path()))
}

fn is_store_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == STORE_FILE || name.ends_with(".sqlite") || name.ends_with(".db")
}

fn same_machine_bytes(cli: &Path, scenario_machine: &Path) -> Result<(), String> {
    let left = read_bytes(cli)?;
    let right = read_bytes(scenario_machine)?;
    if left != right {
        return Err("scenario machine path does not match the machine argument".into());
    }
    Ok(())
}

fn resolve_scenario_machine(scenario_path: &Path, relative: &str) -> Result<PathBuf, String> {
    resolve_relative_definition(scenario_path, relative, "scenario machine")
}

fn resolve_prior_machine(scenario_path: &Path, relative: &str) -> Result<PathBuf, String> {
    let dir = scenario_path.parent().unwrap_or_else(|| Path::new("."));
    let bundled = dir.join("prior_machine.ik.toml");
    if bundled.is_file() {
        return Ok(bundled);
    }
    resolve_relative_definition(scenario_path, relative, "prior machine")
}

fn resolve_relative_definition(
    scenario_path: &Path,
    relative: &str,
    what: &str,
) -> Result<PathBuf, String> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute() {
        return Err(format!("{what} path must be relative to the scenario file"));
    }
    let base = scenario_path.parent().unwrap_or_else(|| Path::new("."));
    let joined = base.join(relative_path);
    if !joined.is_file() {
        return Err(format!("{what} not found at {}", joined.display()));
    }
    Ok(joined)
}

fn load_prior_machine(
    scenario_path: &Path,
    scenario: &ScenarioFile,
) -> Result<Option<(PathBuf, crate::declarative::CompiledMachine)>, String> {
    let Some(relative) = prior_machine_path(scenario) else {
        return Ok(None);
    };
    let path = resolve_prior_machine(scenario_path, relative)?;
    let compiled = load_def(&path).map_err(|errors| errors.join("; "))?;
    Ok(Some((path, compiled)))
}

fn prior_machine_path(scenario: &ScenarioFile) -> Option<&str> {
    match scenario.steps.first() {
        Some(StepSpec::PriorMachine { path, .. }) => Some(path.as_str()),
        _ => None,
    }
}

fn prior_ref(
    loaded: Option<&(PathBuf, crate::declarative::CompiledMachine)>,
) -> Option<(&dyn Machine, &MachineIdentity)> {
    loaded.map(|(_, compiled)| {
        let machine: &dyn Machine = compiled;
        (machine, compiled.identity())
    })
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))
}

fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = read_bytes(path)?;
    serde_json::from_slice(&bytes).map_err(|err| format!("{}: {err}", path.display()))
}

#[derive(Debug, Deserialize)]
struct ScenarioFile {
    scenario_version: u64,
    machine: String,
    aggregate: String,
    steps: Vec<StepSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StepSpec {
    Command {
        #[serde(default)]
        label: Option<String>,
        name: String,
        #[serde(default)]
        args: Map<String, Value>,
        #[serde(default)]
        operation_id: Option<String>,
        #[serde(default)]
        handle: Option<String>,
        #[serde(default)]
        use_handle: Option<String>,
        #[serde(default)]
        present: Option<String>,
        expect: String,
    },
    PriorMachine {
        #[serde(default)]
        label: Option<String>,
        path: String,
    },
    DropReply {
        #[serde(default)]
        label: Option<String>,
        of: String,
    },
    Lookup {
        #[serde(default)]
        label: Option<String>,
        operation_id: String,
        expect: String,
    },
    Duplicate {
        #[serde(default)]
        label: Option<String>,
        of: String,
    },
    Reopen {
        #[serde(default)]
        label: Option<String>,
    },
    EffectOutcome {
        #[serde(default)]
        label: Option<String>,
        effect_id: String,
        payload_digest: String,
        idempotency_key: String,
        planned_generation: u64,
        outcome_digest: String,
        #[serde(default)]
        operation_id: Option<String>,
        expect: String,
    },
}

fn parse_scenario(bytes: &[u8]) -> Result<ScenarioFile, String> {
    let scenario: ScenarioFile =
        serde_json::from_slice(bytes).map_err(|err| format!("scenario: {err}"))?;
    if scenario.scenario_version != 1 {
        return Err(format!(
            "unsupported scenario_version {}",
            scenario.scenario_version
        ));
    }
    if scenario.aggregate.is_empty() || scenario.machine.is_empty() {
        return Err("aggregate id and machine path are required".into());
    }
    validate_steps(&scenario.steps)?;
    Ok(scenario)
}

fn validate_steps(steps: &[StepSpec]) -> Result<(), String> {
    let mut labels = BTreeMap::<String, &'static str>::new();
    let mut handles = BTreeSet::new();
    for (index, step) in steps.iter().enumerate() {
        let label = resolved_label(step_label(step), index)?;
        if labels.insert(label.clone(), step_kind(step)).is_some() {
            return Err(format!("duplicate step label `{label}`"));
        }
        validate_expect(step)?;
        validate_operation_id(step, &label)?;
        if let Some(handle) = step_use_handle(step) {
            if !handles.contains(handle) {
                return Err(format!(
                    "use_handle `{handle}` was not retained by an earlier step"
                ));
            }
        }
        validate_reference(step, &label, &labels)?;
        if let Some(handle) = step_retains(step) {
            handles.insert(handle.to_string());
        }
    }
    validate_prior_machine(steps)?;
    Ok(())
}

fn validate_prior_machine(steps: &[StepSpec]) -> Result<(), String> {
    let positions: Vec<usize> = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| matches!(step, StepSpec::PriorMachine { .. }))
        .map(|(index, _)| index)
        .collect();
    if positions.len() > 1 {
        return Err("only one prior_machine step is allowed".into());
    }
    if positions.first() == Some(&0) || positions.is_empty() {
        return validate_present_fields(steps, !positions.is_empty());
    }
    Err("prior_machine must be the first step".into())
}

fn validate_present_fields(steps: &[StepSpec], has_prior: bool) -> Result<(), String> {
    for step in steps {
        let StepSpec::Command {
            present: Some(side),
            ..
        } = step
        else {
            continue;
        };
        match side.as_str() {
            "new" => {}
            "prior" if has_prior => {}
            "prior" => return Err("present=prior requires a prior_machine step".into()),
            other => return Err(format!("present must be prior or new, not `{other}`")),
        }
    }
    Ok(())
}

fn validate_operation_id(step: &StepSpec, label: &str) -> Result<(), String> {
    if let Some(operation_id) = step_explicit_operation(step) {
        return OperationId::parse(operation_id)
            .map(|_| ())
            .map_err(|err| err.to_string());
    }
    if step_assigns_operation(step) {
        OperationId::parse(&format!("op-{label}"))
            .map(|_| ())
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn validate_reference(
    step: &StepSpec,
    label: &str,
    labels: &BTreeMap<String, &'static str>,
) -> Result<(), String> {
    let Some(target) = step_refers_to(step) else {
        return Ok(());
    };
    let Some(kind) = labels.get(target) else {
        return Err(format!("step `{label}` refers to unknown label `{target}`"));
    };
    if matches!(step, StepSpec::Duplicate { .. }) && !matches!(*kind, "command" | "effect_outcome")
    {
        return Err(format!(
            "duplicate `{target}` must name a command or effect_outcome step"
        ));
    }
    Ok(())
}

fn validate_expect(step: &StepSpec) -> Result<(), String> {
    match step {
        StepSpec::Command { expect, .. } => expect_one_of(expect, &["committed", "noop"], true),
        StepSpec::Lookup { expect, .. } => expect_one_of(
            expect,
            &["committed_exact", "conflict", "not_recorded"],
            false,
        ),
        StepSpec::EffectOutcome { expect, .. } => {
            expect_one_of(expect, &["committed", "noop", "committed_exact"], true)
        }
        StepSpec::DropReply { .. }
        | StepSpec::Duplicate { .. }
        | StepSpec::Reopen { .. }
        | StepSpec::PriorMachine { .. } => Ok(()),
    }
}

fn expect_one_of(expect: &str, allowed: &[&str], allow_refused: bool) -> Result<(), String> {
    if allowed.contains(&expect) || (allow_refused && refused_code(expect).is_some()) {
        return Ok(());
    }
    Err(format!("unsupported expectation `{expect}`"))
}

fn refused_code(expect: &str) -> Option<&str> {
    expect
        .strip_prefix("refused:")
        .filter(|code| !code.is_empty())
}

fn step_label(step: &StepSpec) -> Option<&str> {
    match step {
        StepSpec::Command { label, .. }
        | StepSpec::PriorMachine { label, .. }
        | StepSpec::DropReply { label, .. }
        | StepSpec::Lookup { label, .. }
        | StepSpec::Duplicate { label, .. }
        | StepSpec::Reopen { label, .. }
        | StepSpec::EffectOutcome { label, .. } => label.as_deref(),
    }
}

fn step_kind(step: &StepSpec) -> &'static str {
    match step {
        StepSpec::Command { .. } => "command",
        StepSpec::PriorMachine { .. } => "prior_machine",
        StepSpec::DropReply { .. } => "drop_reply",
        StepSpec::Lookup { .. } => "lookup",
        StepSpec::Duplicate { .. } => "duplicate",
        StepSpec::Reopen { .. } => "reopen",
        StepSpec::EffectOutcome { .. } => "effect_outcome",
    }
}

fn step_use_handle(step: &StepSpec) -> Option<&str> {
    match step {
        StepSpec::Command { use_handle, .. } => use_handle.as_deref(),
        _ => None,
    }
}

fn step_retains(step: &StepSpec) -> Option<&str> {
    match step {
        StepSpec::Command { handle, .. } => handle.as_deref(),
        _ => None,
    }
}

fn step_refers_to(step: &StepSpec) -> Option<&str> {
    match step {
        StepSpec::DropReply { of, .. } | StepSpec::Duplicate { of, .. } => Some(of.as_str()),
        _ => None,
    }
}

fn step_explicit_operation(step: &StepSpec) -> Option<&str> {
    match step {
        StepSpec::Command { operation_id, .. } | StepSpec::EffectOutcome { operation_id, .. } => {
            operation_id.as_deref()
        }
        _ => None,
    }
}

fn step_assigns_operation(step: &StepSpec) -> bool {
    matches!(
        step,
        StepSpec::Command { .. } | StepSpec::EffectOutcome { .. }
    )
}

fn resolved_label(label: Option<&str>, index: usize) -> Result<String, String> {
    let label = label.map_or_else(|| format!("step-{}", index + 1), str::to_string);
    if valid_token(&label) {
        Ok(label)
    } else {
        Err(format!("label `{label}` must match [A-Za-z0-9._:-]+"))
    }
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

#[derive(Clone)]
struct Submitted {
    kind: SubmittedKind,
    capability: Capability,
    command: Value,
    effect: Option<EffectOutcomeV1>,
}

#[derive(Clone, Copy)]
enum SubmittedKind {
    Command,
    Record,
}

struct StepNote {
    committed: bool,
    operation_id: Option<String>,
}

struct Drive {
    events: Vec<Value>,
    steps: Vec<Value>,
    faults: Vec<Value>,
    seams: Vec<String>,
    all_met: bool,
    failed_label: Option<String>,
}

struct PresentChoice<'a> {
    machine: &'a dyn Machine,
    identity: MachineIdentity,
    side: &'static str,
}

struct Session<'a> {
    store: Option<Store>,
    store_path: &'a Path,
    machine: &'a dyn Machine,
    identity: MachineIdentity,
    prior: Option<&'a dyn Machine>,
    prior_identity: Option<MachineIdentity>,
    created: MachineIdentity,
    aggregate: AggregateId,
    view: Option<Projection>,
    handles: BTreeMap<String, Capability>,
    submitted: BTreeMap<String, Submitted>,
    notes: BTreeMap<String, StepNote>,
    accepted_outcomes: BTreeMap<String, String>,
    events: Vec<Value>,
    steps: Vec<Value>,
    faults: Vec<Value>,
    seams: Vec<String>,
    all_met: bool,
    failed_label: Option<String>,
    stopped: bool,
}

enum Classified {
    Committed {
        capability: Capability,
    },
    Noop,
    Historical {
        operation_id: String,
        first: u64,
        last: u64,
    },
    Refused {
        code: String,
        message: String,
    },
}

impl Classified {
    fn outcome(&self) -> &'static str {
        match self {
            Self::Committed { .. } => "committed",
            Self::Noop => "noop",
            Self::Historical { .. } => "historical",
            Self::Refused { .. } => "refused",
        }
    }

    fn code(&self) -> Option<&str> {
        match self {
            Self::Refused { code, .. } => Some(code.as_str()),
            _ => None,
        }
    }
}

fn drive(
    store: Store,
    store_path: &Path,
    machine: &dyn Machine,
    identity: &MachineIdentity,
    prior: Option<(&dyn Machine, &MachineIdentity)>,
    scenario: &ScenarioFile,
) -> Result<Drive, String> {
    let mut session = Session {
        store: Some(store),
        store_path,
        machine,
        identity: identity.clone(),
        prior: prior.map(|(loaded, _)| loaded),
        prior_identity: prior.map(|(_, id)| id.clone()),
        created: identity.clone(),
        aggregate: AggregateId(scenario.aggregate.clone()),
        view: None,
        handles: BTreeMap::new(),
        submitted: BTreeMap::new(),
        notes: BTreeMap::new(),
        accepted_outcomes: BTreeMap::new(),
        events: Vec::new(),
        steps: Vec::new(),
        faults: Vec::new(),
        seams: Vec::new(),
        all_met: true,
        failed_label: None,
        stopped: false,
    };
    if prior.is_none() {
        session.open_genesis()?;
    }
    for (index, step) in scenario.steps.iter().enumerate() {
        if session.stopped {
            break;
        }
        session.apply(step, index)?;
    }
    Ok(session.finish())
}

impl<'a> Session<'a> {
    fn finish(self) -> Drive {
        Drive {
            events: self.events,
            steps: self.steps,
            faults: self.faults,
            seams: self.seams,
            all_met: self.all_met,
            failed_label: self.failed_label,
        }
    }

    fn open_genesis(&mut self) -> Result<(), String> {
        let machine = self.machine;
        let aggregate = self.aggregate.clone();
        let identity = self.identity.clone();
        let created = self
            .store_mut()?
            .create_aggregate(machine, &aggregate, &identity);
        self.record_genesis(created)
    }

    fn record_genesis(&mut self, created: Result<CommandResult, Diagnostic>) -> Result<(), String> {
        let request = json!({ "aggregate": self.aggregate.0 });
        match created {
            Ok(_) => {
                self.refresh()?;
                let name = self.state_name();
                let generation = self.generation();
                let event = trace_commit("create", "(none)", &name, generation);
                self.push_step(
                    "genesis",
                    "create",
                    request,
                    "committed",
                    None,
                    "committed",
                    true,
                    event,
                );
            }
            Err(err) => {
                let code = err.code.as_str();
                let event = trace_refusal("create", "(none)", &code, &err.message, None);
                self.push_step(
                    "genesis",
                    "create",
                    request,
                    "refused",
                    Some(&code),
                    "committed",
                    false,
                    event,
                );
                self.stopped = true;
            }
        }
        Ok(())
    }

    fn apply(&mut self, step: &StepSpec, index: usize) -> Result<(), String> {
        let label = resolved_label(step_label(step), index)?;
        match step {
            StepSpec::Command {
                name,
                args,
                operation_id,
                handle,
                use_handle,
                present,
                expect,
                ..
            } => self.apply_command(
                &label,
                name,
                args,
                operation_id.as_deref(),
                handle.as_deref(),
                use_handle.as_deref(),
                present.as_deref(),
                expect,
            ),
            StepSpec::PriorMachine { path, .. } => self.apply_prior_machine(&label, path),
            StepSpec::DropReply { of, .. } => self.apply_drop(&label, of),
            StepSpec::Lookup {
                operation_id,
                expect,
                ..
            } => self.apply_lookup(&label, operation_id, expect),
            StepSpec::Duplicate { of, .. } => self.apply_duplicate(&label, of),
            StepSpec::Reopen { .. } => self.apply_reopen(&label),
            StepSpec::EffectOutcome {
                effect_id,
                payload_digest,
                idempotency_key,
                planned_generation,
                outcome_digest,
                operation_id,
                expect,
                ..
            } => self.apply_effect(
                &label,
                effect_id,
                payload_digest,
                idempotency_key,
                *planned_generation,
                outcome_digest,
                operation_id.as_deref(),
                expect,
            ),
        }
    }

    fn apply_command(
        &mut self,
        label: &str,
        name: &str,
        args: &Map<String, Value>,
        operation_id: Option<&str>,
        handle: Option<&str>,
        use_handle: Option<&str>,
        present: Option<&str>,
        expect: &str,
    ) -> Result<(), String> {
        let from = self.state_name();
        let operation_id = self.operation_id(operation_id, label)?;
        let reused = self.submitted.contains_key(operation_id.as_str());
        let capability = self.capability_for(use_handle)?;
        let stale = use_handle.is_some() && capability != self.current_capability()?;
        let command = command_value(name, args);
        let (machine, presented, side) = {
            let choice = self.select_present(present)?;
            (choice.machine, choice.identity, choice.side)
        };
        let request = CommandRequest {
            aggregate_id: self.aggregate.clone(),
            machine: presented.clone(),
            capability: capability.clone(),
            command: command.clone(),
            observation: None,
        };
        let classified = classify(
            self.store_mut()?
                .command_op(machine, &request, &operation_id),
        );
        self.remember_command(operation_id.as_str(), capability, command.clone());
        let retained = self.retain(handle, &classified);
        self.refresh()?;
        let passed = expectation_met(expect, classified.outcome(), classified.code()) && retained;
        let event = mutation_event(
            name,
            &from,
            &self.state_name(),
            self.generation(),
            &classified,
        );
        if stale {
            self.note_fault(
                "stale_handle_used",
                classified.code() == Some("IK_CAPABILITY_STALE"),
                json!({
                    "diagnostic_code": classified.code(),
                    "generation": self.generation(),
                }),
            );
        }
        if reused {
            self.note_fault(
                "duplicate_submission",
                duplicate_fired(&classified),
                json!({
                    "outcome": classified.outcome(),
                    "diagnostic_code": classified.code(),
                    "generation": self.generation(),
                }),
            );
        }
        self.notes.insert(
            label.to_string(),
            StepNote {
                committed: classified.outcome() == "committed",
                operation_id: Some(operation_id.as_str().to_string()),
            },
        );
        let summary = json!({
            "name": name,
            "args": command.get("args").cloned().unwrap_or(json!({})),
            "operation_id": operation_id.as_str(),
            "use_handle": use_handle,
        });
        self.push_step(
            label,
            "command",
            summary,
            classified.outcome(),
            classified.code(),
            expect,
            passed,
            event,
        );
        if self.prior.is_some() {
            self.attach_present(side, &presented, Some(name));
        }
        Ok(())
    }

    fn apply_prior_machine(&mut self, label: &str, path: &str) -> Result<(), String> {
        let Some(machine) = self.prior else {
            return Err(format!("prior machine `{path}` was not loaded"));
        };
        let identity = self
            .prior_identity
            .clone()
            .ok_or_else(|| format!("prior machine `{path}` has no identity"))?;
        if self.view.is_some() {
            return Err("prior_machine must run before the instance is created".into());
        }
        let aggregate = self.aggregate.clone();
        let created = self
            .store_mut()?
            .create_aggregate(machine, &aggregate, &identity);
        self.created = identity.clone();
        let request = json!({ "aggregate": self.aggregate.0, "path": path });
        match created {
            Ok(_) => {
                self.refresh()?;
                let name = self.state_name();
                let generation = self.generation();
                let event = trace_commit("create", "(none)", &name, generation);
                self.push_step(
                    label,
                    "prior_machine",
                    request,
                    "committed",
                    None,
                    "committed",
                    true,
                    event,
                );
                self.attach_present("prior", &identity, None);
            }
            Err(err) => {
                let code = err.code.as_str();
                let event = trace_refusal("create", "(none)", &code, &err.message, None);
                self.push_step(
                    label,
                    "prior_machine",
                    request,
                    "refused",
                    Some(&code),
                    "committed",
                    false,
                    event,
                );
                self.stopped = true;
            }
        }
        Ok(())
    }

    fn select_present(&self, present: Option<&str>) -> Result<PresentChoice<'a>, String> {
        match present {
            None if self.prior.is_some() => self.choice_prior(),
            None | Some("new") => Ok(PresentChoice {
                machine: self.machine,
                identity: self.identity.clone(),
                side: "new",
            }),
            Some("prior") => self.choice_prior(),
            Some(other) => Err(format!("present must be prior or new, not `{other}`")),
        }
    }

    fn choice_prior(&self) -> Result<PresentChoice<'a>, String> {
        let Some(machine) = self.prior else {
            return Err("present=prior requires a prior_machine step".into());
        };
        let Some(identity) = self.prior_identity.clone() else {
            return Err("present=prior requires a prior_machine step".into());
        };
        Ok(PresentChoice {
            machine,
            identity,
            side: "prior",
        })
    }

    fn attach_present(&mut self, side: &str, presented: &MachineIdentity, command: Option<&str>) {
        let code = self
            .steps
            .last()
            .and_then(|step| step.get("response"))
            .and_then(|response| response.get("diagnostic_code"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let Some(last) = self.steps.last_mut() else {
            return;
        };
        let Some(object) = last.as_object_mut() else {
            return;
        };
        object.insert(
            "present".into(),
            present_observation(&self.created, presented, side, command, code.as_deref()),
        );
    }

    fn apply_drop(&mut self, label: &str, of: &str) -> Result<(), String> {
        let committed = self.notes.get(of).is_some_and(|note| note.committed);
        let proof = self.drop_proof(of, committed)?;
        let fired = proof
            .get("lookup")
            .and_then(|item| item.get("outcome"))
            .and_then(Value::as_str)
            == Some("committed_exact")
            && committed;
        self.note_fault("reply_dropped", fired, proof);
        let outcome = if committed {
            "reply_dropped"
        } else {
            "not_committed"
        };
        let event = trace_noop(
            &format!("drop_reply {of}"),
            &self.state_name(),
            self.generation(),
            "reply discarded; the commit stands",
        );
        self.notes.insert(
            label.to_string(),
            StepNote {
                committed: false,
                operation_id: None,
            },
        );
        self.push_step(
            label,
            "drop_reply",
            json!({ "of": of }),
            outcome,
            None,
            "reply_dropped",
            committed,
            event,
        );
        Ok(())
    }

    fn apply_lookup(
        &mut self,
        label: &str,
        operation_id: &str,
        expect: &str,
    ) -> Result<(), String> {
        let id = OperationId::parse(operation_id).map_err(|err| err.to_string())?;
        let lookup = self.lookup_id(&id, operation_id)?;
        let (outcome, event) = lookup_presentation(
            &format!("lookup {operation_id}"),
            &self.state_name(),
            self.generation(),
            lookup,
        );
        self.refresh()?;
        let passed = expectation_met(expect, outcome, None);
        self.notes.insert(
            label.to_string(),
            StepNote {
                committed: false,
                operation_id: Some(operation_id.to_string()),
            },
        );
        self.push_step(
            label,
            "lookup",
            json!({ "operation_id": operation_id }),
            outcome,
            None,
            expect,
            passed,
            event,
        );
        Ok(())
    }

    fn apply_duplicate(&mut self, label: &str, of: &str) -> Result<(), String> {
        let operation_id = self
            .notes
            .get(of)
            .and_then(|note| note.operation_id.clone())
            .ok_or_else(|| format!("duplicate target `{of}` has no operation id"))?;
        let submitted = self
            .submitted
            .get(&operation_id)
            .ok_or_else(|| format!("operation `{operation_id}` was not submitted"))?
            .clone();
        let id = OperationId::parse(&operation_id).map_err(|err| err.to_string())?;
        let from = self.state_name();
        let generation_before = self.generation();
        let classified = self.resubmit(&submitted, &id)?;
        self.refresh()?;
        let fired = duplicate_fired(&classified);
        self.note_fault(
            "duplicate_submission",
            fired,
            json!({
                "outcome": classified.outcome(),
                "generation_before": generation_before,
                "generation_after": self.generation(),
            }),
        );
        let passed = classified.outcome() == "historical" && self.generation() == generation_before;
        let event = mutation_event(
            &format!("duplicate {of}"),
            &from,
            &self.state_name(),
            self.generation(),
            &classified,
        );
        self.notes.insert(
            label.to_string(),
            StepNote {
                committed: false,
                operation_id: Some(operation_id.clone()),
            },
        );
        self.push_step(
            label,
            "duplicate",
            json!({ "of": of, "operation_id": operation_id }),
            classified.outcome(),
            classified.code(),
            "historical",
            passed,
            event,
        );
        Ok(())
    }

    fn apply_reopen(&mut self, label: &str) -> Result<(), String> {
        let before = self.state_after();
        self.store.take();
        match Store::open(self.store_path) {
            Ok(store) => {
                self.store = Some(store);
                self.refresh()?;
                let fired = before == self.state_after();
                self.note_fault(
                    "reopened",
                    fired,
                    json!({
                        "generation_before": before.get("generation").cloned().unwrap_or(Value::Null),
                        "generation_after": self.generation(),
                        "state_unchanged": fired,
                    }),
                );
                let outcome = if fired { "reopened" } else { "diverged" };
                let event = trace_read(
                    "reopen",
                    &self.state_name(),
                    self.generation(),
                    Some("store reopened"),
                );
                self.notes.insert(
                    label.to_string(),
                    StepNote {
                        committed: false,
                        operation_id: None,
                    },
                );
                self.push_step(
                    label,
                    "reopen",
                    json!({}),
                    outcome,
                    None,
                    "reopened",
                    fired,
                    event,
                );
            }
            Err(err) => {
                let code = err.code.as_str();
                let message = err.message.clone();
                let from = self.state_name();
                self.note_fault(
                    "reopened",
                    false,
                    json!({"diagnostic_code": code, "state_unchanged": false}),
                );
                let event = trace_refusal("reopen", &from, &code, &message, None);
                self.push_step(
                    label,
                    "reopen",
                    json!({}),
                    "refused",
                    Some(&code),
                    "reopened",
                    false,
                    event,
                );
                self.stopped = true;
            }
        }
        Ok(())
    }

    fn apply_effect(
        &mut self,
        label: &str,
        effect_id: &str,
        payload_digest: &str,
        idempotency_key: &str,
        planned_generation: u64,
        outcome_digest: &str,
        operation_id: Option<&str>,
        expect: &str,
    ) -> Result<(), String> {
        let from = self.state_name();
        let generation_before = self.generation();
        let operation_id = self.operation_id(operation_id, label)?;
        let reused = self.submitted.contains_key(operation_id.as_str());
        let prior = self.accepted_outcomes.get(effect_id).cloned();
        let conflicts = prior
            .as_ref()
            .is_some_and(|digest| digest != outcome_digest)
            || expect == "refused:IK_EFFECT_OUTCOME_CONFLICT";
        let effect = EffectOutcomeV1 {
            effect_id: EffectId(effect_id.to_string()),
            aggregate_id: self.aggregate.clone(),
            planned_generation,
            payload_digest: payload_digest.to_string(),
            idempotency_key: idempotency_key.to_string(),
            outcome_digest: outcome_digest.to_string(),
        };
        let capability = self.current_capability()?;
        let request = EffectOutcomeRequest {
            aggregate_id: self.aggregate.clone(),
            machine: self.identity.clone(),
            capability: capability.clone(),
            effect: effect.clone(),
        };
        let classified = classify(self.store_mut()?.record_effect_op(&request, &operation_id));
        self.remember_record(operation_id.as_str(), capability, effect);
        self.refresh()?;
        if matches!(classified.outcome(), "committed" | "noop" | "historical") {
            self.accepted_outcomes
                .insert(effect_id.to_string(), outcome_digest.to_string());
        }
        let fired = effect_conflict_fired(
            &classified,
            generation_before,
            self.generation(),
            prior.as_deref(),
            self.recorded_digest(),
        );
        if reused {
            self.note_fault(
                "duplicate_submission",
                duplicate_fired(&classified),
                json!({
                    "outcome": classified.outcome(),
                    "generation_before": generation_before,
                    "generation_after": self.generation(),
                }),
            );
        }
        if conflicts {
            self.note_fault(
                "conflicting_effect_outcome",
                fired,
                json!({
                    "diagnostic_code": classified.code(),
                    "generation_before": generation_before,
                    "generation_after": self.generation(),
                    "prior_digest": prior,
                }),
            );
        }
        let passed = expectation_met(expect, classified.outcome(), classified.code());
        let event = mutation_event(
            &format!("record {effect_id}"),
            &from,
            &self.state_name(),
            self.generation(),
            &classified,
        );
        self.notes.insert(
            label.to_string(),
            StepNote {
                committed: classified.outcome() == "committed",
                operation_id: Some(operation_id.as_str().to_string()),
            },
        );
        let summary = json!({
            "effect_id": effect_id,
            "payload_digest": payload_digest,
            "idempotency_key": idempotency_key,
            "planned_generation": planned_generation,
            "outcome_digest": outcome_digest,
            "operation_id": operation_id.as_str(),
        });
        self.push_step(
            label,
            "effect_outcome",
            summary,
            classified.outcome(),
            classified.code(),
            expect,
            passed,
            event,
        );
        Ok(())
    }

    fn lookup_id(&mut self, id: &OperationId, raw: &str) -> Result<Lookup, String> {
        if self.submitted.contains_key(raw) {
            let kind = self.submitted.get(raw).map(|item| match item.kind {
                SubmittedKind::Command => "command",
                SubmittedKind::Record => "record",
            });
            return match kind {
                Some("command") => self.lookup_command(id, raw),
                Some("record") => self.lookup_record(id, raw),
                _ => Err(format!("operation `{raw}` has no stored request")),
            };
        }
        self.lookup_unknown(id)
    }

    fn lookup_command(&mut self, id: &OperationId, raw: &str) -> Result<Lookup, String> {
        let submitted = self
            .submitted
            .get(raw)
            .ok_or_else(|| format!("missing {raw}"))?;
        let request = CommandRequest {
            aggregate_id: self.aggregate.clone(),
            machine: self.identity.clone(),
            capability: submitted.capability.clone(),
            command: submitted.command.clone(),
            observation: None,
        };
        self.store_mut()?
            .lookup_operation(id, &OperationRequest::Command(&request))
            .map_err(|err| err.to_string())
    }

    fn lookup_record(&mut self, id: &OperationId, raw: &str) -> Result<Lookup, String> {
        let submitted = self
            .submitted
            .get(raw)
            .ok_or_else(|| format!("missing {raw}"))?;
        let effect = submitted
            .effect
            .clone()
            .ok_or_else(|| format!("missing effect for {raw}"))?;
        let request = EffectOutcomeRequest {
            aggregate_id: self.aggregate.clone(),
            machine: self.identity.clone(),
            capability: submitted.capability.clone(),
            effect,
        };
        self.store_mut()?
            .lookup_operation(id, &OperationRequest::Record(&request))
            .map_err(|err| err.to_string())
    }

    fn lookup_unknown(&mut self, id: &OperationId) -> Result<Lookup, String> {
        let request = CommandRequest {
            aggregate_id: self.aggregate.clone(),
            machine: self.identity.clone(),
            capability: self.current_capability()?,
            command: json!({"name": "lookup", "args": {}}),
            observation: None,
        };
        self.store_mut()?
            .lookup_operation(id, &OperationRequest::Command(&request))
            .map_err(|err| err.to_string())
    }

    fn resubmit(&mut self, submitted: &Submitted, id: &OperationId) -> Result<Classified, String> {
        match submitted.kind {
            SubmittedKind::Command => {
                let request = CommandRequest {
                    aggregate_id: self.aggregate.clone(),
                    machine: self.identity.clone(),
                    capability: submitted.capability.clone(),
                    command: submitted.command.clone(),
                    observation: None,
                };
                let machine = self.machine;
                Ok(classify(
                    self.store_mut()?.command_op(machine, &request, id),
                ))
            }
            SubmittedKind::Record => {
                let effect = submitted
                    .effect
                    .clone()
                    .ok_or_else(|| "missing effect".to_string())?;
                let request = EffectOutcomeRequest {
                    aggregate_id: self.aggregate.clone(),
                    machine: self.identity.clone(),
                    capability: submitted.capability.clone(),
                    effect,
                };
                Ok(classify(self.store_mut()?.record_effect_op(&request, id)))
            }
        }
    }

    fn remember_command(&mut self, operation_id: &str, capability: Capability, command: Value) {
        self.submitted
            .entry(operation_id.to_string())
            .or_insert(Submitted {
                kind: SubmittedKind::Command,
                capability,
                command,
                effect: None,
            });
    }

    fn remember_record(
        &mut self,
        operation_id: &str,
        capability: Capability,
        effect: EffectOutcomeV1,
    ) {
        self.submitted
            .entry(operation_id.to_string())
            .or_insert(Submitted {
                kind: SubmittedKind::Record,
                capability,
                command: Value::Null,
                effect: Some(effect),
            });
    }

    fn retain(&mut self, handle: Option<&str>, classified: &Classified) -> bool {
        let Some(handle) = handle else {
            return true;
        };
        let Classified::Committed { capability } = classified else {
            return false;
        };
        self.handles.insert(handle.to_string(), capability.clone());
        true
    }

    fn capability_for(&self, use_handle: Option<&str>) -> Result<Capability, String> {
        match use_handle {
            Some(name) => self
                .handles
                .get(name)
                .cloned()
                .ok_or_else(|| format!("handle `{name}` was not retained")),
            None => self.current_capability(),
        }
    }

    fn current_capability(&self) -> Result<Capability, String> {
        self.view
            .as_ref()
            .map(|view| view.capability.clone())
            .ok_or_else(|| "aggregate has no current capability".to_string())
    }

    fn operation_id(&self, explicit: Option<&str>, label: &str) -> Result<OperationId, String> {
        let raw = explicit.map_or_else(|| format!("op-{label}"), str::to_string);
        OperationId::parse(&raw).map_err(|err| err.to_string())
    }

    fn store_mut(&mut self) -> Result<&mut Store, String> {
        self.store
            .as_mut()
            .ok_or_else(|| "store is not open".to_string())
    }

    fn refresh(&mut self) -> Result<(), String> {
        let aggregate = self.aggregate.clone();
        let projection = self
            .store_mut()?
            .show(&aggregate)
            .map_err(|err| err.to_string())?;
        self.view = Some(projection);
        Ok(())
    }

    fn state_name(&self) -> String {
        self.view
            .as_ref()
            .map(|view| name_of(&view.state))
            .unwrap_or_else(|| "(none)".into())
    }

    fn generation(&self) -> u64 {
        self.view.as_ref().map(|view| view.generation).unwrap_or(0)
    }

    fn recorded_digest(&self) -> Option<&str> {
        match self.view.as_ref().map(|view| &view.effect) {
            Some(EffectStatus::Recorded(_, digest, _)) => Some(digest.as_str()),
            _ => None,
        }
    }

    fn state_after(&self) -> Value {
        let Some(view) = &self.view else {
            return Value::Null;
        };
        json!({
            "state": name_of(&view.state),
            "generation": view.generation,
            "facts": facts_of(&view.state),
            "effect": serde_json::to_value(&view.effect).unwrap_or(Value::Null),
        })
    }

    fn note_fault(&mut self, name: &str, fired: bool, proof: Value) {
        self.faults.push(json!({
            "name": name,
            "seam_fired": fired,
            "proof": proof,
        }));
        if fired {
            self.seams.push(name.to_string());
        } else {
            self.all_met = false;
            if self.failed_label.is_none() {
                self.failed_label = Some(format!("fault:{name}"));
            }
        }
    }

    fn drop_proof(&mut self, of: &str, committed: bool) -> Result<Value, String> {
        let operation_id = self
            .notes
            .get(of)
            .and_then(|note| note.operation_id.clone());
        let lookup = match operation_id.as_deref() {
            Some(raw) => Some(self.lookup_summary(raw)?),
            None => None,
        };
        Ok(json!({
            "of": of,
            "named_step_committed": committed,
            "generation": self.generation(),
            "operation_id": operation_id,
            "lookup": lookup,
        }))
    }

    fn lookup_summary(&mut self, raw: &str) -> Result<Value, String> {
        let id = OperationId::parse(raw).map_err(|err| err.to_string())?;
        Ok(lookup_proof(&self.lookup_id(&id, raw)?))
    }

    fn push_step(
        &mut self,
        label: &str,
        kind: &str,
        request: Value,
        outcome: &str,
        code: Option<&str>,
        expectation: &str,
        passed: bool,
        event: Value,
    ) {
        let verdict = if passed { "pass" } else { "fail" };
        self.steps.push(json!({
            "label": label,
            "kind": kind,
            "request": request,
            "response": response_of(outcome, code),
            "state_after": self.state_after(),
            "expectation": expectation,
            "verdict": verdict,
        }));
        self.events.push(event);
        self.notes.entry(label.to_string()).or_insert(StepNote {
            committed: outcome == "committed",
            operation_id: None,
        });
        if !passed && self.failed_label.is_none() {
            self.all_met = false;
            self.failed_label = Some(label.to_string());
        }
    }
}

fn lookup_proof(lookup: &Lookup) -> Value {
    match lookup {
        Lookup::CommittedExact(history) => json!({
            "outcome": "committed_exact",
            "first_sequence": history.record.first_sequence,
            "last_sequence": history.record.last_sequence,
            "generation": history.record.generation,
        }),
        Lookup::Conflict {
            current_generation,
            current_sequence,
            ..
        } => json!({
            "outcome": "conflict",
            "current_generation": current_generation,
            "current_sequence": current_sequence,
        }),
        Lookup::NotRecorded {
            current_generation,
            current_sequence,
        } => json!({
            "outcome": "not_recorded",
            "current_generation": current_generation,
            "current_sequence": current_sequence,
        }),
    }
}

fn duplicate_fired(classified: &Classified) -> bool {
    classified.outcome() == "historical" || classified.code() == Some("IK_OPERATION_CONFLICT")
}

fn effect_conflict_fired(
    classified: &Classified,
    before: u64,
    after: u64,
    prior: Option<&str>,
    recorded: Option<&str>,
) -> bool {
    if classified.code() != Some("IK_EFFECT_OUTCOME_CONFLICT") || before != after {
        return false;
    }
    match prior {
        Some(prior) => recorded == Some(prior),
        None => true,
    }
}

fn classify(result: Result<OperationOutcome<CommandResult>, Diagnostic>) -> Classified {
    match result {
        Err(err) => Classified::Refused {
            code: err.code.as_str(),
            message: err.message,
        },
        Ok(OperationOutcome::Historical(history)) => historical_of(history),
        Ok(OperationOutcome::Committed(result)) if result.mutation => Classified::Committed {
            capability: result.capability,
        },
        Ok(OperationOutcome::Committed(_)) => Classified::Noop,
    }
}

fn historical_of(history: OperationHistory) -> Classified {
    Classified::Historical {
        operation_id: history.record.operation_id,
        first: history.record.first_sequence,
        last: history.record.last_sequence,
    }
}

fn expectation_met(expect: &str, outcome: &str, code: Option<&str>) -> bool {
    if let Some(expected) = refused_code(expect) {
        return outcome == "refused" && code == Some(expected);
    }
    if expect == "committed_exact" {
        return outcome == "committed_exact" || outcome == "historical";
    }
    outcome == expect
}

fn lookup_presentation(
    attempted: &str,
    state: &str,
    generation: u64,
    lookup: Lookup,
) -> (&'static str, Value) {
    match lookup {
        Lookup::CommittedExact(history) => (
            "committed_exact",
            trace_historical(
                attempted,
                &history.record.operation_id,
                history.record.first_sequence,
                history.record.last_sequence,
            ),
        ),
        Lookup::Conflict { .. } => (
            "conflict",
            trace_read(attempted, state, generation, Some("conflict")),
        ),
        Lookup::NotRecorded { .. } => (
            "not_recorded",
            trace_read(attempted, state, generation, Some("not_recorded")),
        ),
    }
}

fn mutation_event(
    attempted: &str,
    from: &str,
    to: &str,
    generation: u64,
    classified: &Classified,
) -> Value {
    match classified {
        Classified::Committed { .. } => trace_commit(attempted, from, to, generation),
        Classified::Noop => trace_noop(attempted, to, generation, "nothing committed"),
        Classified::Historical {
            operation_id,
            first,
            last,
        } => trace_historical(attempted, operation_id, *first, *last),
        Classified::Refused { code, message } => {
            trace_refusal(attempted, to, code, message, Some(generation))
        }
    }
}

fn command_value(name: &str, args: &Map<String, Value>) -> Value {
    json!({ "name": name, "args": args })
}

fn name_of(state: &Value) -> String {
    state
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn facts_of(state: &Value) -> Value {
    let mut facts = Map::new();
    if let Some(object) = state.as_object() {
        for (key, value) in object {
            if key != "state" {
                facts.insert(key.clone(), value.clone());
            }
        }
    }
    Value::Object(facts)
}

fn response_of(outcome: &str, code: Option<&str>) -> Value {
    let mut map = Map::new();
    map.insert("outcome".into(), json!(outcome));
    if let Some(code) = code {
        map.insert("diagnostic_code".into(), json!(code));
    }
    Value::Object(map)
}

fn trace_commit(attempted: &str, from: &str, to: &str, generation: u64) -> Value {
    json!({
        "kind": "commit",
        "attempted": attempted,
        "fromState": from,
        "toState": to,
        "generation": generation.to_string(),
    })
}

fn trace_refusal(
    attempted: &str,
    from: &str,
    code: &str,
    message: &str,
    generation: Option<u64>,
) -> Value {
    let mut map = Map::new();
    map.insert("kind".into(), json!("refusal"));
    map.insert("attempted".into(), json!(attempted));
    map.insert("fromState".into(), json!(from));
    map.insert("code".into(), json!(code));
    map.insert("message".into(), json!(message));
    if let Some(generation) = generation {
        map.insert("generation".into(), json!(generation.to_string()));
    }
    Value::Object(map)
}

fn trace_noop(attempted: &str, state: &str, generation: u64, message: &str) -> Value {
    json!({
        "kind": "no-op",
        "attempted": attempted,
        "state": state,
        "generation": generation.to_string(),
        "message": message,
    })
}

fn trace_read(attempted: &str, state: &str, generation: u64, message: Option<&str>) -> Value {
    let mut map = Map::new();
    map.insert("kind".into(), json!("read"));
    map.insert("attempted".into(), json!(attempted));
    map.insert("state".into(), json!(state));
    map.insert("generation".into(), json!(generation.to_string()));
    if let Some(message) = message {
        map.insert("message".into(), json!(message));
    }
    Value::Object(map)
}

fn trace_historical(attempted: &str, operation_id: &str, first: u64, last: u64) -> Value {
    json!({
        "kind": "historical",
        "attempted": attempted,
        "operationId": operation_id,
        "firstSequence": first.to_string(),
        "lastSequence": last.to_string(),
    })
}

fn result_document(
    scenario_bytes: &[u8],
    identity: &MachineIdentity,
    aggregate: &str,
    driven: &Drive,
) -> Result<Value, String> {
    Ok(json!({
        "schema": RESULT_SCHEMA,
        "scenario_digest": sha256_hex(scenario_bytes),
        "machine_id": identity.machine_id,
        "spec_hash": identity.spec_hash.0,
        "aggregate": aggregate,
        "runtime": runtime_identity()?,
        "steps": driven.steps,
        "faults_applied": driven.faults,
        "seams_fired": driven.seams,
        "all_expectations_met": driven.all_met,
    }))
}

fn runtime_identity() -> Result<Value, String> {
    let path = std::env::current_exe().map_err(|err| format!("current executable: {err}"))?;
    let bytes = read_bytes(&path)?;
    Ok(json!({
        "ik_version": RUNTIME_VERSION,
        "binary_sha256": sha256_hex(&bytes),
        "store_schema": SCHEMA_VERSION,
    }))
}

fn present_observation(
    created: &MachineIdentity,
    presented: &MachineIdentity,
    side: &str,
    command: Option<&str>,
    code: Option<&str>,
) -> Value {
    let mut map = Map::new();
    map.insert("created_version".into(), json!(created.version.0));
    map.insert("presented".into(), json!(side));
    map.insert("presented_version".into(), json!(presented.version.0));
    if let Some(name) = command {
        map.insert("command".into(), json!(name));
    }
    if let Some(code) = code {
        map.insert("code".into(), json!(code));
    }
    Value::Object(map)
}

fn write_bundle(
    out: &Path,
    scenario: &[u8],
    machine: &Path,
    prior: Option<&Path>,
    result: &Value,
    events: &[Value],
) -> Result<(), String> {
    fs::write(out.join("scenario.json"), scenario)
        .map_err(|err| format!("write scenario.json: {err}"))?;
    fs::copy(machine, out.join("machine.ik.toml")).map_err(|err| format!("copy machine: {err}"))?;
    if let Some(prior) = prior {
        fs::copy(prior, out.join("prior_machine.ik.toml"))
            .map_err(|err| format!("copy prior machine: {err}"))?;
    }
    fs::write(out.join("result.json"), canonical_json(result))
        .map_err(|err| format!("write result.json: {err}"))?;
    let machine_id = result
        .get("machine_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let trace = json!({
        "schema": TRACE_SCHEMA,
        "product": "Ikaros",
        "runtime_version": RUNTIME_VERSION,
        "machine_id": machine_id,
        "advisory": true,
        "origin": "observed",
        "reproducible": true,
        "redacted": false,
        "events": events,
        "reproduction": "ik scenario reproduce . --out <fresh-dir>",
    });
    fs::write(out.join("trace.json"), canonical_json(&trace))
        .map_err(|err| format!("write trace.json: {err}"))?;
    write_sums(out)
}

fn write_sums(out: &Path) -> Result<(), String> {
    let mut names = vec![
        "machine.ik.toml",
        "result.json",
        "scenario.json",
        "trace.json",
    ];
    if out.join("prior_machine.ik.toml").is_file() {
        names.push("prior_machine.ik.toml");
    }
    let mut lines = String::new();
    for name in names {
        let digest = sha256_hex(&read_bytes(&out.join(name))?);
        lines.push_str(&digest);
        lines.push_str("  ");
        lines.push_str(name);
        lines.push('\n');
    }
    fs::write(out.join("SHA256SUMS"), lines).map_err(|err| format!("write SHA256SUMS: {err}"))
}
