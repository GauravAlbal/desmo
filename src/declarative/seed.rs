//! Guard-guided deterministic auto-seeder for bounded exploration and starter corpora.
//!
//! Synthesizes concrete argument assignments by inspecting declared types,
//! guard literals (`c - 1`, `c`, `c + 1`), `text_in` valid/invalid boundary
//! strings, initial fact values, and fact-to-argument equality propagation.
//!
//! Auto-seeded runs are explicitly advisory (`auto_seeded_bounded`) and never
//! replace an explicit `--corpus` for `--release` checks.

use super::def::{CommandDef, Definition};
use super::explore::{CaseCorpus, InputCase};
use super::expr::{self, Expr, Operand};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const AUTO_SEED_NOTE: &str =
    "guard-guided auto-seeded cases (advisory; edit and pin for --release)";

const MAX_VALUES_PER_ARG: usize = 6;
const MAX_CASES_PER_COMMAND: usize = 16;

/// Synthesize a deterministic, guard-guided [`CaseCorpus`] for `def`.
pub fn auto_seed_corpus(def: &Definition) -> CaseCorpus {
    let mut fact_pools = initial_fact_pools(def);
    harvest_global_invariant_pools(def, &mut fact_pools);

    // Pass 1: harvest per-command arg pools and propagate literal/arg values into fact pools.
    let mut cmd_arg_pools: BTreeMap<String, BTreeMap<String, Vec<Value>>> = BTreeMap::new();
    for (cmd_name, cmd) in &def.commands {
        let mut arg_pools = init_arg_pools(cmd);
        harvest_command_guards(cmd, &mut arg_pools, &mut fact_pools);
        propagate_then_to_facts(cmd, &arg_pools, &mut fact_pools);
        cmd_arg_pools.insert(cmd_name.clone(), arg_pools);
    }

    // Pass 2: propagate fact pools back into arguments compared against facts
    // (e.g., `$receipt_artifact_digest == artifact_digest`).
    for (cmd_name, cmd) in &def.commands {
        if let Some(arg_pools) = cmd_arg_pools.get_mut(cmd_name) {
            propagate_facts_to_args(cmd, &fact_pools, arg_pools);
        }
    }

    let mut corpus = CaseCorpus::new();
    for (cmd_name, cmd) in &def.commands {
        let declared = cmd.args.clone().unwrap_or_default();
        if declared.is_empty() {
            corpus.insert(cmd_name.clone(), "default", BTreeMap::new());
            continue;
        }
        let arg_pools = cmd_arg_pools
            .get(cmd_name)
            .cloned()
            .unwrap_or_default();
        let cases = synthesize_command_cases(&declared, &arg_pools);
        for case in cases {
            corpus.insert(cmd_name.clone(), case.name, case.args);
        }
    }
    corpus
}

/// Render the guard-guided auto-seeded corpus as an editable JSON document
/// suitable for `ik corpus init --smart`.
pub fn smart_starter_corpus(def: &Definition) -> Value {
    let corpus = auto_seed_corpus(def);
    let mut commands = Map::new();
    for (cmd, cases) in corpus.commands {
        let arr: Vec<Value> = cases
            .into_iter()
            .map(|c| json!({ "name": c.name, "args": c.args }))
            .collect();
        commands.insert(cmd, Value::Array(arr));
    }
    json!({
        "_note": AUTO_SEED_NOTE,
        "commands": commands,
    })
}

fn push_unique(pool: &mut Vec<Value>, value: Value) {
    if pool.len() < MAX_VALUES_PER_ARG && !pool.contains(&value) {
        pool.push(value);
    }
}

fn initial_fact_pools(def: &Definition) -> BTreeMap<String, Vec<Value>> {
    let mut pools: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for (name, fact) in &def.facts {
        let entry = pools.entry(name.clone()).or_default();
        if let Some(init) = &fact.initial {
            push_unique(entry, init.clone());
            if let Some(n) = init.as_i64() {
                push_unique(entry, json!(n.saturating_add(1)));
            }
        }
    }
    pools
}

fn harvest_global_invariant_pools(def: &Definition, fact_pools: &mut BTreeMap<String, Vec<Value>>) {
    let mut empty_args = BTreeMap::new();
    for inv in def.invariants.values() {
        if let Some(when) = &inv.when {
            if let Ok(ast) = expr::parse(when) {
                harvest_expr(&ast, &BTreeMap::new(), &mut empty_args, fact_pools);
            }
        }
        if let Ok(ast) = expr::parse(&inv.assert) {
            harvest_expr(&ast, &BTreeMap::new(), &mut empty_args, fact_pools);
        }
    }
}

fn init_arg_pools(cmd: &CommandDef) -> BTreeMap<String, Vec<Value>> {
    let mut pools = BTreeMap::new();
    let Some(declared) = &cmd.args else {
        return pools;
    };
    for (arg, ty) in declared {
        let mut vals = Vec::new();
        match ty.as_str() {
            "string" => {
                // Two distinct readable strings let equality AND revision/mismatch paths fire.
                vals.push(json!("example"));
                vals.push(json!("example_alt"));
            }
            "int" => {
                vals.push(json!(1));
                vals.push(json!(0));
            }
            "bool" => {
                vals.push(json!(true));
                vals.push(json!(false));
            }
            _ => {}
        }
        pools.insert(arg.clone(), vals);
    }
    pools
}

fn command_guard_sources(cmd: &CommandDef) -> Vec<&str> {
    let mut out = Vec::new();
    if let Some(when) = &cmd.when {
        out.push(when.as_str());
    }
    if let Some(outcomes) = &cmd.outcomes {
        for o in outcomes {
            if let Some(when) = &o.when {
                out.push(when.as_str());
            }
        }
    }
    out
}

fn harvest_command_guards(
    cmd: &CommandDef,
    arg_pools: &mut BTreeMap<String, Vec<Value>>,
    fact_pools: &mut BTreeMap<String, Vec<Value>>,
) {
    let declared = cmd.args.clone().unwrap_or_default();
    for src in command_guard_sources(cmd) {
        if let Ok(ast) = expr::parse(src) {
            harvest_expr(&ast, &declared, arg_pools, fact_pools);
        }
    }
}

fn harvest_expr(
    ast: &Expr,
    declared_args: &BTreeMap<String, String>,
    arg_pools: &mut BTreeMap<String, Vec<Value>>,
    fact_pools: &mut BTreeMap<String, Vec<Value>>,
) {
    match ast {
        Expr::Or(parts) | Expr::And(parts) => {
            for p in parts {
                harvest_expr(p, declared_args, arg_pools, fact_pools);
            }
        }
        Expr::Cmp(cmp) => {
            harvest_cmp_pair(&cmp.left, &cmp.right, declared_args, arg_pools, fact_pools);
            harvest_cmp_pair(&cmp.right, &cmp.left, declared_args, arg_pools, fact_pools);
        }
        Expr::TextIn(pred) => {
            let valid_len = (pred.min.max(1)).min(pred.max) as usize;
            let ch = pred.set.sample_valid_char();
            let valid_s: String = std::iter::repeat_n(ch, valid_len).collect();
            let mut samples = vec![json!(valid_s)];
            if let Some(alt_ch) = pred.set.sample_alt_valid_char() {
                let alt_s: String = std::iter::repeat_n(alt_ch, valid_len).collect();
                samples.push(json!(alt_s));
            }
            if pred.min > 0 {
                let short_s: String =
                    std::iter::repeat_n(ch, (pred.min - 1) as usize).collect();
                samples.push(json!(short_s));
            }
            if let Some(bad_ch) = pred.set.sample_invalid_char() {
                let mut bad_s = String::new();
                bad_s.push(bad_ch);
                for _ in 1..valid_len {
                    bad_s.push(ch);
                }
                samples.push(json!(bad_s));
            }
            match &pred.value {
                Operand::Arg(name) => {
                    if let Some(pool) = arg_pools.get_mut(name) {
                        // Put valid text_in samples at the front so the primary happy case passes.
                        for (idx, val) in samples.into_iter().enumerate() {
                            if idx < 2 {
                                insert_front_unique(pool, val);
                            } else {
                                push_unique(pool, val);
                            }
                        }
                    }
                }
                Operand::Fact(name) => {
                    let pool = fact_pools.entry(name.clone()).or_default();
                    for val in samples {
                        push_unique(pool, val);
                    }
                }
                _ => {}
            }
        }
        Expr::Exists(_) | Expr::NotExists(_) => {}
    }
}

fn insert_front_unique(pool: &mut Vec<Value>, value: Value) {
    if let Some(pos) = pool.iter().position(|v| v == &value) {
        pool.remove(pos);
    }
    pool.insert(0, value);
    if pool.len() > MAX_VALUES_PER_ARG {
        pool.truncate(MAX_VALUES_PER_ARG);
    }
}

fn harvest_cmp_pair(
    lhs: &Operand,
    rhs: &Operand,
    declared_args: &BTreeMap<String, String>,
    arg_pools: &mut BTreeMap<String, Vec<Value>>,
    fact_pools: &mut BTreeMap<String, Vec<Value>>,
) {
    let Operand::Literal(lit) = rhs else {
        return;
    };
    let mut vals = Vec::new();
    if let Some(n) = lit.as_i64() {
        vals.push(json!(n));
        vals.push(json!(n.saturating_sub(1)));
        vals.push(json!(n.saturating_add(1)));
    } else {
        vals.push(lit.clone());
    }
    match lhs {
        Operand::Arg(name) => {
            if let Some(pool) = arg_pools.get_mut(name) {
                for v in vals {
                    if matches_declared_type(&v, declared_args.get(name).map(String::as_str)) {
                        insert_front_unique(pool, v);
                    }
                }
            }
        }
        Operand::Fact(name) => {
            let pool = fact_pools.entry(name.clone()).or_default();
            for v in vals {
                push_unique(pool, v);
            }
        }
        Operand::Arith { base, add, amount } => {
            if let (Operand::Arg(name), Some(n)) = (base.as_ref(), lit.as_i64()) {
                let target = if *add {
                    n.saturating_sub(*amount)
                } else {
                    n.saturating_add(*amount)
                };
                if let Some(pool) = arg_pools.get_mut(name) {
                    for v in [
                        json!(target),
                        json!(target.saturating_sub(1)),
                        json!(target.saturating_add(1)),
                    ] {
                        push_unique(pool, v);
                    }
                }
            }
        }
        _ => {}
    }
}

fn matches_declared_type(v: &Value, ty: Option<&str>) -> bool {
    match ty {
        Some("int") => v.as_i64().is_some(),
        Some("string") => v.is_string(),
        Some("bool") => v.is_boolean(),
        _ => true,
    }
}

fn propagate_then_to_facts(
    cmd: &CommandDef,
    arg_pools: &BTreeMap<String, Vec<Value>>,
    fact_pools: &mut BTreeMap<String, Vec<Value>>,
) {
    let mut thens: Vec<&BTreeMap<String, String>> = Vec::new();
    if let Some(t) = &cmd.then {
        thens.push(t);
    }
    if let Some(outcomes) = &cmd.outcomes {
        for o in outcomes {
            if let Some(t) = &o.then {
                thens.push(t);
            }
        }
    }
    for map in thens {
        for (fact, rhs) in map {
            if let Some(arg) = rhs.strip_prefix('$') {
                if let Some(src_vals) = arg_pools.get(arg) {
                    let dst = fact_pools.entry(fact.clone()).or_default();
                    for v in src_vals.iter().take(3) {
                        push_unique(dst, v.clone());
                    }
                }
            } else if let Some(s) = rhs.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
                let dst = fact_pools.entry(fact.clone()).or_default();
                push_unique(dst, json!(s));
            } else if rhs == "true" || rhs == "false" {
                let dst = fact_pools.entry(fact.clone()).or_default();
                push_unique(dst, json!(rhs == "true"));
            } else if let Ok(n) = rhs.parse::<i64>() {
                let dst = fact_pools.entry(fact.clone()).or_default();
                push_unique(dst, json!(n));
            }
        }
    }
}

fn propagate_facts_to_args(
    cmd: &CommandDef,
    fact_pools: &BTreeMap<String, Vec<Value>>,
    arg_pools: &mut BTreeMap<String, Vec<Value>>,
) {
    let declared = cmd.args.clone().unwrap_or_default();
    let mut pairs: BTreeSet<(String, String)> = BTreeSet::new();
    for src in command_guard_sources(cmd) {
        if let Ok(ast) = expr::parse(src) {
            collect_arg_fact_links(&ast, &mut pairs);
        }
    }
    for (arg, fact) in pairs {
        let Some(fvals) = fact_pools.get(&fact) else {
            continue;
        };
        let Some(apool) = arg_pools.get_mut(&arg) else {
            continue;
        };
        let ty = declared.get(&arg).map(String::as_str);
        // Put the fact's primary value first so chained commands match immediately.
        for (idx, v) in fvals.iter().enumerate() {
            if !matches_declared_type(v, ty) {
                continue;
            }
            if idx == 0 {
                insert_front_unique(apool, v.clone());
            } else {
                push_unique(apool, v.clone());
            }
            if let Some(n) = v.as_i64() {
                push_unique(apool, json!(n.saturating_sub(1)));
                push_unique(apool, json!(n.saturating_add(1)));
            }
        }
    }
}

fn collect_arg_fact_links(ast: &Expr, pairs: &mut BTreeSet<(String, String)>) {
    match ast {
        Expr::Or(parts) | Expr::And(parts) => {
            for p in parts {
                collect_arg_fact_links(p, pairs);
            }
        }
        Expr::Cmp(cmp) => {
            let mut args = BTreeSet::new();
            let mut facts = BTreeSet::new();
            collect_operand_refs(&cmp.left, &mut args, &mut facts);
            collect_operand_refs(&cmp.right, &mut args, &mut facts);
            for a in &args {
                for f in &facts {
                    pairs.insert((a.clone(), f.clone()));
                }
            }
        }
        _ => {}
    }
}

fn collect_operand_refs(
    op: &Operand,
    args: &mut BTreeSet<String>,
    facts: &mut BTreeSet<String>,
) {
    match op {
        Operand::Arg(a) => {
            args.insert(a.clone());
        }
        Operand::Fact(f) => {
            facts.insert(f.clone());
        }
        Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => {
            collect_operand_refs(base, args, facts);
        }
        Operand::BinArith { left, right, .. } => {
            collect_operand_refs(left, args, facts);
            collect_operand_refs(right, args, facts);
        }
        Operand::State
        | Operand::EffectStatus
        | Operand::EffectKind
        | Operand::EffectOutcome
        | Operand::Literal(_) => {}
    }
}

fn synthesize_command_cases(
    declared: &BTreeMap<String, String>,
    arg_pools: &BTreeMap<String, Vec<Value>>,
) -> Vec<InputCase> {
    let arg_names: Vec<String> = declared.keys().cloned().collect();
    let defaults: BTreeMap<String, Value> = arg_names
        .iter()
        .map(|name| {
            let val = arg_pools
                .get(name)
                .and_then(|p| p.first())
                .cloned()
                .unwrap_or_else(|| fallback_value(declared[name].as_str()));
            (name.clone(), val)
        })
        .collect();

    let mut cases: Vec<InputCase> = Vec::new();
    let mut seen: Vec<BTreeMap<String, Value>> = Vec::new();

    let mut push_case = |name: String, args: BTreeMap<String, Value>| {
        if cases.len() >= MAX_CASES_PER_COMMAND || seen.contains(&args) {
            return;
        }
        seen.push(args.clone());
        cases.push(InputCase { name, args });
    };

    // Case 0: primary synthesized case (all preferred values).
    push_case("auto_primary".to_string(), defaults.clone());

    // Case 1: all-alternate case (e.g., lets a second call produce a distinct digest/value).
    let mut alt_all = defaults.clone();
    for name in &arg_names {
        if let Some(v) = arg_pools.get(name).and_then(|p| p.get(1)) {
            alt_all.insert(name.clone(), v.clone());
        }
    }
    push_case("auto_alt".to_string(), alt_all);

    // Sweep each argument across its remaining harvested boundary values.
    for name in &arg_names {
        if let Some(pool) = arg_pools.get(name) {
            for (idx, val) in pool.iter().enumerate().skip(1) {
                let mut varied = defaults.clone();
                varied.insert(name.clone(), val.clone());
                push_case(format!("auto_{name}_{idx}"), varied);
            }
        }
    }

    cases
}

fn fallback_value(ty: &str) -> Value {
    match ty {
        "int" => json!(1),
        "bool" => json!(true),
        "string" => json!("example"),
        _ => Value::Null,
    }
}
