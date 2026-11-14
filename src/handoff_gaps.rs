//! What is missing, stated actively so a reader need not infer it.
//!
//! Three closed-world lists, each over a stated scope:
//! - gaps in the definition itself (derived from the definition alone);
//! - gaps in the evidence (commands with no cited committed run, or no cited refused run);
//! - gaps against the requirement, only when the author supplied it split into atoms: atoms no
//!   definition element is traced to, sentences of the requirement no atom covers, and commands
//!   no atom is traced to.
//!
//! None of these is a completeness claim. A requirement statement nobody wrote down, a state
//! that can be occupied forever, and a rule neither document mentions are not enumerable and the
//! scope sentence says so. Atomization is the author's: Desmo checks that spans are verbatim,
//! that traces name real elements, and that no sentence is silently left out of the atoms.

use crate::declarative::gaps::{definition_gaps, Gap};
use crate::declarative::points::decision_points;
use crate::declarative::Definition;
use crate::handoff_origin::{OriginEntry, Origins, Requirement};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const SCOPE: &str = "enumerated: gaps derived from a flat declarative definition; commands with no cited committed run and guarded commands with no cited guard refusal, counting only runs observed against this definition; and, when the requirement is given as atoms, atoms with no traced element, sentences an atom does not fully cover, and commands no atom is traced to. not enumerated: requirement statements nobody wrote down, rules neither document mentions, whether a state can be occupied forever, reducer code and effects. A short list does not mean the definition is good";

/// A statement of the requirement, quoted verbatim.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Atom {
    pub id: String,
    pub span: String,
}

/// An atom traced to a definition element: `command:NAME`, `state:NAME`, `fact:NAME`,
/// `invariant:NAME` or `point:RULE/KEY`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Trace {
    pub atom: String,
    pub element: String,
}

/// An atom that is not behavior (background, a definition, a non-goal), with the reason.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NotApplicable {
    pub atom: String,
    pub reason: String,
}

/// Compute the `gaps` value of a handoff. `Ok(Null)` for a recovery handoff, which has no
/// definition to read.
pub(crate) fn check(
    kind: &str,
    evidence: &[PathBuf],
    findings: &[Value],
    rules: &Value,
    requirement: Option<&Requirement>,
    entries: &[OriginEntry],
    origins: &Origins,
) -> Result<Value, String> {
    if kind == "recovery" {
        return Ok(Value::Null);
    }
    let (machine, prior) = crate::diagram::machines_in_evidence(evidence);
    let load = |path: Option<PathBuf>| path.and_then(|found| crate::declarative::load_definition(&found).ok());
    let Some(def) = load(machine) else {
        return Ok(unavailable("no declarative definition was found in the evidence; definition gaps are not enumerated"));
    };
    if def.chart.hierarchical {
        return Ok(unavailable("a hierarchical machine is not analysed here: its flat lowering holds state-encoding guards the author never wrote"));
    }
    let prior = load(prior).filter(|old| !old.chart.hierarchical);
    let trace = trace_section(&def, prior.as_ref(), rules, requirement, entries, origins)?;
    Ok(assemble(&def, prior.as_ref(), findings, trace))
}

fn unavailable(note: &str) -> Value {
    json!({"scope": SCOPE, "available": false, "note": note})
}

fn gap_value(item: &Gap, is_new: bool) -> Value {
    json!({"kind": item.kind, "subject": item.subject, "detail": item.detail, "material": item.material, "new": is_new})
}

fn assemble(def: &Definition, prior: Option<&Definition>, findings: &[Value], trace: Value) -> Value {
    let before: BTreeSet<(&str, String)> = prior
        .map(|old| definition_gaps(old).into_iter().map(|item| (item.kind, item.subject)).collect())
        .unwrap_or_default();
    let definition: Vec<Value> = definition_gaps(def)
        .iter()
        .map(|item| gap_value(item, prior.is_some() && !before.contains(&(item.kind, item.subject.clone()))))
        .collect();
    let evidence = evidence_gaps(def, findings);
    let material = definition.iter().filter(|item| item["material"] == true).count();
    let unmapped = trace["unmapped"].as_array().map_or(0, Vec::len);
    let uncovered = trace["uncovered_sentences"].as_array().map_or(0, Vec::len);
    let untraced = trace["untraced_commands"].as_array().map_or(0, Vec::len);
    json!({
        "scope": SCOPE,
        "available": true,
        "definition": definition,
        "evidence": evidence,
        "requirement": trace,
        "counts": {
            "definition": definition.len(),
            "material": material,
            "evidence": evidence.len(),
            "unmapped_atoms": unmapped,
            "uncovered_sentences": uncovered,
            "untraced_commands": untraced,
        },
    })
}

/// The command a run finding attempted, when its text starts with one of the definition's commands.
fn attempted_command<'a>(finding: &'a Value, def: &Definition) -> Option<&'a str> {
    let first = finding["attempted"].as_str()?.split_whitespace().next()?;
    def.commands.contains_key(first).then_some(first)
}

/// The refusal code a scenario finding carries in its scope (`refused IK_...`).
fn refusal_code(finding: &Value) -> Option<&str> {
    let scope = finding["scope"].as_str()?;
    let rest = scope.split("refused ").nth(1)?;
    rest.split(|ch: char| !(ch.is_ascii_uppercase() || ch == '_')).next().filter(|code| code.starts_with("IK_"))
}

/// A command with a guard refuses a bad input with this code. A refusal for another reason (a
/// stale handle, a state that does not allow the command, a version mismatch) does not exercise it.
const GUARD_REFUSAL: &str = "IK_INVARIANT_FAILED";

/// Evidence gaps. Only runs observed against this definition count: continuity observations ran
/// against the prior version, so they say nothing about this definition's commands.
fn evidence_gaps(def: &Definition, findings: &[Value]) -> Vec<Value> {
    let mut committed = BTreeSet::new();
    let mut guard_refused = BTreeSet::new();
    let mut any = false;
    for finding in findings.iter().filter(|finding| finding["evidence_class"] == "observed_trace") {
        let Some(command) = attempted_command(finding, def) else {
            continue;
        };
        any = true;
        match finding["status"].as_str() {
            Some("committed") => committed.insert(command),
            Some("refused") if refusal_code(finding) == Some(GUARD_REFUSAL) => guard_refused.insert(command),
            _ => false,
        };
    }
    let row = |kind: &str, subject: &str, detail: &str| json!({"kind": kind, "subject": subject, "detail": detail});
    if !any {
        return vec![row(
            "no_run_evidence",
            "machine",
            "no cited run of this definition names a command (continuity observations ran against the prior version and are not counted)",
        )];
    }
    let mut out = Vec::new();
    for (name, cmd) in &def.commands {
        if !committed.contains(name.as_str()) {
            out.push(row("no_committed_run_cited", name, "no committed run of it against this definition is cited"));
        }
        let guarded = cmd.when.is_some() || cmd.outcomes.iter().flatten().any(|outcome| outcome.when.is_some());
        if guarded && !guard_refused.contains(name.as_str()) {
            out.push(row("no_guard_refusal_cited", name, "no run of it refused with IK_INVARIANT_FAILED is cited, so what its guard refuses is not exercised"));
        }
    }
    out
}

fn not_atomized(note: &str) -> Value {
    json!({"atomized": false, "note": note, "unmapped": [], "uncovered_sentences": [], "untraced_commands": []})
}

fn trace_section(
    def: &Definition,
    prior: Option<&Definition>,
    rules: &Value,
    requirement: Option<&Requirement>,
    entries: &[OriginEntry],
    origins: &Origins,
) -> Result<Value, String> {
    let Some(found) = requirement else {
        return Ok(not_atomized("no requirement was supplied, so statements it makes with no definition element are not enumerated"));
    };
    if found.atoms.is_empty() {
        return Ok(not_atomized("the requirement was not given as atoms, so statements it makes with no definition element are not enumerated"));
    }
    let problems = trace_problems(def, rules, found);
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    Ok(traced(def, prior, found, &implicit_traces(found, entries, origins)))
}

fn trace_problems(def: &Definition, rules: &Value, found: &Requirement) -> Vec<String> {
    let mut out = Vec::new();
    let mut ids = BTreeSet::new();
    for atom in &found.atoms {
        if atom.id.trim().is_empty() {
            out.push("an atom needs a non-empty id".to_string());
        } else if !ids.insert(atom.id.as_str()) {
            out.push(format!("atom `{}` appears more than once", atom.id));
        }
        out.extend(span_problem(&found.text, atom));
    }
    let mut applicable = BTreeSet::new();
    for item in &found.not_applicable {
        if !ids.contains(item.atom.as_str()) {
            out.push(format!("not_applicable names atom `{}`, which is not listed", item.atom));
        } else if item.reason.trim().is_empty() {
            out.push(format!("atom `{}` is marked not applicable with no reason", item.atom));
        }
        applicable.insert(item.atom.as_str());
    }
    for item in &found.traces {
        if !ids.contains(item.atom.as_str()) {
            out.push(format!("a trace names atom `{}`, which is not listed", item.atom));
        }
        if applicable.contains(item.atom.as_str()) {
            out.push(format!("atom `{}` is both traced and marked not applicable", item.atom));
        }
        if let Some(problem) = element_problem(def, rules, &item.element) {
            out.push(format!("trace {} -> `{}`: {problem}", item.atom, item.element));
        }
    }
    out
}

fn span_problem(text: &str, atom: &Atom) -> Option<String> {
    if atom.span.trim().is_empty() {
        return Some(format!("atom `{}` needs a span", atom.id));
    }
    match text.matches(atom.span.as_str()).count() {
        0 => Some(format!("atom `{}`: the span is not a verbatim part of the requirement text", atom.id)),
        1 => None,
        count => Some(format!("atom `{}`: the span occurs {count} times in the requirement text; quote enough to be unique", atom.id)),
    }
}

fn element_problem(def: &Definition, rules: &Value, element: &str) -> Option<String> {
    let Some((kind, name)) = element.split_once(':') else {
        return Some("an element is `command:NAME`, `state:NAME`, `fact:NAME`, `invariant:NAME` or `point:RULE/KEY`".to_string());
    };
    let known = match kind {
        "command" => def.commands.contains_key(name),
        "state" => def.states.contains_key(name),
        "fact" => def.facts.contains_key(name),
        "invariant" => def.invariants.contains_key(name),
        "point" => return point_problem(rules, name),
        _ => return Some(format!("`{kind}` is not an element kind")),
    };
    (!known).then(|| format!("the definition has no {kind} `{name}`"))
}

fn point_problem(rules: &Value, name: &str) -> Option<String> {
    let Some((rule, key)) = name.split_once('/') else {
        return Some("a point is `point:RULE/KEY`".to_string());
    };
    let row = rules.as_array()?.iter().find(|row| row["name"].as_str() == Some(rule));
    let Some(guard) = row.and_then(crate::handoff_origin::guard_of) else {
        return Some(format!("rule `{rule}` is not a single-guard rule under review"));
    };
    match decision_points(guard) {
        Ok(points) if points.iter().any(|point| point.key == key) => None,
        Ok(_) => Some(format!("`{key}` is not a decision point of rule `{rule}`")),
        Err(message) => Some(message),
    }
}

/// A stated origin whose span lies inside an atom traces that atom to the rule's command and point.
fn implicit_traces(found: &Requirement, entries: &[OriginEntry], origins: &Origins) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for rule in &origins.rules {
        for point in rule.points.iter().filter(|point| point.status == "stated") {
            let entry = entries.iter().find(|item| item.rule == rule.rule && item.point == point.key && item.guard_hash == rule.guard_hash);
            let Some(entry) = entry else { continue };
            let Some(at) = unique_position(&found.text, &entry.span) else { continue };
            for atom in found.atoms.iter().filter(|atom| unique_position(&found.text, &atom.span).is_some_and(|(a, b)| a <= at.0 && at.1 <= b)) {
                out.push((atom.id.clone(), format!("command:{}", rule.rule)));
                out.push((atom.id.clone(), format!("point:{}/{}", rule.rule, point.key)));
            }
        }
    }
    out
}

/// Byte range of `span` in `text` when it occurs exactly once; an ambiguous quote locates nothing.
fn unique_position(text: &str, span: &str) -> Option<(usize, usize)> {
    if span.is_empty() || text.matches(span).count() != 1 {
        return None;
    }
    text.find(span).map(|start| (start, start + span.len()))
}

/// Commands a requirement is expected to speak to: all of them for a delivery, and for a change
/// only those that are new or differ from the prior definition.
fn relevant_commands<'a>(def: &'a Definition, prior: Option<&Definition>) -> Vec<&'a String> {
    def.commands
        .iter()
        .filter(|(name, cmd)| {
            prior.is_none_or(|old| old.commands.get(name.as_str()).is_none_or(|before| serde_json::to_value(before).ok() != serde_json::to_value(cmd).ok()))
        })
        .map(|(name, _)| name)
        .collect()
}

fn traced(def: &Definition, prior: Option<&Definition>, found: &Requirement, implicit: &[(String, String)]) -> Value {
    let mut by_atom: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for item in &found.traces {
        by_atom.entry(item.atom.as_str()).or_default().insert(item.element.as_str());
    }
    for (atom, element) in implicit {
        by_atom.entry(atom.as_str()).or_default().insert(element.as_str());
    }
    let reasons: BTreeMap<&str, &str> = found.not_applicable.iter().map(|item| (item.atom.as_str(), item.reason.as_str())).collect();
    let unmapped: Vec<Value> = found
        .atoms
        .iter()
        .filter(|atom| !by_atom.contains_key(atom.id.as_str()) && !reasons.contains_key(atom.id.as_str()))
        .map(|atom| json!({"id": atom.id, "span": atom.span}))
        .collect();
    let dismissed: Vec<Value> = found
        .atoms
        .iter()
        .filter_map(|atom| reasons.get(atom.id.as_str()).map(|reason| json!({"id": atom.id, "span": atom.span, "reason": reason})))
        .collect();
    let covered_commands: BTreeSet<&str> = by_atom.values().flatten().filter_map(|element| element.strip_prefix("command:")).collect();
    let untraced: Vec<&String> = relevant_commands(def, prior).into_iter().filter(|name| !covered_commands.contains(name.as_str())).collect();
    let by_stated_origin: Vec<Value> = implicit.iter().map(|(atom, element)| json!({"atom": atom, "element": element})).collect();
    json!({
        "atomized": true,
        "atoms": found.atoms.len(),
        "traced": by_atom.len(),
        "not_applicable": dismissed,
        "unmapped": unmapped,
        "uncovered_sentences": uncovered_sentences(found),
        "untraced_commands": untraced,
        "commands_checked": if prior.is_some() { "new_or_changed" } else { "all" },
        "traced_by_stated_origin": by_stated_origin,
    })
}

/// Sentences of the requirement text with a word no atom covers. Every alphanumeric character
/// of a sentence must lie inside some atom, so an awkward statement cannot be left out by
/// atomizing only a fragment of its sentence. Spans are unique in the text (checked earlier).
fn uncovered_sentences(found: &Requirement) -> Vec<String> {
    let ranges: Vec<(usize, usize)> = found
        .atoms
        .iter()
        .filter_map(|atom| found.text.find(&atom.span).map(|start| (start, start + atom.span.len())))
        .collect();
    sentences(&found.text)
        .into_iter()
        .filter(|(start, end, _)| {
            found.text[*start..*end]
                .char_indices()
                .any(|(offset, ch)| ch.is_alphanumeric() && !ranges.iter().any(|(a, b)| *a <= start + offset && start + offset < *b))
        })
        .map(|(_, _, text)| text)
        .collect()
}

/// Sentence ranges split at `.`, `;`, `!`, `?` and newlines that end a statement.
fn sentences(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        let boundary = matches!(ch, '.' | ';' | '!' | '?' | '\n') && chars.peek().is_none_or(|(_, next)| next.is_whitespace());
        let last = chars.peek().is_none();
        if boundary || last {
            let end = at + ch.len_utf8();
            let piece = text[start..end].trim();
            if piece.chars().any(char::is_alphanumeric) {
                out.push((start, end, piece.to_string()));
            }
            start = end;
        }
    }
    out
}
