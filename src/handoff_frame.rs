//! The frame every handoff carries: what was checked, what was not, what is assumed, and what
//! duties have a recorded contribution. Desmo owns the state transitions. It does not own whether
//! the criteria are valid or whether the code and effects that perform a transition do what is
//! intended; like any formal checking tool it assumes that execution is perfect and reliable.
//! The duty rows name the party EXPECTED to perform each duty and say what contribution, if any,
//! the evidence records. They never judge a contribution and never claim one happened.

use serde_json::{json, Value};

const NOT_CHECKED: [&str; 4] = [
    "that the criteria are valid",
    "that the code and effects performing a transition do what is intended",
    "that the definition says what was needed",
    "anything the requirement does not state",
];
const ASSUMED: &str = "execution is perfect and reliable";
const READINESS: &str = "the owner's decision; this report does not state it";
const NOT_RECORDED: &str = "not recorded";

/// What this kind of handoff asks of the evidence, phrased as a question so the line cannot be
/// read as a result; the outcomes live in the Demonstrated, Explored and Existing blocks. A
/// recovery handoff reads the store; it does not check transitions, so it must not borrow the
/// delivery sentence.
fn checked(kind: &str) -> &'static str {
    match kind {
        "recovery" => "whether a recorded operation exists in the store under the given id and request",
        "change" => "whether the state transitions behave as the definition says, and, only where continuity evidence is cited, whether existing instances keep their version",
        _ => "whether the state transitions behave as the definition says, within the cited evidence",
    }
}

/// Add `scope` and `duties` to the handoff document. Call after the decisions are attached.
pub(crate) fn attach(document: &mut Value) {
    let kind = document["kind"].as_str().unwrap_or("delivery").to_string();
    let rows = duties(document);
    let claim_text = claim(document, &kind);
    let Some(object) = document.as_object_mut() else {
        return;
    };
    object.insert("claim".into(), Value::String(claim_text));
    object.insert(
        "scope".into(),
        json!({
            "checked": checked(&kind),
            "not_checked": NOT_CHECKED,
            "assumed": [ASSUMED],
            "readiness": READINESS,
        }),
    );
    object.insert("duties".into(), Value::Array(rows));
}

fn duty(name: &str, expected: &str, status: String) -> Value {
    json!({"duty": name, "expected_party": expected, "status": status})
}

fn requirement_status(document: &Value) -> String {
    let pinned = &document["origins"]["requirement"];
    if let (Some(source), Some(provenance)) = (pinned["source"].as_str(), pinned["provenance"].as_str()) {
        return format!("author-stated, not verified: {source} ({provenance})");
    }
    let source = document["why"]["source"].as_str().filter(|text| !text.is_empty());
    source.map_or_else(
        || NOT_RECORDED.to_string(),
        |text| format!("author-stated, not verified: {text}"),
    )
}

fn origin_status(document: &Value) -> Option<String> {
    let counts = &document["origins"]["counts"];
    let total = counts["total"].as_u64().filter(|count| *count > 0)?;
    let n = |key: &str| counts[key].as_u64().unwrap_or(0);
    Some(format!(
        "{total} decision points: {} stated, {} chosen and acknowledged, {} chosen not acknowledged, {} default, {} unrecorded",
        n("stated"),
        n("chosen_acknowledged"),
        n("chosen"),
        n("default"),
        n("unrecorded"),
    ))
}

/// The one-line summary of what the handoff lists as missing.
fn gap_status(document: &Value) -> Option<String> {
    let gaps = document.get("gaps").filter(|value| !value.is_null())?;
    if gaps["available"] != true {
        return Some(format!("not enumerated: {}", gaps["note"].as_str().unwrap_or("")));
    }
    let counts = &gaps["counts"];
    let n = |key: &str| counts[key].as_u64().unwrap_or(0);
    let requirement = if gaps["requirement"]["atomized"] == true {
        format!(
            "{} unmapped atoms, {} sentences not fully covered, {} untraced commands (atomization is the author's)",
            n("unmapped_atoms"),
            n("uncovered_sentences"),
            n("untraced_commands")
        )
    } else {
        "requirement not given as atoms".to_string()
    };
    Some(format!(
        "{} in the definition ({} about what the machine accepts or does), {} in the evidence; {requirement}",
        n("definition"),
        n("material"),
        n("evidence")
    ))
}

fn names_of(rows: &[Value], kind: &str) -> Vec<String> {
    rows.iter().filter(|row| row["kind"] == kind).filter_map(|row| row["subject"].as_str().map(str::to_string)).collect()
}

fn push_wrapped(out: &mut String, text: &str) {
    for line in crate::diagram::wrap_text(text, 100) {
        out.push_str(&line);
        out.push('\n');
    }
}

/// Gaps that are about documentation or dependence rather than what the machine accepts, one
/// line per kind so they do not crowd out the ones that matter.
fn other_lines(out: &mut String, rows: &[Value]) {
    for (kind, label) in [
        ("unguarded_command", "Commands with no guard (accepted whenever the instance is in a source state)"),
        ("fact_never_read", "Facts stored but read by no guard or invariant (host code may read them)"),
    ] {
        let names = names_of(rows, kind);
        if !names.is_empty() {
            push_wrapped(out, &format!("{label}: {}.", names.join(", ")));
        }
    }
}

fn evidence_lines(out: &mut String, rows: &[Value]) {
    if let Some(row) = rows.iter().find(|row| row["kind"] == "no_run_evidence") {
        push_wrapped(out, &format!("Evidence: {}.", row["detail"].as_str().unwrap_or("")));
        return;
    }
    for (kind, label) in [
        ("no_committed_run_cited", "No committed run against this definition is cited for"),
        ("no_guard_refusal_cited", "No run refused with IK_INVARIANT_FAILED (a guard refusal) is cited for"),
    ] {
        let names = names_of(rows, kind);
        if !names.is_empty() {
            push_wrapped(out, &format!("{label}: {}.", names.join(", ")));
        }
    }
}

fn quoted(items: &Value, with_reason: bool) -> Vec<String> {
    items
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|item| {
                    let base = format!("{} \"{}\"", item["id"].as_str().unwrap_or(""), item["span"].as_str().unwrap_or(""));
                    if with_reason {
                        format!("{base} because {}", item["reason"].as_str().unwrap_or(""))
                    } else {
                        base
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn plain(items: &Value) -> Vec<String> {
    items.as_array().map(|rows| rows.iter().filter_map(|item| item.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn list_line(out: &mut String, label: &str, items: &[String]) {
    let text = if items.is_empty() { format!("{label}: none found.") } else { format!("{label}: {}.", items.join("; ")) };
    push_wrapped(out, &text);
}

fn requirement_lines(out: &mut String, requirement: &Value) {
    if requirement["atomized"] != true {
        push_wrapped(out, &format!("Requirement: {}.", requirement["note"].as_str().unwrap_or("")));
        return;
    }
    list_line(out, "Requirement statements traced to no definition element", &quoted(&requirement["unmapped"], false));
    list_line(out, "Requirement sentences with a word no atom covers", &plain(&requirement["uncovered_sentences"]));
    let scope = if requirement["commands_checked"] == "new_or_changed" { "new or changed commands only" } else { "all commands" };
    list_line(out, &format!("Commands no requirement statement is traced to ({scope})"), &plain(&requirement["untraced_commands"]));
    let dismissed = quoted(&requirement["not_applicable"], true);
    if !dismissed.is_empty() {
        push_wrapped(out, &format!("Atoms the author marked not applicable: {}.", dismissed.join("; ")));
    }
    let implicit: Vec<String> = requirement["traced_by_stated_origin"]
        .as_array()
        .map(|rows| rows.iter().map(|row| format!("{} -> {}", row["atom"].as_str().unwrap_or(""), row["element"].as_str().unwrap_or(""))).collect())
        .unwrap_or_default();
    if !implicit.is_empty() {
        push_wrapped(out, &format!("Traced by a stated origin, not by a trace entry: {}.", implicit.join(", ")));
    }
}

/// What is missing: gaps in the definition, in the evidence and against the requirement, each over
/// a stated scope. The scope sentence is always printed, so a short or empty list cannot read as
/// "nothing is missing". A recovery handoff has no definition to read.
fn gaps_block(document: &Value) -> String {
    let Some(gaps) = document.get("gaps").filter(|value| !value.is_null()) else {
        return String::new();
    };
    let mut out = String::from("What is missing\n\n");
    if gaps["available"] == true {
        let rows: Vec<Value> = gaps["definition"].as_array().cloned().unwrap_or_default();
        let (material, other): (Vec<Value>, Vec<Value>) = rows.into_iter().partition(|row| row["material"] == true);
        if material.is_empty() {
            out.push_str("No definition gap about what the machine accepts or does was found in the enumerated scope.\n");
        } else {
            let table: Vec<Vec<String>> = material
                .iter()
                .map(|row| {
                    let kind = row["kind"].as_str().unwrap_or("");
                    let marked = if row["new"] == true { format!("{kind} (new)") } else { kind.to_string() };
                    vec![marked, row["subject"].as_str().unwrap_or("").to_string(), row["detail"].as_str().unwrap_or("").to_string()]
                })
                .collect();
            out.push_str(&crate::diagram::md_table(&["definition gap", "subject", "detail"], &table));
        }
        other_lines(&mut out, &other);
        evidence_lines(&mut out, &gaps["evidence"].as_array().cloned().unwrap_or_default());
        requirement_lines(&mut out, &gaps["requirement"]);
    } else {
        push_wrapped(&mut out, &format!("Not enumerated: {}.", gaps["note"].as_str().unwrap_or("")));
    }
    push_wrapped(&mut out, &format!("{}.", gaps["scope"].as_str().unwrap_or("")));
    out.push('\n');
    out
}

fn duties(document: &Value) -> Vec<Value> {
    let mut rows = vec![duty("Requirement source", "Owner", requirement_status(document))];
    if let Some(status) = rule_status(document) {
        rows.push(duty("Rule acceptance", "Owner", status));
    }
    if let Some(status) = origin_status(document) {
        rows.push(duty("Criteria origin", "Author records, Owner acknowledges", status));
    }
    if let Some(status) = gap_status(document) {
        rows.push(duty("Known gaps", "Author lists, Owner decides", status));
    }
    rows.push(duty("Independent review", "Reviewer", NOT_RECORDED.to_string()));
    rows.push(duty(
        "Approver identity",
        "Platform",
        format!("{NOT_RECORDED}: an acceptance binds content, not who accepted"),
    ));
    rows.push(duty("Parties in roles", "", NOT_RECORDED.to_string()));
    rows
}

/// (pending acceptances, rules shown) for the rules this handoff asks about; None when none.
fn rule_counts(document: &Value) -> Option<(usize, usize)> {
    let total = document["diagram"]["rules"]
        .as_array()?
        .iter()
        .filter(|rule| rule["status"].as_str() != Some("unchanged"))
        .count();
    if total == 0 {
        return None;
    }
    let pending: usize = document["decisions"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|item| item["kind"] == "rule_acceptance")
                .filter_map(|item| item["rules"].as_array())
                .map(Vec::len)
                .sum()
        })
        .unwrap_or(0);
    Some((pending, total))
}

fn rule_status(document: &Value) -> Option<String> {
    let (pending, total) = rule_counts(document)?;
    Some(if pending == 0 {
        format!("{total} of {total} accepted for the displayed content; a rule acceptance, not a release approval")
    } else {
        format!("{pending} of {total} awaiting acceptance")
    })
}

/// The one sentence Desmo stands behind: what was checked, what is open, and what is not
/// claimed. It is derived from the same facts as the rest of the frame, so it cannot say more
/// than the handoff does. An author quotes it; `ik readout` checks that they did.
fn claim(document: &Value, kind: &str) -> String {
    let pending = match rule_counts(document) {
        Some((pending, total)) if pending > 0 => format!("{pending} of {total} rule acceptances are awaited; "),
        _ => String::new(),
    };
    let gaps = match document.get("gaps").filter(|value| !value.is_null()) {
        Some(gaps) if gaps["available"] == true => {
            format!("{} material definition gaps are listed over a stated scope; ", gaps["counts"]["material"].as_u64().unwrap_or(0))
        }
        Some(_) => "definition gaps were not enumerated; ".to_string(),
        None => String::new(),
    };
    format!(
        "Desmo checked {}; {pending}{gaps}it does not claim that the criteria are valid, that execution did what was intended, or readiness, which is the owner's decision",
        checked(kind)
    )
}

/// The human-facing block: the scope statement, then the duty table.
pub(crate) fn text(document: &Value) -> String {
    let checked_line = document["scope"]["checked"].as_str().unwrap_or("");
    let mut out = String::from("Scope\n");
    out.push_str(&format!("Checked: {checked_line}.\n"));
    out.push_str("Not checked: that the criteria are valid; that the code and effects performing a\n");
    out.push_str("transition do what is intended (execution is assumed perfect and reliable); that the\n");
    out.push_str("definition says what was needed; anything the requirement does not state.\n");
    out.push_str(&format!("Readiness: {READINESS}.\n"));
    push_wrapped(&mut out, &format!("Claim: {}.", document["claim"].as_str().unwrap_or("")));
    out.push('\n');
    let rows: Vec<Vec<String>> = document["duties"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|row| {
                    ["duty", "expected_party", "status"]
                        .iter()
                        .map(|key| row[*key].as_str().unwrap_or("").to_string())
                        .collect()
                })
                .collect()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        return out;
    }
    out.push_str("Who attested what\n\n");
    out.push_str(&crate::diagram::md_table(&["duty", "expected party", "recorded"], &rows));
    out.push('\n');
    out.push_str(&origin_block(document));
    out.push_str(&gaps_block(document));
    out
}

/// Where each criterion came from. An acknowledgment never turns a chosen point into a stated one.
/// The scope sentence is always printed, including when no point was enumerated, so an empty
/// table cannot read as "nothing was left out". A recovery handoff has no rules to enumerate.
fn origin_block(document: &Value) -> String {
    if document["kind"].as_str() == Some("recovery") {
        return String::new();
    }
    let mut rows: Vec<Vec<String>> = Vec::new();
    for rule in document["origins"]["rules"].as_array().cloned().unwrap_or_default() {
        let name = rule["rule"].as_str().unwrap_or("");
        for point in rule["points"].as_array().cloned().unwrap_or_default() {
            let label = format!("{} {}", point["kind"].as_str().unwrap_or(""), point["label"].as_str().unwrap_or(""));
            let origin = point["origin"].as_str().unwrap_or("").to_string();
            let detail = point["detail"].as_str().unwrap_or("").to_string();
            rows.push(vec![name.to_string(), label, origin, detail]);
        }
    }
    let mut out = String::from("Where each criterion came from\n\n");
    if rows.is_empty() {
        out.push_str("No decision points were enumerated.\n");
    } else {
        out.push_str(&crate::diagram::md_table(&["rule", "decision point", "origin", "detail"], &rows));
    }
    let scope = document["origins"]["scope"].as_str().unwrap_or("");
    for line in crate::diagram::wrap_text(&format!("{scope}."), 100) {
        out.push_str(&line);
        out.push('\n');
    }
    out.push('\n');
    out
}
