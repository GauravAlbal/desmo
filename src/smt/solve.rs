//! Subprocess Z3 / cvc5 runner and obligation analysis.

use super::encode::GuardValue;
use super::query::{
    dump_query, encode_bmc, encode_guard_eval, encode_horn, encode_selectability_any, Pinned,
    Target,
};
use super::replay::{bmc_trace, lowered_of, replay_command_from, replay_takes, replay_visits};
use super::Unsupported;
use crate::declarative::compile::CompiledMachine;
use crate::declarative::def::Definition;
use crate::declarative::expr::{self, Expr};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static SMT_FILE_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct SolverBin {
    pub name: String,
    pub path: PathBuf,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SolverReply {
    Sat,
    Unsat,
    Unknown(String),
}

#[derive(Clone, Debug, Default)]
pub struct ModelMap {
    pub ints: BTreeMap<String, i64>,
    pub bools: BTreeMap<String, bool>,
}

#[derive(Clone, Debug)]
pub struct SolveResult {
    pub reply: SolverReply,
    pub model: ModelMap,
    pub raw: String,
    pub ms: u128,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    SolverAsserted,
    CheckedInvariant,
    CheckedExhaustive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofStatus {
    Witness,
    ProvenImpossible,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct ObligationReport {
    pub id: String,
    pub status: ProofStatus,
    pub trust: Trust,
    pub detail: String,
    pub replay: Option<String>,
    pub z3_ms: Option<u128>,
    pub cvc5_ms: Option<u128>,
    pub z3: Option<String>,
    pub cvc5: Option<String>,
    pub scope: Option<String>,
    pub reachable_digest: Option<String>,
    pub invariant: Option<String>,
}

pub fn discover_solvers() -> (Option<SolverBin>, Option<SolverBin>) {
    (find_named("z3"), find_named("cvc5"))
}

fn find_named(name: &str) -> Option<SolverBin> {
    let env_key = if name == "z3" { "SMT_Z3" } else { "SMT_CVC5" };
    if let Ok(path) = std::env::var(env_key) {
        if let Some(bin) = probe_bin(name, Path::new(&path)) {
            return Some(bin);
        }
    }
    for candidate in candidate_paths(name) {
        if let Some(bin) = probe_bin(name, &candidate) {
            return Some(bin);
        }
    }
    which(name).and_then(|path| probe_bin(name, &path))
}

fn candidate_paths(name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in ["/tmp/smtvenv3", "/tmp/smtvenv2", "/tmp/smtvenv"] {
        out.push(PathBuf::from(format!("{root}/bin/{name}")));
        push_site_binaries(root, name, &mut out);
    }
    out.push(PathBuf::from(format!("/opt/homebrew/bin/{name}")));
    out.push(PathBuf::from(format!("/usr/local/bin/{name}")));
    out
}

fn push_site_binaries(root: &str, name: &str, out: &mut Vec<PathBuf>) {
    let Ok(pythons) = std::fs::read_dir(PathBuf::from(root).join("lib")) else {
        return;
    };
    for py in pythons.flatten() {
        let sp = py.path().join("site-packages");
        out.push(sp.join(name).join("bin").join(name));
        out.push(sp.join("bin").join(name));
    }
}

fn which(name: &str) -> Option<PathBuf> {
    let output = Command::new("which").arg(name).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let path = text.trim();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

pub(crate) fn probe_named(name: &str, path: &Path) -> Option<SolverBin> {
    probe_bin(name, path)
}

fn probe_bin(name: &str, path: &Path) -> Option<SolverBin> {
    if !path.is_file() {
        return None;
    }
    let output = Command::new(path).arg("--version").output().ok()?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if err.is_empty() {
            return None;
        }
        return Some(SolverBin {
            name: name.to_string(),
            path: path.to_path_buf(),
            version: err.lines().next().unwrap_or("unknown").to_string(),
        });
    }
    Some(SolverBin {
        name: name.to_string(),
        path: path.to_path_buf(),
        version: version.lines().next().unwrap_or("unknown").to_string(),
    })
}

pub fn run_smt2(bin: &SolverBin, smt2: &str, timeout: Duration) -> Result<SolveResult, String> {
    let start = Instant::now();
    let path = std::env::temp_dir().join(format!(
        "ik-smt-{}-{}-{}.smt2",
        bin.name,
        std::process::id(),
        SMT_FILE_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, smt2).map_err(|err| format!("write {}: {err}", path.display()))?;
    let mut cmd = Command::new(&bin.path);
    if bin.name == "z3" {
        cmd.arg(format!("-T:{}", timeout.as_secs()));
        if smt2.contains("(set-logic HORN)") {
            cmd.arg("fp.engine=spacer");
        }
        cmd.arg(&path);
        cmd.stdin(Stdio::null());
    } else {
        cmd.arg("--lang=smt2")
            .arg(format!("--tlimit={}", timeout.as_millis()));
        let file =
            std::fs::File::open(&path).map_err(|err| format!("open {}: {err}", path.display()))?;
        cmd.stdin(Stdio::from(file));
    }
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| {
            let _ = std::fs::remove_file(&path);
            format!("run {}: {err}", bin.name)
        })?;
    let _ = std::fs::remove_file(&path);
    let raw = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(SolveResult {
        reply: classify_reply(&raw),
        model: parse_model(&raw),
        raw,
        ms: start.elapsed().as_millis(),
    })
}

fn classify_reply(raw: &str) -> SolverReply {
    for line in raw.lines() {
        let t = line.trim();
        if t == "sat" {
            return SolverReply::Sat;
        }
        if t == "unsat" {
            return SolverReply::Unsat;
        }
        if t == "unknown" {
            return SolverReply::Unknown("solver returned unknown".into());
        }
    }
    SolverReply::Unknown(
        raw.lines()
            .next()
            .unwrap_or("empty solver output")
            .to_string(),
    )
}

fn parse_model(raw: &str) -> ModelMap {
    let mut model = ModelMap::default();
    for (key, value) in scan_pairs(raw) {
        if value == "true" || value == "false" {
            model.bools.insert(key, value == "true");
        } else if let Some(n) = parse_smt_int(&value) {
            model.ints.insert(key, n);
        }
    }
    model
}

fn scan_pairs(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let tokens = tokenize(raw);
    let mut i = 0;
    while i + 2 < tokens.len() {
        if tokens[i] == "(" && is_symbol(&tokens[i + 1]) {
            let key = tokens[i + 1].clone();
            if tokens[i + 2] == "(" && i + 4 < tokens.len() && tokens[i + 3] == "-" {
                if let Some(n) = tokens.get(i + 4) {
                    out.push((key, format!("(- {n})")));
                }
                i += 1;
                continue;
            }
            if tokens[i + 2] != "(" && tokens[i + 2] != ")" {
                out.push((key, tokens[i + 2].clone()));
            }
        }
        i += 1;
    }
    out
}

fn tokenize(raw: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '(' | ')' => {
                flush(&mut cur, &mut tokens);
                tokens.push(ch.to_string());
            }
            '|' => {
                flush(&mut cur, &mut tokens);
                let mut name = String::from("|");
                for next in chars.by_ref() {
                    name.push(next);
                    if next == '|' {
                        break;
                    }
                }
                tokens.push(name);
            }
            c if c.is_whitespace() => flush(&mut cur, &mut tokens),
            c => cur.push(c),
        }
    }
    flush(&mut cur, &mut tokens);
    tokens
}

fn flush(cur: &mut String, tokens: &mut Vec<String>) {
    if !cur.is_empty() {
        tokens.push(std::mem::take(cur));
    }
}

fn is_symbol(tok: &str) -> bool {
    tok.starts_with('|') || tok.starts_with("g:") || tok.contains(':')
}

fn parse_smt_int(text: &str) -> Option<i64> {
    if let Some(inner) = text
        .strip_prefix("(-")
        .map(|s| s.trim().trim_end_matches(')').trim())
    {
        if inner == "9223372036854775808" {
            return Some(i64::MIN);
        }
        return inner.parse::<i64>().ok().map(|n| -n);
    }
    text.parse().ok()
}

pub fn eval_guard_with_solver(
    expr: &Expr,
    pinned: &Pinned,
    solver: &SolverBin,
) -> Result<GuardValue, String> {
    let query = encode_guard_eval(expr, pinned).map_err(|err| err.reason)?;
    let solved = run_smt2(solver, &query.text, Duration::from_secs(10))?;
    if solved.reply != SolverReply::Sat {
        return Err(format!("guard eval not sat: {:?}", solved.reply));
    }
    let err = solved
        .model
        .bools
        .get("|g:err|")
        .copied()
        .ok_or_else(|| "model missing |g:err|".to_string())?;
    let val = solved
        .model
        .bools
        .get("|g:val|")
        .copied()
        .ok_or_else(|| "model missing |g:val|".to_string())?;
    if err {
        Ok(GuardValue::Error)
    } else if val {
        Ok(GuardValue::True)
    } else {
        Ok(GuardValue::False)
    }
}

pub fn analyze_target(
    def: &Definition,
    machine: &CompiledMachine,
    id: &str,
    target: Target,
    z3: Option<&SolverBin>,
    cvc5: Option<&SolverBin>,
) -> ObligationReport {
    match try_analyze(def, machine, id, target, z3, cvc5) {
        Ok(report) => report,
        Err(err) => unknown(id, err),
    }
}

fn try_analyze(
    def: &Definition,
    machine: &CompiledMachine,
    id: &str,
    target: Target,
    z3: Option<&SolverBin>,
    cvc5: Option<&SolverBin>,
) -> Result<ObligationReport, String> {
    match super::exhaustive::search(def, machine, &target) {
        Ok(ex) => {
            if let Some(report) = exhaustive_report(def, machine, id, &target, &ex, z3, cvc5)? {
                return Ok(report);
            }
        }
        Err(err) => {
            if z3.is_none() {
                return Ok(unknown(id, err));
            }
        }
    }
    let Some(z3) = z3 else {
        return Ok(unknown(id, "z3 not installed"));
    };
    if let Target::Outcome { command, index } = &target {
        if let Some(report) = local_outcome(def, machine, id, command, *index, z3, cvc5)? {
            return Ok(report);
        }
    }
    bmc_then_horn(def, machine, id, target, z3, cvc5)
}

fn exhaustive_report(
    def: &Definition,
    machine: &CompiledMachine,
    id: &str,
    target: &Target,
    ex: &super::exhaustive::ExhaustiveResult,
    z3: Option<&SolverBin>,
    cvc5: Option<&SolverBin>,
) -> Result<Option<ObligationReport>, String> {
    if let Some(steps) = &ex.witness {
        super::replay::replay_trace(machine, steps)?;
        return Ok(Some(ObligationReport {
            id: id.to_string(),
            status: ProofStatus::Witness,
            trust: Trust::CheckedExhaustive,
            detail: format!(
                "kernel BFS replayed {}; {}",
                super::exhaustive::format_trace(steps),
                ex.scope.sentence()
            ),
            replay: Some(super::exhaustive::format_trace(steps)),
            z3_ms: None,
            cvc5_ms: None,
            z3: z3.map(|s| s.version.clone()),
            cvc5: cvc5.map(|s| s.version.clone()),
            scope: Some(ex.scope.sentence()),
            reachable_digest: Some(ex.reachable_digest.clone()),
            invariant: None,
        }));
    }
    match certify_unreachable(def, machine, id, target, ex, z3, cvc5)? {
        Some(report) => Ok(Some(report)),
        None if ex.complete => Ok(Some(checked_exhaustive(id, ex, z3, cvc5))),
        None => Ok(None),
    }
}

fn certify_unreachable(
    def: &Definition,
    machine: &CompiledMachine,
    id: &str,
    target: &Target,
    ex: &super::exhaustive::ExhaustiveResult,
    z3: Option<&SolverBin>,
    cvc5: Option<&SolverBin>,
) -> Result<Option<ObligationReport>, String> {
    if z3.is_none() {
        return Ok(None);
    }
    let mut last = String::new();
    for inv in invariant_candidates(ex) {
        if inv.is_empty() || inv == last {
            continue;
        }
        last = inv.to_string();
        if let Some(report) = invariant_status(def, machine, id, target, ex, inv, z3, cvc5)? {
            return Ok(Some(report));
        }
    }
    Ok(None)
}

fn invariant_candidates(ex: &super::exhaustive::ExhaustiveResult) -> Vec<&str> {
    if ex.complete || ex.projected_smt == ex.invariant_smt {
        vec![ex.invariant_smt.as_str()]
    } else {
        vec![ex.projected_smt.as_str(), ex.invariant_smt.as_str()]
    }
}

fn invariant_status(
    def: &Definition,
    machine: &CompiledMachine,
    id: &str,
    target: &Target,
    ex: &super::exhaustive::ExhaustiveResult,
    invariant: &str,
    z3: Option<&SolverBin>,
    cvc5: Option<&SolverBin>,
) -> Result<Option<ObligationReport>, String> {
    let check = match super::certificate::check_invariant_replay(
        def,
        invariant,
        target,
        z3,
        cvc5,
        Some(machine),
    ) {
        Ok(check) => check,
        Err(_) => return Ok(None),
    };
        if check.dual_checked() {
            return Ok(Some(proven_invariant(id, ex, z3, cvc5, invariant, &check)));
        }
    if check.disagreed() {
        return Ok(Some(solver_asserted(
            id,
            ex,
            z3,
            cvc5,
            format!("solvers disagree on inductive I; {}", check.summary()),
        )));
    }
    if let Some(note) = confirmed_hit(&check) {
        if note.contains("kernel selected") || note.contains("kernel ran") {
            return Ok(Some(ObligationReport {
                id: id.to_string(),
                status: ProofStatus::Witness,
                trust: Trust::CheckedExhaustive,
                detail: note.clone(),
                replay: Some(note),
                z3_ms: check.excludes.z3_ms,
                cvc5_ms: check.excludes.cvc5_ms,
                z3: z3.map(|s| s.version.clone()),
                cvc5: cvc5.map(|s| s.version.clone()),
                scope: Some(ex.scope.sentence()),
                reachable_digest: Some(ex.reachable_digest.clone()),
                invariant: Some(invariant.to_string()),
            }));
        }
    }
    Ok(None)
}

fn confirmed_hit(check: &super::certificate::InvariantCheck) -> Option<String> {
    check
        .replay_notes()
        .into_iter()
        .find(|note| note.contains("kernel") && !note.contains("did not replay"))
}

fn proven_invariant(
    id: &str,
    ex: &super::exhaustive::ExhaustiveResult,
    z3: Option<&SolverBin>,
    cvc5: Option<&SolverBin>,
    invariant: &str,
    check: &super::certificate::InvariantCheck,
) -> ObligationReport {
    ObligationReport {
        id: id.to_string(),
        status: ProofStatus::ProvenImpossible,
        trust: Trust::CheckedInvariant,
        detail: format!(
            "inductive I dual-checked over symbolic arguments; nodes={}; digest={}; {}; {}",
            ex.nodes,
            ex.reachable_digest,
            ex.scope.sentence(),
            check.summary()
        ),
        replay: None,
        z3_ms: check.initial.z3_ms,
        cvc5_ms: check.initial.cvc5_ms,
        z3: z3.map(|s| s.version.clone()),
        cvc5: cvc5.map(|s| s.version.clone()),
        scope: Some(ex.scope.sentence()),
        reachable_digest: Some(ex.reachable_digest.clone()),
        invariant: Some(invariant.to_string()),
    }
}

fn solver_asserted(
    id: &str,
    ex: &super::exhaustive::ExhaustiveResult,
    z3: Option<&SolverBin>,
    cvc5: Option<&SolverBin>,
    detail: String,
) -> ObligationReport {
    ObligationReport {
        id: id.to_string(),
        status: ProofStatus::ProvenImpossible,
        trust: Trust::SolverAsserted,
        detail,
        replay: None,
        z3_ms: None,
        cvc5_ms: None,
        z3: z3.map(|s| s.version.clone()),
        cvc5: cvc5.map(|s| s.version.clone()),
        scope: Some(ex.scope.sentence()),
        reachable_digest: Some(ex.reachable_digest.clone()),
        invariant: Some(ex.invariant_smt.clone()),
    }
}

fn checked_exhaustive(
    id: &str,
    ex: &super::exhaustive::ExhaustiveResult,
    z3: Option<&SolverBin>,
    cvc5: Option<&SolverBin>,
) -> ObligationReport {
    ObligationReport {
        id: id.to_string(),
        status: ProofStatus::ProvenImpossible,
        trust: Trust::CheckedExhaustive,
        detail: format!(
            "complete finite-model BFS never reached the target; nodes={}; digest={}; {}",
            ex.nodes, ex.reachable_digest, ex.scope.sentence()
        ),
        replay: None,
        z3_ms: None,
        cvc5_ms: None,
        z3: z3.map(|s| s.version.clone()),
        cvc5: cvc5.map(|s| s.version.clone()),
        scope: Some(ex.scope.sentence()),
        reachable_digest: Some(ex.reachable_digest.clone()),
        invariant: Some(ex.invariant_smt.clone()),
    }
}

fn local_outcome(
    def: &Definition,
    _machine: &CompiledMachine,
    id: &str,
    command: &str,
    index: usize,
    z3: &SolverBin,
    cvc5: Option<&SolverBin>,
) -> Result<Option<ObligationReport>, String> {
    let query = encode_selectability_any(def, command, index).map_err(unsup)?;
    dump_query(&format!("{id}-select"), &query);
    let z = run_smt2(z3, &query.text, Duration::from_secs(10))?;
    match z.reply {
        SolverReply::Sat => Ok(None),
        SolverReply::Unknown(reason) => Ok(Some(unknown(id, reason))),
        SolverReply::Unsat => Ok(Some(confirm_unsat(id, &query.text, z, z3, cvc5)?)),
    }
}

fn confirm_unsat(
    id: &str,
    smt2: &str,
    z: SolveResult,
    z3: &SolverBin,
    cvc5: Option<&SolverBin>,
) -> Result<ObligationReport, String> {
    let trust = Trust::SolverAsserted;
    let mut detail = "local selectability unsat on every source".to_string();
    let mut cvc5_ms = None;
    let mut cvc5_ver = None;
    if let Some(other) = cvc5 {
        let c = run_smt2(other, smt2, Duration::from_secs(10))?;
        cvc5_ms = Some(c.ms);
        cvc5_ver = Some(other.version.clone());
        if c.reply == SolverReply::Sat {
            return Ok(disagreement(id, &z, &c, z3, other));
        }
        if c.reply == SolverReply::Unsat {
            detail = "local selectability unsat on every source; independently confirmed".into();
        } else {
            detail = format!("{detail}; cvc5 did not confirm ({:?})", c.reply);
        }
    }
    Ok(ObligationReport {
        id: id.to_string(),
        status: ProofStatus::ProvenImpossible,
        trust,
        detail,
        replay: None,
        z3_ms: Some(z.ms),
        cvc5_ms,
        z3: Some(z3.version.clone()),
        cvc5: cvc5_ver,
            scope: None,
            reachable_digest: None,
            invariant: None,
    })
}

fn bmc_then_horn(
    def: &Definition,
    machine: &CompiledMachine,
    id: &str,
    target: Target,
    z3: &SolverBin,
    cvc5: Option<&SolverBin>,
) -> Result<ObligationReport, String> {
    if let Some(report) = initial_state_witness(def, id, &target, z3) {
        return Ok(report);
    }
    let mut z3_ms = 0;
    let skip_bmc = lowered_of(def)
        .map(|row| row.transitions.len() > 40)
        .unwrap_or(true);
    if !skip_bmc {
        for depth in [1usize, 2, 4, 6] {
            let query = encode_bmc(def, &target, depth).map_err(unsup)?;
            dump_query(&format!("{id}-bmc-{depth}"), &query);
            let z = run_smt2(z3, &query.text, Duration::from_secs(8))?;
            z3_ms += z.ms;
            if z.reply == SolverReply::Sat {
                if let Some(report) = replay_bmc(def, machine, id, &target, &query, &z, depth, z3) {
                    if report.status == ProofStatus::Witness {
                        return Ok(report);
                    }
                }
            }
        }
    }
    horn_impossible(def, id, target, z3, cvc5, z3_ms, 0)
}

fn initial_state_witness(
    def: &Definition,
    id: &str,
    target: &Target,
    z3: &SolverBin,
) -> Option<ObligationReport> {
    let name = match target {
        Target::State(name) | Target::Terminal(name) => name,
        Target::Outcome { .. } | Target::CommandFrom { .. } => return None,
    };
    let lowered = lowered_of(def).ok()?;
    if lowered.initial_state != *name {
        return None;
    }
    Some(ObligationReport {
        id: id.to_string(),
        status: ProofStatus::Witness,
        trust: Trust::CheckedExhaustive,
        detail: "target is the initial state; empty trace".into(),
        replay: Some(format!("kernel started in {name}")),
        z3_ms: Some(0),
        cvc5_ms: None,
        z3: Some(z3.version.clone()),
        cvc5: None,
            scope: None,
            reachable_digest: None,
            invariant: None,
    })
}

fn replay_bmc(
    def: &Definition,
    machine: &CompiledMachine,
    id: &str,
    target: &Target,
    query: &super::query::SmtQuery,
    solved: &SolveResult,
    depth: usize,
    z3: &SolverBin,
) -> Option<ObligationReport> {
    let lowered = lowered_of(def).ok()?;
    let steps = bmc_trace(&lowered, query, &solved.model, depth).ok()?;
    let replayed = match target {
        Target::State(name) | Target::Terminal(name) => replay_visits(machine, &steps, name),
        Target::Outcome { command, index } => replay_takes(machine, &steps, command, *index),
        Target::CommandFrom { command, source } => replay_command_from(machine, &steps, command, source),
    };
    match replayed {
        Ok(state) => Some(ObligationReport {
            id: id.to_string(),
            status: ProofStatus::Witness,
            trust: Trust::CheckedExhaustive,
            detail: format!("bmc depth {depth} replayed {} steps", steps.len()),
            replay: Some(format!(
                "kernel reached {}",
                state.get("state").and_then(Value::as_str).unwrap_or("?")
            )),
            z3_ms: Some(solved.ms),
            cvc5_ms: None,
            z3: Some(z3.version.clone()),
            cvc5: None,
            scope: None,
            reachable_digest: None,
            invariant: None,
        }),
        Err(err) => Some(ObligationReport {
            id: id.to_string(),
            status: ProofStatus::Unknown,
            trust: Trust::SolverAsserted,
            detail: format!("bmc model did not replay: {err}"),
            replay: Some(err),
            z3_ms: Some(solved.ms),
            cvc5_ms: None,
            z3: Some(z3.version.clone()),
            cvc5: None,
            scope: None,
            reachable_digest: None,
            invariant: None,
        }),
    }
}

fn horn_impossible(
    def: &Definition,
    id: &str,
    target: Target,
    z3: &SolverBin,
    cvc5: Option<&SolverBin>,
    mut z3_ms: u128,
    cvc5_ms: u128,
) -> Result<ObligationReport, String> {
    let query = encode_horn(def, &target).map_err(unsup)?;
    dump_query(&format!("{id}-horn"), &query);
    let z = run_smt2(z3, &query.text, Duration::from_secs(20))?;
    z3_ms += z.ms;
    let mut cvc5_ms = if cvc5_ms == 0 { None } else { Some(cvc5_ms) };
    let mut cvc5_ver = cvc5.map(|s| s.version.clone());
    let other_reply = if let Some(other) = cvc5 {
        let c = run_smt2(other, &query.text, Duration::from_secs(20))?;
        cvc5_ms = Some(cvc5_ms.unwrap_or(0) + c.ms);
        cvc5_ver = Some(other.version.clone());
        if replies_disagree(&z.reply, &c.reply) {
            return Ok(disagreement(id, &z, &c, z3, other));
        }
        Some(c.reply)
    } else {
        None
    };
    if z.reply == SolverReply::Sat {
        return Ok(ObligationReport {
            id: id.to_string(),
            status: ProofStatus::ProvenImpossible,
            trust: Trust::SolverAsserted,
            detail: "horn sat (inductive invariant exists); invariant not independently extracted"
                .into(),
            replay: None,
            z3_ms: Some(z3_ms),
            cvc5_ms,
            z3: Some(z3.version.clone()),
            cvc5: cvc5_ver,
            scope: None,
            reachable_digest: None,
            invariant: None,
        });
    }
    if z.reply == SolverReply::Unsat {
        let extra = match other_reply {
            Some(SolverReply::Unsat) => "; cvc5 also unsat",
            Some(other) => return Ok(unknown_timed(
                id,
                format!("horn unsat (counterexample exists) but no replayable trace was extracted; cvc5={other:?}"),
                z3_ms,
                cvc5_ms.unwrap_or(0),
                z3,
                cvc5,
            )),
            None => "",
        };
        return Ok(ObligationReport {
            id: id.to_string(),
            status: ProofStatus::Unknown,
            trust: Trust::SolverAsserted,
            detail: format!(
                "horn unsat (counterexample exists) but no replayable trace was extracted{extra}"
            ),
            replay: None,
            z3_ms: Some(z3_ms),
            cvc5_ms,
            z3: Some(z3.version.clone()),
            cvc5: cvc5_ver,
            scope: None,
            reachable_digest: None,
            invariant: None,
        });
    }
    let reason = match z.reply {
        SolverReply::Unknown(reason) => reason,
        other => format!("horn reply {other:?}"),
    };
    Ok(unknown_timed(
        id,
        reason,
        z3_ms,
        cvc5_ms.unwrap_or(0),
        z3,
        cvc5,
    ))
}

fn replies_disagree(left: &SolverReply, right: &SolverReply) -> bool {
    matches!(
        (left, right),
        (SolverReply::Sat, SolverReply::Unsat) | (SolverReply::Unsat, SolverReply::Sat)
    )
}

fn disagreement(
    id: &str,
    z: &SolveResult,
    c: &SolveResult,
    z3: &SolverBin,
    cvc5: &SolverBin,
) -> ObligationReport {
    ObligationReport {
        id: id.to_string(),
        status: ProofStatus::Unknown,
        trust: Trust::SolverAsserted,
        detail: format!("solvers disagree: z3={:?} cvc5={:?}", z.reply, c.reply),
        replay: None,
        z3_ms: Some(z.ms),
        cvc5_ms: Some(c.ms),
        z3: Some(z3.version.clone()),
        cvc5: Some(cvc5.version.clone()),
            scope: None,
            reachable_digest: None,
            invariant: None,
    }
}

fn unknown(id: &str, detail: impl Into<String>) -> ObligationReport {
    ObligationReport {
        id: id.to_string(),
        status: ProofStatus::Unknown,
        trust: Trust::SolverAsserted,
        detail: detail.into(),
        replay: None,
        z3_ms: None,
        cvc5_ms: None,
        z3: None,
        cvc5: None,
            scope: None,
            reachable_digest: None,
            invariant: None,
    }
}

fn unknown_timed(
    id: &str,
    detail: String,
    z3_ms: u128,
    cvc5_ms: u128,
    z3: &SolverBin,
    cvc5: Option<&SolverBin>,
) -> ObligationReport {
    ObligationReport {
        id: id.to_string(),
        status: ProofStatus::Unknown,
        trust: Trust::SolverAsserted,
        detail,
        replay: None,
        z3_ms: Some(z3_ms),
        cvc5_ms: Some(cvc5_ms),
        z3: Some(z3.version.clone()),
        cvc5: cvc5.map(|s| s.version.clone()),
            scope: None,
            reachable_digest: None,
            invariant: None,
    }
}

fn unsup(err: Unsupported) -> String {
    err.reason
}

pub fn kernel_eval(expr: &Expr, pinned: &Pinned) -> GuardValue {
    let mut facts = BTreeMap::new();
    for (name, value) in &pinned.facts {
        if let Some(v) = value {
            facts.insert(name.clone(), v.clone());
        }
    }
    let ctx = expr::EvalCtx {
        facts: &facts,
        args: &pinned.args,
        state: &pinned.state,
    };
    match expr::eval(expr, &ctx) {
        Ok(true) => GuardValue::True,
        Ok(false) => GuardValue::False,
        Err(_) => GuardValue::Error,
    }
}

pub fn parse_target(id: &str) -> Option<Target> {
    let parts: Vec<&str> = id.split('/').collect();
    match parts.as_slice() {
        ["outcome", command, index, "executable"] => Some(Target::Outcome {
            command: (*command).to_string(),
            index: index.parse().ok()?,
        }),
        ["command", command, "from", source, "executable"] => Some(Target::CommandFrom {
            command: (*command).to_string(),
            source: (*source).to_string(),
        }),
        ["state", name, "executable"] => Some(Target::State((*name).to_string())),
        ["terminal", name, "reachable"] => Some(Target::Terminal((*name).to_string())),
        _ => None,
    }
}
