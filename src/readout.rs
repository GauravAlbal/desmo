//! `ik readout`: a mechanical lint, and a stamp, for the prose an agent writes around an
//! `ik handoff`.
//!
//! The lint checks wording and quoting only. It cannot tell whether a summary is true. Each
//! defect names its vector, the line, what is wrong and the exact fix, so an agent can repair
//! it without guessing. `--stamp` writes a hash line only when the lint is clean; the line
//! binds the summary text and the handoff text it was linted against. A hash binds content,
//! not who wrote it.

use crate::canonical::sha256_hex;
use serde_json::{json, Value};
use std::fs;

const USAGE: &str = "usage: ik readout <summary.md> --handoff <handoff.txt> [--stamp] [--json]\n  <handoff.txt> is the text output of `ik handoff` (without --json). Exit 0 clean, 1 defects, 2 usage.";
const STAMP_PREFIX: &str = "<!-- ikaros-readout:";
const NOTE: &str = "mechanical lint: it checks wording and quoting, not whether the summary is true";
const STAMP_NOTE: &str = "binds content, not author; says the lint was clean, not that the summary is accurate";
const MATERIAL: [&str; 5] = ["unguarded_argument", "fact_never_set", "dead_end", "unreachable_state", "no_invariants"];

struct Vector {
    id: &'static str,
    wrong: &'static str,
    fix: &'static str,
    terms: &'static [&'static str],
}

static READINESS: Vector = Vector {
    id: "readiness_word",
    wrong: "Desmo states no readiness; readiness is the owner's decision. This word asserts a verdict that nothing in the handoff establishes.",
    fix: "Say what the handoff shows, in its own words, as a `>` quote. If you must refer to readiness, write: the owner decides readiness.",
    terms: &[
        "ready", "complete", "completed", "fully", "verified", "safe", "safely", "correct", "correctly", "approved", "sign-off",
        "signed off", "good to go", "ship it",
    ],
};

static COMPLETENESS: Vector = Vector {
    id: "completeness_claim",
    wrong: "'none found' is a count over a stated scope, not a statement that nothing exists. Desmo never claims completeness, so a summary that does makes the work look more finished than the handoff says.",
    fix: "Quote the handoff's `What is missing` lines with `>`, including its scope sentence, and name what you did about each listed gap.",
    terms: &[
        "nothing missing", "nothing is missing", "no gaps", "no gap", "no missing", "none missing", "no issues", "no problems",
        "no defects", "all cases", "every case", "all requirements", "covers everything", "everything is covered", "exhaustive",
        "exhaustively",
    ],
};

static PROOF: Vector = Vector {
    id: "proof_word",
    wrong: "Evidence in a handoff is observed or explored within a stated scope, and declared is never demonstrated. This word claims more than any handoff line does.",
    fix: "Use the handoff's own status words (demonstrated, explored only, declared, not demonstrated) as a `>` quote, or describe what was run and cite its evidence id.",
    terms: &["prove", "proves", "proved", "proven", "proof", "ensure", "ensured", "guarantee", "guarantees", "guaranteed", "certain", "certainly", "ensures"],
};

static QUOTE: Vector = Vector {
    id: "quote_not_in_handoff",
    wrong: "A `>` block must be verbatim handoff text so a reader can trust it as Desmo output. This block is not found in the handoff you gave: it was reworded, edited, or comes from another handoff.",
    fix: "Copy the lines exactly from the handoff. Separate non-adjacent lines with a `>` line holding only `…`. Or remove the `>` and write it as your own words.",
    terms: &[],
};

static STATUS: Vector = Vector {
    id: "status_not_quoted",
    wrong: "The handoff's Status line is the one line that says whether a human decision is pending. The summary does not quote it, so a reader may think nothing is pending.",
    fix: "Add the handoff's Status line verbatim as a `>` quote near the top of the summary.",
    terms: &[],
};

static SCOPE: Vector = Vector {
    id: "not_checked_not_quoted",
    wrong: "The handoff's `Not checked:` paragraph is what stops a reader treating the summary as assurance. The summary does not quote it.",
    fix: "Quote the whole `Not checked:` paragraph verbatim with `>`.",
    terms: &[],
};

static CLAIM: Vector = Vector {
    id: "claim_not_quoted",
    wrong: "The handoff carries one sentence Desmo stands behind (its `Claim:` line). The summary does not quote it, so a reader sees your wording of the result and not the sentence the product itself sanctions.",
    fix: "Quote the handoff's whole `Claim:` paragraph verbatim with `>`. You may add your own words after it; they must not say more than it does.",
    terms: &[],
};

static GAP: Vector = Vector {
    id: "gap_not_mentioned",
    wrong: "The handoff lists this as a material gap and the summary never mentions it. Leaving out a listed gap makes the work look cleaner than the handoff says.",
    fix: "Name it, or quote its table row with `>`, and say what you did about it: fixed, accepted by the owner, or left for the owner to decide.",
    terms: &[],
};

static STALE: Vector = Vector {
    id: "stamp_stale",
    wrong: "The stamp no longer matches. The summary or the handoff changed after it was stamped, so the stamp says nothing about what is here.",
    fix: "Run `ik readout <summary.md> --handoff <handoff.txt> --stamp` again. It stamps only if the lint is clean.",
    terms: &[],
};

static MALFORMED: Vector = Vector {
    id: "stamp_malformed",
    wrong: "A stamp line is present but does not have the form written by `ik readout --stamp`, or there is more than one. A hand-written stamp proves nothing.",
    fix: "Delete the stamp line and run `ik readout <summary.md> --handoff <handoff.txt> --stamp`.",
    terms: &[],
};

static PROSE_VECTORS: [&Vector; 3] = [&READINESS, &COMPLETENESS, &PROOF];

struct Defect {
    vector: &'static Vector,
    line: usize,
    found: String,
}

struct Summary<'a> {
    prose: Vec<(usize, &'a str)>,
    quotes: Vec<(usize, String)>,
    stamps: Vec<&'a str>,
}

struct Handoff {
    claim: Option<String>,
    status: Option<String>,
    not_checked: Option<String>,
    gaps: Vec<(String, String)>,
}

fn norm(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has_term(lower: &str, term: &str) -> bool {
    lower.match_indices(term).any(|(at, _)| {
        let before = lower[..at].chars().next_back();
        let after = lower[at + term.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

fn is_ellipsis(text: &str) -> bool {
    matches!(text.trim(), "…" | "[…]" | "..." | "[...]")
}

fn parse_summary(raw: &str) -> Summary<'_> {
    let mut out = Summary { prose: Vec::new(), quotes: Vec::new(), stamps: Vec::new() };
    let mut segment: Vec<&str> = Vec::new();
    let mut start = 0;
    let flush = |segment: &mut Vec<&str>, start: usize, quotes: &mut Vec<(usize, String)>| {
        if !segment.is_empty() {
            quotes.push((start, norm(&segment.join(" "))));
            segment.clear();
        }
    };
    for (index, line) in raw.lines().enumerate() {
        let number = index + 1;
        if let Some(rest) = line.trim_start().strip_prefix('>') {
            if is_ellipsis(rest) {
                flush(&mut segment, start, &mut out.quotes);
            } else {
                if segment.is_empty() {
                    start = number;
                }
                segment.push(rest);
            }
            continue;
        }
        flush(&mut segment, start, &mut out.quotes);
        if line.trim_start().starts_with(STAMP_PREFIX) {
            out.stamps.push(line.trim());
        } else {
            out.prose.push((number, line));
        }
    }
    flush(&mut segment, start, &mut out.quotes);
    out
}

fn parse_handoff(text: &str) -> Handoff {
    let lines: Vec<&str> = text.lines().collect();
    let status = lines.iter().find(|line| line.starts_with("Status: ")).map(|line| norm(line));
    let not_checked = lines.iter().position(|line| line.starts_with("Not checked:")).map(|at| {
        let block: Vec<&str> = lines[at..].iter().take_while(|line| !line.starts_with("Readiness:")).copied().collect();
        norm(&block.join(" "))
    });
    let claim = lines.iter().position(|line| line.starts_with("Claim: ")).map(|at| {
        let block: Vec<&str> = lines[at..].iter().take_while(|line| !line.trim().is_empty()).copied().collect();
        norm(&block.join(" "))
    });
    Handoff { claim, status, not_checked, gaps: material_gaps(&lines) }
}

/// Rows of the `What is missing` table whose kind is material, as (kind, subject).
fn material_gaps(lines: &[&str]) -> Vec<(String, String)> {
    let Some(at) = lines.iter().position(|line| line.trim() == "What is missing") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen_table = false;
    for line in &lines[at + 1..] {
        if !line.starts_with('|') {
            if seen_table {
                break;
            }
            continue;
        }
        seen_table = true;
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if let (Some(kind), Some(subject)) = (cells.get(1), cells.get(2)) {
            if MATERIAL.contains(kind) {
                out.push((kind.to_string(), subject.to_string()));
            }
        }
    }
    out
}

fn excerpt(line: &str) -> String {
    line.trim().chars().take(110).collect()
}

fn prose_defects(summary: &Summary<'_>) -> Vec<Defect> {
    let mut out = Vec::new();
    for (number, line) in &summary.prose {
        let lower = line.to_lowercase();
        for vector in PROSE_VECTORS {
            if let Some(term) = vector.terms.iter().find(|term| has_term(&lower, term)) {
                out.push(Defect { vector, line: *number, found: format!("`{term}` in: {}", excerpt(line)) });
            }
        }
    }
    out
}

fn quote_defects(summary: &Summary<'_>, handoff_norm: &str) -> Vec<Defect> {
    summary
        .quotes
        .iter()
        .filter(|(_, text)| !handoff_norm.contains(text.as_str()))
        .map(|(line, text)| Defect { vector: &QUOTE, line: *line, found: excerpt(text) })
        .collect()
}

fn quoted(summary: &Summary<'_>, needle: &str) -> bool {
    summary.quotes.iter().any(|(_, text)| text.contains(needle))
}

fn required_defects(summary: &Summary<'_>, handoff: &Handoff) -> Vec<Defect> {
    let mut out = Vec::new();
    if let Some(status) = handoff.status.as_deref().filter(|status| !quoted(summary, status)) {
        out.push(Defect { vector: &STATUS, line: 0, found: format!("the handoff says: {status}") });
    }
    if let Some(text) = handoff.not_checked.as_deref().filter(|text| !quoted(summary, text)) {
        out.push(Defect { vector: &SCOPE, line: 0, found: excerpt(text) });
    }
    if let Some(text) = handoff.claim.as_deref().filter(|text| !quoted(summary, text)) {
        out.push(Defect { vector: &CLAIM, line: 0, found: excerpt(text) });
    }
    let mentioned = |needle: &str| {
        let needle = needle.to_lowercase();
        summary.prose.iter().any(|(_, line)| line.to_lowercase().contains(&needle))
            || summary.quotes.iter().any(|(_, text)| text.to_lowercase().contains(&needle))
    };
    for (kind, subject) in &handoff.gaps {
        if !mentioned(gap_needle(kind, subject)) {
            out.push(Defect { vector: &GAP, line: 0, found: format!("{kind} {subject}") });
        }
    }
    out
}

fn stamp_field<'a>(stamp: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    let value = stamp.split_whitespace().find_map(|word| word.strip_prefix(prefix.as_str()))?;
    (value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())).then_some(value)
}

fn body_hash(raw: &str) -> String {
    let body: Vec<&str> = raw.lines().filter(|line| !line.trim_start().starts_with(STAMP_PREFIX)).collect();
    sha256_hex(body.join("\n").trim_end().as_bytes())
}

fn stamp_defects(summary: &Summary<'_>, raw: &str, handoff_text: &str) -> Vec<Defect> {
    let found = |vector: &'static Vector, text: &str| vec![Defect { vector, line: 0, found: text.to_string() }];
    let [stamp] = summary.stamps.as_slice() else {
        return if summary.stamps.is_empty() { Vec::new() } else { found(&MALFORMED, "more than one stamp line") };
    };
    let (Some(body), Some(handoff)) = (stamp_field(stamp, "summary"), stamp_field(stamp, "handoff")) else {
        return found(&MALFORMED, &excerpt(stamp));
    };
    if !stamp.contains(" lint=clean ") {
        return found(&MALFORMED, &excerpt(stamp));
    }
    if body != body_hash(raw) {
        return found(&STALE, "the summary text changed after it was stamped");
    }
    if handoff != sha256_hex(handoff_text.as_bytes()) {
        return found(&STALE, "the handoff text is not the one that was stamped");
    }
    Vec::new()
}

fn lint(raw: &str, handoff_text: &str, check_stamp: bool) -> Vec<Defect> {
    let summary = parse_summary(raw);
    let handoff = parse_handoff(handoff_text);
    let handoff_norm = norm(handoff_text);
    let mut out = prose_defects(&summary);
    out.extend(quote_defects(&summary, &handoff_norm));
    out.extend(required_defects(&summary, &handoff));
    if check_stamp {
        out.extend(stamp_defects(&summary, raw, handoff_text));
    }
    out.sort_by_key(|defect| defect.line);
    out
}

fn render(defects: &[Defect], json: bool) -> String {
    if json {
        let rows: Vec<Value> = defects
            .iter()
            .map(|d| json!({"vector": d.vector.id, "line": d.line, "found": d.found, "wrong": d.vector.wrong, "fix": d.vector.fix}))
            .collect();
        return json!({"schema": "ik.readout.v1", "clean": defects.is_empty(), "defects": rows, "note": NOTE}).to_string();
    }
    let mut out = String::new();
    for d in defects {
        let place = if d.line == 0 { "summary".to_string() } else { format!("summary line {}", d.line) };
        out.push_str(&format!("DEFECT {} ({place})\n  found: {}\n  wrong: {}\n  fix:   {}\n\n", d.vector.id, d.found, d.vector.wrong, d.vector.fix));
    }
    if defects.is_empty() {
        out.push_str(&format!("clean: no mechanical defect found. {NOTE}.\n"));
    } else {
        out.push_str(&format!("{} defect(s). Fix each and run again; nothing is stamped until the lint is clean. {NOTE}.\n", defects.len()));
    }
    out
}

fn stamp_line(raw: &str, handoff_text: &str) -> String {
    format!(
        "{STAMP_PREFIX} lint=clean summary={} handoff={} ({STAMP_NOTE}) -->",
        body_hash(raw),
        sha256_hex(handoff_text.as_bytes())
    )
}

struct Args {
    summary: String,
    handoff: String,
    stamp: bool,
    json: bool,
}

fn parse_args(args: &[String]) -> Result<Option<Args>, String> {
    let (mut summary, mut handoff, mut stamp, mut json) = (None, None, false, false);
    let mut items = args.iter();
    while let Some(arg) = items.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--stamp" => stamp = true,
            "--json" => json = true,
            "--handoff" => handoff = Some(items.next().ok_or("--handoff requires a path")?.clone()),
            other if other.starts_with('-') => return Err(format!("unknown option `{other}`")),
            other if summary.is_none() => summary = Some(other.to_string()),
            other => return Err(format!("unexpected argument `{other}`")),
        }
    }
    Ok(Some(Args {
        summary: summary.ok_or("a summary file is required")?,
        handoff: handoff.ok_or("--handoff <handoff.txt> is required")?,
        stamp,
        json,
    }))
}

fn read(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("cannot read {path}: {error}"))
}

fn usage_error(message: &str) -> i32 {
    eprintln!("ik readout: {message}");
    eprintln!("{USAGE}");
    2
}

fn execute(args: &Args) -> Result<i32, String> {
    let raw = read(&args.summary)?;
    let handoff_text = read(&args.handoff)?;
    let handoff = parse_handoff(&handoff_text);
    if handoff.not_checked.is_none() {
        return Err("the --handoff file has no `Not checked:` paragraph; give the text output of `ik handoff`".into());
    }
    let defects = lint(&raw, &handoff_text, !args.stamp);
    print!("{}", render(&defects, args.json));
    if !defects.is_empty() {
        return Ok(1);
    }
    if args.stamp {
        let body: Vec<&str> = raw.lines().filter(|line| !line.trim_start().starts_with(STAMP_PREFIX)).collect();
        let stamped = format!("{}\n\n{}\n", body.join("\n").trim_end(), stamp_line(&raw, &handoff_text));
        fs::write(&args.summary, stamped).map_err(|error| format!("cannot write {}: {error}", args.summary))?;
        if !args.json {
            println!("stamped {}", args.summary);
        }
    }
    Ok(0)
}

pub fn run(args: &[String]) -> i32 {
    match parse_args(args) {
        Ok(None) => {
            println!("{USAGE}");
            0
        }
        Ok(Some(parsed)) => execute(&parsed).unwrap_or_else(|message| usage_error(&message)),
        Err(message) => usage_error(&message),
    }
}

/// What the summary must contain to count as naming a gap. A machine-level gap has no
/// subject to quote, so it is matched by its plain word (`invariant`), not the kind id.
fn gap_needle<'a>(kind: &'a str, subject: &'a str) -> &'a str {
    match (subject, kind) {
        ("machine", "no_invariants") => "invariant",
        ("machine", _) => kind,
        _ => subject,
    }
}
