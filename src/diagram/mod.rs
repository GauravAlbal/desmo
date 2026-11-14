//! Lifecycle diagrams. Source of truth is `ik.diagram.v1`; the route map, optional mermaid,
//! and table are projections. The diagram never invents a fact the table omits.

mod analysis;
mod build;
mod cli;
mod layout;
mod mdtable;
mod mermaid;
mod pretty;
mod rails;
mod regions;
mod risk;
mod route;
mod rules;
mod table;

use crate::declarative::Definition;
use serde_json::{json, Value};
use std::path::PathBuf;

pub use analysis::{analyze_change, analyze_def, classify_guard, Analytics, GuardKind};
pub use cli::run;
pub use layout::{command_counts, layout_chart, layout_graph, DrawnEdge, RiverLayout};
pub use mdtable::{escape_cell, md_table, wrap_text, MAX_LINE};
pub use mermaid::structurally_ok;

const MAX_STATES: usize = 12;
const MAX_LEVEL0: usize = 8;
const MAX_SPURS: usize = 4;
const MAX_FACTS: usize = 5;
const MAX_LEGEND: usize = 5;
const MAX_LEGEND_LINES: usize = 2;
const LABEL_CAP: usize = 48;

/// Which journey the diagram is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagramKind {
    Delivery,
    Change,
    Recovery,
}

/// Output encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagramFormat {
    Markdown,
    Text,
    Mermaid,
    Table,
    Json,
}

/// Evidence class for a stroke. New/removed are extra flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StrokeClass {
    Declared,
    Explored,
    Observed,
    Refused,
}

/// One emphasized glyph class (Von Restorff). Exactly one per diagram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emphasis {
    YouAreHere,
    New,
    Stuck,
}

/// One station on the route map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Station {
    pub id: String,
    pub initial: bool,
    pub terminal: bool,
    pub distance: Option<u32>,
    pub is_new: bool,
    pub is_removed: bool,
    pub stuck: bool,
    pub trap: bool,
    pub trap_loops_only: bool,
    pub unreachable: bool,
    pub collapsed: bool,
    pub collapsed_names: Vec<String>,
}

/// One hop or refused stub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hop {
    pub from: String,
    pub to: String,
    pub command: String,
    pub stroke: StrokeClass,
    pub step: Option<u32>,
    pub guarded: bool,
    pub effect: bool,
    pub is_new: bool,
    pub is_removed: bool,
    pub guard_kind: GuardKind,
    pub stub: bool,
    pub refused_reason: Option<String>,
    pub main_line: bool,
    pub guard_text: String,
    pub arg_names: Vec<String>,
}

/// One table row. Every hop has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRow {
    pub command: String,
    pub from: String,
    pub to: String,
    pub guard: String,
    pub effect: String,
    pub class: String,
}

/// Cohort heat-table cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CohortRow {
    pub states: Vec<String>,
    pub existing: String,
    pub new_instances: String,
}

/// Change-kind projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeView {
    pub prior_version: u64,
    pub new_version: u64,
    pub added_commands: Vec<String>,
    pub removed_commands: Vec<String>,
    pub added_states: Vec<String>,
    pub removed_states: Vec<String>,
    pub guards_changed: Vec<String>,
    pub reachability_line: String,
    pub old_every_reaches_end: bool,
    pub new_dead_ends: Vec<String>,
    pub finish_changed: Vec<String>,
    pub cohorts: Vec<CohortRow>,
    pub continuity_note: String,
    pub risk: Vec<RiskRow>,
    pub no_change: Vec<String>,
    pub divergence: Vec<String>,
    pub observed_note: String,
    pub live_instances: String,
    pub callers: String,
    pub rule_effect: String,
    pub compat: crate::declarative::compat::Compat,
    pub machine_id: String,
}

/// One risk-ledger row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiskRow {
    pub target: String,
    pub impact: String,
    pub basis: String,
    pub status: String,
}

/// One authored region lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionLane {
    pub name: String,
    pub parent_name: Option<String>,
    pub children: Vec<String>,
    pub states: Vec<String>,
    pub terminals: Vec<String>,
    pub hops: Vec<RegionHop>,
    pub parent_hops: Vec<RegionHop>,
}

/// A hop inside one region, not a product edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionHop {
    pub from: String,
    pub to: String,
    pub command: String,
    pub guard: String,
    pub stroke: StrokeClass,
    pub cross_reads: Vec<String>,
}

/// Joint command that changes more than one region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JointRail {
    pub command: String,
    pub changes: Vec<String>,
    pub guard: String,
    /// Per-region authored from → to (`any` when the leaf is a wildcard).
    pub legs: Vec<(String, String, String)>,
}

/// Authored region projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionView {
    pub lanes: Vec<RegionLane>,
    pub joints: Vec<JointRail>,
}

/// One recovery event the evidence supports. Nothing is listed that the evidence lacks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineStep {
    pub id: &'static str,
    pub label: String,
    /// `observed` (seen directly) or `found by lookup` (recovered by operation id).
    pub how: &'static str,
    pub note: String,
}

/// Recovery timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryView {
    pub timeline: Vec<TimelineStep>,
    pub command: String,
    pub you_are_here: Option<String>,
    pub fork: Vec<(String, String)>,
    pub subject: String,
    pub operation_id: Option<String>,
    pub resolved_outcome: Option<String>,
    pub resolved_how: Option<String>,
}

/// Source-of-truth model (`ik.diagram.v1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagram {
    pub kind: DiagramKind,
    pub level: u8,
    pub ascii: bool,
    pub facts_line: String,
    pub facts_chunks: Vec<String>,
    pub stations: Vec<Station>,
    pub hops: Vec<Hop>,
    pub you_are_here: Option<String>,
    pub next: Vec<String>,
    pub loops: Vec<String>,
    pub stuck: Vec<String>,
    pub unreachable: Vec<String>,
    pub shortest_from_here: Option<u32>,
    pub rows: Vec<TableRow>,
    pub legend: Vec<&'static str>,
    pub emphasis: Emphasis,
    pub change: Option<ChangeView>,
    pub recovery: Option<RecoveryView>,
    pub regions: Option<RegionView>,
    pub focus: Option<String>,
    pub safe_next: Option<String>,
    pub rules: Vec<RuleEntry>,
}

/// One rule in the ledger: a guarded command. `hash` binds an acceptance to the exact
/// guard text, so changing the guard makes the acceptance stale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleEntry {
    pub name: String,
    pub moves: String,
    pub rule: String,
    pub verbatim: String,
    /// The raw guard texts, one per distinct guard of the command.
    pub guards: Vec<String>,
    pub hash: String,
    /// `authored` (delivery), or against the prior version: `new`, `changed`,
    /// `removed` (guard dropped) or `unchanged`.
    pub status: &'static str,
}

impl DiagramKind {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "delivery" => Ok(Self::Delivery),
            "change" => Ok(Self::Change),
            "recovery" => Ok(Self::Recovery),
            other => Err(format!(
                "--kind must be one of delivery, change, recovery (got `{other}`)"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Delivery => "delivery",
            Self::Change => "change",
            Self::Recovery => "recovery",
        }
    }
}

impl DiagramFormat {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "markdown" => Ok(Self::Markdown),
            "text" => Ok(Self::Text),
            "mermaid" => Ok(Self::Mermaid),
            "table" => Ok(Self::Table),
            "json" => Ok(Self::Json),
            other => Err(format!(
                "--format must be one of markdown, text, mermaid, table, json (got `{other}`)"
            )),
        }
    }
}

impl Diagram {
    /// Monospace route map (primary projection).
    pub fn to_route_map(&self) -> String {
        route::render(self)
    }

    /// Facts strip + route map + legend. The compressed view for handoff and blind tests.
    pub fn to_compressed_view(&self) -> String {
        route::compressed(self)
    }

    /// Secondary mermaid flowchart. Labels carry the encoding without styling.
    pub fn to_mermaid(&self) -> String {
        mermaid::render(self)
    }

    /// Adjacent plain-text table. The diagram is a compression of this table.
    pub fn to_table_markdown(&self) -> String {
        table::render(self)
    }

    /// Default human form: facts, route map, table. Mermaid is `--format mermaid` only.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str(&self.to_compressed_view());
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&self.to_table_markdown());
        out
    }

    /// Drawn edge list (command, source, target) for conservation checks.
    pub fn drawn_edge_list(&self) -> Vec<layout::DrawnEdge> {
        if let Some(view) = &self.regions {
            let mut drawn = Vec::new();
            for lane in &view.lanes {
                for hop in lane.hops.iter().chain(lane.parent_hops.iter()) {
                    drawn.push(layout::DrawnEdge {
                        command: hop.command.clone(),
                        from: hop.from.clone(),
                        to: hop.to.clone(),
                    });
                }
            }
            for joint in &view.joints {
                drawn.push(layout::DrawnEdge {
                    command: joint.command.clone(),
                    from: "*".into(),
                    to: "*".into(),
                });
            }
            drawn.sort();
            drawn.dedup();
            return drawn;
        }
        let mut drawn: Vec<layout::DrawnEdge> = self
            .hops
            .iter()
            .map(|hop| layout::DrawnEdge {
                command: hop.command.clone(),
                from: hop.from.clone(),
                to: hop.to.clone(),
            })
            .collect();
        drawn.sort();
        drawn.dedup();
        drawn
    }

    /// Additive JSON field for `ik handoff --json` (`ik.diagram.v1`).
    pub fn json_field(&self) -> Value {
        json!({
            "schema": "ik.diagram.v1",
            "kind": self.kind.as_str(),
            "level": self.level,
            "facts": {
                "line": self.facts_line,
                "chunks": self.facts_chunks,
                "basis": "structural",
            },
            "you_are_here": self.you_are_here,
            "next": self.next,
            "loops": self.loops,
            "stuck": self.stuck,
            "unreachable": self.unreachable,
            "emphasis": match self.emphasis {
                Emphasis::YouAreHere => "you_are_here",
                Emphasis::New => "new",
                Emphasis::Stuck => "stuck",
            },
            "legend": self.legend,
            "route_map": self.to_route_map(),
            "compressed": self.to_compressed_view(),
            "mermaid": self.to_mermaid(),
            "table": self.rows.iter().map(|row| json!({
                "command": row.command,
                "from": row.from,
                "to": row.to,
                "guard": row.guard,
                "effect": row.effect,
                "class": row.class,
            })).collect::<Vec<_>>(),
            "change": self.change.as_ref().map(|view| json!({
                "machine_id": view.machine_id,
                "prior_version": view.prior_version,
                "new_version": view.new_version,
                "reachability": view.reachability_line,
                "added_commands": view.added_commands,
                "changed_guards": view.guards_changed,
                "removed_commands": view.removed_commands,
                "cohorts": view.cohorts.iter().map(|row| json!({
                    "states": row.states,
                    "existing": row.existing,
                    "new_instances": row.new_instances,
                })).collect::<Vec<_>>(),
                "continuity": view.continuity_note,
                "compat": compat_json(&view.compat),
                "diff_markers": ["=", "+", "~", "-", "!"],
                "risk": view.risk.iter().map(|row| json!({
                    "target": row.target,
                    "impact": row.impact,
                    "basis": row.basis,
                    "status": row.status,
                })).collect::<Vec<_>>(),
            })),
            "recovery": self.recovery.as_ref().map(|view| json!({
                "timeline": view.timeline.iter().map(|step| json!({
                    "id": step.id,
                    "label": step.label,
                    "how": step.how,
                    "note": step.note,
                })).collect::<Vec<_>>(),
                "command": view.command,
                "you_are_here": view.you_are_here,
                "fork": view.fork.iter().map(|(k, v)| json!({ "outcome": k, "resolution": v })).collect::<Vec<_>>(),
                "subject": view.subject,
                "operation_id": view.operation_id,
                "resolved_outcome": view.resolved_outcome,
                "resolved_how": view.resolved_how,
            })),
            "drawn_edges": self.drawn_edge_list().iter().map(|e| json!({
                "command": e.command,
                "from": e.from,
                "to": e.to,
            })).collect::<Vec<_>>(),
            "safe_next": self.safe_next,
            "rules": self.rules.iter().map(|rule| json!({
                "name": rule.name,
                "moves": rule.moves,
                "rule": rule.rule,
                "verbatim": rule.verbatim,
                "guards": rule.guards,
                "hash": rule.hash,
                "status": rule.status,
            })).collect::<Vec<_>>(),
            "region_lanes": self.regions.as_ref().map(|view| view.lanes.iter().map(|lane| json!({
                "name": lane.name,
                "parent": lane.parent_name,
                "children": lane.children,
                "states": lane.states,
            })).collect::<Vec<_>>()),
            "table_markdown": self.to_table_markdown(),
            "markdown": self.to_markdown(),
        })
    }
}

fn compat_json(compat: &crate::declarative::compat::Compat) -> Value {
    json!({
        "states": compat.states.iter().map(|item| json!({
            "state": item.state,
            "status": if item.reasons.is_empty() { "no difference found" } else { "review" },
            "exits": item.exits.iter().map(|exit| json!({
                "command": exit.command,
                "verdict": exit.verdict,
            })).collect::<Vec<_>>(),
            "gained": item.gained,
            "reasons": item.reasons,
        })).collect::<Vec<_>>(),
        "global": compat.global,
        "not_established": compat.not_established,
    })
}

/// Operation ids named in evidence (`attempted` JSON or `operation_id`).
pub fn distinct_operation_ids(findings: &[Value]) -> Vec<String> {
    build::distinct_operation_ids(findings)
}

/// Build a diagram from a definition, optional prior, and folded report findings.
pub fn build(
    def: Option<&Definition>,
    prior: Option<&Definition>,
    findings: &[Value],
    kind: DiagramKind,
    subject: &str,
    operation_id: Option<&str>,
) -> Diagram {
    build_with(
        def,
        prior,
        findings,
        kind,
        subject,
        operation_id,
        1,
        None,
        false,
        None,
    )
}

pub fn build_with(
    def: Option<&Definition>,
    prior: Option<&Definition>,
    findings: &[Value],
    kind: DiagramKind,
    subject: &str,
    operation_id: Option<&str>,
    level: u8,
    focus: Option<&str>,
    ascii: bool,
    sidecar: Option<&std::path::Path>,
) -> Diagram {
    build::build(
        def,
        prior,
        findings,
        kind,
        subject,
        operation_id,
        level,
        focus,
        ascii,
        sidecar,
    )
}

/// Discover `machine.ik.toml` / `prior_machine.ik.toml` in scenario bundles.
pub fn machines_in_evidence(paths: &[PathBuf]) -> (Option<PathBuf>, Option<PathBuf>) {
    let mut machine = None;
    let mut prior = None;
    for path in paths {
        let dir = if path.is_dir() {
            path.clone()
        } else {
            match path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
                _ => continue,
            }
        };
        let candidate = dir.join("machine.ik.toml");
        if machine.is_none() && candidate.is_file() {
            machine = Some(candidate);
        }
        let prior_candidate = dir.join("prior_machine.ik.toml");
        if prior.is_none() && prior_candidate.is_file() {
            prior = Some(prior_candidate);
        }
    }
    (machine, prior)
}

/// Handoff embedding: omit the diagram when no definition is available (except recovery).
pub fn from_handoff(kind: &str, evidence: &[PathBuf], findings: &[Value]) -> Option<Diagram> {
    let parsed = DiagramKind::parse(kind).ok()?;
    let (machine_path, prior_path) = machines_in_evidence(evidence);
    let def = machine_path
        .as_ref()
        .and_then(|path| crate::declarative::load_definition(path).ok());
    let prior = prior_path
        .as_ref()
        .and_then(|path| crate::declarative::load_definition(path).ok());
    if parsed != DiagramKind::Recovery && def.is_none() {
        return None;
    }
    let subject = subject_from_findings(findings).unwrap_or("");
    let named = distinct_operation_ids(findings);
    let operation = operation_from_findings(findings).or_else(|| {
        if named.len() == 1 {
            Some(named[0].clone())
        } else {
            None
        }
    });
    Some(build(
        def.as_ref(),
        prior.as_ref(),
        findings,
        parsed,
        subject,
        operation.as_deref(),
    ))
}

fn subject_from_findings(findings: &[Value]) -> Option<&str> {
    findings
        .iter()
        .find_map(|finding| finding["id"].as_str().and_then(aggregate_from_id))
}

fn aggregate_from_id(id: &str) -> Option<&str> {
    let rest = id.rsplit_once('#')?.1;
    let (aggregate, step) = rest.split_once('/')?;
    if aggregate.is_empty() || step.is_empty() {
        None
    } else {
        Some(aggregate)
    }
}

fn operation_from_findings(findings: &[Value]) -> Option<String> {
    findings.iter().find_map(|finding| {
        let attempted = finding["attempted"].as_str()?;
        let value: Value = serde_json::from_str(attempted).ok()?;
        value
            .get("operation_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    })
}

fn class_label(stroke: StrokeClass, is_new: bool, is_removed: bool) -> String {
    let mut parts = Vec::new();
    if is_removed {
        parts.push("removed");
    }
    if is_new {
        parts.push("new");
    }
    parts.push(match stroke {
        StrokeClass::Declared => "declared",
        StrokeClass::Explored => "explored",
        StrokeClass::Observed => "observed",
        StrokeClass::Refused => "refused",
    });
    parts.join(" ")
}

fn display_label(text: &str) -> String {
    if text.chars().count() <= LABEL_CAP {
        return text.to_string();
    }
    let keep = LABEL_CAP.saturating_sub(3);
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= keep {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn mermaid_id(name: &str) -> String {
    let mut out = String::from("s_");
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out == "s_" {
        out.push_str("node");
    }
    out
}

/// Display width treating East-Asian and emoji as two columns; box glyphs as one.
pub fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

fn char_width(ch: char) -> usize {
    let u = ch as u32;
    if ch == '\n' || ch == '\r' {
        0
    } else if ch.is_ascii() {
        1
    } else if (0x1100..=0x115F).contains(&u)
        || (0x2E80..=0xA4CF).contains(&u)
        || (0xAC00..=0xD7A3).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0xFE10..=0xFE19).contains(&u)
        || (0xFE30..=0xFE6F).contains(&u)
        || (0xFF00..=0xFF60).contains(&u)
        || (0xFFE0..=0xFFE6).contains(&u)
        || (0x1F300..=0x1FAFF).contains(&u)
    {
        2
    } else {
        1
    }
}
