//! Deterministic verification-plan compiler.
//!
//! A validated [`Definition`] yields a canonical [`VerificationPlanV1`]. This
//! module reads semantic authority and emits evidence obligations. It does not
//! execute transitions, persist state, or create runtime authority.

use super::def::{AuthoredCoordinate, CommandDef, Definition};
use crate::canonical::digest_json;
use serde::{Deserialize, Serialize};

/// Plan schema without impossibility records. A plan with no records keeps this
/// version and serializes without the field, so its digest is unchanged.
pub const PLAN_SCHEMA_BASE: u64 = 1;

/// Plan schema carrying impossibility records; used only when a plan has some.
pub const PLAN_SCHEMA_VERSION: u64 = 2;

/// Versioned, spec-hash-bound verification contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationPlanV1 {
    pub schema_version: u64,
    pub machine: PlanMachine,
    pub obligations: Vec<VerificationObligationV1>,
    pub challenges: Vec<SemanticChallengeV1>,
    /// Positive success goals proved impossible from guards, invariants, or fact-flow.
    /// Generated product-context rows may be discharged; authored behavior is never
    /// satisfied by a proof. Schema-1 ordered-outcome proofs stay recorded and retained.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub impossibilities: Vec<ImpossibilityRecordV1>,
    /// Authored-behavior goals and the generated copies that implement them.
    /// Present only when region lowering produced an origin map.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authored_goals: Vec<AuthoredGoalV1>,
}

/// Explicit record that a positive success obligation cannot be witnessed.
///
/// `discharged` is true only for generated product-context goals with a sound
/// proof. Authored goals stay mandatory. Schema-1 ordered-outcome proofs are
/// recorded and retained (`discharged=false`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpossibilityRecordV1 {
    pub obligation_id: String,
    pub kind: String,
    /// `command_source`, `ordered_outcome`, or `reachability`.
    pub class: String,
    /// When true, the obligation is not demanded as a successful commit.
    pub discharged: bool,
    pub reason: String,
    pub proving_source: String,
    pub coordinate: SemanticCoordinate,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub basis: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
    /// `SolverAsserted`, `CheckedInvariant`, or `CheckedExhaustive`. Empty on
    /// in-house records so default `ik check` JSON stays unchanged.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub trust: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub certificate_digest: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub solvers: Vec<String>,
}

/// Stable authored coordinate plus the generated copies that implement it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredGoalV1 {
    pub obligation_id: String,
    pub kind: String,
    pub coordinate: AuthoredCoordinate,
    pub generated_ids: Vec<String>,
}

/// Coordinate-stable plan delta. Matching is by logical coordinate, not spec_hash.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationPlanDiff {
    pub obligations: Vec<CoordinateDelta>,
    pub challenges: Vec<CoordinateDelta>,
}

/// One obligation or challenge classified across two plans.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoordinateDelta {
    pub coordinate: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_digest: Option<String>,
}

impl VerificationPlanDiff {
    pub fn obligations_added(&self) -> Vec<String> {
        named(&self.obligations, "ADDED")
    }
    pub fn obligations_removed(&self) -> Vec<String> {
        named(&self.obligations, "REMOVED")
    }
    pub fn obligations_modified(&self) -> Vec<String> {
        named(&self.obligations, "MODIFIED")
    }
    pub fn obligations_unchanged(&self) -> Vec<String> {
        named(&self.obligations, "UNCHANGED")
    }
    pub fn status(&self, coordinate: &str) -> Option<&str> {
        self.obligations
            .iter()
            .find(|d| d.coordinate == coordinate)
            .map(|d| d.status.as_str())
    }
}

fn named(rows: &[CoordinateDelta], status: &str) -> Vec<String> {
    rows.iter()
        .filter(|d| d.status == status)
        .map(|d| d.coordinate.clone())
        .collect()
}

/// Diff two plans by logical coordinate and local semantic digest.
pub fn plan_diff(old: &VerificationPlanV1, new: &VerificationPlanV1) -> VerificationPlanDiff {
    VerificationPlanDiff {
        obligations: classify(
            old.obligations
                .iter()
                .map(|o| (o.obligation_id.clone(), o.local_semantic_digest.clone())),
            new.obligations
                .iter()
                .map(|o| (o.obligation_id.clone(), o.local_semantic_digest.clone())),
        ),
        challenges: classify(
            old.challenges
                .iter()
                .map(|c| (c.challenge_id.clone(), c.local_semantic_digest.clone())),
            new.challenges
                .iter()
                .map(|c| (c.challenge_id.clone(), c.local_semantic_digest.clone())),
        ),
    }
}

fn classify(
    old: impl IntoIterator<Item = (String, String)>,
    new: impl IntoIterator<Item = (String, String)>,
) -> Vec<CoordinateDelta> {
    let old: std::collections::BTreeMap<_, _> = old.into_iter().collect();
    let new: std::collections::BTreeMap<_, _> = new.into_iter().collect();
    let mut keys: std::collections::BTreeSet<_> = old.keys().cloned().collect();
    keys.extend(new.keys().cloned());
    keys.into_iter()
        .map(|coordinate| {
            let old_digest = old.get(&coordinate).cloned();
            let new_digest = new.get(&coordinate).cloned();
            let status = match (&old_digest, &new_digest) {
                (Some(a), Some(b)) if a == b => "UNCHANGED",
                (Some(_), Some(_)) => "MODIFIED",
                (None, Some(_)) => "ADDED",
                (Some(_), None) => "REMOVED",
                (None, None) => "UNCHANGED",
            };
            CoordinateDelta {
                coordinate,
                status: status.into(),
                old_digest,
                new_digest,
            }
        })
        .collect()
}

/// Machine identity slice bound into the plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanMachine {
    pub machine_id: String,
    pub machine_version: u64,
    pub spec_hash: String,
}

/// One required evidence item derived from declared semantics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationObligationV1 {
    pub obligation_id: String,
    pub kind: String,
    pub machine_id: String,
    pub spec_hash: String,
    pub local_semantic_digest: String,
    pub coordinate: SemanticCoordinate,
    pub evidence_class: String,
    pub mandatory: bool,
}

/// Semantic location an obligation or challenge refers to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticCoordinate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invariant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

/// Plausible competing behavior. Empty in VFY-01; filled by the challenge compiler.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticChallengeV1 {
    pub challenge_id: String,
    pub kind: String,
    pub source_obligation_ids: Vec<String>,
    pub coordinate: SemanticCoordinate,
    pub transformation: String,
    pub local_semantic_digest: String,
    pub expected_relation: String,
    pub mandatory: bool,
}

/// Compile a canonical verification plan from a validated definition.
pub fn verification_plan(def: &Definition) -> VerificationPlanV1 {
    let spec_hash = digest_json(def);
    let mut obligations = Vec::new();
    emit_state_obligations(def, &spec_hash, &mut obligations);
    emit_command_obligations(def, &spec_hash, &mut obligations);
    emit_outcome_obligations(def, &spec_hash, &mut obligations);
    emit_invariant_obligations(def, &spec_hash, &mut obligations);
    emit_terminal_obligations(def, &spec_hash, &mut obligations);
    emit_effect_obligations(def, &spec_hash, &mut obligations);
    emit_kernel_law_obligations(def, &spec_hash, &mut obligations);
    let authored_goals = authored_goal_rows(def);
    emit_authored_obligations(def, &spec_hash, &authored_goals, &mut obligations);
    let impossibilities = super::feasibility::impossible_success_records(def);
    let skip: std::collections::BTreeSet<_> = impossibilities
        .iter()
        .filter(|row| row.discharged)
        .map(|row| row.obligation_id.clone())
        .collect();
    obligations.retain(|obligation| !skip.contains(&obligation.obligation_id));
    obligations.sort_by(|a, b| a.obligation_id.cmp(&b.obligation_id));
    VerificationPlanV1 {
        schema_version: if impossibilities.is_empty() && authored_goals.is_empty() {
            PLAN_SCHEMA_BASE
        } else {
            PLAN_SCHEMA_VERSION
        },
        machine: PlanMachine {
            machine_id: def.machine_id.clone(),
            machine_version: def.machine_version,
            spec_hash: spec_hash.clone(),
        },
        obligations,
        challenges: super::challenge::semantic_challenges(def, &spec_hash),
        impossibilities,
        authored_goals,
    }
}

fn authored_goal_rows(def: &Definition) -> Vec<AuthoredGoalV1> {
    let mut rows = Vec::new();
    for authored in &def.origin.authored {
        let mut generated_ids = def.origin.generated_ids_for(authored);
        generated_ids.sort();
        generated_ids.dedup();
        rows.push(AuthoredGoalV1 {
            obligation_id: authored.obligation_id(),
            kind: authored.obligation_kind().to_string(),
            coordinate: authored.clone(),
            generated_ids,
        });
    }
    rows
}

fn emit_authored_obligations(
    def: &Definition,
    spec_hash: &str,
    goals: &[AuthoredGoalV1],
    out: &mut Vec<VerificationObligationV1>,
) {
    for goal in goals {
        out.push(obligation(
            def,
            spec_hash,
            &goal.kind,
            &goal.obligation_id,
            authored_semantic(&goal.coordinate),
            "authored",
        ));
    }
}

fn authored_semantic(coord: &AuthoredCoordinate) -> SemanticCoordinate {
    SemanticCoordinate {
        state: coord.state.clone(),
        command: coord.command.clone(),
        source: None,
        outcome_index: coord.ordinal,
        invariant: None,
        layer: coord.layer.clone(),
        region: coord.region.clone(),
    }
}

fn emit_state_obligations(
    def: &Definition,
    spec_hash: &str,
    out: &mut Vec<VerificationObligationV1>,
) {
    for (name, state) in &def.states {
        if state.initial {
            continue;
        }
        out.push(obligation(
            def,
            spec_hash,
            "OBL-STATE-001",
            &format!("state/{name}/executable"),
            SemanticCoordinate {
                state: Some(name.clone()),
                ..SemanticCoordinate::default()
            },
            "executable",
        ));
    }
}

fn emit_command_obligations(
    def: &Definition,
    spec_hash: &str,
    out: &mut Vec<VerificationObligationV1>,
) {
    for (command, cmd) in &def.commands {
        for source in &cmd.from {
            out.push(obligation(
                def,
                spec_hash,
                "OBL-CMD-001",
                &format!("command/{command}/from/{source}/executable"),
                SemanticCoordinate {
                    command: Some(command.clone()),
                    source: Some(source.clone()),
                    ..SemanticCoordinate::default()
                },
                "executable",
            ));
        }
    }
}

fn emit_outcome_obligations(
    def: &Definition,
    spec_hash: &str,
    out: &mut Vec<VerificationObligationV1>,
) {
    for (command, cmd) in &def.commands {
        for index in 0..outcome_count(cmd) {
            out.push(obligation(
                def,
                spec_hash,
                "OBL-OUT-001",
                &format!("outcome/{command}/{index}/executable"),
                SemanticCoordinate {
                    command: Some(command.clone()),
                    outcome_index: Some(index),
                    ..SemanticCoordinate::default()
                },
                "executable",
            ));
        }
    }
}

fn emit_invariant_obligations(
    def: &Definition,
    spec_hash: &str,
    out: &mut Vec<VerificationObligationV1>,
) {
    for name in def.invariants.keys() {
        out.push(obligation(
            def,
            spec_hash,
            "OBL-INV-001",
            &format!("invariant/{name}/evaluated"),
            SemanticCoordinate {
                invariant: Some(name.clone()),
                ..SemanticCoordinate::default()
            },
            "evaluated",
        ));
    }
}

fn emit_terminal_obligations(
    def: &Definition,
    spec_hash: &str,
    out: &mut Vec<VerificationObligationV1>,
) {
    for (name, state) in &def.states {
        if !state.terminal {
            continue;
        }
        out.push(obligation(
            def,
            spec_hash,
            "OBL-TERM-001",
            &format!("terminal/{name}/reachable"),
            SemanticCoordinate {
                state: Some(name.clone()),
                ..SemanticCoordinate::default()
            },
            "executable",
        ));
        out.push(obligation(
            def,
            spec_hash,
            "OBL-TERM-002",
            &format!("terminal/{name}/mutation_refused"),
            SemanticCoordinate {
                state: Some(name.clone()),
                ..SemanticCoordinate::default()
            },
            "refusal",
        ));
    }
}

fn emit_effect_obligations(
    def: &Definition,
    spec_hash: &str,
    out: &mut Vec<VerificationObligationV1>,
) {
    let mut effect_cmds = Vec::new();
    for (command, cmd) in &def.commands {
        if cmd.effect.is_some() {
            effect_cmds.push(command.clone());
        }
    }
    if effect_cmds.is_empty() {
        return;
    }
    for command in &effect_cmds {
        out.push(obligation(
            def,
            spec_hash,
            "OBL-EFFECT-001",
            &format!("effect/{command}/planned"),
            SemanticCoordinate {
                command: Some(command.clone()),
                ..SemanticCoordinate::default()
            },
            "effect",
        ));
    }
    out.push(obligation(
        def,
        spec_hash,
        "OBL-EFFECT-002",
        "effect/exact_duplicate",
        SemanticCoordinate::default(),
        "effect",
    ));
    out.push(obligation(
        def,
        spec_hash,
        "OBL-EFFECT-003",
        "effect/conflict",
        SemanticCoordinate::default(),
        "effect",
    ));
    out.push(obligation(
        def,
        spec_hash,
        "OBL-EFFECT-004",
        "effect/single_pending",
        SemanticCoordinate::default(),
        "effect",
    ));
}

fn emit_kernel_law_obligations(
    def: &Definition,
    spec_hash: &str,
    out: &mut Vec<VerificationObligationV1>,
) {
    out.push(obligation(
        def,
        spec_hash,
        "OBL-CAP-001",
        "capability/stale",
        SemanticCoordinate::default(),
        "refusal",
    ));
    out.push(obligation(
        def,
        spec_hash,
        "OBL-REFUSE-001",
        "refusal/immutability",
        SemanticCoordinate::default(),
        "refusal",
    ));
    out.push(obligation(
        def,
        spec_hash,
        "OBL-REPLAY-001",
        "replay/equivalence",
        SemanticCoordinate::default(),
        "replay",
    ));
    out.push(obligation(
        def,
        spec_hash,
        "OBL-DETERMINISM-001",
        "determinism/fold",
        SemanticCoordinate::default(),
        "determinism",
    ));
    out.push(obligation(
        def,
        spec_hash,
        "OBL-EVENT-001",
        "event/frozen_choice",
        SemanticCoordinate::default(),
        "replay",
    ));
}

fn obligation(
    def: &Definition,
    spec_hash: &str,
    kind: &str,
    suffix: &str,
    coordinate: SemanticCoordinate,
    evidence_class: &str,
) -> VerificationObligationV1 {
    VerificationObligationV1 {
        obligation_id: suffix.to_string(),
        kind: kind.to_string(),
        machine_id: def.machine_id.clone(),
        spec_hash: spec_hash.to_string(),
        local_semantic_digest: local_semantic_digest(def, kind, &coordinate),
        coordinate,
        evidence_class: evidence_class.to_string(),
        mandatory: true,
    }
}

fn local_semantic_digest(def: &Definition, kind: &str, coordinate: &SemanticCoordinate) -> String {
    digest_json(&local_meaning(def, kind, coordinate))
}

fn local_meaning(
    def: &Definition,
    kind: &str,
    coordinate: &SemanticCoordinate,
) -> serde_json::Value {
    match kind {
        "OBL-STATE-001" | "OBL-TERM-001" | "OBL-TERM-002" => {
            if let Some(region) = &coordinate.region {
                return serde_json::json!({
                    "kind": kind,
                    "tier": "authored",
                    "region": region,
                    "state": coordinate.state,
                });
            }
            let name = coordinate.state.as_deref().unwrap_or("");
            let state = def.states.get(name);
            serde_json::json!({
                "kind": kind,
                "state": name,
                "initial": state.map(|s| s.initial),
                "terminal": state.map(|s| s.terminal),
            })
        }
        "OBL-CMD-001" => {
            let command = coordinate.command.as_deref().unwrap_or("");
            let cmd = def.commands.get(command);
            serde_json::json!({
                "kind": kind,
                "command": command,
                "source": coordinate.source,
                "from": cmd.map(|c| &c.from),
                "to": cmd.and_then(|c| c.to.as_ref()),
                "when": cmd.and_then(|c| c.when.as_ref()),
                "outcomes": cmd.and_then(|c| c.outcomes.as_ref()),
                "effect": cmd.and_then(|c| c.effect.as_ref()),
            })
        }
        "OBL-OUT-001" => {
            if let Some(layer) = &coordinate.layer {
                return serde_json::json!({
                    "kind": kind,
                    "tier": "authored",
                    "command": coordinate.command,
                    "layer": layer,
                    "index": coordinate.outcome_index,
                });
            }
            let command = coordinate.command.as_deref().unwrap_or("");
            let index = coordinate.outcome_index.unwrap_or(0);
            let cmd = def.commands.get(command);
            let outcome = cmd.and_then(|c| c.outcomes.as_ref().and_then(|o| o.get(index)));
            serde_json::json!({
                "kind": kind,
                "command": command,
                "index": index,
                "when": outcome.and_then(|o| o.when.as_ref()).or(cmd.and_then(|c| c.when.as_ref())),
                "to": outcome.map(|o| o.to.as_str()).or(cmd.and_then(|c| c.to.as_deref())),
                "then": outcome.and_then(|o| o.then.as_ref()).or(cmd.and_then(|c| c.then.as_ref())),
            })
        }
        "OBL-INV-001" => {
            let name = coordinate.invariant.as_deref().unwrap_or("");
            let inv = def.invariants.get(name);
            serde_json::json!({
                "kind": kind,
                "invariant": name,
                "when": inv.and_then(|i| i.when.as_ref()),
                "assert": inv.map(|i| &i.assert),
            })
        }
        "OBL-EFFECT-001" => {
            let command = coordinate.command.as_deref().unwrap_or("");
            serde_json::json!({
                "kind": kind,
                "command": command,
                "effect": def.commands.get(command).and_then(|c| c.effect.as_ref()),
            })
        }
        _ => serde_json::json!({ "kind": kind }),
    }
}

fn outcome_count(cmd: &CommandDef) -> usize {
    cmd.outcomes.as_ref().map(|o| o.len()).unwrap_or(1)
}
