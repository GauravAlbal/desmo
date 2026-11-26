//! Declarative machine definitions (`*.ik.toml`, schema_version 1) and their strict
//! static checks. The definition is inspectable data; the compiled machine is a plain
//! [`crate::machine::Machine`] with no runtime branching on domain concepts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u64 = 1;

/// Reserved words that cannot name facts or arguments.
pub const RESERVED: &[&str] = &["state", "exists", "not_exists", "true", "false"];

/// Provenance from region lowering. Omitted from spec-hash serialization.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OriginMap {
    /// Authored coordinates that remain even when no generated copy survives.
    pub authored: Vec<AuthoredCoordinate>,
    /// Generated obligation id or product label → authored coordinates it implements.
    pub generated: BTreeMap<String, Vec<AuthoredCoordinate>>,
    /// Remaining product label → region leaf assignment.
    pub products: BTreeMap<String, BTreeMap<String, String>>,
}

/// One authored region or joint command, before product lowering.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OriginEdge {
    pub command: String,
    pub layer: String,
    pub region: Option<String>,
    pub from: Vec<String>,
    pub to: String,
    pub when: Option<String>,
    pub changes: Vec<String>,
}

/// Stable authored identity. Parent fallback and child override are distinct.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AuthoredCoordinate {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

impl AuthoredCoordinate {
    pub fn outcome(command: &str, layer: &str, ordinal: usize) -> Self {
        Self {
            kind: "outcome".to_string(),
            command: Some(command.to_string()),
            layer: Some(layer.to_string()),
            ordinal: Some(ordinal),
            region: None,
            state: None,
        }
    }

    pub fn state(region: &str, state: &str) -> Self {
        Self {
            kind: "state".to_string(),
            command: None,
            layer: None,
            ordinal: None,
            region: Some(region.to_string()),
            state: Some(state.to_string()),
        }
    }

    pub fn terminal(region: &str, state: &str) -> Self {
        Self {
            kind: "terminal".to_string(),
            command: None,
            layer: None,
            ordinal: None,
            region: Some(region.to_string()),
            state: Some(state.to_string()),
        }
    }

    pub fn obligation_id(&self) -> String {
        match self.kind.as_str() {
            "outcome" => format!(
                "authored/outcome/{}/{}/{}/executable",
                self.command.as_deref().unwrap_or(""),
                self.layer.as_deref().unwrap_or("child"),
                self.ordinal.unwrap_or(0)
            ),
            "terminal" => format!(
                "authored/terminal/{}/{}/reachable",
                self.region.as_deref().unwrap_or(""),
                self.state.as_deref().unwrap_or("")
            ),
            _ => format!(
                "authored/state/{}/{}/executable",
                self.region.as_deref().unwrap_or(""),
                self.state.as_deref().unwrap_or("")
            ),
        }
    }

    pub fn obligation_kind(&self) -> &'static str {
        match self.kind.as_str() {
            "outcome" => "OBL-OUT-001",
            "terminal" => "OBL-TERM-001",
            _ => "OBL-STATE-001",
        }
    }
}

impl OriginMap {
    pub fn is_empty(&self) -> bool {
        self.authored.is_empty() && self.generated.is_empty() && self.products.is_empty()
    }

    pub fn generated_ids_for(&self, authored: &AuthoredCoordinate) -> Vec<String> {
        self.generated
            .iter()
            .filter(|(_, coords)| coords.contains(authored))
            .map(|(id, _)| id.clone())
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub schema_version: u64,
    pub machine_id: String,
    pub machine_version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub facts: BTreeMap<String, FactDef>,
    pub states: BTreeMap<String, StateDef>,
    pub commands: BTreeMap<String, CommandDef>,
    #[serde(default)]
    pub invariants: BTreeMap<String, InvariantDef>,
    /// Lowering provenance. Never part of spec_hash.
    #[serde(skip)]
    pub origin: OriginMap,
    /// Authored structure. Never part of spec_hash.
    #[serde(skip)]
    pub chart: super::chart::Chart,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactDef {
    #[serde(rename = "type")]
    pub ty: String,
    /// One of `counter`, `replace`, `once`, `kernel_only`. Default `replace`.
    #[serde(default = "default_policy")]
    pub policy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial: Option<Value>,
}

fn default_policy() -> String {
    "replace".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateDef {
    #[serde(default)]
    pub initial: bool,
    #[serde(default)]
    pub terminal: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDef {
    pub from: Vec<String>,
    /// Single-destination sugar; mutually exclusive with `outcomes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Guard for the single-destination sugar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Deterministic conditional outcomes, evaluated in order; first match wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcomes: Option<Vec<OutcomeDef>>,
    /// Declared command arguments: name -> "int" | "string" | "bool".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<BTreeMap<String, String>>,
    /// Fact assignments applied on commit: `fact = literal | $arg`, or `+N` on a counter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub then: Option<BTreeMap<String, String>>,
    /// Declare that this transition plans one external effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<EffectDef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub then: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectDef {
    /// Opaque effect kind.
    pub kind: String,
    /// Effect id: literal or `$arg`.
    pub id: String,
    /// Payload digest: literal or `$arg`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_digest: Option<String>,
    /// Idempotency key: literal or `$arg`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvariantDef {
    /// Optional state/fact condition that gates the assertion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// The asserted condition over state and facts.
    pub assert: String,
}

/// Closest candidate within Levenshtein edit distance 2 (for "did you mean?" diagnostics).
pub fn did_you_mean<'a>(needle: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<(&'a str, usize)> = None;
    for &cand in candidates {
        if cand == needle {
            continue;
        }
        let d = edit_distance(needle, cand);
        let max_d = if needle.len() <= 3 { 1 } else { 2 };
        if d <= max_d && best.map_or(true, |(_, bd)| d < bd) {
            best = Some((cand, d));
        }
    }
    best.map(|(s, _)| s)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

fn suggest_suffix(needle: &str, keys: impl Iterator<Item = impl AsRef<str>>) -> String {
    let owned: Vec<String> = keys.map(|k| k.as_ref().to_string()).collect();
    let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
    did_you_mean(needle, &refs)
        .map(|s| format!("; did you mean `{s}`?"))
        .unwrap_or_default()
}

/// Static-check failure collection.
pub type StaticErrors = Vec<String>;

pub fn check(def: &Definition) -> StaticErrors {
    let mut errors = StaticErrors::new();
    check_header(def, &mut errors);
    check_states(def, &mut errors);
    check_facts(def, &mut errors);
    check_commands(def, &mut errors);
    check_invariants(def, &mut errors);
    errors
}

/// Header invariants: schema version, identity fields, and at least one state.
fn check_header(def: &Definition, errors: &mut Vec<String>) {
    if def.schema_version != SCHEMA_VERSION {
        errors.push(format!(
            "unsupported schema_version {} (expected {SCHEMA_VERSION})",
            def.schema_version
        ));
    }
    if def.machine_id.trim().is_empty() {
        errors.push("machine_id must be non-empty".into());
    }
    if def.machine_version == 0 {
        errors.push("machine_version must be >= 1".into());
    }
    if def.states.is_empty() {
        errors.push("at least one state must be declared".into());
    }
}

/// State layout: exactly one initial state, and it must not be terminal.
fn check_states(def: &Definition, errors: &mut Vec<String>) {
    let initial_states: Vec<&String> = def
        .states
        .iter()
        .filter(|(_, s)| s.initial)
        .map(|(name, _)| name)
        .collect();
    if initial_states.len() != 1 {
        errors.push(format!(
            "exactly one initial state must be declared (found {})",
            initial_states.len()
        ));
    }
    if let Some(initial) = initial_states.first() {
        if def.states[*initial].terminal {
            errors.push(format!("initial state `{initial}` must not be terminal"));
        }
    }
}

/// Fact declarations: names, types, policies, and initial values.
fn check_facts(def: &Definition, errors: &mut Vec<String>) {
    for (name, fact) in &def.facts {
        if RESERVED.contains(&name.as_str()) {
            errors.push(format!("fact `{name}` uses a reserved word"));
        }
        if !matches!(fact.ty.as_str(), "int" | "string" | "bool") {
            errors.push(format!(
                "fact `{name}` has unknown type `{}` (expected one of: int, string, bool)",
                fact.ty
            ));
        }
        if !matches!(
            fact.policy.as_str(),
            "counter" | "replace" | "once" | "kernel_only"
        ) {
            errors.push(format!(
                "fact `{name}` has unknown policy `{}` (expected one of: counter, replace, once, kernel_only)",
                fact.policy
            ));
        }
        if fact.policy == "counter" {
            match &fact.initial {
                Some(Value::Number(n)) if n.is_i64() => {}
                _ => errors.push(format!(
                    "counter fact `{name}` requires an integer `initial` value"
                )),
            }
        }
        if let Some(v) = &fact.initial {
            if !value_matches_type(v, &fact.ty) {
                errors.push(format!(
                    "fact `{name}` initial value does not match type `{}`",
                    fact.ty
                ));
            }
        }
    }
}

/// Command checks, one per declared command.
fn check_commands(def: &Definition, errors: &mut Vec<String>) {
    for (name, cmd) in &def.commands {
        check_command(def, name, cmd, errors);
    }
}

fn check_command(def: &Definition, name: &str, cmd: &CommandDef, errors: &mut Vec<String>) {
    check_command_name(name, errors);
    check_command_sources(def, cmd, name, errors);
    check_dest_shape(cmd, name, errors);
    check_command_args(cmd.args.as_ref(), name, errors);
    check_command_to(def, cmd, name, errors);
    check_command_when(def, cmd, name, errors);
    check_command_outcomes(def, cmd, name, errors);
    if let Some(then) = &cmd.then {
        for issue in check_then(then, def, cmd, name) {
            errors.push(issue);
        }
    }
    check_effect(cmd, name, errors);
}

fn check_command_name(name: &str, errors: &mut Vec<String>) {
    if RESERVED.contains(&name) {
        errors.push(format!("command `{name}` uses a reserved word"));
    }
    if name == "genesis" || name.starts_with("kernel.") {
        errors.push(format!(
            "command `{name}` uses the reserved kernel event namespace"
        ));
    }
}

fn check_command_sources(def: &Definition, cmd: &CommandDef, name: &str, errors: &mut Vec<String>) {
    if cmd.from.is_empty() {
        errors.push(format!(
            "command `{name}` must declare at least one source state"
        ));
    }
    for from in &cmd.from {
        if !def.states.contains_key(from) {
            let hint = suggest_suffix(from, def.states.keys());
            errors.push(format!(
                "command `{name}` references unknown source state `{from}`{hint}"
            ));
        }
        if let Some(s) = def.states.get(from) {
            if s.terminal {
                errors.push(format!(
                    "command `{name}` lists terminal state `{from}` as a source"
                ));
            }
        }
    }
}

/// Destination shape: `to`/`when` sugar and `outcomes` are mutually exclusive,
/// and exactly one of them must be present.
fn check_dest_shape(cmd: &CommandDef, name: &str, errors: &mut Vec<String>) {
    let has_to = cmd.to.is_some() || cmd.when.is_some();
    if has_to && cmd.outcomes.is_some() {
        errors.push(format!(
            "command `{name}` must not combine `to`/`when` with `outcomes`"
        ));
    }
    if !has_to && cmd.outcomes.is_none() {
        errors.push(format!("command `{name}` must declare `to` or `outcomes`"));
    }
}

fn check_command_args(
    args: Option<&BTreeMap<String, String>>,
    name: &str,
    errors: &mut Vec<String>,
) {
    let Some(args) = args else { return };
    for (arg, ty) in args {
        if RESERVED.contains(&arg.as_str()) {
            errors.push(format!(
                "command `{name}` argument `${arg}` uses a reserved word"
            ));
        }
        if !matches!(ty.as_str(), "int" | "string" | "bool") {
            errors.push(format!(
                "command `{name}` argument `${arg}` has unknown type `{ty}` (expected one of: int, string, bool)"
            ));
        }
    }
}

fn check_command_to(def: &Definition, cmd: &CommandDef, name: &str, errors: &mut Vec<String>) {
    let Some(to) = &cmd.to else { return };
    if !def.states.contains_key(to) {
        let hint = suggest_suffix(to, def.states.keys());
        errors.push(format!("command `{name}` targets unknown state `{to}`{hint}"));
    }
}

fn check_command_when(def: &Definition, cmd: &CommandDef, name: &str, errors: &mut Vec<String>) {
    let Some(when) = &cmd.when else { return };
    if cmd.to.is_none() {
        errors.push(format!("command `{name}` has `when` without `to`"));
    }
    match crate::declarative::expr::parse(when) {
        Ok(expr) => {
            if let Some(issues) = check_guard(&expr, def, cmd) {
                for issue in issues {
                    errors.push(format!("command `{name}` guard: {issue}"));
                }
            }
        }
        Err(e) => errors.push(format!("command `{name}` guard parse error: {e}")),
    }
}

fn check_command_outcomes(
    def: &Definition,
    cmd: &CommandDef,
    name: &str,
    errors: &mut Vec<String>,
) {
    let Some(outcomes) = &cmd.outcomes else {
        return;
    };
    if outcomes.is_empty() {
        errors.push(format!(
            "command `{name}` declares an empty `outcomes` list"
        ));
    }
    for (i, outcome) in outcomes.iter().enumerate() {
        check_outcome(def, outcome, i, cmd, name, errors);
        if outcome.when.is_none() && i + 1 < outcomes.len() {
            errors.push(format!(
                "command `{name}` outcome {i} is unconditional, so later outcomes are unreachable"
            ));
        }
    }
}

fn check_outcome(
    def: &Definition,
    outcome: &OutcomeDef,
    i: usize,
    cmd: &CommandDef,
    name: &str,
    errors: &mut Vec<String>,
) {
    if !def.states.contains_key(&outcome.to) {
        let hint = suggest_suffix(&outcome.to, def.states.keys());
        errors.push(format!(
            "command `{name}` outcome {i} targets unknown state `{}`{hint}",
            outcome.to
        ));
    }
    if let Some(when) = &outcome.when {
        match crate::declarative::expr::parse(when) {
            Ok(expr) => {
                if let Some(issues) = check_guard(&expr, def, cmd) {
                    for issue in issues {
                        errors.push(format!("command `{name}` outcome {i} guard: {issue}"));
                    }
                }
            }
            Err(e) => errors.push(format!(
                "command `{name}` outcome {i} guard parse error: {e}"
            )),
        }
    }
    if let Some(then) = &outcome.then {
        for issue in check_then(then, def, cmd, name) {
            errors.push(format!("command `{name}` outcome {i}: {issue}"));
        }
    }
}

fn check_effect(cmd: &CommandDef, name: &str, errors: &mut Vec<String>) {
    let Some(effect) = &cmd.effect else { return };
    if effect.id.is_empty() {
        errors.push(format!("command `{name}` effect declares an empty id"));
    }
    for (field, value) in [
        ("id", effect.id.as_str()),
        (
            "payload_digest",
            effect.payload_digest.as_deref().unwrap_or(""),
        ),
        (
            "idempotency_key",
            effect.idempotency_key.as_deref().unwrap_or(""),
        ),
    ] {
        if let Some(arg) = value.strip_prefix('$') {
            if !cmd
                .args
                .as_ref()
                .map(|a| a.contains_key(arg))
                .unwrap_or(false)
            {
                errors.push(format!(
                    "command `{name}` effect {field} references unknown arg `${arg}`"
                ));
            }
        } else if value.is_empty() {
            errors.push(format!("command `{name}` effect {field} is required"));
        }
    }
}

/// Invariant fact references in `when` and `assert` clauses.
fn check_invariants(def: &Definition, errors: &mut Vec<String>) {
    for (inv_name, inv) in &def.invariants {
        if let Some(when) = &inv.when {
            match crate::declarative::expr::parse(when) {
                Ok(expr) => check_invariant_fact_refs(&expr, def, inv_name, "when ", errors),
                Err(e) => errors.push(format!("invariant `{inv_name}` when parse error: {e}")),
            }
        }
        match crate::declarative::expr::parse(&inv.assert) {
            Ok(expr) => check_invariant_fact_refs(&expr, def, inv_name, "", errors),
            Err(e) => errors.push(format!("invariant `{inv_name}` assert parse error: {e}")),
        }
    }
}

/// Emit `unknown fact` errors for every fact reference in an invariant expression.
/// `when_prefix` is `"when "` for a `when` clause and `""` for an `assert` clause.
fn check_invariant_fact_refs(
    expr: &crate::declarative::expr::Expr,
    def: &Definition,
    inv_name: &str,
    when_prefix: &str,
    errors: &mut Vec<String>,
) {
    let mut facts = Vec::new();
    collect_fact_refs(expr, &mut facts);
    for f in facts {
        if !def.facts.contains_key(&f) {
            errors.push(format!(
                "invariant `{inv_name}` {when_prefix}references unknown fact `{f}`"
            ));
        }
    }
}

fn value_matches_type(v: &Value, ty: &str) -> bool {
    match ty {
        "int" => v.as_i64().is_some(),
        "string" => v.is_string(),
        "bool" => v.is_boolean(),
        _ => false,
    }
}

fn collect_fact_refs(expr: &crate::declarative::expr::Expr, out: &mut Vec<String>) {
    use crate::declarative::expr::Expr as E;
    match expr {
        E::Or(parts) | E::And(parts) => {
            for p in parts {
                collect_fact_refs(p, out);
            }
        }
        E::Cmp(cmp) => {
            collect_operand_refs(&cmp.left, out);
            collect_operand_refs(&cmp.right, out);
        }
        E::Exists(name) | E::NotExists(name) => out.push(name.clone()),
        E::TextIn(pred) => collect_operand_refs(&pred.value, out),
    }
}

fn collect_operand_refs(op: &crate::declarative::expr::Operand, out: &mut Vec<String>) {
    use crate::declarative::expr::Operand;
    match op {
        Operand::Fact(name) => out.push(name.clone()),
        Operand::Arith { base, .. } | Operand::Mul { base, .. } | Operand::Div { base, .. } => {
            collect_operand_refs(base, out)
        }
        Operand::BinArith { left, right, .. } => {
            collect_operand_refs(left, out);
            collect_operand_refs(right, out);
        }
        _ => {}
    }
}

/// Type-check a guard: facts exist, arg refs exist, operand types are compatible.
fn check_guard(
    expr: &crate::declarative::expr::Expr,
    def: &Definition,
    cmd: &CommandDef,
) -> Option<Vec<String>> {
    let mut issues = Vec::new();
    walk_guard(expr, def, cmd, &mut issues);
    if issues.is_empty() {
        None
    } else {
        Some(issues)
    }
}

fn walk_guard(
    expr: &crate::declarative::expr::Expr,
    def: &Definition,
    cmd: &CommandDef,
    issues: &mut Vec<String>,
) {
    use crate::declarative::expr::Expr as E;
    match expr {
        E::Or(parts) | E::And(parts) => {
            for p in parts {
                walk_guard(p, def, cmd, issues);
            }
        }
        E::Exists(name) | E::NotExists(name) => {
            if !def.facts.contains_key(name) {
                issues.push(format!("unknown fact `{name}`"));
            }
        }
        E::Cmp(cmp) => {
            let lt = operand_ty(&cmp.left, def, cmd, issues);
            let rt = operand_ty(&cmp.right, def, cmd, issues);
            if let (Some(l), Some(r)) = (lt, rt) {
                if !types_compatible(l, r, cmp.op) {
                    issues.push(format!("type mismatch: cannot compare {l:?} with {r:?}"));
                }
            }
        }
        E::TextIn(pred) => match operand_ty(&pred.value, def, cmd, issues) {
            Some(Ty::Str) => {}
            Some(other) => {
                issues.push(format!("text_in VALUE must be a string, not {other:?}"));
            }
            None => {}
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ty {
    Int,
    Str,
    Bool,
}

fn operand_ty(
    op: &crate::declarative::expr::Operand,
    def: &Definition,
    cmd: &CommandDef,
    issues: &mut Vec<String>,
) -> Option<Ty> {
    use crate::declarative::expr::Operand;
    match op {
        Operand::Fact(name) => fact_operand_ty(name, def, issues),
        Operand::Arg(name) => arg_operand_ty(name, cmd, issues),
        Operand::State => Some(Ty::Str),
        Operand::Literal(v) => literal_operand_ty(v),
        Operand::Arith { base, .. } => arith_operand_ty(base, def, cmd, issues),
        Operand::BinArith { left, right, .. } => {
            arith_operand_ty(left, def, cmd, issues);
            arith_operand_ty(right, def, cmd, issues)
        }
        Operand::Mul { base, amount } => scale_operand_ty("*", *amount, base, def, cmd, issues),
        Operand::Div { base, amount } => scale_operand_ty("/", *amount, base, def, cmd, issues),
    }
}

fn fact_operand_ty(name: &str, def: &Definition, issues: &mut Vec<String>) -> Option<Ty> {
    match def.facts.get(name) {
        Some(f) => fact_ty(f),
        None => {
            issues.push(format!("unknown fact `{name}`"));
            None
        }
    }
}

fn arg_operand_ty(name: &str, cmd: &CommandDef, issues: &mut Vec<String>) -> Option<Ty> {
    match cmd.args.as_ref().and_then(|a| a.get(name)) {
        Some(ty) => ty_from_str(ty),
        None => {
            issues.push(format!("unknown argument `${name}`"));
            None
        }
    }
}

fn ty_from_str(ty: &str) -> Option<Ty> {
    match ty {
        "int" => Some(Ty::Int),
        "string" => Some(Ty::Str),
        "bool" => Some(Ty::Bool),
        _ => None,
    }
}

fn literal_operand_ty(v: &Value) -> Option<Ty> {
    if v.is_string() {
        Some(Ty::Str)
    } else if v.as_i64().is_some() {
        Some(Ty::Int)
    } else if v.is_boolean() {
        Some(Ty::Bool)
    } else {
        None
    }
}

fn arith_operand_ty(
    base: &crate::declarative::expr::Operand,
    def: &Definition,
    cmd: &CommandDef,
    issues: &mut Vec<String>,
) -> Option<Ty> {
    match operand_ty(base, def, cmd, issues) {
        Some(Ty::Int) => Some(Ty::Int),
        other => {
            issues.push("arithmetic requires an integer operand".into());
            other
        }
    }
}

fn scale_operand_ty(
    op: &str,
    amount: i64,
    base: &crate::declarative::expr::Operand,
    def: &Definition,
    cmd: &CommandDef,
    issues: &mut Vec<String>,
) -> Option<Ty> {
    if amount <= 0 {
        issues.push(format!("`{op}` requires a positive integer constant"));
    }
    arith_operand_ty(base, def, cmd, issues)
}

fn fact_ty(f: &FactDef) -> Option<Ty> {
    match f.ty.as_str() {
        "int" => Some(Ty::Int),
        "string" => Some(Ty::Str),
        "bool" => Some(Ty::Bool),
        _ => None,
    }
}

fn types_compatible(l: Ty, r: Ty, op: crate::declarative::expr::CmpOp) -> bool {
    use crate::declarative::expr::CmpOp;
    if l != r {
        return false;
    }
    match op {
        CmpOp::Eq | CmpOp::Ne => true,
        _ => l == Ty::Int || l == Ty::Str,
    }
}

/// Check `then` assignments: targets exist, are writable, and values type-match.
fn check_then(
    then: &BTreeMap<String, String>,
    def: &Definition,
    cmd: &CommandDef,
    cmd_name: &str,
) -> Vec<String> {
    let mut issues = Vec::new();
    for (fact, value) in then {
        let Some(fdef) = def.facts.get(fact) else {
            issues.push(format!("`then` assigns unknown fact `{fact}`"));
            continue;
        };
        if fdef.policy == "kernel_only" {
            issues.push(format!("`then` may not assign kernel_only fact `{fact}`"));
        }
        if fdef.policy == "counter" {
            if !(value.starts_with('+') && value[1..].parse::<i64>().is_ok()) {
                issues.push(format!(
                    "counter fact `{fact}` accepts only `+N` increments, got `{value}`"
                ));
            }
            continue;
        }
        if value.starts_with('+') && value[1..].parse::<i64>().is_ok() {
            issues.push(format!(
                "`then` value `{value}` for `{fact}` uses `+N`, which is only for counter facts"
            ));
            continue;
        }
        if let Some(arg) = value.strip_prefix('$') {
            match cmd.args.as_ref().and_then(|a| a.get(arg)) {
                None => issues.push(format!(
                    "`then` for `{fact}` references unknown arg `${arg}`"
                )),
                Some(arg_ty) if arg_ty != &fdef.ty => issues.push(format!(
                    "`then` for `{fact}` assigns `${arg}` of type `{arg_ty}` to `{}` fact",
                    fdef.ty
                )),
                Some(_) => {}
            }
        } else {
            let literal = parse_literal(value);
            match literal {
                Some(v) if value_matches_type(&v, &fdef.ty) => {}
                Some(_) => issues.push(format!(
                    "`then` value `{value}` for `{fact}` does not match type `{}` ({})",
                    fdef.ty,
                    then_type_hint(&fdef.ty)
                )),
                None => issues.push(format!(
                    "`then` value `{value}` for `{fact}` is not a literal or `$arg` ({})",
                    then_type_hint(&fdef.ty)
                )),
            }
        }
    }
    let _ = cmd_name;
    issues
}

fn then_type_hint(ty: &str) -> &'static str {
    match ty {
        "string" => r#"use inner quotes like '"paid"' or '""', or "$arg""#,
        "bool" => r#"use "true", "false", or "$arg""#,
        "int" => r#"use an integer string like "1", "+1" on a counter, or "$arg""#,
        _ => r#"use a literal ('"text"', "1", "true"/"false") or "$arg""#,
    }
}

fn parse_literal(value: &str) -> Option<Value> {
    if value == "true" {
        return Some(Value::Bool(true));
    }
    if value == "false" {
        return Some(Value::Bool(false));
    }
    if let Ok(n) = value.parse::<i64>() {
        return Some(Value::Number(n.into()));
    }
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        return Some(Value::String(value[1..value.len() - 1].to_string()));
    }
    None
}
