//! Origin of each decision point in a rule's guard: who chose the value or relation, and what
//! the requirement says about it.
//!
//! The author records an origin; Desmo stores and binds it. Desmo does not judge whether a
//! quoted requirement span means what the author says. It checks only necessary conditions: the
//! span is a verbatim part of the requirement text, a constant's literal (or a stated
//! equivalent) appears in it, and an operator is named by a phrase from a small vocabulary,
//! matched by whole words, longest phrase first. An owner's acknowledgment never turns a chosen
//! point into a stated one.
//!
//! Everything not recorded reads `unrecorded`. The enumeration is closed: comparison operators,
//! constants and `text_in` sets and bounds of declarative guards, for rules that have exactly one
//! guard. Existence tests, connectives, reducer code, effects, rules with several guards and
//! absent rules are not enumerated, and the output says so.

use crate::canonical::digest_json;
use crate::declarative::points::{decision_points, operators_named, symbol, vocabulary, Point};
use serde::Deserialize;
use serde_json::{json, Value};

const STATED: &str = "stated";
const CHOSEN: &str = "chosen";
const CHOSEN_ACK: &str = "chosen_acknowledged";
const DEFAULT: &str = "default";
const UNRECORDED: &str = "unrecorded";
const PROVENANCES: [&str; 3] = ["owner-authored", "agent-restated", "unknown"];

const SCOPE: &str = "enumerated: comparison operators, constants and text_in sets and bounds in declarative guards, for rules with one guard; not enumerated: existence tests, and/or connectives, reducer code, effects, rules with several guards, and rules the definition does not have";

/// The requirement text spans are checked against, and where the author says it came from.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Requirement {
    pub text: String,
    pub source: String,
    /// `owner-authored`, `agent-restated` or `unknown` (the default). Set by the author; not verified.
    #[serde(default)]
    pub provenance: String,
    /// The requirement split into verbatim statements. Optional; without it requirement gaps are
    /// not enumerated.
    #[serde(default)]
    pub atoms: Vec<crate::handoff_gaps::Atom>,
    /// Atoms traced to definition elements.
    #[serde(default)]
    pub traces: Vec<crate::handoff_gaps::Trace>,
    /// Atoms that are not behavior, each with a reason.
    #[serde(default)]
    pub not_applicable: Vec<crate::handoff_gaps::NotApplicable>,
}

/// One recorded origin, bound to the guard it was recorded against.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginEntry {
    pub rule: String,
    pub point: String,
    pub guard_hash: String,
    /// `stated`, `chosen` or `default`.
    pub origin: String,
    #[serde(default)]
    pub span: String,
    /// For a constant the requirement states in other units, for example `14 days`.
    #[serde(default)]
    pub equivalent: String,
    #[serde(default)]
    pub default_id: String,
}

/// The owner saw a chosen point and acknowledged it. The agent records this after the owner says so.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginAck {
    pub rule: String,
    pub point: String,
    pub guard_hash: String,
}

pub(crate) struct PointStatus {
    pub key: String,
    pub kind: &'static str,
    pub label: String,
    pub status: &'static str,
    pub detail: String,
}

pub(crate) struct RuleOrigins {
    pub rule: String,
    pub guard_hash: String,
    pub points: Vec<PointStatus>,
}

/// The result of checking every rule under review.
pub(crate) struct Origins {
    pub rules: Vec<RuleOrigins>,
    /// Rules under review whose decision points are not enumerated because they have several guards.
    pub skipped: Vec<String>,
}

struct Records<'a> {
    requirement: Option<&'a Requirement>,
    entries: &'a [OriginEntry],
    acks: &'a [OriginAck],
}

pub(crate) fn guard_of(row: &Value) -> Option<&str> {
    match row["guards"].as_array()?.as_slice() {
        [only] => only.as_str(),
        _ => None,
    }
}

fn under_review(row: &Value) -> bool {
    let status = row["status"].as_str().unwrap_or("");
    status != "unchanged" && status != "removed"
}

/// Statuses for every decision point of every rule under review. All problems are reported together.
pub(crate) fn check(
    rules: &Value,
    requirement: Option<&Requirement>,
    entries: &[OriginEntry],
    acks: &[OriginAck],
) -> Result<Origins, String> {
    let all: Vec<&Value> = rules.as_array().map(|rows| rows.iter().filter(|row| under_review(row)).collect()).unwrap_or_default();
    let records = Records { requirement, entries, acks };
    let mut problems = requirement_problems(requirement);
    problems.extend(unknown_rules(&all, &records));
    let mut out = Origins { rules: Vec::new(), skipped: Vec::new() };
    for row in all {
        if guard_of(row).is_none() {
            out.skipped.push(row["name"].as_str().unwrap_or("").to_string());
            continue;
        }
        match rule_origins(row, &records) {
            Ok(item) => out.rules.push(item),
            Err(mut found) => problems.append(&mut found),
        }
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems.join("\n"))
    }
}

fn requirement_problems(requirement: Option<&Requirement>) -> Vec<String> {
    let Some(found) = requirement else {
        return Vec::new();
    };
    let provenance = found.provenance.as_str();
    if provenance.is_empty() || PROVENANCES.contains(&provenance) {
        return Vec::new();
    }
    vec![format!("requirement.provenance `{provenance}` is not one of {}", PROVENANCES.join(", "))]
}

fn unknown_rules(rows: &[&Value], records: &Records) -> Vec<String> {
    let mut named: Vec<&str> = records.entries.iter().map(|item| item.rule.as_str()).collect();
    named.extend(records.acks.iter().map(|item| item.rule.as_str()));
    named.sort_unstable();
    named.dedup();
    named.into_iter().filter_map(|name| unknown_rule(rows, name)).collect()
}

fn unknown_rule(rows: &[&Value], name: &str) -> Option<String> {
    match rows.iter().find(|row| row["name"].as_str() == Some(name)) {
        None => Some(format!("origin recorded for rule `{name}`, which is not under review in the rule ledger")),
        Some(row) if guard_of(row).is_none() => {
            Some(format!("origin recorded for rule `{name}`, which has several guards; its decision points are not enumerated"))
        }
        Some(_) => None,
    }
}

fn rule_origins(row: &Value, records: &Records) -> Result<RuleOrigins, Vec<String>> {
    let name = row["name"].as_str().unwrap_or("");
    let hash = row["hash"].as_str().unwrap_or("");
    let points = decision_points(guard_of(row).unwrap_or(""))
        .map_err(|message| vec![format!("rule `{name}`: {message}")])?;
    let mut problems = unknown_points(name, hash, &points, records);
    let mut statuses = Vec::new();
    for point in &points {
        match point_status(name, hash, point, records) {
            Ok(status) => statuses.push(status),
            Err(message) => problems.push(message),
        }
    }
    if problems.is_empty() {
        Ok(RuleOrigins { rule: name.to_string(), guard_hash: hash.to_string(), points: statuses })
    } else {
        Err(problems)
    }
}

/// A current entry or acknowledgment naming a point the guard does not have is a typo, not staleness.
fn unknown_points(name: &str, hash: &str, points: &[Point], records: &Records) -> Vec<String> {
    let has = |key: &str| points.iter().any(|point| point.key == key);
    let mut named: Vec<&str> = records
        .entries
        .iter()
        .filter(|item| item.rule == name && item.guard_hash == hash)
        .map(|item| item.point.as_str())
        .collect();
    named.extend(records.acks.iter().filter(|item| item.rule == name && item.guard_hash == hash).map(|item| item.point.as_str()));
    named.sort_unstable();
    named.dedup();
    named
        .into_iter()
        .filter(|key| !has(key))
        .map(|key| format!("rule `{name}`: origin recorded for `{key}`, which is not a decision point of its guard"))
        .collect()
}

fn point_status(name: &str, hash: &str, point: &Point, records: &Records) -> Result<PointStatus, String> {
    let context = format!("rule `{name}`, point `{}`", point.key);
    let for_point: Vec<&OriginEntry> = records.entries.iter().filter(|item| item.rule == name && item.point == point.key).collect();
    let current: Vec<&&OriginEntry> = for_point.iter().filter(|item| item.guard_hash == hash).collect();
    let acknowledged = records.acks.iter().any(|ack| ack.rule == name && ack.point == point.key && ack.guard_hash == hash);
    let entry = match current.as_slice() {
        [] if acknowledged => return Err(format!("{context}: only a chosen point can be acknowledged; no current origin is recorded for it")),
        [] if for_point.is_empty() => return Ok(status(point, UNRECORDED, "no origin recorded".to_string())),
        [] => return Ok(status(point, UNRECORDED, "stale: the guard changed since this origin was recorded".to_string())),
        [only] => **only,
        _ => return Err(format!("{context}: more than one current origin is recorded; keep one")),
    };
    if acknowledged && entry.origin != CHOSEN {
        return Err(format!("{context}: only a chosen point can be acknowledged"));
    }
    origin_status(point, entry, records.requirement, acknowledged).map_err(|message| format!("{context}: {message}"))
}

fn status(point: &Point, status: &'static str, detail: String) -> PointStatus {
    PointStatus { key: point.key.clone(), kind: point.kind, label: point.label.clone(), status, detail }
}

fn origin_status(
    point: &Point,
    entry: &OriginEntry,
    requirement: Option<&Requirement>,
    acknowledged: bool,
) -> Result<PointStatus, String> {
    match entry.origin.as_str() {
        STATED => stated(point, entry, requirement),
        CHOSEN if acknowledged => Ok(status(point, CHOSEN_ACK, "the author's choice; the owner has seen it".to_string())),
        CHOSEN => Ok(status(point, CHOSEN, "the author's choice; no requirement text is cited for it".to_string())),
        DEFAULT if !entry.default_id.is_empty() => Ok(status(point, DEFAULT, entry.default_id.clone())),
        DEFAULT => Err("a default needs default_id, the identity and version of the default".to_string()),
        other => Err(format!("origin `{other}` is not one of stated, chosen, default")),
    }
}

fn stated(point: &Point, entry: &OriginEntry, requirement: Option<&Requirement>) -> Result<PointStatus, String> {
    let Some(found) = requirement else {
        return Err("`stated` needs a `requirement` {text, source} to quote".to_string());
    };
    if let Some(problem) = span_problem(point, entry, found) {
        return Err(problem);
    }
    let provenance = if found.provenance.is_empty() { "unknown" } else { found.provenance.as_str() };
    let mut detail = format!("{}: \"{}\" (requirement is {provenance}, not verified)", found.source, entry.span);
    if !entry.equivalent.is_empty() {
        detail.push_str(&format!("; the span's `{}` is asserted equal to {}", entry.equivalent, point.label));
    }
    Ok(status(point, STATED, detail))
}

/// Necessary conditions only. A span that passes is not shown to mean what the author says.
fn span_problem(point: &Point, entry: &OriginEntry, requirement: &Requirement) -> Option<String> {
    if entry.span.trim().is_empty() {
        return Some("`stated` needs a span quoted from the requirement".to_string());
    }
    if !requirement.text.contains(&entry.span) {
        return Some("the span is not a verbatim part of the requirement text".to_string());
    }
    if !entry.equivalent.is_empty() && !entry.span.contains(&entry.equivalent) {
        return Some(format!("the span does not contain the stated equivalent `{}`", entry.equivalent));
    }
    match (point.operator, &point.literal) {
        (Some(op), _) => operator_problem(op, &entry.span),
        (None, Some(literal)) if entry.equivalent.is_empty() && !entry.span.contains(literal.as_str()) => {
            Some(format!("the span does not contain `{literal}` (or give `equivalent` for a value stated in other units)"))
        }
        _ => None,
    }
}

fn operator_problem(op: crate::declarative::expr::CmpOp, span: &str) -> Option<String> {
    if operators_named(span).contains(&op) {
        return None;
    }
    Some(format!("the span names none of the phrases for `{}`: {}", symbol(op), vocabulary(op).join(", ")))
}

fn word(status: &str) -> &'static str {
    match status {
        STATED => "stated",
        CHOSEN_ACK => "chosen by the author, acknowledged by the owner",
        CHOSEN => "chosen by the author, not acknowledged",
        DEFAULT => "default",
        _ => "unrecorded",
    }
}

fn counts(rules: &[RuleOrigins]) -> Value {
    let all = || rules.iter().flat_map(|rule| rule.points.iter());
    let n = |wanted: &str| all().filter(|point| point.status == wanted).count();
    json!({
        "total": all().count(),
        "stated": n(STATED),
        "chosen_acknowledged": n(CHOSEN_ACK),
        "chosen": n(CHOSEN),
        "default": n(DEFAULT),
        "unrecorded": n(UNRECORDED),
    })
}

fn scope_text(skipped: &[String]) -> String {
    if skipped.is_empty() {
        return SCOPE.to_string();
    }
    let names = skipped.iter().map(|name| format!("`{name}`")).collect::<Vec<_>>().join(", ");
    format!("{SCOPE}; skipped here because they have several guards: {names}")
}

/// The document's `origins` value.
pub(crate) fn to_value(origins: &Origins, requirement: Option<&Requirement>) -> Value {
    let rules: Vec<Value> = origins
        .rules
        .iter()
        .map(|rule| {
            let points: Vec<Value> = rule
                .points
                .iter()
                .map(|p| json!({"point": p.key, "kind": p.kind, "label": p.label, "status": p.status, "origin": word(p.status), "detail": p.detail}))
                .collect();
            json!({"rule": rule.rule, "guard_hash": rule.guard_hash, "points": points})
        })
        .collect();
    let pinned = requirement.map(|found| {
        let provenance = if found.provenance.is_empty() { "unknown" } else { found.provenance.as_str() };
        json!({"source": found.source, "provenance": provenance, "text_digest": digest_json(&found.text)})
    });
    json!({
        "scope": scope_text(&origins.skipped),
        "requirement": pinned,
        "counts": counts(&origins.rules),
        "skipped": origins.skipped,
        "rules": rules,
    })
}
