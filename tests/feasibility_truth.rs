//! Known-solution feasibility suite: ground truth independent of any backend.
//!
//! The in-house engine must not discharge a replayable witness and must not
//! mark a REACHABLE obligation impossible. Precision is reported, not gated.

use ik::canonical::canonical_json;
use ik::declarative::{
    check_machine, parse_and_compile, parse_definition, verification_plan, Bounds, CaseCorpus,
    CheckResult, CompiledMachine, Definition,
};
use ik::effect::EffectStatus;
use ik::machine::{Decision, Machine};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/feasibility_truth")
}

fn load_truth() -> Value {
    let text = fs::read_to_string(root().join("truth.json")).expect("truth.json");
    serde_json::from_str(&text).expect("truth json")
}

fn cases(truth: &Value) -> &[Value] {
    truth["cases"].as_array().expect("cases")
}

fn fixture(rel: &str) -> PathBuf {
    root().join(rel)
}

fn load_def(case: &Value) -> Definition {
    let path = fixture(case["machine"].as_str().expect("machine"));
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    parse_definition(&text).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()))
}

fn parse_all_or_panic(truth: &Value) {
    let mut errors = Vec::new();
    for case in cases(truth) {
        let path = fixture(case["machine"].as_str().expect("machine"));
        let text =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        if let Err(e) = parse_definition(&text) {
            errors.push(format!(
                "{}: {e:?}",
                path.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    assert!(
        errors.is_empty(),
        "fixture parse errors:\n{}",
        errors.join("\n")
    );
}

fn load_machine(case: &Value) -> CompiledMachine {
    let path = fixture(case["machine"].as_str().expect("machine"));
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    parse_and_compile(&text).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()))
}

fn load_corpus(case: &Value) -> CaseCorpus {
    let path = fixture(case["corpus"].as_str().expect("corpus"));
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn bounds_of(case: &Value) -> Bounds {
    let row = &case["brute_force"];
    Bounds {
        max_nodes: row["max_nodes"].as_u64().unwrap_or(128) as usize,
        max_depth: row["max_depth"].as_u64().unwrap_or(8) as usize,
    }
}

fn command_json(name: &str, args: &BTreeMap<String, Value>) -> Value {
    if args.is_empty() {
        json!({ "name": name })
    } else {
        json!({ "name": name, "args": args })
    }
}

fn source_of(state: &Value) -> Option<&str> {
    state.get("state").and_then(Value::as_str)
}

fn replay(machine: &CompiledMachine, trace: &[Value]) -> bool {
    let mut state = machine.initial_state();
    if trace.is_empty() {
        return false;
    }
    for step in trace {
        let name = step["command"].as_str().expect("command");
        let args = step.get("args").cloned().unwrap_or_else(|| json!({}));
        let command = match args.as_object() {
            Some(map) if map.is_empty() => json!({ "name": name }),
            Some(_) => json!({ "name": name, "args": args }),
            None => json!({ "name": name }),
        };
        if !commit_ok(machine, &mut state, &command) {
            return false;
        }
    }
    true
}

fn commit_ok(machine: &CompiledMachine, state: &mut Value, command: &Value) -> bool {
    let decision = match machine.decide(state, None, &EffectStatus::None, command) {
        Ok(decision) => decision,
        Err(_) => return false,
    };
    let Decision::Emit { event, .. } = decision else {
        return false;
    };
    match machine.apply(state, &event) {
        Ok(applied) => {
            *state = applied.state;
            true
        }
        Err(_) => false,
    }
}

#[derive(Clone, Debug, Default)]
struct Hits {
    states: BTreeSet<String>,
    terminals: BTreeSet<String>,
    sources: BTreeSet<(String, String)>,
    outcomes: BTreeSet<(String, usize)>,
    complete: bool,
}

fn candidates(def: &Definition, corpus: &CaseCorpus, reverse: bool) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for (name, cmd) in &def.commands {
        let needs = cmd.args.as_ref().map(|a| !a.is_empty()).unwrap_or(false);
        if needs {
            let Some(cases) = corpus.commands.get(name) else {
                continue;
            };
            for case in cases {
                out.push((name.clone(), command_json(name, &case.args)));
            }
        } else {
            out.push((name.clone(), command_json(name, &BTreeMap::new())));
        }
    }
    out.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| canonical_json(&a.1).cmp(&canonical_json(&b.1)))
    });
    if reverse {
        out.reverse();
    }
    out
}

fn try_commit(
    machine: &CompiledMachine,
    state: &Value,
    command: &Value,
) -> Option<(Value, bool, String, usize)> {
    let decision = machine
        .decide(state, None, &EffectStatus::None, command)
        .ok()?;
    let Decision::Emit { event, .. } = decision else {
        return None;
    };
    let applied = machine.apply(state, &event).ok()?;
    let source = source_of(state)?.to_string();
    let index = event
        .payload
        .get("outcome")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    Some((applied.state, applied.terminal, source, index))
}

fn brute(machine: &CompiledMachine, corpus: &CaseCorpus, bounds: Bounds, reverse: bool) -> Hits {
    let def = machine.definition();
    let cands = candidates(def, corpus, reverse);
    let mut hits = Hits::default();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    let initial = machine.initial_state();
    if let Some(label) = source_of(&initial) {
        hits.states.insert(label.to_string());
    }
    queue.push_back((initial, 0usize));
    let mut truncated = false;
    while let Some((state, depth)) = queue.pop_front() {
        let key = canonical_json(&state);
        if !seen.insert(key) {
            continue;
        }
        if seen.len() > bounds.max_nodes {
            truncated = true;
            break;
        }
        if depth >= bounds.max_depth {
            truncated = true;
            continue;
        }
        for (name, command) in &cands {
            let Some((next, terminal, source, index)) = try_commit(machine, &state, command) else {
                continue;
            };
            hits.sources.insert((name.clone(), source));
            hits.outcomes.insert((name.clone(), index));
            if let Some(label) = source_of(&next) {
                hits.states.insert(label.to_string());
                if terminal {
                    hits.terminals.insert(label.to_string());
                }
            }
            queue.push_back((next, depth + 1));
        }
    }
    hits.complete = !truncated;
    hits
}

fn strip_exec(oid: &str, prefix: &str, suffix: &str) -> Option<String> {
    oid.strip_prefix(prefix)?
        .strip_suffix(suffix)
        .map(str::to_string)
}

fn generated_hit(hits: &Hits, oid: &str) -> bool {
    if let Some(name) = strip_exec(oid, "state/", "/executable") {
        return hits.states.contains(&name);
    }
    if let Some(name) = strip_exec(oid, "terminal/", "/reachable") {
        return hits.terminals.contains(&name);
    }
    if let Some(body) = strip_exec(oid, "command/", "/executable") {
        if let Some((cmd, src)) = body.split_once("/from/") {
            return hits.sources.contains(&(cmd.to_string(), src.to_string()));
        }
    }
    if let Some(body) = strip_exec(oid, "outcome/", "/executable") {
        if let Some((cmd, idx)) = body.rsplit_once('/') {
            if let Ok(index) = idx.parse::<usize>() {
                return hits.outcomes.contains(&(cmd.to_string(), index));
            }
        }
    }
    false
}

fn engine_impossible(result: &CheckResult, oid: &str) -> Option<bool> {
    result
        .report
        .impossibilities
        .iter()
        .find(|row| row.obligation_id == oid)
        .map(|row| row.discharged)
}

fn check_of(case: &Value) -> CheckResult {
    let def = load_def(case);
    let corpus = load_corpus(case);
    check_machine(&def, None, &corpus, &[], bounds_of(case)).expect("check")
}

#[test]
fn suite_has_at_least_sixty_cases() {
    let truth = load_truth();
    let n = cases(&truth).len();
    assert!(n >= 60, "need at least 60 cases, got {n}");
}

#[test]
fn construction_traces_replay_on_the_kernel() {
    let truth = load_truth();
    parse_all_or_panic(&truth);
    let mut failures = Vec::new();
    for case in cases(&truth) {
        let id = case["id"].as_str().unwrap();
        let machine = load_machine(case);
        for obligation in case["obligations"].as_array().unwrap() {
            if obligation["truth"] != "REACHABLE" {
                continue;
            }
            let oid = obligation["id"].as_str().unwrap();
            let trace = obligation["trace"].as_array().cloned().unwrap_or_default();
            if !replay(&machine, &trace) {
                failures.push(format!("{id}:{oid}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "REACHABLE traces that did not replay: {failures:?}"
    );
}

#[test]
fn brute_force_is_stable_across_candidate_order() {
    let truth = load_truth();
    let mut mismatches = Vec::new();
    for case in cases(&truth) {
        let id = case["id"].as_str().unwrap();
        let machine = load_machine(case);
        let corpus = load_corpus(case);
        let bounds = bounds_of(case);
        let a = brute(&machine, &corpus, bounds, false);
        let b = brute(&machine, &corpus, bounds, true);
        if a.states != b.states || a.sources != b.sources || a.outcomes != b.outcomes {
            mismatches.push(id.to_string());
        }
        if a.complete != b.complete {
            mismatches.push(format!("{id}:complete"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "brute-force order disagreement: {mismatches:?}"
    );
}

#[test]
fn brute_force_agrees_with_construction() {
    let truth = load_truth();
    let mut disagreements = Vec::new();
    for case in cases(&truth) {
        disagreements.extend(case_disagreements(case));
    }
    assert!(
        disagreements.is_empty(),
        "construction vs brute-force disagreements: {disagreements:?}"
    );
}

fn case_disagreements(case: &Value) -> Vec<String> {
    let id = case["id"].as_str().unwrap();
    let machine = load_machine(case);
    let corpus = load_corpus(case);
    let hits = brute(&machine, &corpus, bounds_of(case), false);
    let mut out = Vec::new();
    for obligation in case["obligations"].as_array().unwrap() {
        let oid = obligation["id"].as_str().unwrap();
        if oid.starts_with("authored/") {
            continue;
        }
        let truth = obligation["truth"].as_str().unwrap();
        let hit = generated_hit(&hits, oid);
        match (truth, hit) {
            ("REACHABLE", false) => {
                let trace = obligation["trace"].as_array().cloned().unwrap_or_default();
                if !replay(&machine, &trace) {
                    out.push(format!("{id}:{oid}: REACHABLE missing brute and replay"));
                }
            }
            ("UNREACHABLE", true) | ("UNKNOWN", true) => {
                out.push(format!("{id}:{oid}: {truth} but brute found a witness"));
            }
            _ => {}
        }
    }
    out
}

#[test]
fn in_house_engine_is_sound_and_does_not_discharge_witnesses() {
    let truth = load_truth();
    let mut unsound = Vec::new();
    let mut discharged_witness = Vec::new();
    let mut unreachable = 0usize;
    let mut proved = 0usize;
    for case in cases(&truth) {
        let id = case["id"].as_str().unwrap();
        let result = check_of(case);
        for obligation in case["obligations"].as_array().unwrap() {
            let oid = obligation["id"].as_str().unwrap();
            let truth_row = obligation["truth"].as_str().unwrap();
            let record = engine_impossible(&result, oid);
            if truth_row == "UNREACHABLE" {
                unreachable += 1;
                if record.is_some() {
                    proved += 1;
                }
            }
            if matches!(truth_row, "REACHABLE" | "UNKNOWN") && record.is_some() {
                unsound.push(format!("{id}:{oid}:{truth_row}"));
            }
            if truth_row == "REACHABLE" && record == Some(true) {
                discharged_witness.push(format!("{id}:{oid}"));
            }
        }
    }
    let precision = if unreachable == 0 {
        0.0
    } else {
        proved as f64 / unreachable as f64
    };
    eprintln!(
        "in-house feasibility: unsound={} discharged_witness={} precision={:.4} proved={proved} unreachable={unreachable}",
        unsound.len(),
        discharged_witness.len(),
        precision
    );
    assert!(
        unsound.is_empty(),
        "UNSOUND impossible on REACHABLE: {unsound:?}"
    );
    assert!(
        discharged_witness.is_empty(),
        "discharged a witness-replayable obligation: {discharged_witness:?}"
    );
}

#[test]
fn ik_check_json_records_the_same_impossibilities() {
    let truth = load_truth();
    let case = cases(&truth)
        .iter()
        .find(|row| row["id"] == "dead_always_false_guard")
        .expect("fixture");
    let library = check_of(case);
    let out = Command::new(env!("CARGO_BIN_EXE_ik"))
        .arg("check")
        .arg(fixture(case["machine"].as_str().unwrap()))
        .arg("--corpus")
        .arg(fixture(case["corpus"].as_str().unwrap()))
        .arg("--json")
        .output()
        .expect("ik check");
    assert!(!out.stdout.is_empty(), "ik check --json wrote no stdout");
    let json: Value = serde_json::from_slice(&out.stdout).expect("check json");
    let cli_ids: BTreeSet<String> = json["report"]["impossibilities"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter_map(|row| row["obligation_id"].as_str().map(str::to_string))
        .collect();
    let lib_ids: BTreeSet<String> = library
        .report
        .impossibilities
        .iter()
        .map(|row| row.obligation_id.clone())
        .collect();
    assert_eq!(cli_ids, lib_ids, "library vs ik check --json");
}

#[test]
fn scorer_catches_wrong_backend_and_accepts_unknown() {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/score-known-feasibility.py");
    let truth = root().join("truth.json");
    let out = Command::new("python3")
        .arg(&script)
        .arg("--truth")
        .arg(&truth)
        .arg("--self-test")
        .arg("--json")
        .output()
        .expect("scorer self-test");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: Value = serde_json::from_slice(&out.stdout).expect("self-test json");
    assert_eq!(json["wrong_backend_caught"], true, "{json}");
    assert_eq!(json["unknown_backend_sound"], true, "{json}");
    assert_eq!(json["unknown_precision"], 0.0, "{json}");
}

#[test]
fn tagged_ids_exist_on_the_plan() {
    let truth = load_truth();
    let mut missing = Vec::new();
    for case in cases(&truth) {
        let id = case["id"].as_str().unwrap();
        let def = load_def(case);
        let plan = verification_plan(&def);
        let mut known: BTreeSet<String> = plan
            .obligations
            .iter()
            .map(|row| row.obligation_id.clone())
            .collect();
        for row in &plan.impossibilities {
            known.insert(row.obligation_id.clone());
        }
        for row in &plan.authored_goals {
            known.insert(row.obligation_id.clone());
        }
        for obligation in case["obligations"].as_array().unwrap() {
            let oid = obligation["id"].as_str().unwrap();
            if !known.contains(oid) {
                missing.push(format!("{id}:{oid}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "obligation ids not on the plan: {missing:?}"
    );
}

#[test]
fn unreachable_obligations_match_kernel_explorer() {
    let mut truth = load_truth();
    let stamp = std::env::var("STAMP_TRUTH").is_ok();
    let mut bugs = Vec::new();
    let mut missing_fields = Vec::new();
    for case in truth["cases"].as_array_mut().expect("cases") {
        let id = case["id"].as_str().expect("id").to_string();
        let def = load_def(case);
        let machine = load_machine(case);
        for obligation in case["obligations"].as_array_mut().expect("obligations") {
            if obligation["truth"].as_str() != Some("UNREACHABLE") {
                continue;
            }
            let oid = obligation["id"].as_str().expect("oid").to_string();
            if oid.starts_with("authored/") {
                if stamp {
                    obligation["mechanical"] = json!("authored_goal");
                    obligation["domain"] =
                        json!("authored coordinate; impossibility never discharges");
                    obligation["complete"] = json!(false);
                }
                continue;
            }
            let Some(target) = ik::smt::parse_target(&oid) else {
                continue;
            };
            let ex = match ik::smt::exhaustive_search(&def, &machine, &target) {
                Ok(ex) => ex,
                Err(err) => {
                    bugs.push(format!("{id}:{oid}: explorer error {err}"));
                    continue;
                }
            };
            if let Some(steps) = &ex.witness {
                bugs.push(format!(
                    "{id}:{oid}: explorer reached an UNREACHABLE obligation via {}",
                    ik::smt::format_trace(steps)
                ));
                continue;
            }
            let method = if ex.complete {
                "kernel_exhaustive"
            } else {
                "unbounded_sample"
            };
            if stamp {
                obligation["mechanical"] = json!(method);
                obligation["domain"] = json!(ex.scope.sentence());
                obligation["complete"] = json!(ex.complete);
                obligation["nodes"] = json!(ex.nodes);
                continue;
            }
            let got_method = obligation
                .get("mechanical")
                .and_then(|row| row.as_str())
                .unwrap_or("");
            let domain = obligation
                .get("domain")
                .and_then(|row| row.as_str())
                .unwrap_or("");
            if got_method.is_empty() || domain.is_empty() {
                missing_fields.push(format!("{id}:{oid}"));
                continue;
            }
            if got_method != method {
                bugs.push(format!(
                    "{id}:{oid}: mechanical={got_method} explorer={method} ({:?})",
                    ex.incomplete_reason
                ));
            }
        }
    }
    if stamp {
        let path = root().join("truth.json");
        let text = serde_json::to_string_pretty(&truth).expect("stamp json");
        fs::write(&path, text + "\n").expect("stamp truth.json");
        return;
    }
    assert!(bugs.is_empty(), "mechanical/truth disagreements: {bugs:?}");
    assert!(
        missing_fields.is_empty(),
        "UNREACHABLE obligations missing mechanical/domain: {missing_fields:?}"
    );
}

fn adversarial_case(id: &str) -> bool {
    [
        "overflow",
        "div_",
        "negative",
        "short_circuit",
        "mul_overflow",
        "nested_false",
        "guard_error",
        "add_overflow",
    ]
    .iter()
    .any(|key| id.contains(key))
}

#[test]
fn by_construction_adversarial_cases_are_mechanically_rechecked() {
    let truth = load_truth();
    let mut missing = Vec::new();
    let mut seen = 0usize;
    for case in cases(&truth) {
        let id = case["id"].as_str().expect("id");
        if !adversarial_case(id) {
            continue;
        }
        seen += 1;
        let def = load_def(case);
        let machine = load_machine(case);
        for obligation in case["obligations"].as_array().expect("obligations") {
            let oid = obligation["id"].as_str().expect("oid");
            let truth_row = obligation["truth"].as_str().expect("truth");
            if truth_row == "REACHABLE" {
                let trace = obligation["trace"].as_array().expect("trace");
                if !replay(&machine, trace) {
                    missing.push(format!("{id}:{oid}: reachable trace did not replay"));
                }
                continue;
            }
            if truth_row != "UNREACHABLE" || oid.starts_with("authored/") {
                continue;
            }
            let mechanical = obligation
                .get("mechanical")
                .and_then(Value::as_str)
                .unwrap_or("");
            if mechanical.is_empty() {
                missing.push(format!("{id}:{oid}: missing mechanical"));
                continue;
            }
            let Some(target) = ik::smt::parse_target(oid) else {
                continue;
            };
            let ex = ik::smt::exhaustive_search(&def, &machine, &target)
                .unwrap_or_else(|err| panic!("{id}:{oid}: explorer {err}"));
            if ex.witness.is_some() {
                missing.push(format!("{id}:{oid}: explorer found a witness"));
            }
        }
    }
    assert!(seen >= 8, "adversarial cases seen={seen}");
    assert!(missing.is_empty(), "{missing:?}");
}
