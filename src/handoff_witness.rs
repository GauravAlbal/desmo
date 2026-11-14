//! Rule witnesses as the handoff reads them: the examples an agent authored for each
//! rule that is about to be put in front of a human. A missing or inadequate witness
//! set is an agent gap, so the handoff refuses before a human sees anything.

use crate::canonical::digest_json;
use crate::declarative::witness::{check, Example};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// Examples and declared-equivalent variants for one rule.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuleWitnesses {
    pub rule: String,
    pub examples: Vec<WitnessExample>,
    #[serde(default)]
    pub equivalent: Vec<EquivalentVariant>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WitnessExample {
    #[serde(default)]
    pub args: Map<String, Value>,
    #[serde(default)]
    pub facts: Map<String, Value>,
    #[serde(default)]
    pub state: String,
    /// `accepted` or `rejected`.
    pub outcome: String,
}

/// A variant the author states is the same rule, with the reason.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EquivalentVariant {
    pub variant: String,
    pub reason: String,
}

/// A rule whose witnesses passed.
pub(crate) struct Checked {
    pub rule: String,
    pub examples_hash: String,
    pub accepted: Vec<String>,
    pub rejected: Vec<String>,
    pub variants_checked: usize,
    pub equivalent: Vec<String>,
}

impl Checked {
    pub(crate) fn to_value(&self) -> Value {
        json!({
            "accepted": self.accepted,
            "rejected": self.rejected,
            "variants_checked": self.variants_checked,
            "declared_equivalent": self.equivalent,
        })
    }
}

/// Check the witnesses of every rule that needs them. All problems are reported together.
pub(crate) fn check_rules(rules: &Value, witnesses: &[RuleWitnesses]) -> Result<Vec<Checked>, String> {
    let rows = rules.as_array().cloned().unwrap_or_default();
    let mut problems = unknown_rules(&rows, witnesses);
    let mut checked = Vec::new();
    for row in rows.iter().filter(|row| needs_witnesses(row)) {
        let name = row["name"].as_str().unwrap_or("");
        let Some(set) = witnesses.iter().find(|item| item.rule == name) else {
            problems.push(format!(
                "rule `{name}` needs examples (rule_witnesses) before a human is asked to accept it"
            ));
            continue;
        };
        match check_one(row, set) {
            Ok(item) => checked.push(item),
            Err(mut found) => problems.append(&mut found),
        }
    }
    if problems.is_empty() {
        Ok(checked)
    } else {
        Err(problems.join("\n"))
    }
}

/// A rule is witnessed when it has exactly one guard and is not unchanged or removed.
fn needs_witnesses(row: &Value) -> bool {
    let status = row["status"].as_str().unwrap_or("");
    status != "unchanged" && status != "removed" && guard_of(row).is_some()
}

fn guard_of(row: &Value) -> Option<&str> {
    match row["guards"].as_array()?.as_slice() {
        [only] => only.as_str(),
        _ => None,
    }
}

fn unknown_rules(rows: &[Value], witnesses: &[RuleWitnesses]) -> Vec<String> {
    witnesses
        .iter()
        .filter(|item| !rows.iter().any(|row| row["name"].as_str() == Some(item.rule.as_str())))
        .map(|item| format!("examples for rule `{}`, which is not in the rule ledger", item.rule))
        .collect()
}

fn check_one(row: &Value, set: &RuleWitnesses) -> Result<Checked, Vec<String>> {
    let name = set.rule.as_str();
    let guard = guard_of(row).unwrap_or("");
    let examples = convert(set).map_err(|message| vec![format!("rule `{name}`: {message}")])?;
    let equivalent: Vec<String> = set.equivalent.iter().map(|item| item.variant.clone()).collect();
    let verdict = check(guard, &examples, &equivalent)
        .map_err(|message| vec![format!("rule `{name}`: guard does not parse: {message}")])?;
    if !verdict.adequate() {
        return Err(problems_of(name, &verdict));
    }
    Ok(Checked {
        rule: name.to_string(),
        examples_hash: digest_json(&json!({"examples": set.examples, "equivalent": set.equivalent})),
        accepted: describe(set, "accepted"),
        rejected: describe(set, "rejected"),
        variants_checked: verdict.variants_checked,
        equivalent: set
            .equivalent
            .iter()
            .map(|item| format!("{} (author: {})", item.variant, item.reason))
            .collect(),
    })
}

fn problems_of(name: &str, verdict: &crate::declarative::witness::Verdict) -> Vec<String> {
    let mut out = Vec::new();
    for item in &verdict.mismatches {
        out.push(format!("rule `{name}`: {item}"));
    }
    for item in &verdict.uncovered_clauses {
        out.push(format!("rule `{name}`: clause not toggled by the examples: {item}"));
    }
    for item in &verdict.surviving {
        out.push(format!(
            "rule `{name}`: the examples do not tell the rule from this variant (add an example that separates them, or declare it equivalent with a reason): {item}"
        ));
    }
    out
}

fn convert(set: &RuleWitnesses) -> Result<Vec<Example>, String> {
    if set.examples.is_empty() {
        return Err("no examples were given".into());
    }
    set.examples
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let accepts = match item.outcome.as_str() {
                "accepted" => true,
                "rejected" => false,
                other => {
                    return Err(format!(
                        "example {}: outcome must be `accepted` or `rejected`, not `{other}`",
                        index + 1
                    ))
                }
            };
            Ok(Example {
                args: to_map(&item.args),
                facts: to_map(&item.facts),
                state: item.state.clone(),
                accepts,
            })
        })
        .collect()
}

fn to_map(map: &Map<String, Value>) -> BTreeMap<String, Value> {
    map.iter().map(|(key, value)| (key.clone(), value.clone())).collect()
}

fn describe(set: &RuleWitnesses, outcome: &str) -> Vec<String> {
    set.examples
        .iter()
        .filter(|item| item.outcome == outcome)
        .map(describe_example)
        .collect()
}

const SHOWN: usize = 28;

fn describe_example(example: &WitnessExample) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (key, value) in &example.args {
        parts.push(format!("${key}={}", shorten(value)));
    }
    for (key, value) in &example.facts {
        parts.push(format!("{key}={}", shorten(value)));
    }
    if !example.state.is_empty() {
        parts.push(format!("state={}", example.state));
    }
    parts.join(", ")
}

fn shorten(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() <= SHOWN {
        return text;
    }
    match value.as_str() {
        Some(content) => {
            let head: String = content.chars().take(SHOWN - 10).collect();
            format!("\"{head}…\" ({} chars)", content.chars().count())
        }
        None => {
            let head: String = text.chars().take(SHOWN - 2).collect();
            format!("{head}…")
        }
    }
}
