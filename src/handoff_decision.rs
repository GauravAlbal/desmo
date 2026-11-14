//! Decisions a handoff asks of the human, computed from the diagram's rule ledger and
//! change summary. The kernel can show that a rule is declared and checked before a
//! move; it cannot show that the rule matches the business intent. That is an
//! acceptance the human gives, bound to the rule's hash so a changed rule asks again.

use crate::census::CensusRow;
use crate::handoff_witness::Checked;
use serde_json::{json, Value};

/// A rule the human accepted, bound to the exact guard text and examples by hash.
pub(crate) struct Acceptance {
    pub rule: String,
    pub guard_hash: String,
    pub examples_hash: String,
}

/// Every decision the handoff must put in front of the human.
pub(crate) fn required(
    kind: &str,
    diagram: &Value,
    accepted: &[Acceptance],
    checked: &[Checked],
    census: Option<&[CensusRow]>,
) -> Vec<Value> {
    let mut out = Vec::new();
    if kind != "recovery" {
        out.extend(rule_decision(kind, &diagram["rules"], accepted, checked));
    }
    if kind == "change" {
        out.extend(version_decision(&diagram["change"], census));
    }
    out
}

fn rule_decision(
    kind: &str,
    rules: &Value,
    accepted: &[Acceptance],
    checked: &[Checked],
) -> Option<Value> {
    let pending: Vec<Value> = rules
        .as_array()?
        .iter()
        .filter(|rule| rule["status"].as_str() != Some("unchanged"))
        .filter_map(|rule| pending_row(rule, accepted, checked))
        .collect();
    if pending.is_empty() {
        return None;
    }
    let scope = if kind == "change" {
        "each rule that is new or changed in this version"
    } else {
        "each rule below"
    };
    Some(json!({
        "kind": "rule_acceptance",
        "question": format!("Accept or reject {scope} as the business rule."),
        "basis": [
            "Enforced: the definition declares each rule and the kernel checks it before the move.",
            "Distinguished: the examples show how the rule treats those inputs and differ from the one-step variants listed. This is not a proof the rule is right.",
            "Accepted: only the person who owns the intent can say the behavior is what the business wants. Nothing here establishes that.",
        ],
        "rules": pending,
    }))
}

fn pending_row(rule: &Value, accepted: &[Acceptance], checked: &[Checked]) -> Option<Value> {
    let name = rule["name"].as_str()?;
    let hash = rule["hash"].as_str()?;
    let found = checked.iter().find(|item| item.rule == name);
    let examples_hash = found.map(|item| item.examples_hash.as_str()).unwrap_or("");
    let named: Vec<&Acceptance> = accepted.iter().filter(|item| item.rule == name).collect();
    if named
        .iter()
        .any(|item| item.guard_hash == hash && item.examples_hash == examples_hash)
    {
        return None;
    }
    let state = if named.is_empty() {
        "needs your acceptance"
    } else {
        "acceptance is stale: the rule or its examples changed after it was accepted"
    };
    let mut row = rule.clone();
    row["acceptance"] = json!(state);
    row["examples_hash"] = json!(examples_hash);
    row["examples"] = found.map(Checked::to_value).unwrap_or(Value::Null);
    Some(row)
}

fn version_decision(change: &Value, census: Option<&[CensusRow]>) -> Option<Value> {
    let old = change["prior_version"].as_u64().filter(|version| *version > 0)?;
    let new = change["new_version"].as_u64()?;
    let (found, missing) = census_lines(change, old, census);
    let compat = &change["compat"];
    let mut not_established = missing;
    not_established.extend(strings_of(&compat["not_established"]));
    Some(json!({
        "kind": "version_policy",
        "question": format!(
            "Keep existing instances on version {old}, or let them finish and create new instances on version {new}?"
        ),
        "options": [
            {
                "label": format!("Keep existing instances on version {old} until they finish"),
                "consequence": keep_consequence(change, old),
            },
            {
                "label": format!("Drain, then create new version {new} instances"),
                "consequence": format!(
                    "version {old} instances finish under version {old}; instances created after that begin under version {new}. This does not move an unfinished instance to version {new}: stored state stays bound to the version it was created under. Recreating a business operation can repeat external work unless the domain contract allows it."
                ),
            },
        ],
        "compat_rows": compat_rows(compat),
        "compat_global": strings_of(&compat["global"]),
        "found": found,
        "not_established": not_established,
    }))
}

/// What the store says about version-`old` instances, or why it says nothing.
fn census_lines(change: &Value, old: u64, census: Option<&[CensusRow]>) -> (Vec<String>, Vec<String>) {
    let Some(rows) = census else {
        return (
            Vec::new(),
            vec![format!(
                "how many instances are on version {old} (no store was given with --store, so no census was taken)"
            )],
        );
    };
    let id = change["machine_id"].as_str().unwrap_or("");
    let mine: Vec<&CensusRow> = rows
        .iter()
        .filter(|row| row.machine_id == id && row.machine_version == old)
        .collect();
    let total = |finished: bool| -> u64 {
        mine.iter().filter(|row| row.terminal == finished).map(|row| row.count).sum()
    };
    let mut lines = vec![format!(
        "version {old} instances in the store: {} unfinished, {} finished",
        total(false),
        total(true)
    )];
    let by_state: Vec<String> = mine
        .iter()
        .filter(|row| !row.terminal)
        .map(|row| format!("{} {}", row.state.as_deref().unwrap_or("(not named)"), row.count))
        .collect();
    if !by_state.is_empty() {
        lines.push(format!("unfinished by state: {}", by_state.join(", ")));
    }
    lines.push(format!(
        "version {old} can be retired when its unfinished count is 0 (the host keeps both versions registered until then)"
    ));
    (lines, Vec::new())
}

fn compat_rows(compat: &Value) -> Vec<Vec<String>> {
    compat["states"]
        .as_array()
        .map(|states| states.iter().filter_map(compat_row).collect())
        .unwrap_or_default()
}

fn compat_row(state: &Value) -> Option<Vec<String>> {
    let exits = state["exits"].as_array()?;
    let gained = names(&state["gained"]);
    if exits.is_empty() && gained.is_empty() {
        return None;
    }
    let old: Vec<&str> = exits.iter().filter_map(|exit| exit["command"].as_str()).collect();
    let changed: Vec<String> = exits
        .iter()
        .filter(|exit| exit["verdict"].as_str() != Some("same"))
        .map(|exit| {
            format!(
                "{}: {}",
                exit["command"].as_str().unwrap_or(""),
                exit["verdict"].as_str().unwrap_or("")
            )
        })
        .collect();
    let mut now = if changed.is_empty() { "unchanged".to_string() } else { changed.join("; ") };
    if !gained.is_empty() {
        now.push_str(&format!("; adds {gained}"));
    }
    let reasons = strings_of(&state["reasons"]);
    Some(vec![
        state["state"].as_str().unwrap_or("").to_string(),
        old.join(", "),
        now,
        if reasons.is_empty() { "no difference found".to_string() } else { reasons.join("; ") },
    ])
}

fn strings_of(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn keep_consequence(change: &Value, old: u64) -> String {
    let mut text = format!("existing instances keep the version {old} rules");
    let added = names(&change["added_commands"]);
    if !added.is_empty() {
        text.push_str(&format!(" and cannot use: {added}"));
    }
    let removed = names(&change["removed_commands"]);
    if !removed.is_empty() {
        text.push_str(&format!("; the new version removes {removed}, which they keep"));
    }
    text.push_str(". The host must register both versions until no instance is left on the old one.");
    text
}

fn names(value: &Value) -> String {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}
